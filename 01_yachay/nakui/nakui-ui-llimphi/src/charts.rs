//! Render de gráficos de los desgloses del tablero/reporte: filas de
//! barra, leyenda, torta/dona (`pie_canvas`), columnas/línea de una
//! serie (`plot_canvas`) y multi-serie (`multi_plot_canvas`). Todo es
//! presentación pura sobre `View::paint_with` (vello) + helpers de
//! `widgets`.

use super::*;

/// Una fila de desglose: etiqueta + barra + valor. Si `on_drill` está
/// presente, la fila es clickeable (con hover) y dispara el drill-down.
pub(crate) fn breakdown_row(
    key: String,
    bar: String,
    value: String,
    value_w: f32,
    on_drill: Option<Msg>,
    theme: &Theme,
) -> View<Msg> {
    let mut row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0),
            height: length(0.0),
        },
        ..Default::default()
    })
    .children(vec![
        cell_text(key, 96.0, theme.fg_text),
        cell_flex(bar, theme.accent),
        cell_text(value, value_w, theme.fg_muted),
    ]);
    if let Some(msg) = on_drill {
        row = row.hover_fill(theme.bg_panel).on_click(msg);
    }
    row
}

/// Paleta categórica de los gráficos de torta/dona: colores estables
/// por índice de sector (cicla si hay más grupos que colores).
const CHART_COLORS: [(u8, u8, u8); 10] = [
    (76, 145, 224),  // azul
    (236, 151, 56),  // ámbar
    (94, 186, 125),  // verde
    (214, 96, 122),  // rosa
    (149, 117, 205), // violeta
    (76, 194, 196),  // turquesa
    (224, 109, 84),  // teja
    (180, 190, 90),  // oliva
    (140, 140, 150), // gris
    (120, 170, 230), // celeste
];

/// Color del sector `i` del gráfico (cicla sobre [`CHART_COLORS`]).
pub(crate) fn chart_color(i: usize) -> Color {
    let (r, g, b) = CHART_COLORS[i % CHART_COLORS.len()];
    Color::from_rgba8(r, g, b, 255)
}

/// Normaliza un desglose a `(label, magnitud, texto_formateado)`:
/// `magnitud` es el número crudo (para escalar barras/sectores) y
/// `texto` su presentación según el [`ValueFormat`] de la card.
/// Vacío para escalares.
pub(crate) fn breakdown_display(result: &MetricResult, fmt: &ValueFormat) -> Vec<(String, f64, String)> {
    match result {
        MetricResult::Breakdown(rows) => rows
            .iter()
            .map(|(k, n)| (k.clone(), *n as f64, n.to_string()))
            .collect(),
        MetricResult::ValueBreakdown(rows) => rows
            .iter()
            .map(|(k, v)| {
                let value = if v.fract() == 0.0 {
                    Value::from(*v as i64)
                } else {
                    Value::from(*v)
                };
                (k.clone(), *v, format_value(Some(&value), fmt))
            })
            .collect(),
        // Multi-serie se pinta con su propio camino (`multi_chart`).
        MetricResult::MultiBreakdown { .. } => Vec::new(),
        MetricResult::Scalar(_) => Vec::new(),
    }
}

/// Canvas de un gráfico de torta (o dona si `donut`): cada `(valor,
/// color)` es un sector con barrido proporcional al valor sobre el
/// total, arrancando arriba (12 en punto) y girando horario. Los
/// sectores se separan con un trazo fino del color de fondo `gap`.
pub(crate) fn pie_canvas(slices: Vec<(f64, Color)>, donut: bool, gap: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(128.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect: PaintRect| {
        let total: f64 = slices.iter().map(|(v, _)| v.max(0.0)).sum();
        if total <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let r = (rect.w.min(rect.h) as f64) * 0.5 - 4.0;
        if r <= 0.0 {
            return;
        }
        let inner = if donut { r * 0.55 } else { 0.0 };
        let mut a0 = -std::f64::consts::FRAC_PI_2; // arranca arriba
        for (v, color) in &slices {
            if *v <= 0.0 {
                continue;
            }
            let a1 = a0 + (v / total) * std::f64::consts::TAU;
            let path = wedge_path(cx, cy, r, inner, a0, a1);
            scene.fill(Fill::NonZero, Affine::IDENTITY, *color, None, &path);
            scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, gap, None, &path);
            a0 = a1;
        }
    })
}

