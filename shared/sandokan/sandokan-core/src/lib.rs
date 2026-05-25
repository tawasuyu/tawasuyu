//! sandokan-core — el contrato del orquestador.
//!
//! `sandokan` es el orquestador del ecosistema brahman, diseñado como
//! **library horizontal embebible**, no como daemon supremo. Cualquier
//! binario (shuma, nahual-shell, matilda, un agente SSH) embebe un
//! `Engine` y decide si lo corre in-process o delega a otro.
//!
//! Este crate define SOLO el contrato:
//! - [`Intent`] — qué orquestar (una Card + contexto de ejecución).
//! - [`ExecHandle`] — referencia a una entidad encarnada.
//! - [`LifecycleEvent`] / [`TelemetryFrame`] — observabilidad.
//! - [`Engine`] — el trait que `LocalEngine`/`DaemonEngine`/`RemoteEngine`
//!   implementan.
//!
//! Las implementaciones concretas viven en crates separados
//! (`sandokan-local`, `sandokan-daemon`, `sandokan-remote`).

pub mod engine;
pub mod error;
pub mod event;
pub mod intent;

pub use engine::Engine;
pub use error::EngineError;
pub use event::{LifecycleEvent, TelemetryFrame};
pub use intent::{ExecContext, ExecHandle, Intent, IsolationLevel};
