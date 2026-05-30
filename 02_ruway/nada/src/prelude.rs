//! Re-exports de los imports externos para los módulos hijos.
#![allow(unused_imports)]

mod prelude;
mod view;
mod fsutil;
mod actions;
mod session;
mod clipboard;
mod update;
mod keys;

pub(crate) use crate::prelude::*;
pub(crate) use crate::actions::*;
pub(crate) use crate::fsutil::*;
pub(crate) use crate::session::*;
pub(crate) use crate::clipboard::*;

pub(crate) const TREE_WIDTH: f32 = 240.0;
pub(crate) const TREE_ROW_H: f32 = 22.0;
pub(crate) const TREE_INDENT: f32 = 16.0;
pub(crate) const HEADER_H: f32 = 34.0;
/// Altura del status bar inferior (estilo VS Code).
pub(crate) const STATUS_H: f32 = 24.0;
/// Grosor de las lineas accent que separan header/body/status.
pub(crate) const SEP_H: f32 = 1.0;
/// Altura del tab strip (sin contar la línea de acento).
pub(crate) const TAB_STRIP_H: f32 = 26.0;
/// Cuántas líneas mostramos en el viewport del editor. Aproximación
/// estática: (alto ventana ~760 − header 28) / line_height(~18) ≈ 40.
pub(crate) const EDITOR_VISIBLE_LINES: usize = 40;
/// Altura del panel terminal cuando está abierto. ~14 filas de 14px +
/// header 18px ≈ 214px — redondeado a 220.
pub(crate) const TERM_PANEL_H: f32 = 220.0;
/// Altura del panel diff cuando está abierto. ~30 filas de 15px +
/// header 18px ≈ 468px — redondeado a 480.
pub(crate) const DIFF_PANEL_H: f32 = 480.0;

#[derive(Clone)]
pub(crate) enum Msg {
    ToggleNode(usize),
    SelectNode(usize),
    EditKey(KeyEvent),
    EditorPointer(PointerEvent),
    Save,
    SaveResult(Result<(), String>),
    Scroll(i32),
    /// Cambia el tab activo. El índice se asume válido; en caso contrario
    /// se ignora.
