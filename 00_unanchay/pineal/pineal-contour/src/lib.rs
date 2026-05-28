//! `pineal-contour` — isolíneas vía marching squares.
//!
//! Dada una `HeatmapMatrix` y un nivel `iso`, extrae los segmentos
//! donde la función vale exactamente `iso` aproximada por interpolación
//! lineal en los bordes de cada celda. Compone con `pineal-cartesian`
//! para mapas topográficos, líneas de presión, equipotenciales, etc.
//!
//! - [`extract_contour`] — un nivel → `Vec` de segmentos `[x0,y0,x1,y1]`.
//! - [`extract_contours`] — N niveles equiespaciados.
//! - [`paint_contours`] — pinta cada nivel como polilínea contra `Canvas`.
//!
//! Implementación: marching squares clásico — para cada celda 2×2 se
//! computa un índice 4-bit a partir del signo de cada esquina respecto
//! a `iso`; un lookup table de 16 casos da los 0/1/2 segmentos. Sin
//! ambigüedad resolution (los casos 5 y 10 saddle se rompen siempre
//! del mismo lado — suficiente para charts).

#![forbid(unsafe_code)]

use pineal_heatmap::HeatmapMatrix;
use pineal_render::{Canvas, Color, Rect, StrokeStyle};

/// Un segmento de contorno: 4 floats interleaved `[x0, y0, x1, y1]`.
pub type Segment = [f32; 4];

/// Extrae los segmentos del nivel `iso` sobre `matrix`. Los segmentos
/// están en coords de la matriz (celda `(x, y)` ocupa el cuadrado
/// `[x..x+1] × [y..y+1]`). El caller mapea a pixels con un
/// [`pineal-cartesian`] viewport o equivalente.
pub fn extract_contour(matrix: &HeatmapMatrix, iso: f32) -> Vec<Segment> {
    let w = matrix.width();
    let h = matrix.height();
    if w < 2 || h < 2 {
        return Vec::new();
    }
    let mut segs = Vec::new();
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            // Esquinas: nw=00, ne=10, se=11, sw=01 (clockwise desde top-left).
            let nw = matrix.get(x, y);
            let ne = matrix.get(x + 1, y);
            let se = matrix.get(x + 1, y + 1);
            let sw = matrix.get(x, y + 1);
            let mut idx = 0u8;
            if nw >= iso { idx |= 1; }
            if ne >= iso { idx |= 2; }
            if se >= iso { idx |= 4; }
            if sw >= iso { idx |= 8; }
            if idx == 0 || idx == 15 {
                continue;
            }
            // Interpolación lineal sobre cada borde activo.
            let xf = x as f32;
            let yf = y as f32;
            let top = (xf + lerp_t(nw, ne, iso), yf);
            let right = (xf + 1.0, yf + lerp_t(ne, se, iso));
            let bot = (xf + lerp_t(sw, se, iso), yf + 1.0);
            let left = (xf, yf + lerp_t(nw, sw, iso));
            // Lookup table de marching squares — 16 casos.
            match idx {
                1 | 14 => push(&mut segs, top, left),
                2 | 13 => push(&mut segs, top, right),
                3 | 12 => push(&mut segs, left, right),
                4 | 11 => push(&mut segs, right, bot),
                5 => {
                    push(&mut segs, top, left);
                    push(&mut segs, right, bot);
                }
                6 | 9 => push(&mut segs, top, bot),
                7 | 8 => push(&mut segs, left, bot),
                10 => {
                    push(&mut segs, top, right);
                    push(&mut segs, left, bot);
                }
                _ => {}
            }
        }
    }
    segs
}

fn lerp_t(a: f32, b: f32, iso: f32) -> f32 {
    let d = b - a;
    if d.abs() < 1e-9 {
        return 0.5;
    }
    ((iso - a) / d).clamp(0.0, 1.0)
}

fn push(segs: &mut Vec<Segment>, a: (f32, f32), b: (f32, f32)) {
    segs.push([a.0, a.1, b.0, b.1]);
}

/// Extrae N niveles equiespaciados entre `min` y `max` de la matriz
/// (exclusivos en ambos extremos: el primer nivel está en `min + step`,
/// el último en `max - step`).
pub fn extract_contours(matrix: &HeatmapMatrix, n_levels: usize) -> Vec<(f32, Vec<Segment>)> {
    if n_levels == 0 {
        return Vec::new();
    }
    let (min, max) = matrix.min_max();
    if (max - min).abs() < 1e-9 {
        return Vec::new();
    }
    let step = (max - min) / (n_levels + 1) as f32;
    (1..=n_levels)
        .map(|i| {
            let iso = min + step * i as f32;
            (iso, extract_contour(matrix, iso))
        })
        .collect()
}

