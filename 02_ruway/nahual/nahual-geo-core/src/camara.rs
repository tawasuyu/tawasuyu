//! Proyección equirectangular + cámara: `Projection`, hit-test, desenfoque y
//! vuelo a feature. Todo lo que convierte lon/lat ↔ píxeles de pantalla.

use crate::geom::{dist_point_seg, point_in_ring_screen};
use crate::tipos::{BBox, Coord, MapData, MapView};

/// Proyección equirectangular fit-to-bounds + cámara (zoom/pan). Encapsula la
/// matemática para que el render (canvas) y el hit-test (clic) coincidan
/// exactamente — si difirieran, el clic seleccionaría la feature equivocada.
pub struct Projection {
    pub kx: f64,
    pub scale: f64,
    pub ox: f64,
    pub oy: f64,
    pub pmin_x: f64,
    pub max_lat: f64,
    pub pivot_x: f64,
    pub pivot_y: f64,
    pub zoom: f64,
    pub pan: (f64, f64),
}

impl Projection {
    /// Encaja `bb` en `rect` (`x, y, w, h`, físicos) con escala uniforme y la
    /// cámara dada.
    pub fn fit(bb: BBox, rect: (f64, f64, f64, f64), zoom: f64, pan: (f64, f64)) -> Self {
        let (rx, ry, rw, rh) = rect;
        let lat0 = (bb.min_lat + bb.max_lat) * 0.5;
        let kx = lat0.to_radians().cos().abs().max(0.05);
        let pmin_x = bb.min_lon * kx;
        let pw = (bb.max_lon * kx - pmin_x).max(0.0);
        let ph = (bb.max_lat - bb.min_lat).max(0.0);
        let inset = 6.0_f64;
        let aw = (rw - 2.0 * inset).max(1.0);
        let ah = (rh - 2.0 * inset).max(1.0);
        let sx = if pw > 1e-12 { aw / pw } else { f64::INFINITY };
        let sy = if ph > 1e-12 { ah / ph } else { f64::INFINITY };
        let scale = sx.min(sy).min(1.0e6);
        let scale = if scale.is_finite() { scale } else { 1.0 };
        Projection {
            kx,
            scale,
            ox: rx + inset + (aw - pw * scale) * 0.5,
            oy: ry + inset + (ah - ph * scale) * 0.5,
            pmin_x,
            max_lat: bb.max_lat,
            pivot_x: rx + rw * 0.5,
            pivot_y: ry + rh * 0.5,
            zoom,
            pan,
        }
    }

    /// lon/lat → coordenadas de pantalla **antes** de la cámara (fit puro).
    /// Independiente de zoom/pan, base para centrar/encuadrar.
    pub fn base(&self, [lon, lat]: Coord) -> (f64, f64) {
        (
            self.ox + (lon * self.kx - self.pmin_x) * self.scale,
            self.oy + (self.max_lat - lat) * self.scale,
        )
    }

    /// lon/lat → pantalla (Y invertida), pasando por la cámara.
    pub fn to_screen(&self, c: Coord) -> (f64, f64) {
        let (bx, by) = self.base(c);
        (
            self.pivot_x + (bx - self.pivot_x) * self.zoom + self.pan.0,
            self.pivot_y + (by - self.pivot_y) * self.zoom + self.pan.1,
        )
    }

    /// pantalla → lon/lat (inverso exacto de [`to_screen`]).
    pub fn inverse(&self, sx: f64, sy: f64) -> Coord {
        let bx = self.pivot_x + (sx - self.pivot_x - self.pan.0) / self.zoom;
        let by = self.pivot_y + (sy - self.pivot_y - self.pan.1) / self.zoom;
        let lon = ((bx - self.ox) / self.scale + self.pmin_x) / self.kx;
        let lat = self.max_lat - (by - self.oy) / self.scale;
        [lon, lat]
    }
}

// ─── Hit-test ────────────────────────────────────────────────────────────────

