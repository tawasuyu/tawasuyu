//! sandokan-daemon — `DaemonEngine` + loop servidor.
//!
//! Permite que el orquestador corra en un proceso y otros lo consuman
//! sin reimplementar la lógica: el `DaemonEngine` (cliente) implementa
//! el trait [`sandokan_core::Engine`] enviando requests postcard
//! length-prefixed sobre un Unix socket; [`serve`] corre el lado
//! servidor envolviendo cualquier `Engine` (típicamente un `LocalEngine`).
//!
//! Es la pieza que materializa el patrón horizontal de sandokan: el
//! primer binario que arranca gana el socket y expone el engine; los
//! demás se le suman como `DaemonEngine`.

mod client;
pub mod protocol;
mod server;

pub use client::DaemonEngine;
pub use protocol::{read_frame, write_frame, DaemonRequest, DaemonResponse};
pub use server::serve;
