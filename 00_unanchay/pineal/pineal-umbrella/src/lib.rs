//! `pineal` — paraguas re-export.
//!
//! Para **prototipos** que quieren probar varios módulos a la vez
//! sin agregar 8 dependencias a `Cargo.toml`. En producción
//! preferir importar directamente los crates hoja (`pineal-core`,
//! `pineal-cartesian`, …) para que el linker descarte lo no
//! usado y los tiempos de compilación bajen.
//!
//! Las features mapean 1:1 a cada sub-crate:
//!
//! ```toml
//! [dependencies]
//! pineal = { workspace = true, default-features = false,
//!              features = ["cartesian", "stream"] }
//! ```

#![forbid(unsafe_code)]

#[cfg(feature = "core")]
pub use pineal_core as core;

#[cfg(feature = "render")]
pub use pineal_render as render;

#[cfg(feature = "cartesian")]
pub use pineal_cartesian as cartesian;

#[cfg(feature = "stream")]
pub use pineal_stream as stream;

#[cfg(feature = "mesh")]
pub use pineal_mesh as mesh;

#[cfg(feature = "financial")]
pub use pineal_financial as financial;

#[cfg(feature = "polar")]
pub use pineal_polar as polar;

#[cfg(feature = "heatmap")]
pub use pineal_heatmap as heatmap;

#[cfg(feature = "treemap")]
pub use pineal_treemap as treemap;

#[cfg(feature = "flow")]
pub use pineal_flow as flow;

#[cfg(feature = "phosphor")]
pub use pineal_phosphor as phosphor;

#[cfg(feature = "export")]
pub use pineal_export as export;
