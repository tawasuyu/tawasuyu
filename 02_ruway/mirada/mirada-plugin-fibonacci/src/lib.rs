//! Plugin de layout **fibonacci** (espiral áurea): como el `dwindle` BSP, parte
//! recursivamente el eje más largo del área restante y le da una pieza a cada
//! ventana — pero el corte es la **razón áurea** φ⁻¹ ≈ 0.618 en vez de la mitad.
//! Da la clásica espiral de Fibonacci: la maestra grande arriba-izquierda y las
//! demás encogiendo en espiral.
//!
//! La última ventana toma todo el resto. `gap` viene de los `LayoutParams`. Es
//! `CAP_LAYOUT`: función pura, sin importar nada del host.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use mirada_plugin_sdk::{export_layout_plugin, LayoutPlugin, Rect, TileInput, WindowId};

/// Razón áurea inversa: φ⁻¹ = (√5 − 1) / 2.
const GOLDEN: f32 = 0.618_034;

#[derive(Default)]
struct Fibonacci;

/// Encoge un rect por `gap` en cada lado (mínimo 1×1).
fn inset(r: Rect, gap: i32) -> Rect {
    Rect::new(r.x + gap, r.y + gap, (r.w - 2 * gap).max(1), (r.h - 2 * gap).max(1))
}

/// Parte el **eje más largo** de `area` en la razón `ratio`: devuelve
/// `(la pieza para esta ventana, el resto)`.
fn split_long(area: Rect, ratio: f32) -> (Rect, Rect) {
    if area.w >= area.h {
        let w0 = ((area.w as f32 * ratio) as i32).clamp(1, area.w.max(1));
        (
            Rect::new(area.x, area.y, w0, area.h),
            Rect::new(area.x + w0, area.y, (area.w - w0).max(1), area.h),
        )
    } else {
        let h0 = ((area.h as f32 * ratio) as i32).clamp(1, area.h.max(1));
        (
            Rect::new(area.x, area.y, area.w, h0),
            Rect::new(area.x, area.y + h0, area.w, (area.h - h0).max(1)),
        )
    }
}

impl LayoutPlugin for Fibonacci {
    fn tile(&mut self, input: &TileInput) -> Vec<(WindowId, Rect)> {
        let ids = &input.ids;
        let gap = input.params.gap.max(0);
        let n = ids.len();
        let mut out = Vec::with_capacity(n);
        if n == 0 {
            return out;
        }
        let mut area = input.work;
        for (i, &id) in ids.iter().enumerate() {
            if i == n - 1 {
                out.push((id, inset(area, gap)));
                break;
            }
            let (mine, rest) = split_long(area, GOLDEN);
            out.push((id, inset(mine, gap)));
            area = rest;
        }
        out
    }
}

export_layout_plugin!(Fibonacci);
