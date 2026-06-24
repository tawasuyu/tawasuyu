//! `hapiy-capture` — los **backends de captura** tras el trait
//! [`hapiy_core::Capturer`], compartidos por el CLI (`hapiy`) y la GUI
//! (`hapiy-llimphi`).
//!
//! - **nativo** ([`wayland`], feature `wayland`) — cliente `zwlr_screencopy`
//!   propio sobre `wayland-client`. El camino soberano; no depende de grim.
//! - **grim** ([`grim`]) — delega en el binario `grim`. Fallback.
//!
//! [`capturer`] elige según [`Backend`]; `Backend::Auto` prueba el nativo y cae
//! a grim si no está disponible.

pub mod grim;
#[cfg(feature = "wayland")]
pub mod wayland;

use hapiy_core::Capturer;

/// Qué backend usar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Probar el nativo; caer a grim si falla.
    Auto,
    /// Cliente `zwlr_screencopy` nativo (requiere la feature `wayland`).
    Native,
    /// El binario `grim`.
    Grim,
}

/// Construye el [`Capturer`] para el backend pedido.
pub fn capturer(backend: Backend) -> Result<Box<dyn Capturer>, String> {
    match backend {
        Backend::Grim => Ok(Box::new(grim::GrimCapturer)),
        Backend::Native => native(),
        Backend::Auto => match native() {
            Ok(c) => Ok(c),
            Err(e) => {
                eprintln!("hapiy: backend nativo no disponible ({e}); uso grim.");
                Ok(Box::new(grim::GrimCapturer))
            }
        },
    }
}

#[cfg(feature = "wayland")]
fn native() -> Result<Box<dyn Capturer>, String> {
    Ok(Box::new(wayland::WaylandCapturer::connect()?))
}

#[cfg(not(feature = "wayland"))]
fn native() -> Result<Box<dyn Capturer>, String> {
    Err("compilado sin el backend nativo (feature `wayland`)".into())
}
