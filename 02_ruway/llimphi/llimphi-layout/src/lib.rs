//! llimphi-layout — Física del Espacio.
//!
//! Resuelve coordenadas (x, y, width, height) absolutas de un árbol de
//! nodos via Flexbox + CSS Grid. Backend: `taffy`.
//!
//! Fase 3: pendiente.

/// Caja calculada por el solver de layout.
#[derive(Default, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
