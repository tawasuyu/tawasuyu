//! Un plugin de layout de ejemplo: **dwindle** — división recursiva del área.
//!
//! La primera ventana toma la mitad izquierda; la siguiente, la mitad superior
//! del resto; la siguiente, la mitad inferior de *ese* resto; y así, alternando
//! vertical/horizontal. Es una estrategia distinta a las que trae el `Desktop`,
//! y prueba el camino Tier-0 de punta a punta: función pura, **cero
//! importaciones del host**.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

use mirada_plugin_sdk::{export_layout_plugin, LayoutPlugin, Rect, TileInput, WindowId};

#[derive(Default)]
struct Dwindle;

impl LayoutPlugin for Dwindle {
    fn tile(&mut self, input: &TileInput) -> Vec<(WindowId, Rect)> {
        let mut out = Vec::new();
        dwindle(&input.ids, input.work, true, &mut out);
        out
    }
}

/// Reparte `ids` en `rect`, alternando cortes verticales y horizontales.
fn dwindle(ids: &[WindowId], rect: Rect, vertical: bool, out: &mut Vec<(WindowId, Rect)>) {
    match ids.len() {
        0 => {}
        1 => out.push((ids[0], rect)),
        _ => {
            let (head, tail) = if vertical {
                let w = rect.w / 2;
                (
                    Rect::new(rect.x, rect.y, w, rect.h),
                    Rect::new(rect.x + w, rect.y, rect.w - w, rect.h),
                )
            } else {
                let h = rect.h / 2;
                (
                    Rect::new(rect.x, rect.y, rect.w, h),
                    Rect::new(rect.x, rect.y + h, rect.w, rect.h - h),
                )
            };
            out.push((ids[0], head));
            dwindle(&ids[1..], tail, !vertical, out);
        }
    }
}

export_layout_plugin!(Dwindle);
