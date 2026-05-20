//! Bus mediator: integración de `EnteGraph` con el bus interno.
//!
//! Responsabilidades:
//!   - Auth de Announce (verificar identidad reclamada contra SO_PEERCRED)
//!   - Registro de conexiones (`bus_connections` indexado por Ulid)
//!   - Forwarding de Invokes a proveedores
//!   - Tracking de invokes en vuelo (`pending_invokes` por seq)
//!   - Cleanup en cierre de conexión

use super::{EnteGraph, SERVER_SEQ_FLAG};
use arje_bus::{BusMessage, BusPayload, BusRequest, BusResponse, EnteInfo, PeerCreds};
use arje_card::Capability;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use ulid::Ulid;

/// Operaciones que requieren identidad verificada en el bus.
///
/// - `Announce`: establece bus_connections para forwarding.
/// - `UpdateCapabilities`: muta dynamic_provides del Ente — sólo el dueño.
///
/// Invoke, ListEntes y power-mgmt se aceptan anonymous — políticas por
/// capacidad se aplican aguas abajo, no aquí.
fn requires_auth(req: &BusRequest) -> bool {
    matches!(
        req,
        BusRequest::Announce { .. } | BusRequest::UpdateCapabilities { .. }
    )
}

impl EnteGraph {
    pub async fn on_bus_request(
        &mut self,
        peer: PeerCreds,
        from: Option<Ulid>,
        request: BusRequest,
        outbound: mpsc::Sender<BusMessage>,
        reply: oneshot::Sender<BusResponse>,
    ) {
        // ---- Auth: kernel-injected SO_PEERCRED vs identidad reclamada ----
        let from_authenticated = match from {
            None => None,
            Some(claimed) => {
                let expected = self.incarnated.get(&claimed).and_then(|i| i.pid);
                match expected {
                    Some(p) if p.as_raw() == peer.pid => Some(claimed),
                    Some(p) => {
                        warn!(
                            claimed = %claimed, expected_pid = p.as_raw(),
                            actual_pid = peer.pid,
                            "identity mismatch — rechazando request"
                        );
                        let _ = reply.send(BusResponse::Error("identity mismatch".into()));
                        return;
                    }
                    None => {
                        warn!(?claimed, peer_pid = peer.pid, "Ente desconocido reclamando identidad");
                        let _ = reply.send(BusResponse::Error("unknown ente claimed".into()));
                        return;
                    }
                }
            }
        };
        if requires_auth(&request) && from_authenticated.is_none() {
            let _ = reply.send(BusResponse::Error("auth required for this request".into()));
            return;
        }

        // ---- Dispatch ----
        match request {
            BusRequest::Announce { capabilities } => {
                let id = from_authenticated.expect("auth-required guarantees Some");
                let label = self.incarnated.get(&id).map(|i| i.card.label.clone())
                    .unwrap_or_else(|| "anónimo".into());
                info!(%id, %label, ?capabilities, peer_pid = peer.pid, "Announce autenticado");
                self.bus_connections.insert(id, outbound);
                let _ = reply.send(BusResponse::Ok);
            }
            BusRequest::ListEntes => {
                let entes = self.incarnated.values()
                    .map(|i| EnteInfo {
                        id: i.card.id,
                        label: i.card.label.clone(),
                        provides: i.card.provides.iter().cloned().collect(),
                        pid: i.pid.map(|p| p.as_raw()),
                    })
                    .collect();
                let _ = reply.send(BusResponse::Entes(entes));
            }
            BusRequest::PowerOff { interactive } => {
                info!(?from_authenticated, interactive, peer_pid = peer.pid, "PowerOff via bus");
                let _ = reply.send(BusResponse::Ok);
            }
            BusRequest::Reboot { interactive } => {
                info!(?from_authenticated, interactive, "Reboot via bus");
                let _ = reply.send(BusResponse::Ok);
            }
            BusRequest::Suspend { interactive } => {
                info!(?from_authenticated, interactive, "Suspend via bus");
                let _ = reply.send(BusResponse::Ok);
            }
            BusRequest::Hibernate { interactive } => {
                info!(?from_authenticated, interactive, "Hibernate via bus");
                let _ = reply.send(BusResponse::Ok);
            }
            BusRequest::Invoke { cap, blob } => {
                self.forward_invoke(from_authenticated, cap, blob, reply).await;
            }
            BusRequest::UpdateCapabilities { adds, removes } => {
                let id = from_authenticated.expect("auth-required guarantees Some");
                self.apply_capability_update(id, adds, removes);
                let _ = reply.send(BusResponse::Ok);
            }
        }
    }

