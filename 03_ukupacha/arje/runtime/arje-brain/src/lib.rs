//! ente-brain: motor de reglas determinista + observador estadístico.
//!
//! Tres capas:
//!   1. `rules`       — tipos de regla (Triplet: Subject + Event + Action)
//!   2. `engine`      — RuleEngine con HashMap<EventKindDiscriminant, Vec<Arc<Rule>>>
//!                      para dispatch O(1)
//!   3. `dispatch`    — ejecutor async de Actions (vía tokio)
//!   4. `observer`    — sliding window + marginales + co-ocurrencias
//!                    + Shannon entropy + información mutua
//!   5. `crystallize` — detección de patrones estadísticamente significativos
//!                      y materialización en `Rule` ejecutables
//!   6. `introspect`  — Unix socket bincode API para tools externos
//!
//! Diseño de inmutabilidad:
//!   - Rules son `Arc<Rule>` — clonar es zero-copy (refcount bump).
//!   - El motor expone sólo lecturas; mutaciones pasan por `insert/remove`.
//!   - Observer mantiene contadores incrementales — sin recomputación.

pub mod audit;
pub mod autopromote;
pub mod crystallize;
pub mod dispatch;
pub mod engine;
pub mod introspect;
pub mod loader;
pub mod metrics;
pub mod observer;
pub mod rules;

pub use autopromote::{spawn_autopromote_loop, AutopromoteParams};
pub use crystallize::{detect_crystals, Crystal, CrystallizationParams};
pub use dispatch::{dispatch_actions, ActionSink, NullSink};
pub use engine::{EventKindDiscriminant, RuleEngine, SubjectInfo};
pub use introspect::{IntrospectRequest, IntrospectResponse, IntrospectServer, BrainState};
pub use loader::{load_card_file, load_rules_file};
pub use metrics::serve_metrics;
pub use observer::{Observer, TimedEvent};
pub use rules::{Action, EventKind, EventPattern, LogLevel, Rule, Scope};
