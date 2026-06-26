//! Plugin de layout **tres columnas** (centered-master con dos pilas): la(s)
//! ventana(s) maestra(s) ocupan una columna central ancha y el resto se reparte
//! en dos pilas, una a cada lado. Ideal para monitores anchos/ultrawide, donde
//! una sola pila desperdicia el ancho.
//!
//! Honra los `LayoutParams` vigentes del `Desktop`: `master_ratio` fija el ancho
//! de la columna central, `master_count` cuántas ventanas van al centro, `gap`
//! el margen. Con `n <= master_count` cae a una sola columna centrada. El resto
//! se alterna entre la pila derecha (índices pares) y la izquierda (impares),
//! para que se llenen parejas. Es `CAP_LAYOUT`: función pura, sin importar nada.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use mirada_plugin_sdk::{export_layout_plugin, LayoutPlugin, Rect, TileInput, WindowId};

#[derive(Default)]
struct ThreeColumn;

/// Encoge un rect por `gap` en cada lado (mínimo 1×1).
fn inset(r: Rect, gap: i32) -> Rect {
    Rect::new(r.x + gap, r.y + gap, (r.w - 2 * gap).max(1), (r.h - 2 * gap).max(1))
}

/// Apila `ids` en columna dentro de `rect`, repartiendo el alto en partes
/// iguales (la última toma el sobrante), con `gap` de margen por celda.
fn stack_column(ids: &[WindowId], rect: Rect, gap: i32, out: &mut Vec<(WindowId, Rect)>) {
    let n = ids.len() as i32;
    if n == 0 {
        return;
    }
    let h = rect.h / n;
    for (i, &id) in ids.iter().enumerate() {
        let i = i as i32;
        let y = rect.y + i * h;
        let cell_h = if i == n - 1 { rect.y + rect.h - y } else { h };
        out.push((id, inset(Rect::new(rect.x, y, rect.w, cell_h), gap)));
    }
}

impl LayoutPlugin for ThreeColumn {
    fn tile(&mut self, input: &TileInput) -> Vec<(WindowId, Rect)> {
        let ids = &input.ids;
        let work = input.work;
        let gap = input.params.gap.max(0);
        let n = ids.len();
        let mut out = Vec::with_capacity(n);
        if n == 0 {
            return out;
        }
        let nmaster = (input.params.master_count.max(1) as usize).min(n);
        // Con todo en la maestra, una sola columna central a pantalla completa.
        if n <= nmaster {
            stack_column(ids, work, gap, &mut out);
            return out;
        }
        let ratio = input.params.master_ratio.clamp(0.05, 0.95);
        let master_w = (work.w as f32 * ratio) as i32;
        let side = (work.w - master_w).max(2);
        let side_l = side / 2;
        let side_r = side - side_l;
        let left_rect = Rect::new(work.x, work.y, side_l, work.h);
        let master_rect = Rect::new(work.x + side_l, work.y, master_w, work.h);
        let right_rect = Rect::new(work.x + side_l + master_w, work.y, side_r, work.h);

        stack_column(&ids[..nmaster], master_rect, gap, &mut out);

        // El resto se alterna: pares → pila derecha, impares → izquierda.
        let mut left: Vec<WindowId> = Vec::new();
        let mut right: Vec<WindowId> = Vec::new();
        for (i, &id) in ids[nmaster..].iter().enumerate() {
            if i % 2 == 0 {
                right.push(id);
            } else {
                left.push(id);
            }
        }
        stack_column(&left, left_rect, gap, &mut out);
        stack_column(&right, right_rect, gap, &mut out);
        out
    }
}

export_layout_plugin!(ThreeColumn);
