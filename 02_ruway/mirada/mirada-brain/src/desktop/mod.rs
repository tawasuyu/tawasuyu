//! El [`Desktop`] — el estado del escritorio y el bucle `evento → comandos`.
//!
//! Dividido en submódulos por responsabilidad:
//!
//! - [`tipos`]    — `WindowInfo`, `Output`
//! - [`estado`]   — struct `Desktop`, constructores y accesores de sólo lectura
//! - [`sesion`]   — `snapshot` / `restore` (persistencia entre arranques)
//! - [`eventos`]  — `on_event` (handler del protocolo Cuerpo→Cerebro)
//! - [`acciones`] — `apply` y helpers de layout/navegación
//! - [`geometria`]— funciones puras de geometría (foco espacial, rects)

mod acciones;
mod estado;
mod eventos;
mod geometria;
mod sesion;
mod tipos;

#[cfg(test)]
mod tests;

// Re-exports públicos — la API del módulo no cambia.
pub use crate::config::DROPTERM_APP_ID;
pub use estado::Desktop;
pub use tipos::{Output, WindowInfo};
