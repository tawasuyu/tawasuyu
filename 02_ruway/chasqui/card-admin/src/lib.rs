//! `brahman-admin` — observabilidad del broker.
//!
//! Expone un Unix socket separado (no se mezcla con el handshake) en el
//! que cada conexión recibe un `StatusSnapshot` JSON y se cierra. Es
//! single-shot por conexión: pensado para herramientas como
//! `brahman-status`, dashboards y health-checks.
//!
//! Wire format: una línea JSON por conexión, terminada en `\n`. Esto
//! hace trivial inspeccionar con `nc` o `socat` además del cliente
//! tipado de este crate.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod client;
pub mod server;
pub mod snapshot;
pub mod transport;

pub use snapshot::StatusSnapshot;

/// Versión del crate de admin.
pub const ADMIN_VERSION: &str = env!("CARGO_PKG_VERSION");
