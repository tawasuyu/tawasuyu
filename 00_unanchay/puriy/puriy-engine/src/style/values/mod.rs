//! Tipos de valores CSS computados: `ComputedStyle` y todos los enums/structs
//! que la representan (longitudes, flex/grid, colores de gradiente, sombras,
//! transforms, animaciones, viewport, `Sides`/`Corners`), con sus `Default`.
//! Repartido por familia en submódulos hermanos. Extraído de `style/mod.rs`
//! (regla #1). Comparte los tipos del módulo `style` y del crate vía `use super::*`.
use super::*;

mod computed;
pub use computed::*;
// `computed_default` sólo aporta `impl Default for ComputedStyle` (sin nombres
// que reexportar): basta declararlo para que el impl quede activo.
mod computed_default;
mod enums_text;
pub use enums_text::*;
mod enums_svg;
pub use enums_svg::*;
mod layout;
pub use layout::*;
