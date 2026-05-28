//! `pineal-cartesian` — gráficos cartesianos.
//!
//! Componentes:
//!
//! - [`viewport`] — `ChartViewport` con `(x_min, x_max, y_min, y_max)`
//!   y helpers de pan/zoom anchor-preserving.
//! - [`coord_system`] — proyecta valores de dominio → pixeles del plot
//!   usando las escalas de `pineal-core::scale`.
//! - [`series`] — trait `Series` + `LineSeries`. Cada serie decide
//!   LTTB vs raw según densidad.
//! - [`axis`] — ejes con nice-ticks (Wilkinson) y decimación de etiquetas
//!   que no overlappean.
//! - [`view`] — `ChartView` que se inserta como `View<Msg>` declarativo en
//!   un árbol llimphi-ui. Incluye `ChartCache` para translate-only pan-blit
//!   con hash de invalidación.

#![forbid(unsafe_code)]
#![allow(dead_code)]

pub mod viewport;
pub mod coord_system;
pub mod series;
pub mod axis;
pub mod view;

pub use viewport::ChartViewport;
pub use coord_system::CoordinateSystem;
pub use series::{LineSeries, PaintCtx, RenderMode, Series};

pub use view::{
    chart_cache, lapaloma_chart_view, ChartCache, ChartCacheHandle, ChartSeriesItem, ChartView,
};
