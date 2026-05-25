//! sandokan-lifecycle — primitivas de ciclo de vida agnósticas.
//!
//! Lógica pura reutilizable por cualquier supervisor de procesos
//! (shuma, matilda Ghost, chaka_app-shadow, mirada). Sin dependencias de
//! syscalls, proceso, ni UI: solo cálculo.
//!
//! - [`backoff`] — backoff exponencial con tope.
//! - [`ttl`]     — time-to-live anclado a un `Instant`.
//! - [`quota`]   — cuotas de recursos + chequeo de breaches.
//! - [`restart`] — política de restart con conteo + backoff.
//! - [`state`]   — máquina de estados del ciclo de vida.

pub mod backoff;
pub mod ttl;
pub mod quota;
pub mod restart;
pub mod state;

pub use backoff::Backoff;
pub use ttl::Ttl;
pub use quota::{Breach, QuotaAction, QuotaReport, ResourceQuota, ResourceUsage, check_quota};
pub use restart::{RestartPolicy, RestartTracker};
pub use state::LifecycleState;