/// Pinta los segmentos en `coords` de matriz mapeados al `area` destino.
/// Cada nivel se dibuja con `stroke`.
pub fn paint_contour(
    segments: &[Segment],
    matrix_w: usize,
    matrix_h: usize,
    area: Rect,
    stroke: StrokeStyle,
    canvas: &mut dyn Canvas,
) {
    if matrix_w < 2 || matrix_h < 2 {
        return;
    }
    let cell_w = area.w / (matrix_w - 1) as f32;
    let cell_h = area.h / (matrix_h - 1) as f32;
    for seg in segments {
        let coords = [
            area.x + seg[0] * cell_w,
            area.y + seg[1] * cell_h,
            area.x + seg[2] * cell_w,
            area.y + seg[3] * cell_h,
        ];
        canvas.stroke_polyline(&coords, stroke);
    }
}

/// Atajo: pinta N niveles cada uno con su propio color (gradiente
/// lineal entre `color_low` y `color_high` por isolínea).
pub fn paint_contours(
    matrix: &HeatmapMatrix,
    n_levels: usize,
    area: Rect,
    color_low: Color,
    color_high: Color,
    line_width: f32,
    canvas: &mut dyn Canvas,
) {
    let levels = extract_contours(matrix, n_levels);
    if levels.is_empty() {
        return;
    }
    let n = levels.len().max(1) as f32;
    for (i, (_iso, segs)) in levels.iter().enumerate() {
        let t = if n > 1.0 { i as f32 / (n - 1.0) } else { 0.5 };
        let c = lerp_color(color_low, color_high, t);
        paint_contour(segs, matrix.width(), matrix.height(), area, StrokeStyle::new(line_width, c), canvas);
    }
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    Color::rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pineal_render::{PlanRecorder, RenderCmd};

    fn ramp(w: usize, h: usize) -> HeatmapMatrix {
        let data: Vec<f32> = (0..w * h).map(|i| (i / w) as f32).collect();
        HeatmapMatrix::from_data(data, w, h).unwrap()
    }

    #[test]
    fn empty_matrix_no_segments() {
        let m = HeatmapMatrix::new(0, 0);
        assert!(extract_contour(&m, 0.5).is_empty());
    }

    #[test]
    fn ramp_at_mid_level_yields_horizontal_segments() {
        // ramp 4x4: rows = 0,1,2,3. iso=1.5 cae entre row 1 y row 2 → línea horizontal.
        let m = ramp(4, 4);
        let segs = extract_contour(&m, 1.5);
        assert!(!segs.is_empty());
        // Todos los segmentos deberían ser ~horizontales (y0 ≈ y1).
        for s in &segs {
            assert!((s[1] - s[3]).abs() < 0.1, "no-horizontal: {s:?}");
        }
    }

    #[test]
    fn out_of_range_iso_no_segments() {
        let m = ramp(4, 4);
        assert!(extract_contour(&m, -5.0).is_empty());
        assert!(extract_contour(&m, 100.0).is_empty());
    }

    #[test]
    fn extract_contours_produces_n_levels() {
        let m = ramp(4, 4);
        let levels = extract_contours(&m, 3);
        assert_eq!(levels.len(), 3);
        // Niveles ordenados.
        assert!(levels[0].0 < levels[1].0 && levels[1].0 < levels[2].0);
    }

    #[test]
    fn paint_contour_emits_one_polyline_per_segment() {
        let m = ramp(4, 4);
        let segs = extract_contour(&m, 1.5);
        let mut rec = PlanRecorder::new();
        paint_contour(
            &segs,
            m.width(),
            m.height(),
            Rect::new(0.0, 0.0, 300.0, 200.0),
            StrokeStyle::new(1.0, Color::BLACK),
            &mut rec,
        );
        let n = rec
            .into_plan()
            .cmds
            .iter()
            .filter(|c| matches!(c, RenderCmd::StrokePolyline { .. }))
            .count();
        assert_eq!(n, segs.len());
    }

    #[test]
    fn paint_contours_emits_per_level() {
        let m = ramp(8, 8);
        let mut rec = PlanRecorder::new();
        paint_contours(
            &m,
            4,
            Rect::new(0.0, 0.0, 200.0, 200.0),
            Color::from_hex(0x000080),
            Color::from_hex(0xff0000),
            1.0,
            &mut rec,
        );
        let n = rec
            .into_plan()
            .cmds
            .iter()
            .filter(|c| matches!(c, RenderCmd::StrokePolyline { .. }))
            .count();
        assert!(n > 0);
    }

    #[test]
    fn flat_matrix_no_contours() {
        let m = HeatmapMatrix::from_data(vec![5.0; 16], 4, 4).unwrap();
        let levels = extract_contours(&m, 3);
        assert!(levels.is_empty(), "matriz constante no debería tener isolíneas");
    }
}
