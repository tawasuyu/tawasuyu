//! `shuma-intent` — núcleo agnóstico del shell shuma.
//!
//! El shell shuma trabaja con **intenciones**, no comandos sueltos: cada
//! línea del prompt es una [`Intention`] (etapas conectadas por pipes,
//! con tokens de referencia `%cN`/`%pN`). El [`SessionGraph`] mantiene el
//! historial como un grafo de contexto navegable: cada comando es un
//! nodo, cada salida un buffer intermedio referenciable.
//!
//! Todo acá es lógica pura y serializable — el front-end GPUI (las tres
//! zonas: RUN, SENS y el lienzo central) lo rehidrata; la ejecución real
//! la hace `sandokan`.

#![forbid(unsafe_code)]

pub mod parse;
pub mod graph;

pub use graph::{CommandNode, NodeStatus, SessionGraph};
pub use parse::{Intention, Ref, Stage};
