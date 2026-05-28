//! `pineal-hexbin` — bineado hexagonal de scatter densos.
//!
//! Cuando un scatter tiene tantos puntos que la nube tapa el patrón,
//! binear sobre una rejilla hexagonal pointy-top y colorear por
//! densidad es la forma estándar de revelar la distribución. La
//! topología hexagonal evita los "estrías" rectangulares de un binning
//! cuadrado y respeta mejor la isotropía de la nube.
//!
//! - [`HexGrid`] — calcula el bin de cada `(x, y)` con la rejilla
//!   pointy-top (hex con dos vértices arriba/abajo). Cell size = radio
//!   del circumcírculo.
//! - [`paint_hexbin`] — pinta los bins no vacíos como hexágonos
//!   rellenos coloreados por densidad vía `pineal_heatmap::Ramp`.

#![forbid(unsafe_code)]

use pineal_heatmap::Ramp;
use pineal_render::{Canvas, Color, Rect};
use std::collections::HashMap;

/// Rejilla hexagonal pointy-top con bines indexados por `(col, row)`
/// según el offset-coord clásico ("odd-r" en la nomenclatura de Red
/// Blob Games). El bineo es determinista y O(n) sobre los puntos.
#[derive(Debug, Clone)]
pub struct HexGrid {
    radius: f32,
    counts: HashMap<(i32, i32), u32>,
}

impl HexGrid {
    /// Construye una rejilla vacía con celda de `radius` pixels.
    pub fn new(radius: f32) -> Self {
        Self { radius: radius.max(1e-3), counts: HashMap::new() }
    }

    /// Agrega un punto. Incrementa el bin correspondiente.
    pub fn push(&mut self, x: f32, y: f32) {
        let cell = pixel_to_oddr(x, y, self.radius);
        *self.counts.entry(cell).or_insert(0) += 1;
    }

    /// Construye y popula a partir de un slice interleaved `[x0,y0,x1,y1…]`.
    pub fn from_xy(radius: f32, xy: &[f32]) -> Self {
        let mut g = Self::new(radius);
        for chunk in xy.chunks_exact(2) {
            g.push(chunk[0], chunk[1]);
        }
        g
    }

    pub fn radius(&self) -> f32 {
        self.radius
    }

    pub fn cells(&self) -> impl Iterator<Item = ((i32, i32), u32)> + '_ {
        self.counts.iter().map(|(k, v)| (*k, *v))
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// `(min, max)` de cuentas. `(0, 0)` si está vacía.
    pub fn min_max(&self) -> (u32, u32) {
        let mut it = self.counts.values().copied();
        let Some(first) = it.next() else { return (0, 0) };
        it.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v)))
    }
}

/// Convierte pixel `(x, y)` a coords offset `(col, row)` odd-r.
/// Algoritmo: pasar a axial (q, r) por pixel→axial pointy-top, luego
/// redondear vía cube-coordinate-rounding, luego axial→oddr.
fn pixel_to_oddr(x: f32, y: f32, radius: f32) -> (i32, i32) {
    let sqrt3 = 3.0_f32.sqrt();
    let q = (sqrt3 / 3.0 * x - y / 3.0) / radius;
    let r = (2.0 / 3.0 * y) / radius;
    let (qr, rr) = cube_round(q, r);
    let col = qr + (rr - (rr & 1)) / 2;
    (col, rr)
}

fn cube_round(q: f32, r: f32) -> (i32, i32) {
    let x = q;
    let z = r;
    let y = -x - z;
    let mut rx = x.round();
    let ry = y.round();
    let mut rz = z.round();
    let dx = (rx - x).abs();
    let dy = (ry - y).abs();
    let dz = (rz - z).abs();
    if dx > dy && dx > dz {
        rx = -ry - rz;
    } else if dz > dy {
        rz = -rx - ry;
    }
    // En el caso restante (dy es el peor), el output sólo usa rx y rz —
    // no hace falta ajustar ry. La invariante rx+ry+rz=0 no nos importa
    // porque ry no se devuelve.
    (rx as i32, rz as i32)
}

/// Convierte offset `(col, row)` odd-r a centro pixel.
pub fn oddr_to_pixel(col: i32, row: i32, radius: f32) -> (f32, f32) {
    let sqrt3 = 3.0_f32.sqrt();
    let x = radius * sqrt3 * (col as f32 + 0.5 * (row & 1) as f32);
    let y = radius * 3.0 / 2.0 * row as f32;
    (x, y)
}

