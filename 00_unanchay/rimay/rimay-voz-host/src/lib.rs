//! `rimay-voz-host` — el host de captura de la voz manos-libres.
//!
//! Cierra el lazo de `VOZ.md`: corre el **micrófono** y empuja sus frames por
//! la cadena de `rimay-voz` (VAD → STT → máquina), emitiendo
//! [`EventoEscucha`]s que la app dispatcha como `Msg` a su update Elm.
//!
//! Es **IA general** (oír), no de ninguna app: vive en `rimay`, igual que el
//! resto de la voz. Una app que quiera dictado manos-libres lo consume; lo
//! único «de la app» es decidir qué hace con los eventos (shuma, además, mapea
//! `shuma_agente::BloqueSalida` → `rimay_voz::TipoBloque` para el TTS).
//!
//! ## Dos capas, una testeable sin micrófono
//!
//! - [`Lazo`] (siempre) — la lógica pura: muestras `i16` mono → framing → VAD →
//!   STT → máquina → eventos. Sin `tokio`, sin cpal. Un test la alimenta con
//!   audio sintético y verifica los eventos por texto.
//! - [`prep`] (siempre) — downmix + remuestreo + `i16`, puro y testeable.
//! - `escuchar` (feature `microfono`, **ON por default**) — el driver real:
//!   abre cpal en un hilo dedicado (el `Stream` es `!Send`), prepara el audio y
//!   alimenta el [`Lazo`] desde una task async, emitiendo eventos por canal.
//!   Apagable con `--no-default-features` (deja sólo el lazo puro, sin ALSA).
//!
//! ```no_run
//! # #[cfg(feature = "microfono")]
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! let stt = rimay_voz::stt_mock();                 // o el daemon/nube real
//! let (_guardia, mut eventos) = rimay_voz_host::escuchar(stt)?;
//! while let Some(ev) = eventos.recv().await {
//!     // dispatch como Msg a la UI: ev es Escuchando / Desperto / Dictar / SeDurmio
//!     println!("{ev:?}");
//! }
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]

pub mod lazo;
pub mod prep;

pub use lazo::{EventoEscucha, Lazo};

#[cfg(feature = "microfono")]
mod microfono;
#[cfg(feature = "microfono")]
pub use microfono::{
    enrolar, escuchar, escuchar_cfg, escuchar_con, GuardiaEscucha, OpcionesEscucha,
};
