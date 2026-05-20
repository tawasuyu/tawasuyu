//! Encarnación, muerte y supervisión.
//!
//! Aquí vive el flujo: Card → autorizar → soma::incarnate / wasm → registro
//! en el grafo → SIGCHLD → on_death → Restart/OneShot/Delegate.

use super::{EnteGraph, Incarnated};
use crate::events::{ExitStatus, GraphEvent};
use arje_bus::{BusMessage, BusPayload, BusRequest};
use arje_card::{Capability, EntityCard, Payload, Supervision};
use tokio::sync::mpsc;
use tracing::{info, warn};
use ulid::Ulid;

impl EnteGraph {
    /// Encarna las dependencias declaradas en la Semilla. Único punto donde
    /// el Init "decide": después sólo reacciona.
    pub async fn instantiate_seed_dependencies(
        &mut self,
        _tx: &mpsc::Sender<GraphEvent>,
    ) -> anyhow::Result<()> {
        let cards = std::mem::take(&mut self.pending_genesis);
        if cards.is_empty() {
            info!(seed = %self.seed.label, "semilla sin genesis cards");
            return Ok(());
        }
        info!(seed = %self.seed.label, count = cards.len(), "instanciando genesis");
        let seed_id = self.seed.id;
        for card in cards {
            if let Err(e) = self.authorize_and_spawn(card, seed_id).await {
                warn!(?e, "genesis card falló");
            }
        }
        Ok(())
    }

    /// Spawn solicitado por un Ente con `Capability::Spawn`. Verifica auth,
    /// requires del grafo, y delega la encarnación al backend correspondiente
    /// (`arje_soma` para procesos, `arje_wasm` para Wasm).
    pub async fn authorize_and_spawn(
        &mut self,
        mut card: EntityCard,
        requester: Ulid,
    ) -> anyhow::Result<()> {
        if !self.holder_has(requester, &Capability::Spawn) {
            warn!(?requester, "spawn denied: lacks Capability::Spawn");
            return Ok(());
        }
        if let Err(e) = card.validate() {
            warn!(?e, label = %card.label, "card inválida, spawn rechazado");
            return Ok(());
        }
        // Falla rápida sobre `requires` — mejor que daemons en bucle.
        for req in &card.requires {
            if !self.providers.contains_key(req) {
                warn!(?req, label = %card.label, "requires no satisfecho");
                return Ok(());
            }
        }
        // Lineage por defecto = quien pidió el spawn.
        if card.lineage.is_none() {
            card.lineage = Some(requester);
        }

        let pid = match &card.payload {
            Payload::Virtual => None,
            Payload::Native { .. } | Payload::Legacy { .. } => {
                Some(arje_soma::incarnate(&card)?)
            }
            Payload::Wasm { module_sha256, entry } => {
                // Wasm: hilo dedicado, sin PID. Su muerte se observa por
                // estado del runtime, no por SIGCHLD.
                let bytes = arje_cas::resolve(module_sha256)
                    .map_err(|e| anyhow::anyhow!("CAS resolve para {}: {e}", card.label))?;
                arje_wasm::incarnate_wasm(&card, bytes, entry.clone())?;
                None
            }
        };

        if let Some(p) = pid {
            self.by_pid.insert(p.as_raw(), card.id);
        }
        self.register_provider(&card);
        if let Some(parent) = card.lineage {
            self.children.entry(parent).or_default().push(card.id);
        }
        info!(label = %card.label, ?pid, lineage = ?card.lineage, "Ente encarnado");
        self.incarnated.insert(card.id, Incarnated {
            card, pid,
            dynamic_provides: std::collections::BTreeSet::new(),
        });
        Ok(())
    }

    pub async fn on_death(
        &mut self,
        id: Ulid,
        status: ExitStatus,
        _tx: &mpsc::Sender<GraphEvent>,
    ) {
        let Some(inc) = self.incarnated.remove(&id) else { return };
        if let Some(p) = inc.pid {
            self.by_pid.remove(&p.as_raw());
        }
        self.unregister_provider(&inc.card);
        if let Some(parent) = inc.card.lineage {
            if let Some(siblings) = self.children.get_mut(&parent) {
                siblings.retain(|c| c != &id);
            }
        }
        info!(label = %inc.card.label, ?status, "Ente disuelto");

        match inc.card.supervision.clone() {
            Supervision::Restart { initial, max: _ } => {
                // Backoff exponencial: TODO real con timer del runtime.
                tokio::time::sleep(initial).await;
                let new_card = EntityCard { id: Ulid::new(), ..inc.card };
                if let Err(e) = self.authorize_and_spawn(new_card, self.seed.id).await {
                    warn!(?e, "restart falló");
                }
            }
            Supervision::OneShot => {}
            Supervision::Delegate => {
                self.notify_lineage_of_death(&inc, &status);
            }
        }
    }

    /// Fire-and-forget: si el parent tiene conexión al bus, le forwardeamos
    /// un Invoke con la muerte del hijo. Sin retry, sin backpressure.
    fn notify_lineage_of_death(&mut self, inc: &Incarnated, status: &ExitStatus) {
        let Some(parent) = inc.card.lineage else { return };
        info!(
            child = %inc.card.id, parent = %parent, label = %inc.card.label,
            ?status,
            "Supervision::Delegate — muerte notificada al lineage"
        );
        if let Some(out) = self.bus_connections.get(&parent).cloned() {
            let blob = format!("{}:{:?}", inc.card.id, status);
            let seq = self.alloc_invoke_seq();
            let msg = BusMessage {
                from: None,
                seq,
                payload: BusPayload::Request(BusRequest::Invoke {
                    cap: Capability::Endpoint {
                        interface: arje_card::InterfaceId([0xde; 16]),
                        version: 1,
                    },
                    blob: blob.into_bytes(),
                }),
            };
            let _ = out.try_send(msg);
        }
    }
}
