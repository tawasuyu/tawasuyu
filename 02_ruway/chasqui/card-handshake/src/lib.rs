//! `brahman-handshake` — protocolo runtime Init↔módulo sobre Unix socket.
//!
//! Implementa la versión concreta de `shared_wit/protocol.wit` (handshake +
//! lifecycle): un servidor que vive en el Init (o un Admin proxy) y clientes
//! que son los módulos Brahman. Cada conexión arranca con un `Hello` que
//! lleva una [`card_core::Card`]; el servidor valida la Card, deriva el
//! [`TrustLevel`], emite un `HelloAck` con `session-id` ULID, y a partir de
//! ahí acepta `Ping`/`Farewell`.
//!
//! Wire format: frames length-prefixed (4 bytes LE) con cuerpo
//! [`postcard`]-codificado. Compacto, rápido y reversible.
//!
//! Esto NO es la implementación WIT/WASM (que generaría wit-bindgen). Es la
//! implementación nativa Rust↔Rust que cubre el caso común antes de que los
//! módulos WASM consuman el mismo contrato vía ABI generada.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod codec;
pub mod identity;
pub mod messages;
pub mod server;
pub mod client;
pub mod network;
pub mod peer_policy;
pub mod signature;
pub mod transport;

pub use card_core::PROTOCOL_VERSION;

/// Versión del crate de handshake (independiente de `PROTOCOL_VERSION`).
pub const HANDSHAKE_VERSION: &str = env!("CARGO_PKG_VERSION");
