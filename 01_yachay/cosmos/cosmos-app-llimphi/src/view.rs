//! Renderers de contenido de los paneles astrológicos. Cada función
//! devuelve el `View` que se monta en el área central cuando su pestaña
//! está activa (carta, cuerpos, aspectos, aspectario, cualidades,
//! uraniano, lotes/estrellas/puntos como layers genéricas, corpus). El
//! chrome (menú, árbol, pestañas, barra de estado, menús contextuales)
//! vive en [`crate::chrome`]; las gráficas astronómicas en
//! [`crate::astroview`].

use cosmos_engine::{combinaciones_de_carta, corpus_inputs, Corpus};
use cosmos_render::{AspectSummary, LayerKind, RenderModel, UranianGroup};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

use crate::format::{fmt_deg_sign, fmt_dms, signo_de_longitud, simbolo_aspecto, simbolo_cuerpo};
use crate::model::{Model, Msg};

// =====================================================================
// Helpers compartidos
// =====================================================================

pub(crate) fn tile_container<I>(rows: I, theme: &Theme) -> View<Msg>
where
    I: IntoIterator<Item = View<Msg>>,
{
    let _ = theme;
    let children: Vec<View<Msg>> = rows.into_iter().collect();
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(3.0_f32),
        },
        ..Default::default()
    })
    .clip(true)
    .children(children)
}

pub(crate) fn line(text: String, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(size + 4.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text, size, color, Alignment::Start)
}

pub(crate) fn section_label(text: String, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        flex_shrink: 0.0,
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text, 11.0, theme.accent, Alignment::Start)
}

// =====================================================================
// Carta (datos del nacimiento)
// =====================================================================

pub(crate) fn tile_carta(model: &Model, theme: &Theme) -> View<Msg> {
    let bd = &model.chart.birth_data;
    let lugar = bd
        .birthplace_label
        .clone()
        .unwrap_or_else(|| "(sin lugar)".into());
    let fecha = format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC{:+}",
        bd.year,
        bd.month,
        bd.day,
        bd.hour,
        bd.minute,
        bd.tz_offset_minutes as f32 / 60.0
    );
    let lat_long = format!(
        "{:.4}°{} · {:.4}°{}",
        bd.latitude_deg.abs(),
        if bd.latitude_deg >= 0.0 { "N" } else { "S" },
        bd.longitude_deg.abs(),
        if bd.longitude_deg >= 0.0 { "E" } else { "W" }
    );
    let angles = format!(
        "Asc {} · MC {} · Desc {} · IC {}",
        fmt_deg_sign(model.render.ascendant_deg),
        fmt_deg_sign(model.render.midheaven_deg),
        fmt_deg_sign(model.render.descendant_deg),
        fmt_deg_sign(model.render.imum_coeli_deg),
    );

    tile_container(
        vec![
            line(model.chart.label.clone(), 14.0, theme.fg_text),
            line(lugar, 11.0, theme.fg_muted),
            line(fecha, 11.0, theme.fg_muted),
            line(lat_long, 11.0, theme.fg_muted),
            section_label("Ángulos".to_string(), theme),
            line(angles, 11.0, theme.fg_text),
        ],
        theme,
    )
}

// =====================================================================
// Cuerpos
// =====================================================================

pub(crate) fn tile_cuerpos(render: &RenderModel, theme: &Theme) -> View<Msg> {
    let rows: Vec<View<Msg>> = render
        .layers
        .iter()
        .filter(|l| l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies))
        .flat_map(|l| l.glyphs.iter())
        .map(|g| {
            let sign = signo_de_longitud(g.deg);
            let dms = fmt_dms(g.deg.rem_euclid(30.0) as f64);
            let body = simbolo_cuerpo(&g.symbol);
            let casa = g.house.map(|h| format!(" h{h}")).unwrap_or_default();
            // "℞" no está en LiberationSans — usamos "(R)".
            let retro = if g.retrograde { " (R)" } else { "" };
            let dignity = g.dignity_marker.clone().unwrap_or_default();
            let line_str = format!("{body} {dms} {sign}{casa}{retro}{dignity}");
            line(line_str, 12.0, theme.fg_text)
        })
        .collect();
    tile_container(rows, theme)
}