/// Resuelve qué feature cae bajo un clic. `(fx, fy)` es la posición del clic
/// como fracción `[0, 1]` del rect del canvas (DPI-independiente). Devuelve el
/// índice en `data.features`, o `None` si el clic no toca ninguna geometría.
///
/// Prioridad: puntos > líneas > polígonos (lo más específico primero). Todo
/// en espacio de pantalla con la misma [`Projection`] que el render, así el
/// hit coincide con lo que se ve.
pub fn hit_test(data: &MapData, view: &MapView, fx: f64, fy: f64) -> Option<usize> {
    let (rx, ry, rw, rh) = view.rect.lock().ok().and_then(|g| *g)?;
    if rw <= 0.0 || rh <= 0.0 {
        return None;
    }
    let bb = data.bbox()?;
    let proj = Projection::fit(
        bb,
        (rx as f64, ry as f64, rw as f64, rh as f64),
        view.zoom,
        view.pan,
    );
    let cx = rx as f64 + fx * rw as f64;
    let cy = ry as f64 + fy * rh as f64;
    let tol = 7.0_f64;

    for (i, p) in data.points.iter().enumerate() {
        let (sx, sy) = proj.to_screen(*p);
        if (sx - cx).hypot(sy - cy) <= tol + 3.0 {
            return data.point_feat.get(i).copied();
        }
    }
    for (li, line) in data.lines.iter().enumerate() {
        for w in line.windows(2) {
            if dist_point_seg(cx, cy, proj.to_screen(w[0]), proj.to_screen(w[1])) <= tol {
                return data.line_feat.get(li).copied();
            }
        }
    }
    for (pi, poly) in data.polygons.iter().enumerate() {
        if let Some(outer) = poly.first() {
            if point_in_ring_screen(cx, cy, outer, &proj) {
                return data.polygon_feat.get(pi).copied();
            }
        }
    }
    None
}

// ─── Utilidades de cámara ────────────────────────────────────────────────────

/// Convierte un clic (fracción `[0,1]` del rect) a lon/lat, invirtiendo la
/// proyección actual. `None` si todavía no se pintó o no hay datos.
pub fn unproject(data: &MapData, view: &MapView, fx: f64, fy: f64) -> Option<Coord> {
    let (rx, ry, rw, rh) = view.rect.lock().ok().and_then(|g| *g)?;
    if rw <= 0.0 || rh <= 0.0 {
        return None;
    }
    let bb = data.bbox()?;
    let proj = Projection::fit(
        bb,
        (rx as f64, ry as f64, rw as f64, rh as f64),
        view.zoom,
        view.pan,
    );
    Some(proj.inverse(rx as f64 + fx * rw as f64, ry as f64 + fy * rh as f64))
}

/// Caja envolvente de las geometrías de una feature (por su índice).
pub fn feature_bbox(data: &MapData, fi: usize) -> Option<BBox> {
    let mut bb = BBox::empty();
    for (i, p) in data.points.iter().enumerate() {
        if data.point_feat.get(i) == Some(&fi) {
            bb.expand(*p);
        }
    }
    for (i, l) in data.lines.iter().enumerate() {
        if data.line_feat.get(i) == Some(&fi) {
            for c in l {
                bb.expand(*c);
            }
        }
    }
    for (i, poly) in data.polygons.iter().enumerate() {
        if data.polygon_feat.get(i) == Some(&fi) {
            for ring in poly {
                for c in ring {
                    bb.expand(*c);
                }
            }
        }
    }
    (!bb.is_empty()).then_some(bb)
}

/// Centra y encuadra la cámara sobre una feature (vuelo a resultado de
/// búsqueda), y la deja seleccionada. La feature ocupa ~60% del panel; un
/// punto suelto usa un zoom fijo cómodo. No-op si no hay rect/datos.
pub fn focus_on(data: &MapData, view: &mut MapView, fi: usize) {
    let Some((rx, ry, rw, rh)) = view.rect.lock().ok().and_then(|g| *g) else {
        view.selected = Some(fi);
        return;
    };
    let (Some(bb), Some(fbb)) = (data.bbox(), feature_bbox(data, fi)) else {
        view.selected = Some(fi);
        return;
    };
    let proj = Projection::fit(
        bb,
        (rx as f64, ry as f64, rw as f64, rh as f64),
        1.0,
        (0.0, 0.0),
    );
    let (x0, y0) = proj.base([fbb.min_lon, fbb.max_lat]);
    let (x1, y1) = proj.base([fbb.max_lon, fbb.min_lat]);
    let fw = (x1 - x0).abs();
    let fh = (y1 - y0).abs();
    let degenerate = fw < 1e-6 && fh < 1e-6;
    let zoom = if degenerate {
        8.0
    } else {
        (0.6 * (rw as f64 / fw.max(1e-6)).min(rh as f64 / fh.max(1e-6)))
            .clamp(MapView::ZOOM_MIN, MapView::ZOOM_MAX)
    };
    let target = [
        (fbb.min_lon + fbb.max_lon) * 0.5,
        (fbb.min_lat + fbb.max_lat) * 0.5,
    ];
    let (bx, by) = proj.base(target);
    view.zoom = zoom;
    // pan que lleva el centro de la feature al centro del panel.
    view.pan = (-(bx - proj.pivot_x) * zoom, -(by - proj.pivot_y) * zoom);
    view.selected = Some(fi);
}
