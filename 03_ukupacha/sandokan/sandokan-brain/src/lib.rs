//! `sandokan-brain` — el puente que le da **teeth** al cerebro de reglas.
//!
//! El cerebro determinista (`arje-brain-rules`) sabe *decidir* —ante un patrón
//! de eventos, emite un `Action`— pero por sí solo no *actúa* sobre el plano de
//! control: su [`ActionSink`] base sólo loguea/spawnea por bus. Este crate cierra
//! ese lazo (SDD §8 capa 3): [`EngineSink`] implementa `ActionSink` enrutando los
//! **verbos de control** (`Stop`/`SetCpuWeight`/`Freeze`) al contrato
//! [`sandokan_core::Engine`]. Así una regla «si chasqui >80% CPU 30 s → bajá su
//! peso» se vuelve ejecutable **por el mismo contrato** que controla y observa
//! todo lo demás —sin abrir un canal paralelo—.
//!
//! Las acciones que **no** son del contrato `Engine` (spawn/invoke/notify por
//! bus, inhibición) quedan como no-op acá: un host real **compone** este sink
//! con el suyo (p. ej. el `GraphSink` de arje-zero) para cubrir ambos planos.
//!
//! ## Sincronía
//!
//! `ActionSink` es síncrono; `Engine` es `async`. `EngineSink` resuelve el
//! desfase **fire-and-forget**: captura el `Handle` del runtime al construirse y
//! cada verbo lanza una tarea (`Handle::spawn`) que await-ea el método del
//! `Engine`. No bloquea el dispatch del cerebro; un fallo del `Engine` se
//! registra con `warn!` (el cerebro no tiene a quién devolverle el error).
//! Construir el sink exige estar dentro de un contexto tokio —igual que el
//! propio `dispatch_actions`—; usá [`EngineSink::with_handle`] si lo construís
//! fuera.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use arje_brain_rules::ActionSink;
use arje_card::Capability;
use sandokan_core::Engine;
use tokio::runtime::Handle;
use tracing::{trace, warn};
use ulid::Ulid;

/// `ActionSink` con teeth: enruta los verbos de control del cerebro al contrato
/// [`Engine`]. El resto de acciones (bus-level) son no-op (ver el doc del crate).
pub struct EngineSink {
    engine: Arc<dyn Engine>,
    handle: Handle,
}

impl EngineSink {
    /// Construye el sink capturando el runtime tokio **actual**. Debe llamarse
    /// dentro de un contexto async (igual que `dispatch_actions`). Si lo
    /// construís fuera de un runtime, usá [`Self::with_handle`].
    pub fn new(engine: Arc<dyn Engine>) -> Self {
        Self {
            engine,
            handle: Handle::current(),
        }
    }

    /// Variante explícita: el caller provee el `Handle` del runtime donde se
    /// ejecutarán las llamadas al `Engine`.
    pub fn with_handle(engine: Arc<dyn Engine>, handle: Handle) -> Self {
        Self { engine, handle }
    }
}

impl ActionSink for EngineSink {
    // --- Acciones bus-level: fuera del contrato `Engine`. No-op acá; un host
    // real compone EngineSink con el sink de su bus para cubrirlas. ---
    fn spawn(&self, card_blob: &str) {
        trace!(len = card_blob.len(), "EngineSink::spawn fuera del contrato Engine (no-op)");
    }
    fn invoke(&self, _target_cap: Capability, _blob: Vec<u8>) {
        trace!("EngineSink::invoke fuera del contrato Engine (no-op)");
    }
    fn notify(&self, _target_id: Ulid, _message: &str) {
        trace!("EngineSink::notify fuera del contrato Engine (no-op)");
    }
    fn inhibit(&self, _reason: &str) {
        trace!("EngineSink::inhibit fuera del contrato Engine (no-op)");
    }

    // --- Verbos de control (SDD §8 capa 3): el lazo cerrado. ---
    fn stop(&self, target_id: Ulid, grace_ms: u64) {
        let engine = self.engine.clone();
        self.handle.spawn(async move {
            if let Err(e) = engine.stop(target_id, Duration::from_millis(grace_ms)).await {
                warn!(%target_id, grace_ms, error = %e, "EngineSink::stop: el Engine falló");
            }
        });
    }

    fn set_cpu_weight(&self, cgroup_path: &str, weight: u32) {
        let engine = self.engine.clone();
        let path = cgroup_path.to_string();
        self.handle.spawn(async move {
            if let Err(e) = engine.set_cpu_weight(path.clone(), weight).await {
                warn!(%path, weight, error = %e, "EngineSink::set_cpu_weight: el Engine falló");
            }
        });
    }

