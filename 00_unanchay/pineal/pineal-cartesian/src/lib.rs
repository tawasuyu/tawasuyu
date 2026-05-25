//! `pineal-cartesian` — gráficos cartesianos.
//!
//! Este crate trae:
//!
//! - **`viewport`** — `ChartViewport` con `(x_min, x_max, y_min, y_max)`
//!   y helpers de pan/zoom anchor-preserving.
//! - **`coord_system`** — proyecta valores de dominio → pixeles del
//!   plot usando las escalas de `pineal-core::scale`.
//! - **`series`** — trait `Series` + impls `LineSeries`, `BarSeries`,
//!   `AreaSeries`. Cada serie decide LTTB vs raw según densidad.
//! - **`axis`** — ejes con nice-ticks (Wilkinson) y decimación de
//!   etiquetas que no overlappean.
//! - **`picture_cache`** — translate-only pan-blit con hash de
//!   invalidación. Clipea el outer canvas antes del translate
//!   (bug 0.3.0 del Flutter).
//! - **`element`** — el `Element` GPUI que envuelve todo lo de
//!   arriba y se inserta en un layout nahual.
//!
//! Hoy todos los módulos están como placeholders; la primera
//! impl real va a ser `LineSeries` + `element` end-to-end para
//! validar la cadena `core → render → cartesian → gpui`.

#![forbid(unsafe_code)]
#![allow(dead_code)]

pub mod viewport;
pub mod coord_system;
pub mod series;
pub mod axis;

pub mod view;

// Pendientes — siguen como placeholders hasta su fase.
pub mod picture_cache {}

pub use viewport::ChartViewport;
pub use coord_system::CoordinateSystem;
pub use series::{LineSeries, PaintCtx, RenderMode, Series};

pub use view::{
    chart_cache, lapaloma_chart_view, ChartCache, ChartCacheHandle, ChartSeriesItem, ChartView,
};
