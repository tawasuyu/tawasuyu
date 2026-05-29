//! `pixel-verbo-daemon` — modelos de píxel compartidos entre procesos.
//!
//! El problema: cada proceso que quiera correr un modelo de imagen (un
//! tullpu, un nahual visor, un script de proceso por lote) cargaría su
//! propia copia (cientos de MB de pesos, descargas duplicadas). La
//! solución: un [`Servidor`] carga el modelo una vez y lo sirve sobre un
//! socket Unix; cada proceso usa un [`ClienteBloqueante`] que, por
//! implementar `pixel_verbo_core::Proveedor`, es indistinguible de un
//! backend local.
//!
//! ```text
//!   ┌── proceso A ──┐   ┌── proceso B ──┐   ┌── proceso C ──┐
//!   │  Cliente      │   │  Cliente      │   │  Cliente      │
//!   └───────┬───────┘   └───────┬───────┘   └───────┬───────┘
//!           └───────── socket Unix ─────────────────┘
//!                            │
//!                  ┌─────────┴─────────┐
//!                  │  Servidor (Arc<P>) │  ← un modelo en RAM
//!                  └───────────────────┘
//! ```
//!
//! **Sin tokio.** El servidor usa `std::os::unix::net::UnixListener` +
//! `std::thread::spawn` por conexión; el cliente es bloqueante con
//! `std::io::Read/Write`. La aritmética de un modelo de píxel típico (un
//! ONNX corriendo CPU) tarda decenas de milisegundos a segundos por
//! request — el costo de un thread del kernel por cliente es trivial
//! comparado, y nos ahorra arrastrar tokio a la app de escritorio.
//!
//! **Multi-instancia**: para servir varios modelos a la vez se levanta un
//! daemon por modelo, cada uno en su socket.

#![forbid(unsafe_code)]

mod client;
mod server;
mod wire;

pub use client::ClienteBloqueante;
pub use server::Servidor;
pub use wire::{Request, Response};