/// Polígono que aproxima un sector circular entre los ángulos `a0` y
/// `a1` (radianes). Si `inner > 0` es un sector de anillo (dona); si
/// no, una porción de torta con vértice en el centro.
fn wedge_path(cx: f64, cy: f64, r: f64, inner: f64, a0: f64, a1: f64) -> BezPath {
    let mut p = BezPath::new();
    // ~1 segmento cada 7° para que el arco se vea curvo.
    let steps = ((a1 - a0).abs() / 0.12).ceil().max(2.0) as usize;
    let at = |a: f64, rad: f64| (cx + rad * a.cos(), cy + rad * a.sin());
    if inner <= 0.0 {
        p.move_to((cx, cy));
        for i in 0..=steps {
            let a = a0 + (a1 - a0) * (i as f64 / steps as f64);
            p.line_to(at(a, r));
        }
    } else {
        for i in 0..=steps {
            let a = a0 + (a1 - a0) * (i as f64 / steps as f64);
            let pt = at(a, r);
            if i == 0 {
                p.move_to(pt);
            } else {
                p.line_to(pt);
            }
        }
        for i in (0..=steps).rev() {
            let a = a0 + (a1 - a0) * (i as f64 / steps as f64);
            p.line_to(at(a, inner));
        }
    }
    p.close_path();
    p
}

/// Canvas de un gráfico de columnas (o de línea si `line`) sobre el
/// desglose `series` (valor + color por grupo, en el orden del
/// desglose). El eje cero se traza con `axis`; la línea que une los
/// puntos usa `accent`, y cada columna/punto va con el color de su
/// grupo —el mismo de su fila de leyenda—. Soporta valores negativos:
/// el eje cero se posiciona dentro del rango y las columnas crecen
/// hacia arriba o abajo según el signo.
pub(crate) fn plot_canvas(series: Vec<(f64, Color)>, line: bool, axis: Color, accent: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(128.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect: PaintRect| {
        if series.is_empty() {
            return;
        }
        let pad = 6.0_f64;
        let x0 = rect.x as f64 + pad;
        let x1 = (rect.x + rect.w) as f64 - pad;
        let y0 = rect.y as f64 + pad;
        let y1 = (rect.y + rect.h) as f64 - pad;
        let w = (x1 - x0).max(1.0);
        let h = (y1 - y0).max(1.0);
        // El rango siempre incluye el cero, para que el eje base tenga
        // sentido y las columnas arranquen de ahí.
        let lo = series.iter().map(|(v, _)| *v).fold(0.0_f64, f64::min);
        let hi = series.iter().map(|(v, _)| *v).fold(0.0_f64, f64::max);
        let range = (hi - lo).max(1e-9);
        let y_of = |v: f64| y0 + (hi - v) / range * h;
        let zero_y = y_of(0.0);

        // Eje cero.
        let mut axis_path = BezPath::new();
        axis_path.move_to((x0, zero_y));
        axis_path.line_to((x1, zero_y));
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, axis, None, &axis_path);

        let n = series.len();
        let slot = w / n as f64;
        if line {
            let mut path = BezPath::new();
            for (i, (v, _)) in series.iter().enumerate() {
                let cx = x0 + slot * (i as f64 + 0.5);
                let pt = (cx, y_of(*v));
                if i == 0 {
                    path.move_to(pt);
                } else {
                    path.line_to(pt);
                }
            }
            scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, accent, None, &path);
            for (i, (v, color)) in series.iter().enumerate() {
                let cx = x0 + slot * (i as f64 + 0.5);
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    *color,
                    None,
                    &KurboCircle::new((cx, y_of(*v)), 3.0),
                );
            }
        } else {
            let bw = (slot * 0.7).max(1.0);
            for (i, (v, color)) in series.iter().enumerate() {
                let cx = x0 + slot * (i as f64 + 0.5);
                let yv = y_of(*v);
                let (top, bot) = if yv <= zero_y { (yv, zero_y) } else { (zero_y, yv) };
                let r = KurboRect::new(cx - bw / 2.0, top, cx + bw / 2.0, bot);
                scene.fill(Fill::NonZero, Affine::IDENTITY, *color, None, &r);
            }
        }
    })
}

/// Modo de dibujo de un desglose multi-serie.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum MultiMode {
    /// Una polilínea con puntos por serie.
    Line,
    /// Columnas agrupadas: las series se reparten el slot, lado a lado.
    Grouped,
    /// Columnas apiladas: una columna por grupo, segmentos apilados.
    Stacked,
}