/// Dibuja todos los bines de `grid` sobre `canvas`, mapeando densidad a
/// color con `ramp`. `offset` desplaza la grilla (se suma al centro de
/// cada hex). El alto y ancho del rect destino los decide el caller —
/// los hexes se dibujan en sus coords absolutas, no se reescalan.
pub fn paint_hexbin(
    grid: &HexGrid,
    ramp: Ramp,
    offset: (f32, f32),
    canvas: &mut dyn Canvas,
) {
    if grid.is_empty() {
        return;
    }
    let (min, max) = grid.min_max();
    let span = (max - min) as f32;
    let r = grid.radius();

    for ((col, row), count) in grid.cells() {
        let (cx, cy) = oddr_to_pixel(col, row, r);
        let cx = cx + offset.0;
        let cy = cy + offset.1;
        let t = if span > 0.0 {
            (count - min) as f32 / span
        } else {
            0.0
        };
        let color = ramp.sample(t);
        paint_hex(cx, cy, r, color, canvas);
    }
}

/// Hexágono pointy-top centrado en `(cx, cy)` con circumcircle `r`.
/// Se tesselan los 6 triángulos del fan compartiendo el centro.
fn paint_hex(cx: f32, cy: f32, r: f32, color: Color, canvas: &mut dyn Canvas) {
    use std::f32::consts::{FRAC_PI_3, FRAC_PI_2};
    // Fan: [center, v0, center, v1, ..., center, v0].
    let mut coords = Vec::with_capacity(14 * 2);
    let mut colors = Vec::with_capacity(14);
    for i in 0..=6 {
        let a = -FRAC_PI_2 + i as f32 * FRAC_PI_3;
        coords.push(cx);
        coords.push(cy);
        coords.push(cx + r * a.cos());
        coords.push(cy + r * a.sin());
        colors.push(color);
        colors.push(color);
    }
    canvas.fill_triangle_strip(&coords, &colors);
}

/// `Rect` con todos los hexes — útil para que el caller dimensione el
/// viewport antes de pintar.
pub fn bounds(grid: &HexGrid) -> Option<Rect> {
    if grid.is_empty() {
        return None;
    }
    let r = grid.radius();
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for ((col, row), _) in grid.cells() {
        let (cx, cy) = oddr_to_pixel(col, row, r);
        min_x = min_x.min(cx - r);
        max_x = max_x.max(cx + r);
        min_y = min_y.min(cy - r);
        max_y = max_y.max(cy + r);
    }
    Some(Rect::new(min_x, min_y, max_x - min_x, max_y - min_y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pineal_render::{PlanRecorder, RenderCmd};

    #[test]
    fn empty_grid_is_noop() {
        let g = HexGrid::new(10.0);
        let mut rec = PlanRecorder::new();
        paint_hexbin(&g, Ramp::Viridis, (0.0, 0.0), &mut rec);
        assert!(rec.into_plan().cmds.is_empty());
    }

    #[test]
    fn coincident_points_bin_together() {
        let mut g = HexGrid::new(5.0);
        g.push(10.0, 10.0);
        g.push(10.0, 10.0);
        g.push(10.0, 10.0);
        assert_eq!(g.cells().count(), 1);
        let (_, count) = g.cells().next().unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn far_points_bin_separately() {
        let mut g = HexGrid::new(5.0);
        g.push(0.0, 0.0);
        g.push(1000.0, 1000.0);
        assert_eq!(g.cells().count(), 2);
    }

    #[test]
    fn paint_emits_one_strip_per_cell() {
        let mut g = HexGrid::new(8.0);
        g.push(0.0, 0.0);
        g.push(100.0, 100.0);
        g.push(50.0, 50.0);
        let mut rec = PlanRecorder::new();
        paint_hexbin(&g, Ramp::Viridis, (0.0, 0.0), &mut rec);
        let strips = rec
            .into_plan()
            .cmds
            .iter()
            .filter(|c| matches!(c, RenderCmd::FillTriangleStrip { .. }))
            .count();
        assert_eq!(strips, 3);
    }

    #[test]
    fn from_xy_populates_correctly() {
        let xy = [0.0, 0.0, 100.0, 100.0, 100.0, 100.0];
        let g = HexGrid::from_xy(5.0, &xy);
        assert_eq!(g.cells().count(), 2);
        let (_, max) = g.min_max();
        assert_eq!(max, 2);
    }

    #[test]
    fn bounds_covers_extremes() {
        let mut g = HexGrid::new(5.0);
        g.push(0.0, 0.0);
        g.push(100.0, 50.0);
        let b = bounds(&g).unwrap();
        assert!(b.w > 0.0 && b.h > 0.0);
    }
}
