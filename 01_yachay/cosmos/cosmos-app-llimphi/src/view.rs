//! Renderers de contenido de los paneles astrológicos. Cada función
//! devuelve el `View` que se monta en el panel de herramientas cuando su
//! sección está expandida (carta, cuerpos, aspectos, cualidades,
//! uraniano, lotes/estrellas/puntos como layers genéricas, corpus).
//!
//! **Sin tofus**: cuerpos, signos y aspectos se pintan como glyphs
//! vectoriales (mini-canvas) vía [`crate::glyphs`] — nunca unicode
//! astrológico ni abreviaturas tipo "Sag". El chrome (menú, árbol,
//! pestañas, barra de estado, menús contextuales) vive en
//! [`crate::chrome`]; las gráficas astronómicas en [`crate::astroview`].

use std::collections::HashMap;

use cosmos_engine::{combinaciones_de_carta, corpus_inputs, Corpus};
use cosmos_render::{LayerKind, Palette, RenderModel, Rgba, UranianGroup};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

use crate::format::fmt_dms;
use crate::glyphs::{self, sign_id};
use crate::model::{Model, Msg};

/// Alto de fila estándar de las tablas.
const ROW_H: f32 = 20.0;
/// Lado del glyph de cuerpo/aspecto en las filas.
const GLYPH: f32 = 16.0;
/// Lado del glyph de signo (un poco menor para diferenciar).
const SGN: f32 = 14.0;

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
        // Alto guiado por el contenido (los paneles del acordeón se
        // autoajustan a su tabla), no por el espacio disponible.
        flex_grow: 0.0,
        flex_shrink: 0.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(3.0_f32),
        },
        ..Default::default()
    })
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

/// Una fila horizontal de celdas, alto [`ROW_H`].
fn cells_row(cells: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(ROW_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(3.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(cells)
}

/// Celda de texto de ancho fijo. Alto `auto` (= alto del texto) para que
/// el `align_items: Center` de la fila lo centre verticalmente — un texto
/// `Start` se ancla arriba si su nodo es más alto que el glifo.
fn txt_cell(text: String, w: f32, size: f32, color: Color, align: Alignment) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(w),
            height: Dimension::auto(),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(text, size, color, align)
}

fn rgba_to_color(c: Rgba) -> Color {
    let to_byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(to_byte(c.r), to_byte(c.g), to_byte(c.b), to_byte(c.a))
}

/// Color elemental del signo en la longitud dada.
fn sign_color(deg: f32) -> Color {
    rgba_to_color(Palette::dark().sign(sign_id(deg)))
}

/// Grupo compacto cuerpo+signo (glyph del cuerpo seguido del glyph del
/// signo donde cae, coloreado por elemento). `lon = None` → sólo cuerpo.
fn body_sign(name: &str, lon: Option<f32>, theme: &Theme) -> View<Msg> {
    let mut kids = vec![glyphs::body_view(name, GLYPH, theme.fg_text)];
    if let Some(d) = lon {
        kids.push(glyphs::sign_view(sign_id(d), SGN, sign_color(d)));
    }
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: length(GLYPH + SGN + 4.0),
            height: length(ROW_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(kids)
}

/// Mapa cuerpo→longitud eclíptica desde la capa natal de cuerpos. Se usa
/// para resolver el signo de cada extremo de un aspecto.
fn body_lons(render: &RenderModel) -> HashMap<String, f32> {
    let mut m = HashMap::new();
    for l in &render.layers {
        if l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies) {
            for g in &l.glyphs {
                m.insert(g.symbol.clone(), g.deg);
            }
        }
    }
    // Ángulos del chart, por si un aspecto los referencia.
    m.entry("asc".into()).or_insert(render.ascendant_deg);
    m.entry("mc".into()).or_insert(render.midheaven_deg);
    m
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

    let r = &model.render;
    let angles = [
        ("Asc", r.ascendant_deg),
        ("MC", r.midheaven_deg),
        ("Dc", r.descendant_deg),
        ("IC", r.imum_coeli_deg),
    ];
    let mut rows: Vec<View<Msg>> = vec![
        line(model.chart.label.clone(), 14.0, theme.fg_text),
        line(lugar, 11.0, theme.fg_muted),
        line(fecha, 11.0, theme.fg_muted),
        line(lat_long, 11.0, theme.fg_muted),
        section_label("Ángulos".to_string(), theme),
    ];
    for (name, deg) in angles {
        rows.push(cells_row(vec![
            txt_cell(name.to_string(), 32.0, 12.0, theme.fg_text, Alignment::Start),
            txt_cell(
                fmt_dms((deg.rem_euclid(30.0)) as f64),
                56.0,
                12.0,
                theme.fg_muted,
                Alignment::Start,
            ),
            glyphs::sign_view(sign_id(deg), SGN, sign_color(deg)),
        ]));
    }
    tile_container(rows, theme)
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
            let dms = fmt_dms(g.deg.rem_euclid(30.0) as f64);
            let house = g
                .house
                .map(|h| format!("h{h}"))
                .unwrap_or_default();
            let retro = if g.retrograde { "R" } else { "" };
            let dignity = g.dignity_marker.clone().unwrap_or_default();
            cells_row(vec![
                glyphs::body_view(&g.symbol, GLYPH, theme.fg_text),
                txt_cell(dms, 56.0, 12.0, theme.fg_text, Alignment::Start),
                glyphs::sign_view(sign_id(g.deg), SGN, sign_color(g.deg)),
                txt_cell(house, 30.0, 11.0, theme.fg_muted, Alignment::Start),
                txt_cell(retro.to_string(), 14.0, 11.0, theme.fg_destructive, Alignment::Center),
                txt_cell(dignity, 16.0, 11.0, theme.accent, Alignment::Center),
            ])
        })
        .collect();
    tile_container(rows, theme)
}