    fn freeze(&self, cgroup_path: &str, frozen: bool) {
        let engine = self.engine.clone();
        let path = cgroup_path.to_string();
        self.handle.spawn(async move {
            if let Err(e) = engine.freeze(path.clone(), frozen).await {
                warn!(%path, frozen, error = %e, "EngineSink::freeze: el Engine falló");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arje_brain_rules::rules::{Action, EventKind, EventPattern, Rule, Scope};
    use arje_brain_rules::dispatch_actions;
    use async_trait::async_trait;
    use sandokan_core::{EngineError, ExecHandle, Intent, TelemetryFrame};
    use sandokan_lifecycle::LifecycleState;
    use tokio::sync::mpsc::{self, UnboundedSender};

    /// Qué verbo de control recibió el Engine. El mock lo empuja por un canal
    /// para que el test lo espere de forma **determinista** (el dispatch es
    /// fire-and-forget; sin canal habría que dormir y rezar).
    #[derive(Debug, PartialEq, Eq)]
    enum Llamada {
        Stop { id: Ulid, grace_ms: u128 },
        Weight { path: String, weight: u32 },
        Freeze { path: String, frozen: bool },
    }

    struct MockEngine {
        tx: UnboundedSender<Llamada>,
    }

    #[async_trait]
    impl Engine for MockEngine {
        async fn run(&self, _intent: Intent) -> Result<ExecHandle, EngineError> {
            unreachable!("EngineSink no llama run")
        }
        async fn stop(&self, card_id: Ulid, grace: Duration) -> Result<(), EngineError> {
            self.tx
                .send(Llamada::Stop { id: card_id, grace_ms: grace.as_millis() })
                .unwrap();
            Ok(())
        }
        async fn list(&self) -> Result<Vec<ExecHandle>, EngineError> {
            Ok(vec![])
        }
        async fn status(&self, id: Ulid) -> Result<LifecycleState, EngineError> {
            let _ = id;
            Ok(LifecycleState::Running)
        }
        async fn telemetry(&self, id: Ulid) -> Result<TelemetryFrame, EngineError> {
            Err(EngineError::NotFound(id))
        }
        async fn set_cpu_weight(&self, cgroup_path: String, weight: u32) -> Result<(), EngineError> {
            self.tx
                .send(Llamada::Weight { path: cgroup_path, weight })
                .unwrap();
            Ok(())
        }
        async fn freeze(&self, cgroup_path: String, frozen: bool) -> Result<(), EngineError> {
            self.tx
                .send(Llamada::Freeze { path: cgroup_path, frozen })
                .unwrap();
            Ok(())
        }
    }

    fn rule_con(action: Action) -> Rule {
        Rule {
            id: Ulid::new(),
            priority: 5,
            when: EventPattern::Single { kind: EventKind::EnteDied },
            then: vec![action],
            scope: Scope::default(),
        }
    }

    async fn ejecuta(action: Action) -> Llamada {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let engine: Arc<dyn Engine> = Arc::new(MockEngine { tx });
        let sink = EngineSink::new(engine);
        let rules = vec![Arc::new(rule_con(action))];
        dispatch_actions(&rules, &sink).await;
        // El verbo se ejecuta en una tarea spawneada; el canal nos deja
        // esperarla sin dormir.
        rx.recv().await.expect("el Engine debió recibir el verbo")
    }

    #[tokio::test]
    async fn stop_llega_al_engine_por_el_contrato() {
        let id = Ulid::new();
        let got = ejecuta(Action::Stop { target_id: id, grace_ms: 250 }).await;
        assert_eq!(got, Llamada::Stop { id, grace_ms: 250 });
    }

    #[tokio::test]
    async fn set_cpu_weight_llega_al_engine() {
        let got = ejecuta(Action::SetCpuWeight {
            cgroup_path: "pacha/presentando".into(),
            weight: 10,
        })
        .await;
        assert_eq!(
            got,
            Llamada::Weight { path: "pacha/presentando".into(), weight: 10 }
        );
    }

    #[tokio::test]
    async fn freeze_llega_al_engine() {
        let got = ejecuta(Action::Freeze {
            cgroup_path: "pacha/secundario".into(),
            frozen: true,
        })
        .await;
        assert_eq!(
            got,
            Llamada::Freeze { path: "pacha/secundario".into(), frozen: true }
        );
    }
}
