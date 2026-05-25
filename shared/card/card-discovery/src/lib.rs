//! `brahman-card-discovery` — búsqueda de Cards local + DHT.
//!
//! - [`index`] — `CardIndex`: índice en memoria con filtros (label,
//!   kind, capability, id).
//! - [`registry`] — `scan_dir`: carga Cards `*.json` de un directorio.
//! - [`discovery`] — `CardDiscovery`: une el índice local con la malla
//!   P2P vía `brahman-dht`.
//!
//! Lo consume el widget card-browser de `nahual-shell` y `agora_app`.

#![forbid(unsafe_code)]

pub mod index;
pub mod registry;
pub mod discovery;

pub use discovery::CardDiscovery;
pub use index::CardIndex;
pub use registry::scan_dir;