/// Canvas multi-serie sobre un eje común de `n_groups` posiciones: cada
/// `(valores, color)` es una serie alineada 1:1 con los grupos. El modo
/// decide el dibujo: línea (polilínea+puntos por serie), columnas
/// agrupadas (lado a lado) o apiladas (una columna por grupo, segmentos
/// apilados desde el cero). El rango siempre incluye el cero (eje base).
pub(crate) fn multi_plot_canvas(
    n_groups: usize,
    series: Vec<(Vec<f64>, Color)>,
    mode: MultiMode,
    axis: Color,
) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(128.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect: PaintRect| {
        if n_groups == 0 || series.is_empty() {
            return;
        }
        let pad = 6.0_f64;
        let x0 = rect.x as f64 + pad;
        let x1 = (rect.x + rect.w) as f64 - pad;
        let y0 = rect.y as f64 + pad;
        let y1 = (rect.y + rect.h) as f64 - pad;
        let w = (x1 - x0).max(1.0);
        let h = (y1 - y0).max(1.0);
        // El rango incluye el cero. Para apiladas, la cota superior es
        // el mayor total apilado por grupo (sólo suma los aportes
        // positivos, que es lo que se dibuja), no el mayor valor suelto.
        let (lo, hi) = if mode == MultiMode::Stacked {
            let max_stack = (0..n_groups)
                .map(|i| {
                    series
                        .iter()
                        .map(|(v, _)| v.get(i).copied().unwrap_or(0.0).max(0.0))
                        .sum::<f64>()
                })
                .fold(0.0_f64, f64::max);
            (0.0, max_stack)
        } else {
            let all = || series.iter().flat_map(|(v, _)| v.iter().copied());
            (all().fold(0.0_f64, f64::min), all().fold(0.0_f64, f64::max))
        };
        let range = (hi - lo).max(1e-9);
        let y_of = |v: f64| y0 + (hi - v) / range * h;
        let zero_y = y_of(0.0);

        let mut axis_path = BezPath::new();
        axis_path.move_to((x0, zero_y));
        axis_path.line_to((x1, zero_y));
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, axis, None, &axis_path);

        let slot = w / n_groups as f64;
        match mode {
            MultiMode::Line => {
                for (vals, color) in &series {
                    let mut path = BezPath::new();
                    for (i, v) in vals.iter().enumerate() {
                        let cx = x0 + slot * (i as f64 + 0.5);
                        let pt = (cx, y_of(*v));
                        if i == 0 {
                            path.move_to(pt);
                        } else {
                            path.line_to(pt);
                        }
                    }
                    scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, *color, None, &path);
                    for (i, v) in vals.iter().enumerate() {
                        let cx = x0 + slot * (i as f64 + 0.5);
                        scene.fill(
                            Fill::NonZero,
                            Affine::IDENTITY,
                            *color,
                            None,
                            &KurboCircle::new((cx, y_of(*v)), 3.0),
                        );
                    }
                }
            }
            MultiMode::Grouped => {
                // El 80% central del slot se reparte entre las series.
                let ns = series.len();
                let group_w = slot * 0.8;
                let bw = (group_w / ns as f64).max(1.0);
                for i in 0..n_groups {
                    let gstart = x0 + slot * i as f64 + (slot - group_w) / 2.0;
                    for (s, (vals, color)) in series.iter().enumerate() {
                        let v = vals.get(i).copied().unwrap_or(0.0);
                        let yv = y_of(v);
                        let (top, bot) = if yv <= zero_y { (yv, zero_y) } else { (zero_y, yv) };
                        let bx = gstart + bw * s as f64;
                        let r = KurboRect::new(bx, top, bx + bw * 0.9, bot);
                        scene.fill(Fill::NonZero, Affine::IDENTITY, *color, None, &r);
                    }
                }
            }
            MultiMode::Stacked => {
                // Una columna por grupo; los aportes positivos de cada
                // serie se apilan desde el cero hacia arriba.
                let bw = (slot * 0.7).max(1.0);
                for i in 0..n_groups {
                    let cx = x0 + slot * (i as f64 + 0.5);
                    let mut acc = 0.0_f64;
                    for (vals, color) in &series {
                        let v = vals.get(i).copied().unwrap_or(0.0).max(0.0);
                        if v <= 0.0 {
                            continue;
                        }
                        let top = y_of(acc + v);
                        let bot = y_of(acc);
                        let r = KurboRect::new(cx - bw / 2.0, top, cx + bw / 2.0, bot);
                        scene.fill(Fill::NonZero, Affine::IDENTITY, *color, None, &r);
                        acc += v;
                    }
                }
            }
        }
    })
}

/// Fila de leyenda de un gráfico: cuadradito de color + etiqueta +
/// valor (con porcentaje). Clickeable (drill-down) si `on_drill`.
pub(crate) fn legend_row(
    color: Color,
    label: String,
    value: String,
    on_drill: Option<Msg>,
    theme: &Theme,
) -> View<Msg> {
    let swatch = View::new(Style {
        size: Size {
            width: length(12.0),
            height: length(12.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(color)
    .radius(3.0);
    let mut row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0),
            height: length(0.0),
        },
        ..Default::default()
    })
    .children(vec![
        swatch,
        cell_flex(label, theme.fg_text),
        cell_text(value, 96.0, theme.fg_muted),
    ]);
    if let Some(msg) = on_drill {
        row = row.hover_fill(theme.bg_panel).on_click(msg);
    }
    row
}
