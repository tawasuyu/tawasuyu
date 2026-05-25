//! Generación y decimación de ticks para ejes cartesianos.
//!
//! Toda esta lógica es agnóstica de backend: produce listas de
//! valores (ticks en dominio + posiciones en pixel + strings de
//! label). El `Element` GPUI los itera para emitir línea base,
//! segmentos de tick y `draw_text` de cada label.
//!
//! Pipeline canónico:
//! 1. [`ticks_nice`] — Wilkinson nice numbers en el rango del eje.
//! 2. Proyección dominio → pixel via [`crate::CoordinateSystem`].
//! 3. [`decimate_labels`] — descarta labels que se solaparían con
//!    el anterior dado un `min_spacing_px`. Los **ticks** sí
//!    siempre se dibujan (delgados, no estorban); sólo el texto
//!    se decima (sección 4.7 del ARCHITECTURE.md).
//!
//! `format_tick` es heurístico: si `step >= 1`, sin decimales; si
//! no, tantos decimales como hagan falta para distinguir ticks
//! adyacentes. Para escalas temporales el caller pasa su propio
//! format (epoch ms → "HH:MM:SS"), `format_tick` no entiende
//! semántica.

use pineal_core::scale::nice_step;
use pineal_render::{Canvas, Color, Point, StrokeStyle};

use crate::coord_system::CoordinateSystem;
use crate::viewport::ChartViewport;

/// Lado del plot donde vive el eje.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisSide {
    Bottom,
    Left,
    Top,
    Right,
}

impl AxisSide {
    pub fn is_horizontal(self) -> bool {
        matches!(self, AxisSide::Bottom | AxisSide::Top)
    }
}

/// Genera ticks "lindos" para un rango y cantidad objetivo.
///
/// El step es Wilkinson nice (`{1, 2, 5} × 10^k`); los ticks
/// resultantes son múltiplos del step alineados a 0.
/// Garantiza inclusión de bordes que caigan exactamente en
/// múltiplos; ticks fuera del rango se descartan.
pub fn ticks_nice(min: f64, max: f64, target_ticks: usize) -> Vec<f64> {
    debug_assert!(max > min && target_ticks > 0);
    let step = nice_step(min, max, target_ticks);
    let mut t = (min / step).ceil() * step;
    let mut out = Vec::with_capacity(target_ticks + 2);
    // Tolerancia para incluir el borde derecho cuando cae justo
    // por epsilon arriba del max.
    let epsilon = step * 1e-9;
    while t <= max + epsilon {
        out.push(t);
        t += step;
    }
    out
}

/// Filtra una lista de `(pixel_pos, label)` para que los labels
/// no se solapen. Devuelve los **índices** que sobreviven (los
/// del input). Asume input ordenado por `pixel_pos`.
///
/// `min_spacing_px` es la distancia mínima entre el borde
/// derecho de un label aprobado y el borde izquierdo del
/// siguiente. Si no tenés el ancho del label, pasá un valor
/// conservador (≈ 48 px del Flutter doc).
pub fn decimate_labels(
    positions_px: &[f32],
    label_widths_px: &[f32],
    min_spacing_px: f32,
) -> Vec<usize> {
    debug_assert_eq!(positions_px.len(), label_widths_px.len());
    if positions_px.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::with_capacity(positions_px.len());
    // Primero (más a la izquierda) siempre va.
    out.push(0);
    let mut last_right = positions_px[0] + label_widths_px[0] * 0.5;

    for i in 1..positions_px.len() {
        let half_w = label_widths_px[i] * 0.5;
        let my_left = positions_px[i] - half_w;
        if my_left - last_right >= min_spacing_px {
            out.push(i);
            last_right = positions_px[i] + half_w;
        }
    }

    out
}

/// Formateo numérico básico con decimales dependientes del step.
///
/// - `step >= 1` → sin decimales: "1", "20", "300".
/// - `0 < step < 1` → decimales suficientes para distinguir step
///   de step + step (típicamente `-floor(log10(step))`).
/// - Valores absolutos muy chicos quedan en "0".
pub fn format_tick(value: f64, step: f64) -> String {
    if step >= 1.0 {
        format!("{}", value.round() as i64)
    } else if step <= 0.0 {
        format!("{}", value)
    } else {
        let decimals = (-step.log10().floor()) as i32;
        let decimals = decimals.clamp(1, 9) as usize;
        format!("{:.*}", decimals, value)
    }
}

/// Estilo visual del eje. Lo consume el Element en `paint()`.
#[derive(Debug, Clone, Copy)]
pub struct AxisStyle {
    pub tick_length_px: f32,
    pub tick_width_px: f32,
    pub axis_line_width_px: f32,
    pub label_size_px: f32,
    pub label_offset_px: f32,
    /// Min spacing entre labels después de decimar.
    pub label_min_spacing_px: f32,
}

impl Default for AxisStyle {
    fn default() -> Self {
        Self {
            tick_length_px: 4.0,
            tick_width_px: 1.0,
            axis_line_width_px: 1.0,
            label_size_px: 10.0,
            label_offset_px: 4.0,
            label_min_spacing_px: 8.0,
        }
    }
}

const MONO_GLYPH_RATIO: f32 = 0.55;