// =====================================================================
// Aspectos (filtrado por module_id) + aspectario triangular
// =====================================================================

fn aspecto_importancia(kind: &str) -> u8 {
    match kind {
        "conjunction" | "opposition" | "square" | "trine" | "sextile" => 0,
        _ => 1,
    }
}

fn aspecto_color(kind: &str) -> Color {
    let pal = cosmos_render::Palette::dark();
    let c = pal.aspect(kind);
    let to_byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(to_byte(c.r), to_byte(c.g), to_byte(c.b), to_byte(c.a))
}

#[allow(clippy::too_many_arguments)]
fn aspect_row(
    kind_code: &str,
    from: &str,
    to: &str,
    orb_dms: &str,
    dir: &str,
    kind_id: &str,
    theme: &Theme,
) -> View<Msg> {
    let size = 12.0_f32;
    let kind_color = aspecto_color(kind_id);
    let cell = |text: String, color: Color, w: f32| -> View<Msg> {
        View::new(Style {
            size: Size {
                width: length(w),
                height: length(size + 4.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(text, size, color, Alignment::Start)
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(size + 4.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![
        cell(kind_code.to_string(), kind_color, 34.0),
        cell(from.to_string(), theme.fg_text, 44.0),
        cell(to.to_string(), theme.fg_text, 44.0),
        cell(orb_dms.to_string(), theme.fg_muted, 60.0),
        cell(dir.to_string(), theme.fg_muted, 24.0),
    ])
}

pub(crate) fn tile_aspectos(render: &RenderModel, module_id: &str, theme: &Theme) -> View<Msg> {
    let mut asps: Vec<&AspectSummary> = render
        .aspect_summary
        .iter()
        .filter(|a| a.module_id == module_id)
        .collect();
    asps.sort_by(|a, b| {
        aspecto_importancia(&a.kind)
            .cmp(&aspecto_importancia(&b.kind))
            .then_with(|| {
                a.orb_deg
                    .partial_cmp(&b.orb_deg)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    let rows: Vec<View<Msg>> = asps
        .into_iter()
        .take(40)
        .map(|a| {
            let from = simbolo_cuerpo(&a.from_body);
            let to = simbolo_cuerpo(&a.to_body);
            let kind = simbolo_aspecto(&a.kind);
            let dms = fmt_dms(a.orb_deg);
            let dir = match a.applying {
                Some(true) => " ◄",
                Some(false) => " ►",
                None => "",
            };
            aspect_row(kind, from, to, &dms, dir, a.kind.as_str(), theme)
        })
        .collect();
    if rows.is_empty() {
        return tile_container(
            vec![line(
                rimay_localize::t("cosmos-empty"),
                12.0,
                theme.fg_muted,
            )],
            theme,
        );
    }
    tile_container(rows, theme)
}

// =====================================================================
// Uraniano (grupos del dial de 90°)
// =====================================================================

pub(crate) fn tile_uraniano(groups: &[UranianGroup], theme: &Theme) -> View<Msg> {
    if groups.is_empty() {
        return tile_container(
            vec![line(
                "Activá la capa «Uraniano» (menú Capas) para ver los grupos del dial de 90°."
                    .to_string(),
                12.0,
                theme.fg_muted,
            )],
            theme,
        );
    }
    let rows: Vec<View<Msg>> = groups
        .iter()
        .take(40)
        .map(|g| {
            let bodies: Vec<String> = g.bodies.iter().map(|b| simbolo_cuerpo(b).into()).collect();
            line(
                format!("{:.1}°  {}", g.mod90_deg, bodies.join(" ")),
                12.0,
                theme.fg_text,
            )
        })
        .collect();
    tile_container(rows, theme)
}

// =====================================================================
// Cualidades (elementos + modalidades + polaridad)
// =====================================================================

pub(crate) fn tile_cualidades(render: &RenderModel, theme: &Theme) -> View<Msg> {
    let bodies: Vec<(&str, f32)> = render
        .layers
        .iter()
        .filter(|l| l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies))
        .flat_map(|l| l.glyphs.iter())
        .map(|g| (g.symbol.as_str(), g.deg))
        .collect();

    let mut elementos: [Vec<&'static str>; 4] = Default::default();
    let mut modalidades: [Vec<&'static str>; 3] = Default::default();
    let mut polaridad: [Vec<&'static str>; 2] = Default::default();

    for (name, deg) in &bodies {
        let sign_idx = ((deg.rem_euclid(360.0) / 30.0) as usize) % 12;
        let glyph = simbolo_cuerpo(name);
        elementos[sign_idx % 4].push(glyph);
        modalidades[sign_idx % 3].push(glyph);
        polaridad[sign_idx % 2].push(glyph);
    }

    let elem_labels = ["Fuego", "Tierra", "Aire", "Agua"];
    let mod_labels = ["Cardinal", "Fijo", "Mutable"];
    let pol_labels = ["Yang", "Yin"];

    let mut rows: Vec<View<Msg>> = Vec::new();
    rows.push(section_label("Elementos".to_string(), theme));
    for (i, label) in elem_labels.iter().enumerate() {
        rows.push(fila_cualidad(label, &elementos[i], theme));
    }
    rows.push(section_label("Modalidades".to_string(), theme));
    for (i, label) in mod_labels.iter().enumerate() {
        rows.push(fila_cualidad(label, &modalidades[i], theme));
    }
    rows.push(section_label("Polaridad".to_string(), theme));
    for (i, label) in pol_labels.iter().enumerate() {
        rows.push(fila_cualidad(label, &polaridad[i], theme));
    }
    tile_container(rows, theme)
}

fn fila_cualidad(label: &str, glyphs: &[&str], theme: &Theme) -> View<Msg> {
    let count = glyphs.len();
    let bar_len = count.min(10);
    let bar: String = "█".repeat(bar_len) + &"░".repeat(10_usize.saturating_sub(bar_len));
    let glyph_str = glyphs.join(" ");
    let txt = format!("{label:>9}  {bar}  {count}  {glyph_str}");
    line(txt, 12.0, theme.fg_text)
}

// =====================================================================
// Aspectario triangular
// =====================================================================

pub(crate) fn tile_box_graph(render: &RenderModel, theme: &Theme) -> View<Msg> {
    let bodies: Vec<String> = render
        .layers
        .iter()
        .filter(|l| l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies))
        .flat_map(|l| l.glyphs.iter())
        .map(|g| g.symbol.clone())
        .collect();
    if bodies.len() < 2 {
        return tile_container(
            vec![line(
                rimay_localize::t("cosmos-empty"),
                12.0,
                theme.fg_muted,
            )],
            theme,
        );
    }
    let mut aspects: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    for a in &render.aspect_summary {
        if a.module_id != "natal" {
            continue;
        }
        let key = sorted_pair(&a.from_body, &a.to_body);
        aspects.insert(key, a.kind.clone());
    }
    const CELL: f32 = 24.0;
    const LBL: f32 = 26.0;
    let rows: Vec<View<Msg>> = bodies
        .iter()
        .enumerate()
        .map(|(i, body_i)| {
            let mut cells: Vec<View<Msg>> = Vec::with_capacity(i + 1);
            cells.push(box_cell(
                simbolo_cuerpo(body_i),
                theme.fg_text,
                None,
                LBL,
                CELL,
                theme,
            ));
            for body_j in bodies.iter().take(i) {
                let pair = sorted_pair(body_i, body_j);
                let asp = aspects.get(&pair);
                let (text, bg) = match asp {
                    Some(k) => (simbolo_aspecto(k), Some(theme.bg_panel_alt)),
                    None => ("·", None),
                };
                cells.push(box_cell(text, theme.fg_text, bg, CELL, CELL, theme));
            }
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: Dimension::auto(),
                    height: length(CELL),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .children(cells)
        })
        .collect();
    tile_container(rows, theme)
}

fn box_cell(
    text: &'static str,
    fg: Color,
    bg: Option<Color>,
    w: f32,
    h: f32,
    _theme: &Theme,
) -> View<Msg> {
    let mut v = View::new(Style {
        size: Size {
            width: length(w),
            height: length(h),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(text.to_string(), 12.0, fg, Alignment::Center);
    if let Some(c) = bg {
        v = v.fill(c).radius(2.0);
    }
    v
}

fn sorted_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    }
}

// =====================================================================
// Layer genérica (lotes / estrellas fijas / puntos medios)
// =====================================================================

pub(crate) fn tile_layer_glyphs(
    render: &RenderModel,
    kind: LayerKind,
    module_id: &str,
    hint: &str,
    theme: &Theme,
) -> View<Msg> {
    let glyphs: Vec<&cosmos_render::Glyph> = render
        .layers
        .iter()
        .filter(|l| {
            l.module_id == module_id
                && std::mem::discriminant(&l.kind) == std::mem::discriminant(&kind)
        })
        .flat_map(|l| l.glyphs.iter())
        .collect();
    if glyphs.is_empty() {
        return tile_container(vec![line(hint.to_string(), 12.0, theme.fg_muted)], theme);
    }
    let rows: Vec<View<Msg>> = glyphs
        .into_iter()
        .take(40)
        .map(|g| {
            let label = if g.symbol.starts_with("lot:") {
                g.annotation.clone().unwrap_or_else(|| g.symbol.clone())
            } else if g.symbol.starts_with('✦') {
                g.annotation.clone().unwrap_or_else(|| g.symbol.clone())
            } else {
                simbolo_cuerpo(&g.symbol).to_string()
            };
            let casa = g.house.map(|h| format!(" h{h}")).unwrap_or_default();
            let dms = fmt_dms(g.deg.rem_euclid(30.0) as f64);
            let sign = signo_de_longitud(g.deg);
            line(format!("{label}  {dms} {sign}{casa}"), 12.0, theme.fg_text)
        })
        .collect();
    tile_container(rows, theme)
}

// =====================================================================
// Corpus (pasajes interpretativos)
// =====================================================================

pub(crate) fn tile_corpus(render: &RenderModel, corpus: &Corpus, theme: &Theme) -> View<Msg> {
    let (colocaciones, aspectos) = corpus_inputs(render);
    let combinaciones = combinaciones_de_carta(&colocaciones, &aspectos);
    let pasajes = corpus.interpretar(&combinaciones);
    let huecos = corpus.huecos(&combinaciones);

    let header_txt = rimay_localize::t_args(
        "cosmos-corpus-header",
        &[
            ("pasajes", pasajes.len().to_string().into()),
            ("huecos", huecos.len().to_string().into()),
            ("total", combinaciones.len().to_string().into()),
        ],
    );
    let mut rows: Vec<View<Msg>> = Vec::with_capacity(pasajes.len() * 2 + 1);
    rows.push(line(header_txt, 11.0, theme.fg_muted));

    if pasajes.is_empty() {
        rows.push(line(
            rimay_localize::t("cosmos-corpus-vacio"),
            12.0,
            theme.fg_muted,
        ));
    } else {
        for p in pasajes.iter().take(16) {
            rows.push(line(p.combinacion.to_string(), 10.0, theme.accent));
            let txt = recortar(&p.texto, 200);
            rows.push(line(txt, 12.0, theme.fg_text));
        }
    }
    tile_container(rows, theme)
}

fn recortar(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            return out;
        }
        out.push(ch);
    }
    out
}
