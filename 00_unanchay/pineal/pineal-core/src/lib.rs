//! `pineal-core` — primitivas agnósticas de Lapaloma.
//!
//! Cero `gpui`, cero `wgpu`, cero I/O. Todo lo que vive acá puede
//! correr en un test unitario, en un worker thread o en un export
//! a SVG. Las tres reglas del documento de arquitectura aplican:
//!
//! - **P1 Zero boxing.** Los datos viven en `Vec<f32>` planos
//!   indexados, nunca como `Vec<Point2D>`. Cache L1 caliente y el
//!   compilador puede SIMD-loopearlo.
//! - **P2 Zero alloc en hot path.** Buffers se reservan al construir,
//!   se mutan in-place para siempre. Helpers escriben a `&mut Vec`
//!   provistos por el caller, no devuelven `Vec` nuevos.
//! - **P3 Una draw call por capa.** Acá no se dibuja; pero los
//!   tipos exponen slices contiguos listos para mandar al GPU
//!   sin copia.
//!
//! Convención de coordenadas: el buffer canónico es interleaved
//! `[x0, y0, x1, y1, ...]`. Esto es el format que `drawRawPoints`,
//! `Vertices.raw`, `wgpu` vertex buffers y `<polyline points>` SVG
//! consumen sin transformación.

#![forbid(unsafe_code)]

pub mod buffer;
pub mod ring;
pub mod spatial;
pub mod lttb;
pub mod scale;

// Algoritmos de layout: cada uno vive en el crate de la viz que lo
// usa, no acá. Esto es deliberado — `pineal-core` no debe arrastrar
// dependencias de visualización.
//
// - Barnes-Hut + Sugiyama + tree layout: `pineal-mesh`.
// - Squarified treemap: `pineal-treemap`.
// - Sankey layered: `pineal-flow`.
// - FDEB (Force-Directed Edge Bundling): roadmap, vendrá a `pineal-mesh`.