/// Pinta las dos líneas base (X y Y), los tick marks y los labels
/// decimados de ambos ejes. Función reusable entre crates de
/// visualización (cartesian, financial, etc.) — recibe todo por
/// args para no atarse al state de un Element específico.
pub fn paint_axes(
    canvas: &mut dyn Canvas,
    cs: &CoordinateSystem,
    viewport: &ChartViewport,
    color: Color,
    style: AxisStyle,
    target_ticks_x: usize,
    target_ticks_y: usize,
) {
    let plot = cs.plot;
    let axis_stroke = StrokeStyle::new(style.axis_line_width_px, color);
    let tick_stroke = StrokeStyle::new(style.tick_width_px, color);
    let tlen = style.tick_length_px;

    canvas.stroke_line(
        Point::new(plot.x, plot.bottom()),
        Point::new(plot.right(), plot.bottom()),
        axis_stroke,
    );
    canvas.stroke_line(
        Point::new(plot.x, plot.y),
        Point::new(plot.x, plot.bottom()),
        axis_stroke,
    );

    // X axis ticks + labels.
    let x_ticks = ticks_nice(viewport.x_min, viewport.x_max, target_ticks_x);
    let x_step = nice_step(viewport.x_min, viewport.x_max, target_ticks_x);
    let mut x_pos: Vec<f32> = Vec::with_capacity(x_ticks.len());
    let mut x_lbl: Vec<String> = Vec::with_capacity(x_ticks.len());
    let mut x_widths: Vec<f32> = Vec::with_capacity(x_ticks.len());
    for v in &x_ticks {
        let pixel = cs.data_to_pixel(*v, viewport.y_min).x;
        if pixel < plot.x - 0.5 || pixel > plot.right() + 0.5 {
            continue;
        }
        canvas.stroke_line(
            Point::new(pixel, plot.bottom()),
            Point::new(pixel, plot.bottom() + tlen),
            tick_stroke,
        );
        let lbl = format_tick(*v, x_step);
        let w = lbl.len() as f32 * style.label_size_px * MONO_GLYPH_RATIO;
        x_pos.push(pixel);
        x_widths.push(w);
        x_lbl.push(lbl);
    }
    let keep_x = decimate_labels(&x_pos, &x_widths, style.label_min_spacing_px);
    for i in keep_x {
        let half = x_widths[i] * 0.5;
        canvas.draw_text(
            Point::new(
                x_pos[i] - half,
                plot.bottom() + tlen + style.label_offset_px,
            ),
            &x_lbl[i],
            color,
            style.label_size_px,
        );
    }

    // Y axis ticks + labels con decimación vertical.
    let y_ticks = ticks_nice(viewport.y_min, viewport.y_max, target_ticks_y);
    let y_step = nice_step(viewport.y_min, viewport.y_max, target_ticks_y);
    let y_label_pitch = style.label_size_px + style.label_min_spacing_px;
    let mut prev_py: Option<f32> = None;

    for v in &y_ticks {
        let py = cs.data_to_pixel(viewport.x_min, *v).y;
        if py < plot.y - 0.5 || py > plot.bottom() + 0.5 {
            continue;
        }
        canvas.stroke_line(
            Point::new(plot.x - tlen, py),
            Point::new(plot.x, py),
            tick_stroke,
        );
        let label_ok = match prev_py {
            None => true,
            Some(p) => (py - p).abs() >= y_label_pitch,
        };
        if !label_ok {
            continue;
        }
        let lbl = format_tick(*v, y_step);
        let w = lbl.len() as f32 * style.label_size_px * MONO_GLYPH_RATIO;
        canvas.draw_text(
            Point::new(
                plot.x - tlen - style.label_offset_px - w,
                py - style.label_size_px * 0.5,
            ),
            &lbl,
            color,
            style.label_size_px,
        );
        prev_py = Some(py);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_nice_genera_alineados_a_step() {
        let t = ticks_nice(0.0, 10.0, 5);
        assert_eq!(t, vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
    }

    #[test]
    fn ticks_nice_clipea_fuera_de_rango() {
        let t = ticks_nice(0.3, 9.8, 5);
        // step = 2; ticks dentro [0.3, 9.8] son 2,4,6,8.
        assert_eq!(t, vec![2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn ticks_nice_rango_fraccional() {
        let t = ticks_nice(0.0, 1.0, 5);
        // step = 0.2 → 0, 0.2, 0.4, 0.6, 0.8, 1.0
        assert_eq!(t.len(), 6);
        for (i, v) in t.iter().enumerate() {
            assert!((v - (i as f64 * 0.2)).abs() < 1e-9);
        }
    }

    #[test]
    fn decimate_preserva_primero() {
        let pos = vec![0.0, 5.0, 10.0, 100.0];
        let w = vec![20.0; 4];
        // min_spacing 10 px. 0 va; 5 está a 5-10=-5 del borde der → no
        // entra; 10 está a 10-10=0 → no entra; 100 sí.
        let keep = decimate_labels(&pos, &w, 10.0);
        assert_eq!(keep, vec![0, 3]);
    }

    #[test]
    fn decimate_vacio() {
        let keep = decimate_labels(&[], &[], 10.0);
        assert!(keep.is_empty());
    }

    #[test]
    fn decimate_pasa_todo_cuando_hay_lugar() {
        let pos = vec![0.0, 50.0, 100.0];
        let w = vec![10.0, 10.0, 10.0];
        let keep = decimate_labels(&pos, &w, 5.0);
        assert_eq!(keep, vec![0, 1, 2]);
    }

    #[test]
    fn format_tick_integer() {
        assert_eq!(format_tick(42.0, 1.0), "42");
        assert_eq!(format_tick(0.0, 5.0), "0");
        assert_eq!(format_tick(1000.0, 100.0), "1000");
    }

    #[test]
    fn format_tick_fraccional() {
        assert_eq!(format_tick(0.5, 0.1), "0.5");
        assert_eq!(format_tick(0.05, 0.01), "0.05");
    }
}
