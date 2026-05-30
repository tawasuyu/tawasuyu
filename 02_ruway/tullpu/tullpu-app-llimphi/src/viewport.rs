//! Geometría del lienzo de la app `tullpu`: transform fit-contain con
//! zoom/pan, zoom-a-cursor, conversiones local↔imagen, normalización del
//! rect de selección y el side-channel `LIENZO_RECT` para el wheel.
//!
//! Behavior-preserving split de `main.rs` — sin cambios funcionales.

use std::sync::{Mutex, OnceLock};

use llimphi_ui::PaintRect;

use crate::model::*;

/// Side-channel para que [`on_wheel`] —que sólo recibe cursor absoluto, no
/// info de layout— pueda saber si el cursor cayó sobre el lienzo. Lo
/// escribe el closure de `paint_with` del lienzo en cada frame; lo lee
/// `on_wheel` antes de despachar. Es lectura-mostly: `Mutex` es OK para
/// los bytes de un `PaintRect` (16 bytes) y evita atomics-por-campo.
pub(crate) static LIENZO_RECT: OnceLock<Mutex<Option<PaintRect>>> = OnceLock::new();

pub(crate) fn lienzo_rect_set(r: PaintRect) {
    let cell = LIENZO_RECT.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = cell.lock() {
        *g = Some(r);
    }
}

pub(crate) fn lienzo_rect_get() -> Option<PaintRect> {
    LIENZO_RECT.get()?.lock().ok().and_then(|g| *g)
}

pub(crate) fn dentro_de_rect(r: PaintRect, cx: f32, cy: f32) -> bool {
    cx >= r.x && cx <= r.x + r.w && cy >= r.y && cy <= r.y + r.h
}

/// Construye el transform para pintar `(image_w, image_h)` dentro de un
/// rect `(rw, rh)` con `factor_zoom` y `pan` aplicados. Devuelve la escala
/// absoluta y el offset top-left del rectángulo destino, ambos en px.
/// Pura — testeable sin gráficos.
pub(crate) fn transform_lienzo(
    image_w: u32,
    image_h: u32,
    rw: f32,
    rh: f32,
    factor_zoom: f32,
    pan_x: f32,
    pan_y: f32,
) -> Option<(f64, f64, f64)> {
    if image_w == 0 || image_h == 0 || rw <= 0.0 || rh <= 0.0 {
        return None;
    }
    let sx = rw as f64 / image_w as f64;
    let sy = rh as f64 / image_h as f64;
    let s_fit = sx.min(sy);
    let s = s_fit * factor_zoom as f64;
    let dw = image_w as f64 * s;
    let dh = image_h as f64 * s;
    let off_x = (rw as f64 - dw) * 0.5 + pan_x as f64;
    let off_y = (rh as f64 - dh) * 0.5 + pan_y as f64;
    Some((s, off_x, off_y))
}

/// Calcula el nuevo `(pan_x, pan_y)` para que el punto de pantalla
/// `(cursor_x, cursor_y)` siga apuntando al mismo píxel-imagen tras
/// cambiar `factor_zoom` de `zoom_old` a `zoom_new`. Devuelve los pans
/// sin tocar si la imagen o el rect son degenerados (división por cero).
/// Pura — testeable sin gráficos.
pub(crate) fn pan_para_zoom_a_cursor(
    image_w: u32,
    image_h: u32,
    rect: PaintRect,
    cursor_x: f32,
    cursor_y: f32,
    zoom_old: f32,
    zoom_new: f32,
    pan_x: f32,
    pan_y: f32,
) -> (f32, f32) {
    let Some((s_old, off_x, off_y)) =
        transform_lienzo(image_w, image_h, rect.w, rect.h, zoom_old, pan_x, pan_y)
    else {
        return (pan_x, pan_y);
    };
    if s_old <= 0.0 || image_w == 0 || image_h == 0 {
        return (pan_x, pan_y);
    }
    // Cursor en coords-imagen bajo el zoom anterior.
    let tx_old = rect.x as f64 + off_x;
    let ty_old = rect.y as f64 + off_y;
    let ix = (cursor_x as f64 - tx_old) / s_old;
    let iy = (cursor_y as f64 - ty_old) / s_old;
    // Nueva escala y nuevo top-left exigido para que (ix, iy) caiga bajo
    // el cursor: tx_new = cursor - ix * s_new.
    let s_fit_w = rect.w as f64 / image_w as f64;
    let s_fit_h = rect.h as f64 / image_h as f64;
    let s_new = s_fit_w.min(s_fit_h) * zoom_new as f64;
    let tx_new = cursor_x as f64 - ix * s_new;
    let ty_new = cursor_y as f64 - iy * s_new;
    let dw_new = image_w as f64 * s_new;
    let dh_new = image_h as f64 * s_new;
    let pan_x_nuevo = (tx_new - rect.x as f64 - (rect.w as f64 - dw_new) * 0.5) as f32;
    let pan_y_nuevo = (ty_new - rect.y as f64 - (rect.h as f64 - dh_new) * 0.5) as f32;
    (pan_x_nuevo, pan_y_nuevo)
}