// =====================================================================
// Aspectos — tabla unificada geocéntrico + topocéntrico
// =====================================================================

/// Una fila de la tabla unificada: un par (de cuerpos, aspecto) con su
/// orbe geocéntrico y/o topocéntrico.
struct AspRow {
    kind: String,
    from: String,
    to: String,
    geo: Option<f64>,
    topo: Option<f64>,
    applying: Option<bool>,
}

fn sorted_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    }
}

/// Tabla unificada de aspectos: geocéntrico (módulo `natal`) y
/// topocéntrico (módulo `topocentric`) en la misma grilla, con la
/// diferencia de orbe entre ambos y los glyphs del aspecto, los cuerpos
/// y sus signos.
pub(crate) fn tile_aspectos(render: &RenderModel, theme: &Theme) -> View<Msg> {
    let lons = body_lons(render);
    let mut map: HashMap<(String, String, String), AspRow> = HashMap::new();

    for a in &render.aspect_summary {
        let topo = a.module_id == "topocentric";
        if !topo && a.module_id != "natal" {
            continue;
        }
        let (from, to) = sorted_pair(&a.from_body, &a.to_body);
        let key = (from.clone(), to.clone(), a.kind.clone());
        let row = map.entry(key).or_insert_with(|| AspRow {
            kind: a.kind.clone(),
            from,
            to,
            geo: None,
            topo: None,
            applying: None,
        });
        if topo {
            row.topo = Some(a.orb_deg);
        } else {
            row.geo = Some(a.orb_deg);
            row.applying = a.applying;
        }
    }

    let mut rows: Vec<AspRow> = map.into_values().collect();
    // Orden por intensidad: el orbe más cerrado (aspecto más exacto y
    // fuerte) primero, sin importar mayor/menor.
    rows.sort_by(|a, b| {
        let oa = a.geo.or(a.topo).unwrap_or(99.0);
        let ob = b.geo.or(b.topo).unwrap_or(99.0);
        oa.partial_cmp(&ob).unwrap_or(std::cmp::Ordering::Equal)
    });

    if rows.is_empty() {
        return tile_container(
            vec![line(rimay_localize::t("cosmos-empty"), 12.0, theme.fg_muted)],
            theme,
        );
    }

    // Cabecera de columnas.
    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(3.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        txt_cell(String::new(), 4.0, 10.0, theme.fg_muted, Alignment::Start),
        txt_cell(String::new(), GLYPH, 10.0, theme.fg_muted, Alignment::Start),
        txt_cell(String::new(), GLYPH + SGN + 4.0, 10.0, theme.fg_muted, Alignment::Start),
        txt_cell(String::new(), GLYPH + SGN + 4.0, 10.0, theme.fg_muted, Alignment::Start),
        txt_cell("geo".to_string(), 46.0, 10.0, theme.fg_muted, Alignment::Start),
        txt_cell("topo".to_string(), 46.0, 10.0, theme.fg_muted, Alignment::Start),
        txt_cell("Δ".to_string(), 40.0, 10.0, theme.fg_muted, Alignment::Start),
    ]);

    let mut out: Vec<View<Msg>> = Vec::with_capacity(rows.len() + 1);
    out.push(header);
    for row in rows.into_iter().take(60) {
        let orb = row.geo.or(row.topo).unwrap_or(8.0);
        let intensity = (1.0 - orb / 8.0).clamp(0.15, 1.0) as f32;
        let geo = row
            .geo
            .map(fmt_dms)
            .unwrap_or_else(|| "—".to_string());
        let topo = row
            .topo
            .map(fmt_dms)
            .unwrap_or_else(|| "—".to_string());
        let diff = match (row.geo, row.topo) {
            (Some(g), Some(t)) => format!("{:+.0}'", (t - g) * 60.0),
            _ => "—".to_string(),
        };
        let dir = match row.applying {
            Some(true) => glyphs::icon_view(glyphs::Icon::Applying, 12.0, theme.fg_muted),
            Some(false) => glyphs::icon_view(glyphs::Icon::Separating, 12.0, theme.fg_muted),
            None => txt_cell(String::new(), 12.0, 10.0, theme.fg_muted, Alignment::Center),
        };
        // Texto del orbe a más contraste cuanto más fuerte el aspecto.
        let orb_col = if intensity > 0.55 { theme.fg_text } else { theme.fg_muted };
        out.push(cells_row(vec![
            intensity_bar(&row.kind, intensity),
            glyphs::aspect_view(&row.kind, GLYPH),
            body_sign(&row.from, lons.get(&row.from).copied(), theme),
            body_sign(&row.to, lons.get(&row.to).copied(), theme),
            txt_cell(geo, 46.0, 11.0, orb_col, Alignment::Start),
            txt_cell(topo, 46.0, 11.0, orb_col, Alignment::Start),
            txt_cell(diff, 40.0, 11.0, theme.fg_muted, Alignment::Start),
            dir,
        ]));
    }
    tile_container(out, theme)
}

