//! Parsing CSS: hoja de estilos completa (`parse_stylesheet`, at-rules, `@media`,
//! `@keyframes`, `@import`), UA stylesheet y defaults por tag, substitución de
//! `var()`, parsing de selectores (`parse_selector`/`parse_compound`) y de
//! declaraciones (`parse_declarations`/`decl_kind_from_pair` + shorthands de
//! border/box-shadow/animation/transition), y los helpers públicos `parse_color`
//! y `evaluate_media_query`. Extraído de `style/mod.rs` (regla #1). Comparte los
//! tipos del módulo `style` y del crate vía `use super::*`.
use super::*;

mod sheet;
pub(crate) use sheet::*;
mod selectors;
pub(crate) use selectors::*;
mod decls;
pub(crate) use decls::*;
mod props;
pub use props::*;
