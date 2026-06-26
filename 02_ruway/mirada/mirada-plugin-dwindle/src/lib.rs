//! Plugin de layout **dwindle**: división binaria recursiva (BSP) al estilo
//! Hyprland. Cada ventana parte el área restante por su **eje más largo** —el
//! foco recibe la primera mitad, el resto se subdivide igual—, generando el
//! clásico espiral que se acomoda solo a cualquier número de ventanas y a la
//! proporción de la pantalla (en un monitor apaisado el primer corte es
//! vertical; en uno vertical, horizontal).
//!
//! Honra `master_ratio` en la **primera** partición —así `Super+h`/`Super+l`
//! (encoger/agrandar maestra) gobiernan el tamaño de la ventana principal—; las
//! siguientes parten 50/50. Honra `gap` como margen entre celdas.
//!
//! A diferencia del master-stack (una columna fija de pila), dwindle no tiene
//! "pila": el espacio se reparte jerárquicamente, ideal para acomodar muchas
//! ventanas sin que ninguna quede como una rendija.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use mirada_plugin_sdk::{export_layout_plugin, LayoutPlugin, Rect, TileInput, WindowId};

#[derive(Default)]
struct Dwindle;

impl LayoutPlugin for Dwindle {
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
            // La última ventana se queda con todo el área restante.
            if i == n - 1 {
                out.push((id, inset(area, gap)));
                break;
            }
            // La primera partición usa master_ratio; el resto, mitades.
            let ratio = if i == 0 {
                input.params.master_ratio.clamp(0.05, 0.95)
            } else {
                0.5
            };
            let (mine, rest) = split(area, ratio);
            out.push((id, inset(mine, gap)));
            area = rest;
        }
        out
    }
}

/// Parte `area` por su eje más largo: la primera porción mide `ratio` del eje,
/// la segunda el resto. Devuelve `(porción, resto)`.
fn split(area: Rect, ratio: f32) -> (Rect, Rect) {
    if area.w >= area.h {
        let w0 = ((area.w as f32) * ratio) as i32;
        let w0 = w0.clamp(1, (area.w - 1).max(1));
        (
            Rect::new(area.x, area.y, w0, area.h),
            Rect::new(area.x + w0, area.y, area.w - w0, area.h),
        )
    } else {
        let h0 = ((area.h as f32) * ratio) as i32;
        let h0 = h0.clamp(1, (area.h - 1).max(1));
        (
            Rect::new(area.x, area.y, area.w, h0),
            Rect::new(area.x, area.y + h0, area.w, area.h - h0),
        )
    }
}

/// Encoge un rectángulo `gap` píxeles por cada lado (mínimo 1×1).
fn inset(r: Rect, gap: i32) -> Rect {
    Rect::new(r.x + gap, r.y + gap, (r.w - 2 * gap).max(1), (r.h - 2 * gap).max(1))
}

export_layout_plugin!(Dwindle);