/// Convierte un click en coords-panel `(lx, ly)` con dims `(rw, rh)` a
/// la posición del píxel-imagen bajo el cursor (aplicando zoom + pan) y
/// devuelve el RGBA de ese píxel del buffer `image_data` (Rgba8 fila por
/// fila). Devuelve `None` si las dims son degeneradas, si el píxel cae
/// fuera de la imagen o si el buffer no tiene tamaño suficiente. Pura.
pub(crate) fn recoger_color_en(
    image_data: &[u8],
    image_w: u32,
    image_h: u32,
    lx: f32,
    ly: f32,
    rw: f32,
    rh: f32,
    factor_zoom: f32,
    pan_x: f32,
    pan_y: f32,
) -> Option<[u8; 4]> {
    let (s, off_x, off_y) =
        transform_lienzo(image_w, image_h, rw, rh, factor_zoom, pan_x, pan_y)?;
    if s <= 0.0 {
        return None;
    }
    let ix = ((lx as f64 - off_x) / s).floor() as i64;
    let iy = ((ly as f64 - off_y) / s).floor() as i64;
    if ix < 0 || iy < 0 {
        return None;
    }
    let (ix, iy) = (ix as u32, iy as u32);
    if ix >= image_w || iy >= image_h {
        return None;
    }
    let stride = image_w as usize * 4;
    let idx = iy as usize * stride + ix as usize * 4;
    if idx + 4 > image_data.len() {
        return None;
    }
    Some([
        image_data[idx],
        image_data[idx + 1],
        image_data[idx + 2],
        image_data[idx + 3],
    ])
}

/// Convierte un punto local `(lx, ly)` (relativo al rect del panel
/// lienzo de tamaño `rw × rh`) a coords-imagen, aplicando el zoom/pan
/// vigentes. Pura. Devuelve `None` si las dims o la escala son
/// degeneradas — el caller suele caer a un no-op en ese caso.
pub(crate) fn local_a_imagen(
    lx: f32,
    ly: f32,
    rw: f32,
    rh: f32,
    image_w: u32,
    image_h: u32,
    factor_zoom: f32,
    pan_x: f32,
    pan_y: f32,
) -> Option<(f64, f64)> {
    let (s, off_x, off_y) =
        transform_lienzo(image_w, image_h, rw, rh, factor_zoom, pan_x, pan_y)?;
    if s <= 0.0 {
        return None;
    }
    Some(((lx as f64 - off_x) / s, (ly as f64 - off_y) / s))
}

/// Normaliza un drag de selección a un `RectImagen` válido: ordena
/// las esquinas, clampea al rect del lienzo `[0, image_w) × [0, image_h)`
/// y devuelve `None` si el rect resulta degenerado (área cero) — el
/// caller suele descartar selecciones puntuales que el usuario no
/// quiso hacer.
pub(crate) fn rect_imagen_desde_drag(
    drag: &SeleccionDrag,
    image_w: u32,
    image_h: u32,
    factor_zoom: f32,
    pan_x: f32,
    pan_y: f32,
) -> Option<RectImagen> {
    let cur = local_a_imagen(
        drag.cur_lx,
        drag.cur_ly,
        drag.rw,
        drag.rh,
        image_w,
        image_h,
        factor_zoom,
        pan_x,
        pan_y,
    )?;
    let ax = drag.ancla_ix as f64;
    let ay = drag.ancla_iy as f64;
    let bx = cur.0;
    let by = cur.1;
    let (lo_x, hi_x) = if ax <= bx { (ax, bx) } else { (bx, ax) };
    let (lo_y, hi_y) = if ay <= by { (ay, by) } else { (by, ay) };
    // Clamp half-open al rect del lienzo. Floor en el min, ceil en el
    // max para incluir cualquier píxel parcial bajo el cursor.
    let x0 = lo_x.floor().clamp(0.0, image_w as f64) as u32;
    let y0 = lo_y.floor().clamp(0.0, image_h as f64) as u32;
    let x1 = hi_x.ceil().clamp(0.0, image_w as f64) as u32;
    let y1 = hi_y.ceil().clamp(0.0, image_h as f64) as u32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(RectImagen { x0, y0, x1, y1 })
}
