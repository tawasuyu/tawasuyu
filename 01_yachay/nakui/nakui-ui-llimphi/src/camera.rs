//! Cámara de la vista grafo: zoom + pan sobre el lienzo de morfismos.
//!
//! El widget `llimphi-widget-nodegraph` no tiene transform propio —cada
//! nodo se posiciona con un `inset` de taffy en coords del lienzo—, así
//! que la cámara vive en el caller (`panels::build_graph_panel`):
//! transformamos las posiciones de nodo (mundo → pantalla) y escalamos
//! las métricas antes de pasarlas al widget.
//!
//! Convención: `pantalla = mundo * zoom + pan`, donde `pan` está en coords
//! locales al rect del lienzo (las mismas que los `inset` de los nodos) y
//! `mundo` son las coords del auto-layout topológico. Todas las funciones
//! de geometría son puras y testeables sin gráficos; el único estado es el
//! side-channel `CANVAS_RECT`.

use std::sync::{Mutex, OnceLock};

use llimphi_ui::PaintRect;

/// Paso multiplicativo de un "click" de rueda. El zoom de un notch es
/// `ZOOM_BASE`; los botones +/− aplican varios pasos de una.
pub(crate) const ZOOM_BASE: f32 = 1.1;
pub(crate) const ZOOM_MIN: f32 = 0.2;
pub(crate) const ZOOM_MAX: f32 = 4.0;
/// Salto de los botones +/− (≈ ZOOM_BASE³).
pub(crate) const ZOOM_STEP: f32 = 1.331;

/// Side-channel para que `on_wheel` —que sólo recibe el cursor en coords de
/// ventana, sin info de layout— sepa dónde cayó el lienzo del grafo. Lo
/// escribe el `paint_with` de fondo del lienzo en cada frame; lo leen
/// `on_wheel` y los handlers de `ZoomGraph`/`FitGraph`. Lectura-mostly, un
/// `Mutex` sobre 16 bytes alcanza.
static CANVAS_RECT: OnceLock<Mutex<Option<PaintRect>>> = OnceLock::new();

pub(crate) fn canvas_rect_set(r: PaintRect) {
    let cell = CANVAS_RECT.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = cell.lock() {
        *g = Some(r);
    }
}

pub(crate) fn canvas_rect_get() -> Option<PaintRect> {
    CANVAS_RECT.get()?.lock().ok().and_then(|g| *g)
}

pub(crate) fn dentro_de_rect(r: PaintRect, cx: f32, cy: f32) -> bool {
    cx >= r.x && cx <= r.x + r.w && cy >= r.y && cy <= r.y + r.h
}

/// Nuevo `pan` para que el punto-mundo que está bajo `cursor` siga bajo el
/// cursor tras cambiar el zoom de `zoom_old` a `zoom_new`. `cursor` es en
/// coords de ventana; `rect` es el lienzo (para pasar a coords locales).
/// Pura.
pub(crate) fn pan_para_zoom_a_cursor(
    rect: PaintRect,
    cursor: (f32, f32),
    zoom_old: f32,
    zoom_new: f32,
    pan: (f32, f32),
) -> (f32, f32) {
    // Cursor en coords locales al lienzo.
    let lx = cursor.0 - rect.x;
    let ly = cursor.1 - rect.y;
    // Punto-mundo bajo el cursor con el zoom anterior.
    let wx = (lx - pan.0) / zoom_old;
    let wy = (ly - pan.1) / zoom_old;
    // Pan que mantiene (wx, wy) bajo (lx, ly) con el zoom nuevo.
    (lx - wx * zoom_new, ly - wy * zoom_new)
}

/// `(zoom, pan)` que encuadra el bounding-box mundo `[min, max]` dentro de
/// `rect` con un margen de aire. Devuelve `None` si el contenido o el rect
/// son degenerados. Pura.
pub(crate) fn fit_to_view(
    rect: PaintRect,
    min: (f32, f32),
    max: (f32, f32),
) -> Option<(f32, (f32, f32))> {
    let cw = max.0 - min.0;
    let ch = max.1 - min.1;
    if cw <= 1.0 || ch <= 1.0 || rect.w <= 1.0 || rect.h <= 1.0 {
        return None;
    }
    // 0.92 deja un margen alrededor del contenido encuadrado.
    let z = ((rect.w / cw).min(rect.h / ch) * 0.92).clamp(ZOOM_MIN, ZOOM_MAX);
    // Centrar el contenido: pan = centro_lienzo_local − z · centro_mundo.
    let cx_world = (min.0 + max.0) * 0.5;
    let cy_world = (min.1 + max.1) * 0.5;
    let pan_x = rect.w * 0.5 - z * cx_world;
    let pan_y = rect.h * 0.5 - z * cy_world;
    Some((z, (pan_x, pan_y)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> PaintRect {
        PaintRect { x: 100.0, y: 50.0, w: 800.0, h: 600.0 }
    }

    #[test]
    fn zoom_a_cursor_fija_el_punto_bajo_el_cursor() {
        let r = rect();
        let cursor = (300.0, 200.0); // ventana
        let pan = (10.0, 20.0);
        let z_old = 1.0;
        let z_new = 2.0;
        // Punto-mundo bajo el cursor antes.
        let lx = cursor.0 - r.x;
        let ly = cursor.1 - r.y;
        let wx = (lx - pan.0) / z_old;
        let wy = (ly - pan.1) / z_old;
        let pan2 = pan_para_zoom_a_cursor(r, cursor, z_old, z_new, pan);
        // El mismo punto-mundo debe proyectar a la misma posición local.
        let lx2 = wx * z_new + pan2.0;
        let ly2 = wy * z_new + pan2.1;
        assert!((lx2 - lx).abs() < 1e-3, "x se movió: {lx2} vs {lx}");
        assert!((ly2 - ly).abs() < 1e-3, "y se movió: {ly2} vs {ly}");
    }

    #[test]
    fn fit_centra_y_clampa() {
        let r = rect();
        // Contenido pequeño → zoom topado en ZOOM_MAX.
        let fit = fit_to_view(r, (0.0, 0.0), (10.0, 10.0)).unwrap();
        assert!((fit.0 - ZOOM_MAX).abs() < 1e-6);
        // Contenido que cabe holgado → centrado.
        let (z, pan) = fit_to_view(r, (0.0, 0.0), (400.0, 300.0)).unwrap();
        let cx = 200.0 * z + pan.0;
        let cy = 150.0 * z + pan.1;
        assert!((cx - r.w * 0.5).abs() < 1e-3, "centro x: {cx}");
        assert!((cy - r.h * 0.5).abs() < 1e-3, "centro y: {cy}");
    }

    #[test]
    fn fit_degenerado_es_none() {
        let r = rect();
        assert!(fit_to_view(r, (0.0, 0.0), (0.0, 0.0)).is_none());
        let degen = PaintRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        assert!(fit_to_view(degen, (0.0, 0.0), (100.0, 100.0)).is_none());
    }
}
