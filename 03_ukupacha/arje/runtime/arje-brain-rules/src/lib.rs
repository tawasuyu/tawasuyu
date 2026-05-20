//! arje-brain-rules — motor de reglas determinista.
//!
//! Capa base del brain: tipos de regla (triplet Subject+Event+Action),
//! `RuleEngine` con dispatch O(1) por discriminante de evento, y el
//! ejecutor async de acciones. Sin dependencias estadísticas ni de UI.

pub mod rules;
pub mod engine;
pub mod dispatch;

pub use rules::{Action, EventKind, EventPattern, LogLevel, Rule, Scope, TimedEvent};
pub use engine::{EventKindDiscriminant, RuleEngine, SubjectInfo};
pub use dispatch::{dispatch_actions, ActionSink, NullSink};
