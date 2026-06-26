//! Un plugin de layout de ejemplo: **right-master** — pila maestra a la derecha.
//!
//! Es un master-stack reflejado: la(s) ventana(s) maestra(s) van en la columna
//! **derecha** (ancho = `master_ratio` del área) y el resto se apila a la
//! izquierda. Honra los `LayoutParams` que el host le pasa desde el `Desktop`
//! (`master_ratio`, `master_count`, `gap`), así los atajos del usuario —crecer
//! la maestra (`Super+l`), cambiar el nº de maestras (`Super+,`/`Super+.`)—
//! siguen gobernando el teselado aunque lo dibuje este plugin.
//!
//! Ponerlo a la derecha lo hace distinguible del master-stack izquierdo que
//! trae el `Desktop`: si ves la maestra a la derecha, corrió el plugin.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use mirada_plugin_sdk::{export_layout_plugin, LayoutPlugin, Rect, TileInput, WindowId};

#[derive(Default)]
struct RightMaster;

impl LayoutPlugin for RightMaster {
    fn tile(&mut self, input: &TileInput) -> Vec<(WindowId, Rect)> {
        let ids = &input.ids;
        let work = input.work;
        let gap = input.params.gap.max(0);
        let n = ids.len();
        if n == 0 {
            return Vec::new();
        }

        let nmaster = input.params.master_count.max(1).min(n);
        // Una sola columna si todo cabe en el área maestra.
        if n <= nmaster {
            return stack_column(ids, work, gap);
        }

        let ratio = input.params.master_ratio.clamp(0.05, 0.95);
        let master_w = (work.w as f32 * ratio) as i32;
        let stack_w = work.w - master_w;
        let stack_rect = Rect::new(work.x, work.y, stack_w, work.h);
        let master_rect = Rect::new(work.x + stack_w, work.y, master_w, work.h);

        let mut out = stack_column(&ids[..nmaster], master_rect, gap);
        out.extend(stack_column(&ids[nmaster..], stack_rect, gap));
        out
    }
}

/// Apila `ids` en celdas verticales iguales dentro de `col`, con margen `gap`.
fn stack_column(ids: &[WindowId], col: Rect, gap: i32) -> Vec<(WindowId, Rect)> {
    let n = ids.len();
    if n == 0 {
        return Vec::new();
    }
    let cell_h = col.h / n as i32;
    let mut out = Vec::with_capacity(n);
    for (i, &id) in ids.iter().enumerate() {
        let y = col.y + i as i32 * cell_h;
        // La última celda absorbe el resto de la división entera.
        let h = if i == n - 1 { col.y + col.h - y } else { cell_h };
        out.push((id, inset(Rect::new(col.x, y, col.w, h), gap)));
    }
    out
}

/// Encoge un rectángulo `gap` píxeles por cada lado (mínimo 1×1).
fn inset(r: Rect, gap: i32) -> Rect {
    Rect::new(
        r.x + gap,
        r.y + gap,
        (r.w - 2 * gap).max(1),
        (r.h - 2 * gap).max(1),
    )
}

export_layout_plugin!(RightMaster);
