//! `rimay-voz-daemon` — voz compartida entre procesos (el **brazo local** del
//! híbrido).
//!
//! El problema: cada proceso que quiera dictar o leer cargaría su propia copia
//! del modelo (whisper/piper son cientos de MB). La solución, calcada de
//! `rimay-verbo-daemon`: un [`Daemon`] carga los modelos una vez y los sirve
//! sobre un socket Unix; cada proceso usa un [`DaemonClient`] que, por
//! implementar [`Transcriptor`](rimay_voz_core::Transcriptor) **y**
//! [`Locutor`](rimay_voz_core::Locutor), es indistinguible de un backend local.
//!
//! ```text
//!   ┌── proceso A ──┐   ┌── proceso B ──┐   ┌── proceso C ──┐
//!   │ DaemonClient  │   │ DaemonClient  │   │ DaemonClient  │
//!   └───────┬───────┘   └───────┬───────┘   └───────┬───────┘
//!           └───────── socket Unix ─────────────────┘
//!                            │
//!                ┌───────────┴────────────┐
//!                │ Daemon (Arc<dyn STT> + │  ← modelos en RAM, una vez
//!                │          Arc<dyn TTS>)  │
//!                └────────────────────────┘
//! ```
//!
//! Es la contraparte de la rama de nube ([`rimay-voz-nube`]): `VozConfig` elige
//! `Backend::Local` → `DaemonClient::connect(socket)`, o `Backend::Nube` → el
//! backend HTTP. El daemon **no compite** con la nube; sirve cuando el modelo
//! vive en esta máquina.
//!
//! **Multi-instancia**: para servir varios pares se levanta un daemon por par,
//! cada uno en su socket — el daemon es agnóstico del backend (sirve cualquier
//! `Transcriptor`/`Locutor`: mock, whisper, piper).

#![forbid(unsafe_code)]

mod client;
mod server;
mod transport;
mod wire;

pub use client::DaemonClient;
pub use server::Daemon;
pub use wire::{Request, Response};
