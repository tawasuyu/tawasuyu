//! `shuma-line` — el cerebro del input del shell.
//!
//! La función principal del shell es su línea de comandos, y esta línea
//! no es un campo de texto tonto: analiza lo que se escribe para
//! resaltarlo, autocompletarlo y entender sus tuberías. Toda esa
//! inteligencia vive aquí, **agnóstica de frontend** — la usa igual el
//! shell GPUI de brahman que una versión TUI.
//!
//! - [`dialect`] — el [`Dialect`] de la línea (bash hoy; zsh/fish/python
//!   a futuro, conmutable).
//! - [`token`] — el [`Token`] y su [`TokenKind`] (la clase de resaltado).
//! - [`lexer`] — [`tokenize`]: el análisis léxico + clasificación.
//! - [`pipeline`] — [`split_pipeline`]: la línea descompuesta en etapas
//!   separadas por `|`.
//! - [`complete`] — el motor de autocompletado y su [`CompletionSource`].
//! - [`editor`] — [`LineState`], el estado editable del input.
//!
//! Un frontend traduce sus eventos de teclado a métodos de `LineState` y
//! pinta `LineState::tokens()` con un color por `TokenKind`. Nada más.

#![forbid(unsafe_code)]

pub mod complete;
pub mod dialect;
pub mod editor;
pub mod ghost;
pub mod lexer;
pub mod pipeline;
pub mod token;

pub use complete::{complete, Completion, CompletionKind, CompletionSource, StaticSource};
pub use dialect::Dialect;
pub use editor::LineState;
pub use ghost::ghost_suggestion;
pub use lexer::tokenize;
pub use pipeline::{split_pipeline, Pipeline, Stage};
pub use token::{Token, TokenKind};