    /// Muta `dynamic_provides` del Ente y actualiza el índice global de
    /// providers. La Card original (immutable) no se toca.
    fn apply_capability_update(
        &mut self,
        ente_id: Ulid,
        adds: Vec<Capability>,
        removes: Vec<Capability>,
    ) {
        // Adiciones: dedupe contra Card.provides + dynamic_provides existentes.
        let mut added = Vec::new();
        let mut removed = Vec::new();
        if let Some(inc) = self.incarnated.get_mut(&ente_id) {
            for cap in adds {
                if inc.card.provides.contains(&cap) || inc.dynamic_provides.contains(&cap) {
                    continue; // ya provista, no-op
                }
                inc.dynamic_provides.insert(cap.clone());
                added.push(cap);
            }
            for cap in removes {
                if inc.dynamic_provides.remove(&cap) {
                    removed.push(cap);
                }
                // Caps de la Card original no se pueden quitar — silenciosamente
                // ignoradas. Una Card es contrato; sólo el dynamic es mutable.
            }
        }
        // Actualizar índice global. Hacemos esto fuera del scope `inc` para
        // evitar el doble-borrow de self.
        for cap in &added {
            self.register_dynamic_cap(ente_id, cap.clone());
        }
        for cap in &removed {
            self.unregister_dynamic_cap(ente_id, cap);
            // Revocar grants emitidos contra esta cap por este Ente.
            let revoked: Vec<u64> = self.grants.iter()
                .filter(|(_, g)| g.provider == ente_id && &g.cap == cap)
                .map(|(t, _)| *t)
                .collect();
            for t in revoked {
                self.grants.remove(&t);
            }
        }
        info!(
            %ente_id,
            added_count = added.len(),
            removed_count = removed.len(),
            "capabilities actualizadas en runtime"
        );
    }

    /// Enruta un Invoke al proveedor real de la capacidad. Aloca un seq
    /// server-side, registra el reply oneshot en `pending_invokes`, y empuja
    /// el request por la conexión del proveedor.
    async fn forward_invoke(
        &mut self,
        from: Option<Ulid>,
        cap: Capability,
        blob: Vec<u8>,
        reply: oneshot::Sender<BusResponse>,
    ) {
        let provider_id = match self.pick_invokable_provider(&cap) {
            Some(id) => id,
            None => {
                let _ = reply.send(BusResponse::Error(format!("sin proveedor invokable para {cap:?}")));
                return;
            }
        };
        let outbound = match self.bus_connections.get(&provider_id) {
            Some(o) => o.clone(),
            None => {
                let _ = reply.send(BusResponse::Error("proveedor no conectado al bus".into()));
                return;
            }
        };
        let seq = self.alloc_invoke_seq();
        self.pending_invokes.insert(seq, reply);
        debug!(?from, ?cap, ?provider_id, seq, blob_len = blob.len(), "forwardeando Invoke");

        let msg = BusMessage {
            from: None,
            seq,
            payload: BusPayload::Request(BusRequest::Invoke { cap, blob }),
        };
        if outbound.send(msg).await.is_err() {
            if let Some(orig) = self.pending_invokes.remove(&seq) {
                let _ = orig.send(BusResponse::Error("conn del proveedor cerrada".into()));
            }
        }
    }

    fn pick_invokable_provider(&self, cap: &Capability) -> Option<Ulid> {
        // Sólo proveedores con conexión al bus pueden recibir forwards.
        // El propio Ente #0 está en `providers` para varias caps pero no
        // debe recibir forwards — se filtra implícitamente porque la Semilla
        // no tiene conexión al bus.
        self.providers.get(cap)?
            .iter()
            .find(|id| self.bus_connections.contains_key(id))
            .copied()
    }

    pub(in crate::graph) fn alloc_invoke_seq(&mut self) -> u64 {
        self.next_invoke_seq = self.next_invoke_seq.wrapping_add(1);
        SERVER_SEQ_FLAG | self.next_invoke_seq
    }

    pub async fn on_bus_response(&mut self, seq: u64, response: BusResponse) {
        if let Some(orig) = self.pending_invokes.remove(&seq) {
            let _ = orig.send(response);
        } else {
            warn!(seq, "Response sin pending invoke");
        }
    }

    pub async fn on_bus_conn_closed(&mut self, ente_id: Option<Ulid>) {
        if let Some(id) = ente_id {
            self.bus_connections.remove(&id);
            // No revocamos providers — la capacidad sigue declarada en su
            // Card. Sólo perdimos el canal de invocación.
            debug!(%id, "bus connection cerrada");
        }
    }
}