/// Color del aspecto (paleta oscura) con la opacidad dada.
fn aspect_color_intensity(kind: &str, intensity: f32) -> Color {
    let c = Palette::dark().aspect(kind);
    let to = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(to(c.r), to(c.g), to(c.b), to(intensity))
}

/// Barra vertical en el color del aspecto cuya opacidad marca la
/// intensidad — los aspectos exactos se ven más fuertes en la lista,
/// igual que sus líneas en la carta.
fn intensity_bar(kind: &str, intensity: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(4.0),
            height: length(ROW_H - 6.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(aspect_color_intensity(kind, intensity))
    .radius(2.0)
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
            let mut cells: Vec<View<Msg>> = vec![txt_cell(
                format!("{:.1}°", g.mod90_deg),
                52.0,
                12.0,
                theme.fg_text,
                Alignment::Start,
            )];
            for b in &g.bodies {
                cells.push(glyphs::body_view(b, GLYPH, theme.fg_text));
            }
            cells_row(cells)
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

    let mut elementos: [Vec<&str>; 4] = Default::default();
    let mut modalidades: [Vec<&str>; 3] = Default::default();
    let mut polaridad: [Vec<&str>; 2] = Default::default();

    for (name, deg) in &bodies {
        let sign_idx = ((deg.rem_euclid(360.0) / 30.0) as usize) % 12;
        elementos[sign_idx % 4].push(name);
        modalidades[sign_idx % 3].push(name);
        polaridad[sign_idx % 2].push(name);
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

/// Una fila de cualidad: etiqueta + barra (rect rellena sobre track) +
/// los glyphs de los cuerpos que caen ahí.
fn fila_cualidad(label: &str, bodies: &[&str], theme: &Theme) -> View<Msg> {
    let count = bodies.len();
    let frac = (count as f32 / 10.0).clamp(0.0, 1.0);

    let lbl = txt_cell(label.to_string(), 56.0, 11.0, theme.fg_text, Alignment::Start);

    // Barra: track + relleno proporcional.
    let bar = View::new(Style {
        size: Size {
            width: length(64.0_f32),
            height: length(8.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(3.0)
    .children(vec![View::new(Style {
        size: Size {
            width: percent(frac),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.accent)
    .radius(3.0)]);
    let bar_box = View::new(Style {
        size: Size {
            width: length(64.0_f32),
            height: length(ROW_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![bar]);

    let cnt = txt_cell(count.to_string(), 16.0, 11.0, theme.fg_muted, Alignment::Center);

    let mut cells = vec![lbl, bar_box, cnt];
    for b in bodies.iter().take(8) {
        cells.push(glyphs::body_view(b, GLYPH, theme.fg_text));
    }
    cells_row(cells)
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
            vec![line(rimay_localize::t("cosmos-empty"), 12.0, theme.fg_muted)],
            theme,
        );
    }
    let mut aspects: HashMap<(String, String), String> = HashMap::new();
    for a in &render.aspect_summary {
        if a.module_id != "natal" {
            continue;
        }
        let key = sorted_pair(&a.from_body, &a.to_body);
        aspects.insert(key, a.kind.clone());
    }
    const CELL: f32 = 24.0;
    let rows: Vec<View<Msg>> = bodies
        .iter()
        .enumerate()
        .map(|(i, body_i)| {
            let mut cells: Vec<View<Msg>> =
                vec![box_cell(Some(glyphs::body_view(body_i, GLYPH, theme.fg_text)), None)];
            for body_j in bodies.iter().take(i) {
                let pair = sorted_pair(body_i, body_j);
                match aspects.get(&pair) {
                    Some(k) => cells.push(box_cell(
                        Some(glyphs::aspect_view(k, GLYPH)),
                        Some(theme.bg_panel_alt),
                    )),
                    None => cells.push(box_cell(None, None)),
                }
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

fn box_cell(content: Option<View<Msg>>, bg: Option<Color>) -> View<Msg> {
    const CELL: f32 = 24.0;
    let mut v = View::new(Style {
        size: Size {
            width: length(CELL),
            height: length(CELL),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    });
    if let Some(c) = bg {
        v = v.fill(c).radius(2.0);
    }
    if let Some(child) = content {
        v = v.children(vec![child]);
    }
    v
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
    let glyphs_v: Vec<&cosmos_render::Glyph> = render
        .layers
        .iter()
        .filter(|l| {
            l.module_id == module_id
                && std::mem::discriminant(&l.kind) == std::mem::discriminant(&kind)
        })
        .flat_map(|l| l.glyphs.iter())
        .collect();
    if glyphs_v.is_empty() {
        return tile_container(vec![line(hint.to_string(), 12.0, theme.fg_muted)], theme);
    }
    let rows: Vec<View<Msg>> = glyphs_v
        .into_iter()
        .take(40)
        .map(|g| {
            let casa = g.house.map(|h| format!("h{h}")).unwrap_or_default();
            let dms = fmt_dms(g.deg.rem_euclid(30.0) as f64);
            // Lotes y estrellas traen una anotación textual; los puntos
            // medios y demás son cuerpos con glyph.
            let lead: View<Msg> = if g.symbol.starts_with("lot:") || g.symbol.starts_with('✦') {
                let label = g.annotation.clone().unwrap_or_else(|| g.symbol.clone());
                txt_cell(label, 96.0, 11.0, theme.fg_text, Alignment::Start)
            } else {
                glyphs::body_view(&g.symbol, GLYPH, theme.fg_text)
            };
            cells_row(vec![
                lead,
                txt_cell(dms, 56.0, 12.0, theme.fg_text, Alignment::Start),
                glyphs::sign_view(sign_id(g.deg), SGN, sign_color(g.deg)),
                txt_cell(casa, 30.0, 11.0, theme.fg_muted, Alignment::Start),
            ])
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
