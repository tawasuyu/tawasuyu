//! `verbo-daemon` — embeddings compartidos entre procesos.
//!
//! El problema: cada proceso que quiera embeddings cargaría su propia
//! copia del modelo (cientos de MB de RAM, descargas duplicadas). La
//! solución: un [`Daemon`] carga el modelo una vez y lo sirve sobre un
//! socket Unix; cada proceso usa un [`DaemonClient`] que, por
//! implementar `rimay_verbo_core::Provider`, es indistinguible de un backend
//! local.
//!
//! ```text
//!   ┌── proceso A ──┐   ┌── proceso B ──┐   ┌── proceso C ──┐
//!   │ DaemonClient  │   │ DaemonClient  │   │ DaemonClient  │
//!   └───────┬───────┘   └───────┬───────┘   └───────┬───────┘
//!           └───────── socket Unix ─────────────────┘
//!                            │
//!                  ┌─────────┴─────────┐
//!                  │   Daemon (Arc<P>) │  ← un modelo en RAM
//!                  └───────────────────┘
//! ```
//!
//! **Multi-instancia**: para servir varios modelos a la vez se levanta
//! un daemon por modelo, cada uno en su socket — el daemon es agnóstico
//! del backend (sirve cualquier `Provider`: `verbo-mock`, un backend
//! Cohere, uno BGE local).

#![forbid(unsafe_code)]

mod client;
mod server;
mod transport;
mod wire;

pub use client::DaemonClient;
pub use server::Daemon;
pub use wire::{Request, Response};
