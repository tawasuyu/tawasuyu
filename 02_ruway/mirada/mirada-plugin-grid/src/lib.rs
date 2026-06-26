//! Plugin de layout **grilla adaptativa**: reparte las ventanas en una grilla de
//! `cols × rows` con `cols = ⌈√n⌉`, de modo que crece parejo (4 ventanas → 2×2,
//! 9 → 3×3, 5 → 3×2 con la última fila más holgada). La última fila estira sus
//! celdas para llenar el ancho. `gap` viene de los `LayoutParams`.
//!
//! Sin `sqrt` (no_std puro): `cols` se calcula con un lazo entero. Es
//! `CAP_LAYOUT`: función pura, sin importar nada del host.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use mirada_plugin_sdk::{export_layout_plugin, LayoutPlugin, Rect, TileInput, WindowId};

#[derive(Default)]
struct Grid;

/// Encoge un rect por `gap` en cada lado (mínimo 1×1).
fn inset(r: Rect, gap: i32) -> Rect {
    Rect::new(r.x + gap, r.y + gap, (r.w - 2 * gap).max(1), (r.h - 2 * gap).max(1))
}

/// `⌈√n⌉` por lazo entero (sin `f32::sqrt`, que pide `std`).
fn cols_for(n: i32) -> i32 {
    let mut c = 1;
    while c * c < n {
        c += 1;
    }
    c
}

impl LayoutPlugin for Grid {
    fn tile(&mut self, input: &TileInput) -> Vec<(WindowId, Rect)> {
        let ids = &input.ids;
        let work = input.work;
        let gap = input.params.gap.max(0);
        let n = ids.len() as i32;
        let mut out = Vec::with_capacity(ids.len());
        if n == 0 {
            return out;
        }
        let cols = cols_for(n);
        let rows = (n + cols - 1) / cols;
        let cell_h = work.h / rows;
        for (idx, &id) in ids.iter().enumerate() {
            let i = idx as i32;
            let r = i / cols;
            let c = i % cols;
            // Ventanas en ESTA fila (la última puede tener menos → celdas anchas).
            let in_row = (n - r * cols).min(cols).max(1);
            let cell_w = work.w / in_row;
            let x = work.x + c * cell_w;
            let y = work.y + r * cell_h;
            // La última celda de la fila/columna se come el sobrante de la división.
            let w = if c == in_row - 1 { work.x + work.w - x } else { cell_w };
            let h = if r == rows - 1 { work.y + work.h - y } else { cell_h };
            out.push((id, inset(Rect::new(x, y, w, h), gap)));
        }
        out
    }
}

export_layout_plugin!(Grid);
