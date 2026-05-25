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
//! `[x0, y0, x1, y1, ...]`. Esto es el formato que `drawRawPoints`,
//! `Vertices.raw`, `wgpu` vertex buffers y `<polyline points>` SVG
//! consumen sin transformación.

#![forbid(unsafe_code)]

pub mod buffer;
pub mod ring;
pub mod spatial;
pub mod lttb;
pub mod scale;

// Algoritmos de layout — quedan como placeholders hasta que cada
// módulo de visualización (mesh, treemap, flow) los demande.

/// Barnes-Hut quadtree para layouts force-directed.
///
/// Cuando se implemente: el quadtree es un `Vec<f32>` plano de
/// stride 7 (cm_x, cm_y, mass, half_size, center_x, center_y,
/// child_base), no un árbol de objetos. Rebuild O(n) por frame
/// sin allocations.
pub mod barnes_hut {}

/// Sugiyama-lite jerárquico: cycle-removal por DFS + Kahn layering
/// + barycenter ordering con inversion-count crossings.
pub mod sugiyama {}

/// Squarified treemap (Bruls / d3-hierarchy). Worst-aspect formula
/// usa el lado *corto* del rectángulo restante.
pub mod squarify {}

/// Subtree-width tree layout: BFS spanning + bottom-up width
/// measurement + top-down placement. Simpler que Reingold-Tilford.
pub mod tree_layout {}

/// Force-Directed Edge Bundling (FDEB-lite, single quadratic-bezier
/// control point por edge).
pub mod fdeb {}
