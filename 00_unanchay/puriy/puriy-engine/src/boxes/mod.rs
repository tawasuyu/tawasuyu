//! Box tree — output del engine, entrada de `llimphi-raster`.
//!
//! Un [`BoxNode`] es la unidad de pintado: rectángulo con fondo opcional
//! + texto opcional + lista ordenada de hijos. No hay layout real (no
//! corremos taffy todavía) — sólo posicionamiento naive: cada bloque
//! apila vertical, cada inline se concatena en la línea. Es suficiente
//! para que Llimphi pueda dibujar example.com legible.
//!
//! Fase 3 reemplazará este pase por `llimphi-layout` con taffy.

use markup5ever_rcdom::{Handle, NodeData};

use crate::dom::{self, DomTree};
use crate::style::{
    AlignContent, AlignItems, AlignSelf, Appearance, BackgroundClip, BackgroundOrigin,
    BackgroundPosition, BackgroundRepeat, BackgroundSize, BlendMode, BorderLineStyle,
    BoxShadow, BoxSizing, ComputedStyle, Corners, Cursor, Direction,
    FilterFn, FlexDirection, FlexWrap, FontFeatureSetting, FontKerning, FontVariationSetting,
    GridAutoFlow, GridTrackSize, Hyphens, ImageRendering, Isolation, JustifyContent, LengthVal, LinearGradient,
    ListStyleType, ObjectFit, Outline, Overflow, OverflowWrap, OverscrollBehavior, PointerEvents,
    Position, Resize, ScrollBehavior, ScrollSnapType, Sides, StyleEngine, TabSize, TextAlign,
    TextDecorationLine, TextDecorationStyle, TextOrientation, TextOverflow, TextRendering,
    TextShadow,
    TextTransform, Transform, TransformOrigin, UnicodeBidi, UserSelect, VerticalAlign, Visibility, WhiteSpace,
    WillChangeHint, WordBreak, WritingMode,
};

/// Modelo de datos (`Color`/`Display`/`BoxNode`/`BoxTree` + tipos auxiliares).
mod model;
pub use model::*;
/// Mutación/restyle del árbol ya construido (APIs del chrome + set_box_visual).
mod mutate;
pub use mutate::*;
/// Construcción del árbol desde DOM+StyleEngine (build/build_node, svg, imágenes).
mod build;
pub use build::*;

#[cfg(test)]
mod tests;
