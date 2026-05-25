//! Glue entre el bucle primordial y `ente-brain`.
//!
//! Tres responsabilidades:
//!   1. Traducir eventos del grafo (`GraphEvent`) a `arje_brain::EventKind`
//!      + `SubjectInfo` para el observador y el motor.
//!   2. Implementar `ActionSink` para que las Acciones del cerebro tengan
//!      un canal de salida hacia el grafo (Spawn → SpawnRequest, etc.).
//!   3. Encapsular el snapshot de SubjectInfo desde el grafo sin filtrar
//!      detalles internos al cerebro.

use crate::events::GraphEvent;
use crate::graph::EnteGraph;
use arje_brain::{ActionSink, EventKind as BrainEventKind, SubjectInfo};
use arje_card::Capability;
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::warn;
use ulid::Ulid;

/// Traduce un GraphEvent a (EventKind, SubjectInfo) para alimentar el cerebro.
///
/// Devuelve `None` para eventos puramente internos del bus (Response, Close)
/// que no son interesantes para reglas o estadística.
pub fn graph_event_to_brain<'a>(
    evt: &'a GraphEvent,
    graph: &EnteGraph,
) -> Option<(BrainEventKind, SubjectInfo)> {
    match evt {
        GraphEvent::EnteDied { id, .. } => {
            Some((BrainEventKind::EnteDied, subject_info_for(graph, *id)))
        }
        GraphEvent::SpawnRequest { card, .. } => {
            // El "sujeto" del spawn es el child que va a nacer.
            let info = SubjectInfo {
                id: Some(card.id),
                label: Some(card.label.clone()),
                capabilities: card.provides.iter().cloned().collect(),
            };
            Some((BrainEventKind::EnteSpawned, info))
        }
        GraphEvent::BusRequest { from, request, .. } => {
            let kind = match request {
                arje_bus::BusRequest::Announce { .. } => BrainEventKind::BusAnnounce,
                arje_bus::BusRequest::Invoke { cap, .. } => {
                    BrainEventKind::BusInvokeOf(cap.clone())
                }
                _ => BrainEventKind::BusInvoke,
            };
            let info = match from {
                Some(id) => subject_info_for(graph, *id),
                None => SubjectInfo::default(),
            };
            Some((kind, info))
        }
        GraphEvent::CapabilityRequested { from, .. } => {
            Some((BrainEventKind::BusInvoke, subject_info_for(graph, *from)))
        }
        // Responses, ConnClosed, Shutdown — irrelevantes para reglas
        _ => None,
    }
}

fn subject_info_for(graph: &EnteGraph, id: Ulid) -> SubjectInfo {
    // Acceso de sólo lectura — usamos el método público lookup_pid + cards
    // virtuales en el grafo. Si el Ente no existe (ya disuelto), info vacía.
    if let Some(card) = graph.card_for(&id) {
        SubjectInfo {
            id: Some(id),
            label: Some(card.label.clone()),
            capabilities: card.provides.iter().cloned().collect(),
        }
    } else {
        SubjectInfo { id: Some(id), label: None, capabilities: Vec::new() }
    }
}

/// `ActionSink` que enruta acciones del cerebro al bucle primordial.
pub struct GraphSink {
    pub graph_tx: mpsc::Sender<GraphEvent>,
    pub requester: Ulid,
}

impl ActionSink for GraphSink {
    fn spawn(&self, card_blob: &str) {
        // El blob es JSON de EntityCard.
        match serde_json::from_str::<arje_card::EntityCard>(card_blob) {
            Ok(card) => {
                let evt = GraphEvent::SpawnRequest { card, requester: self.requester };
                if self.graph_tx.try_send(evt).is_err() {
                    warn!("brain spawn: graph_tx lleno o cerrado");
                }
            }
            Err(e) => warn!(?e, "brain spawn: blob no parseable como EntityCard JSON"),
        }
    }

    fn invoke(&self, target_cap: Capability, blob: Vec<u8>) {
        // Sin BusClient en proceso — el sink registra la intención. Una mejora
        // futura: spawn un BusClient::connect + call. Por ahora log estructurado.
        warn!(?target_cap, blob_len = blob.len(), "brain invoke: no bus client en glue (TODO)");
    }

    fn notify(&self, target_id: Ulid, message: &str) {
        warn!(%target_id, %message, "brain notify: no implementado en glue");
    }

    fn inhibit(&self, reason: &str) {
        warn!(%reason, "brain inhibit: no implementado en glue");
    }
}

/// Helper para que el grafo exponga la Card de un Ente vivo. Lo añadimos como
/// trait extension porque graph::EnteGraph mantiene `incarnated` privado.
pub trait GraphCardLookup {
    fn card_for(&self, id: &Ulid) -> Option<&arje_card::EntityCard>;
}

impl GraphCardLookup for EnteGraph {
    fn card_for(&self, id: &Ulid) -> Option<&arje_card::EntityCard> {
        // Acceso vía método público que añadiremos en graph/mod.rs.
        self.peek_card(id)
    }
}

// Eliminar el campo `_unused` que rustc puede quejarse — placeholder para
// evitar warning si algún field queda sin uso.
#[allow(dead_code)]
#[derive(Deserialize)]
struct _Touch {}
