//! arje-brain — capa de integración del brain.
//!
//! El brain se divide en tres sub-crates con un DAG de dependencias limpio:
//!   - `arje-brain-rules`     — motor determinista (rules + engine + dispatch)
//!   - `arje-brain-cognitive` — estadística (observer + crystallize)
//!   - `arje-brain-audit`     — accountability (audit chain → CAS)
//!
//! Este crate es la capa que los wirea: `introspect` (socket API),
//! `autopromote` (loop de promoción de cristales), `metrics` (HTTP) y
//! `loader` (carga de cards/rules). Re-exporta la API de los tres
//! sub-crates para compatibilidad de los consumidores históricos.

pub mod introspect;
pub mod autopromote;
pub mod metrics;
pub mod loader;

// --- Re-export de los módulos de las 3 sub-crates ---
pub use arje_brain_rules::{dispatch, engine, rules};
pub use arje_brain_cognitive::{crystallize, observer};
pub use arje_brain_audit::audit;

// --- Re-exports planos (API histórica que consumen arje-zero, chasqui) ---
pub use rules::{Action, EventKind, EventPattern, LogLevel, Rule, Scope, TimedEvent};
pub use engine::{EventKindDiscriminant, RuleEngine, SubjectInfo};
pub use dispatch::{dispatch_actions, ActionSink, NullSink};
pub use crystallize::{detect_crystals, Crystal, CrystallizationParams};
pub use observer::Observer;
pub use audit::AuditLog;

pub use autopromote::{spawn_autopromote_loop, AutopromoteParams};
pub use introspect::{BrainState, IntrospectRequest, IntrospectResponse, IntrospectServer};
pub use loader::{load_card_file, load_rules_file};
pub use metrics::serve_metrics;
