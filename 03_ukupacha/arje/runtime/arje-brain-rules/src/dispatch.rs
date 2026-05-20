//! Despacho asíncrono de Actions. El motor entrega `Vec<Arc<Rule>>` matched;
//! este módulo las traduce a efectos del fractal vía un `ActionSink` trait.
//!
//! Esto invierte la dependencia: ente-brain no conoce a ente-zero. El init
//! implementa `ActionSink` y wirea spawn/invoke/log a sus propias estructuras.

use crate::rules::{Action, LogLevel, Rule};
use std::sync::Arc;
use tracing::{debug, error, info, trace, warn};

/// Backend de ejecución de Actions. ente-zero implementa esto delegando a
/// graph_tx (Spawn → SpawnRequest, Invoke → bus call, etc.).
pub trait ActionSink: Send + Sync {
    /// Spawn una Card decodificada. Implementación: GraphEvent::SpawnRequest.
    fn spawn(&self, card_blob: &str);
    /// Invoke por bus. blob crudo; el sink lo enruta vía bus_mediator.
    fn invoke(&self, target_cap: arje_card::Capability, blob: Vec<u8>);
    /// Notifica a un Ente específico (target_id). Implementación: forward por bus.
    fn notify(&self, target_id: ulid::Ulid, message: &str);
    /// Inhibe un comportamiento (placeholder; semántica depende del sink).
    fn inhibit(&self, reason: &str);
}

/// Sink por defecto que sólo logea. Útil para tests y dev sin runtime.
pub struct NullSink;

impl ActionSink for NullSink {
    fn spawn(&self, card_blob: &str) {
        info!(blob_len = card_blob.len(), "NullSink::spawn (no-op)");
    }
    fn invoke(&self, target_cap: arje_card::Capability, blob: Vec<u8>) {
        info!(?target_cap, blob_len = blob.len(), "NullSink::invoke (no-op)");
    }
    fn notify(&self, target_id: ulid::Ulid, message: &str) {
        info!(%target_id, %message, "NullSink::notify (no-op)");
    }
    fn inhibit(&self, reason: &str) {
        info!(%reason, "NullSink::inhibit (no-op)");
    }
}

/// Ejecuta las reglas matched. Cada Rule puede tener N Actions; ejecutamos
/// todas. Las acciones de Log se evalúan inline (tracing es async-safe).
/// Las acciones de Spawn/Invoke/Notify se delegan al sink — el sink decide
/// si procesarlas sincrónica o asincrónicamente.
pub async fn dispatch_actions(rules: &[Arc<Rule>], sink: &dyn ActionSink) {
    for rule in rules {
        trace!(id = %rule.id, priority = rule.priority, n = rule.then.len(), "dispatching rule");
        for action in &rule.then {
            execute_action(action, sink, rule.id).await;
        }
    }
}

async fn execute_action(action: &Action, sink: &dyn ActionSink, rule_id: ulid::Ulid) {
    match action {
        Action::Log { level, message } => emit_log(level, message, rule_id),
        Action::Notify { target_id, message } => sink.notify(*target_id, message),
        Action::Spawn { card_blob } => sink.spawn(card_blob),
        Action::Invoke { target_cap, blob } => sink.invoke(target_cap.clone(), blob.clone()),
        Action::Inhibit { reason } => sink.inhibit(reason),
    }
}

fn emit_log(level: &LogLevel, message: &str, rule_id: ulid::Ulid) {
    match level {
        LogLevel::Trace => trace!(rule = %rule_id, "{}", message),
        LogLevel::Debug => debug!(rule = %rule_id, "{}", message),
        LogLevel::Info  => info! (rule = %rule_id, "{}", message),
        LogLevel::Warn  => warn! (rule = %rule_id, "{}", message),
        LogLevel::Error => error!(rule = %rule_id, "{}", message),
    }
}
