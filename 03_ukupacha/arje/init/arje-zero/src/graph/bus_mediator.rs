//! Bus mediator: integración de `EnteGraph` con el bus interno.
//!
//! Responsabilidades:
//!   - Auth de Announce (verificar identidad reclamada contra SO_PEERCRED)
//!   - Registro de conexiones (`bus_connections` indexado por Ulid)
//!   - Forwarding de Invokes a proveedores
//!   - Tracking de invokes en vuelo (`pending_invokes` por seq)
//!   - Cleanup en cierre de conexión

use super::{EnteGraph, INHIBIT_TTL, SERVER_SEQ_FLAG};
use arje_bus::{BusMessage, BusPayload, BusRequest, BusResponse, EnteInfo, PeerCreds};
use arje_card::Capability;
use std::time::Instant;
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
        BusRequest::Announce { .. }
            | BusRequest::UpdateCapabilities { .. }
            | BusRequest::KillEnte { .. }
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
                if let Some(reasons) = self.inhibit_block_reason() {
                    warn!(?from_authenticated, ?reasons, "PowerOff denegado por inhibición");
                    let _ = reply.send(BusResponse::Error(format!("inhibited: {reasons}")));
                } else {
                    info!(?from_authenticated, interactive, peer_pid = peer.pid, "PowerOff via bus");
                    let _ = reply.send(BusResponse::Ok);
                }
            }
            BusRequest::Reboot { interactive } => {
                if let Some(reasons) = self.inhibit_block_reason() {
                    warn!(?from_authenticated, ?reasons, "Reboot denegado por inhibición");
                    let _ = reply.send(BusResponse::Error(format!("inhibited: {reasons}")));
                } else {
                    info!(?from_authenticated, interactive, "Reboot via bus");
                    let _ = reply.send(BusResponse::Ok);
                }
            }
            BusRequest::Suspend { interactive } => {
                if let Some(reasons) = self.inhibit_block_reason() {
                    warn!(?from_authenticated, ?reasons, "Suspend denegado por inhibición");
                    let _ = reply.send(BusResponse::Error(format!("inhibited: {reasons}")));
                } else {
                    info!(?from_authenticated, interactive, "Suspend via bus");
                    let _ = reply.send(BusResponse::Ok);
                }
            }
            BusRequest::Hibernate { interactive } => {
                if let Some(reasons) = self.inhibit_block_reason() {
                    warn!(?from_authenticated, ?reasons, "Hibernate denegado por inhibición");
                    let _ = reply.send(BusResponse::Error(format!("inhibited: {reasons}")));
                } else {
                    info!(?from_authenticated, interactive, "Hibernate via bus");
                    let _ = reply.send(BusResponse::Ok);
                }
            }
            BusRequest::Invoke { cap, blob } => {
                self.forward_invoke(from_authenticated, cap, blob, reply).await;
            }
            BusRequest::UpdateCapabilities { adds, removes } => {
                let id = from_authenticated.expect("auth-required guarantees Some");
                self.apply_capability_update(id, adds, removes);
                let _ = reply.send(BusResponse::Ok);
            }
            BusRequest::KillEnte { target, signal } => {
                let caller = from_authenticated.expect("auth-required guarantees Some");
                let resp = self.kill_ente(caller, target, signal);
                let _ = reply.send(resp);
            }
        }
    }

    /// Envía `signal` al PID del Ente target. La autorización ya fue verificada
    /// vía SO_PEERCRED (caller tiene identidad en el grafo); políticas más
    /// finas (caller==parent, capability::Kill explícita) se aplicarán cuando
    /// existan — por ahora cualquier Ente autenticado puede pedir kill.
    fn kill_ente(&mut self, caller: Ulid, target: Ulid, signal: i32) -> BusResponse {
        let Some(inc) = self.incarnated.get(&target) else {
            warn!(%caller, %target, "KillEnte: target no existe en el grafo");
            return BusResponse::Error(format!("ente {target} no existe"));
        };
        let Some(pid) = inc.pid else {
            warn!(%caller, %target, label = %inc.card.label, "KillEnte: target sin PID (Virtual o Wasm)");
            return BusResponse::Error(format!("ente {target} no es matable (sin PID)"));
        };
        let sig = match nix::sys::signal::Signal::try_from(signal) {
            Ok(s) => s,
            Err(_) => {
                warn!(%caller, %target, signal, "KillEnte: signal inválido");
                return BusResponse::Error(format!("signal {signal} inválido"));
            }
        };
        info!(%caller, %target, label = %inc.card.label, pid = pid.as_raw(), ?sig, "KillEnte");
        if let Err(e) = nix::sys::signal::kill(pid, sig) {
            warn!(?e, "KillEnte: kill(2) falló");
            return BusResponse::Error(format!("kill: {e}"));
        }
        BusResponse::Ok
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

    /// Invoke originado por el cerebro. Fire-and-forget: empuja el BusMessage
    /// al proveedor real sin registrar `pending_invokes` (no hay reply que
    /// retornar a un peer del bus — la decisión vivió en proceso). Si no hay
    /// proveedor invokable o el canal está saturado, se registra warn.
    pub async fn forward_brain_invoke(&mut self, cap: Capability, blob: Vec<u8>) {
        if let Some(reasons) = self.inhibit_block_reason() {
            warn!(?cap, ?reasons, "brain invoke descartado por inhibición");
            return;
        }
        let Some(provider_id) = self.pick_invokable_provider(&cap) else {
            warn!(?cap, "brain invoke: sin proveedor invokable, descartado");
            return;
        };
        let Some(outbound) = self.bus_connections.get(&provider_id).cloned() else {
            warn!(?cap, %provider_id, "brain invoke: proveedor sin conexión, descartado");
            return;
        };
        let seq = self.alloc_invoke_seq();
        debug!(?cap, %provider_id, seq, blob_len = blob.len(), "brain invoke forwardeado");
        let msg = BusMessage {
            from: None,
            seq,
            payload: BusPayload::Request(BusRequest::Invoke { cap, blob }),
        };
        if outbound.send(msg).await.is_err() {
            warn!(seq, "brain invoke: outbound del proveedor cerrado");
        }
    }

    /// Notificación dirigida a un Ente por Ulid. Empaqueta el mensaje como
    /// Invoke contra `BRAIN_NOTIFY_IFACE`. Es la única vía de comunicación
    /// directa (no por capacidad anónima) que ofrece el cerebro.
    pub async fn forward_brain_notify(&mut self, target_id: Ulid, message: String) {
        if let Some(reasons) = self.inhibit_block_reason() {
            warn!(%target_id, ?reasons, "brain notify descartado por inhibición");
            return;
        }
        let Some(outbound) = self.bus_connections.get(&target_id).cloned() else {
            warn!(%target_id, "brain notify: target sin conexión al bus, descartado");
            return;
        };
        let seq = self.alloc_invoke_seq();
        debug!(%target_id, seq, len = message.len(), "brain notify forwardeado");
        let msg = BusMessage {
            from: None,
            seq,
            payload: BusPayload::Request(BusRequest::Invoke {
                cap: Capability::Endpoint {
                    interface: arje_bus::BRAIN_NOTIFY_IFACE,
                    version: 1,
                },
                blob: message.into_bytes(),
            }),
        };
        if outbound.send(msg).await.is_err() {
            warn!(seq, %target_id, "brain notify: outbound cerrado");
        }
    }

    /// Spawn originado por el cerebro. A diferencia de `SpawnRequest`
    /// (genesis, restart, Spawn-capability) este pasa por el filtro de
    /// inhibición — si la regla del cerebro entró en distress, el grafo
    /// no acepta más expansión hasta que el TTL vence.
    pub async fn forward_brain_spawn(&mut self, card: arje_card::EntityCard) {
        if let Some(reasons) = self.inhibit_block_reason() {
            warn!(label = %card.label, ?reasons, "brain spawn descartado por inhibición");
            return;
        }
        let seed_id = self.seed.id;
        if let Err(e) = self.authorize_and_spawn(card, seed_id).await {
            warn!(?e, "brain spawn falló");
        }
    }

    /// Aplica una inhibición declarada por el cerebro. Si la razón ya existe,
    /// extiende su TTL — semántica idempotente y "re-affirmable".
    pub fn apply_brain_inhibit(&mut self, reason: String) {
        let expires = Instant::now() + INHIBIT_TTL;
        let was_new = self.inhibits.insert(reason.clone(), expires).is_none();
        if was_new {
            info!(%reason, ttl_secs = INHIBIT_TTL.as_secs(), "inhibición activada");
        } else {
            debug!(%reason, "inhibición re-afirmada");
        }
    }

    /// Si hay inhibiciones vivas, devuelve un string con las razones para
    /// los mensajes de error. También purga las expiradas como side effect.
    /// `None` significa "puede proceder".
    pub(in crate::graph) fn inhibit_block_reason(&mut self) -> Option<String> {
        let now = Instant::now();
        self.inhibits.retain(|_, exp| *exp > now);
        if self.inhibits.is_empty() {
            None
        } else {
            let reasons: Vec<&str> = self.inhibits.keys().map(|s| s.as_str()).collect();
            Some(reasons.join(", "))
        }
    }
}
