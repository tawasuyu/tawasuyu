//! Vistas: barras top/bottom, el sidebar con tiled drag-to-swap y cada uno
//! de los tiles (carta, módulos, armónico, cuerpos, aspectos, box-graph,
//! cualidades, cartas, corpus, uraniano, layers genéricas). El tile
//! AstroCarto vive aparte en [`crate::astrocarto`] por su carga matemática.

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
use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_tiled::{tiled_view_reorderable_cols, TileSpec, TiledPalette};

use crate::astrocarto::tile_astrocarto;
use crate::format::{fmt_deg_sign, fmt_dms, signo_de_longitud, simbolo_aspecto, simbolo_cuerpo};
use crate::model::{Model, Msg, OverlayKind, TileId, HARMONICS, SIDEBAR_WIDTH};
use crate::persist::{chart_path, charts_dir, list_cards};

// =====================================================================
// Barras top/bottom
// =====================================================================

pub(crate) fn header_bar(m: &RenderModel, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(
        rimay_localize::t_args(
            "cosmos-header",
            &[
                ("title", m.title.as_str().into()),
                ("asc", format!("{:.1}", m.ascendant_deg).into()),
                ("mc", format!("{:.1}", m.midheaven_deg).into()),
            ],
        ),
        12.0,
        theme.fg_text,
        Alignment::Start,
    )
}

pub(crate) fn status_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let txt = if let Some(err) = &model.error {
        rimay_localize::t_args("cosmos-status-error", &[("err", err.as_str().into())])
    } else {
        rimay_localize::t_args(
            "cosmos-status",
            &[
                ("ms", model.render.compute_ms.to_string().into()),
                ("layers", model.render.layers.len().to_string().into()),
                ("overlays", model.render.overlays.len().to_string().into()),
                ("aspects", model.render.aspect_summary.len().to_string().into()),
            ],
        )
    };
    let color = if model.error.is_some() {
        theme.fg_text
    } else {
        theme.fg_muted
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(txt, 11.0, color, Alignment::Start)
}

// =====================================================================
// Sidebar — un solo panel con tiled vertical drag-to-swap
// =====================================================================

pub(crate) fn side_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = TiledPalette::from_theme(theme);
    let specs: Vec<TileSpec<Msg>> = model
        .panel_order
        .iter()
        .map(|tid| build_tile(*tid, model, theme))
        .collect();

    let tiled = tiled_view_reorderable_cols(
        specs,
        1,
        |from, to| {
            if from == to {
                None
            } else {
                Some(Msg::SwapTiles(from, to))
            }
        },
        &palette,
    );

    View::new(Style {
        size: Size {
            width: length(SIDEBAR_WIDTH),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![tiled])
}

fn build_tile(tid: TileId, model: &Model, theme: &Theme) -> TileSpec<Msg> {
    let label = rimay_localize::t(tid.label_key());
    let content = match tid {
        TileId::Carta => tile_carta(model, theme),
        TileId::Modulos => tile_modulos(model, theme),
        TileId::Armonico => tile_armonico(model, theme),
        TileId::Cuerpos => tile_cuerpos(&model.render, theme),
        TileId::Aspectos => tile_aspectos(&model.render, "natal", theme),
        TileId::BoxGraph => tile_box_graph(&model.render, theme),
        TileId::Cualidades => tile_cualidades(&model.render, theme),
        TileId::AstroCarto => tile_astrocarto(&model.chart, &model.render, theme),
        TileId::Cartas => tile_cartas(theme),
        TileId::Corpus => tile_corpus(&model.render, &model.corpus, theme),
        TileId::Uraniano => tile_uraniano(&model.render.uranian_groups, theme),
        TileId::Lotes => tile_layer_glyphs(&model.render, LayerKind::Lots, "lots", theme),
        TileId::EstrellasFijas => {
            tile_layer_glyphs(&model.render, LayerKind::FixedStars, "fixed_stars", theme)
        }
        TileId::PuntosMedios => {
            tile_layer_glyphs(&model.render, LayerKind::Midpoints, "midpoints", theme)
        }
        TileId::CrossTransit => tile_aspectos(&model.render, "transit", theme),
        TileId::CrossProgression => tile_aspectos(&model.render, "progression", theme),
        TileId::CrossSolarArc => tile_aspectos(&model.render, "solar_arc", theme),
    };
    TileSpec { label, content }
}

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
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
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

/// Fila de la tabla de aspectos — columnas: tipo (con color), par
/// de cuerpos, orbe, dirección (applying/separating). Usamos un Row
/// con cells de ancho fijo para que las columnas se alineen entre
/// filas.
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
    let size = 11.0_f32;
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
        cell(kind_code.to_string(), kind_color, 28.0),
        cell(from.to_string(), theme.fg_text, 38.0),
        cell(to.to_string(), theme.fg_text, 38.0),
        cell(orb_dms.to_string(), theme.fg_muted, 56.0),
        cell(dir.to_string(), theme.fg_muted, 22.0),
    ])
}

// ----- Cartas (librería multi-archivo) -----

fn tile_cartas(theme: &Theme) -> View<Msg> {
    let pal_btn = ButtonPalette::from_theme(theme);
    let mut rows: Vec<View<Msg>> = Vec::new();

    // Botón "duplicar actual" arriba — captura el chart presente en disco.
    rows.push(button_styled(
        rimay_localize::t("cosmos-cartas-duplicar"),
        Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(22.0_f32),
            },
            flex_shrink: 0.0,
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            margin: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(4.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Start),
            ..Default::default()
        },
        Alignment::Start,
        &pal_btn,
        Msg::DuplicarActual,
    ));

    let cards = list_cards();
    if cards.is_empty() {
        rows.push(line(
            rimay_localize::t("cosmos-cartas-vacio"),
            10.0,
            theme.fg_muted,
        ));
    } else {
        for name in cards {
            rows.push(button_styled(
                name.clone(),
                Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(20.0_f32),
                    },
                    flex_shrink: 0.0,
                    padding: Rect {
                        left: length(8.0_f32),
                        right: length(8.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    margin: Rect {
                        left: length(0.0_f32),
                        right: length(0.0_f32),
                        top: length(0.0_f32),
                        bottom: length(2.0_f32),
                    },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Start),
                    ..Default::default()
                },
                Alignment::Start,
                &pal_btn,
                Msg::CargarCarta(name),
            ));
        }
    }

    let path_hint = charts_dir()
        .map(|p| format!("dir: {}", p.display()))
        .unwrap_or_default();
    rows.push(line(path_hint, 9.0, theme.fg_muted));

    tile_container(rows, theme)
}

// ----- Carta -----

fn tile_carta(model: &Model, theme: &Theme) -> View<Msg> {
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

    let path_hint = chart_path()
        .map(|p| format!("edit: {}", p.display()))
        .unwrap_or_default();
    tile_container(
        vec![
            line(model.chart.label.clone(), 12.0, theme.fg_text),
            line(lugar, 10.0, theme.fg_muted),
            line(fecha, 10.0, theme.fg_muted),
            line(lat_long, 10.0, theme.fg_muted),
            line(angles, 10.0, theme.fg_text),
            line(path_hint, 9.0, theme.fg_muted),
        ],
        theme,
    )
}

// ----- Módulos (toggles de overlays) -----

fn tile_modulos(model: &Model, theme: &Theme) -> View<Msg> {
    let pal_off = ButtonPalette::from_theme(theme);
    let pal_on = ButtonPalette {
        bg: theme.accent,
        bg_hover: theme.accent,
        fg: theme.bg_panel,
        radius: pal_off.radius,
    };

    let rows: Vec<View<Msg>> = OverlayKind::all()
        .iter()
        .map(|kind| {
            let active = model.overlays.contains(kind);
            let palette = if active { &pal_on } else { &pal_off };
            button_styled(
                rimay_localize::t(kind.label()),
                Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(22.0_f32),
                    },
                    flex_shrink: 0.0,
                    padding: Rect {
                        left: length(8.0_f32),
                        right: length(8.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Start),
                    ..Default::default()
                },
                Alignment::Start,
                palette,
                Msg::ToggleOverlay(*kind),
            )
        })
        .collect();

    tile_container(rows, theme)
}

// ----- Armónico (selector H1/H4/H5/H7/H9) -----

fn tile_armonico(model: &Model, theme: &Theme) -> View<Msg> {
    let pal_off = ButtonPalette::from_theme(theme);
    let pal_on = ButtonPalette {
        bg: theme.accent,
        bg_hover: theme.accent,
        fg: theme.bg_panel,
        radius: pal_off.radius,
    };

    let btns: Vec<View<Msg>> = HARMONICS
        .iter()
        .map(|h| {
            let active = model.harmonic == *h;
            let palette = if active { &pal_on } else { &pal_off };
            button_styled(
                format!("H{h}"),
                Style {
                    size: Size {
                        width: length(44.0_f32),
                        height: length(22.0_f32),
                    },
                    margin: Rect {
                        left: length(0.0_f32),
                        right: length(4.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    padding: Rect {
                        left: length(0.0_f32),
                        right: length(0.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Center),
                    ..Default::default()
                },
                Alignment::Center,
                palette,
                Msg::SetHarmonic(*h),
            )
        })
        .collect();

    let row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(btns);

    tile_container(vec![row], theme)
}

// ----- Cuerpos -----

fn tile_cuerpos(render: &RenderModel, theme: &Theme) -> View<Msg> {
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
            // "℞" (U+211E) no está en LiberationSans — usamos "(R)".
            let retro = if g.retrograde { " (R)" } else { "" };
            let dignity = g.dignity_marker.clone().unwrap_or_default();
            let line_str = format!("{body} {dms} {sign}{casa}{retro}{dignity}");
            line(line_str, 11.0, theme.fg_text)
        })
        .collect();
    tile_container(rows, theme)
}

// ----- Aspectos (filtrado por module_id) -----

/// Ranking de importancia del aspecto: 0 = mayor (con/opp/squ/tri/sex),
/// 1 = menor (quincunx/semi-sextile/semi-square/sesquiquadrate/quintile).
/// Sortear primero por esta clave, después por orb ascendente, da la
/// tabla con los aspectos más fuertes y cerrados arriba.
fn aspecto_importancia(kind: &str) -> u8 {
    match kind {
        "conjunction" | "opposition" | "square" | "trine" | "sextile" => 0,
        _ => 1,
    }
}

/// Color del aspecto desde la paleta agnóstica de `cosmos-render`,
/// traducido al `peniko::Color` que usa Llimphi. Mismas decisiones
/// cromáticas que el wheel — así el color del aspecto en la tabla
/// concuerda con el color de la línea en el wheel.
fn aspecto_color(kind: &str) -> Color {
    let pal = cosmos_render::Palette::dark();
    let c = pal.aspect(kind);
    let to_byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(to_byte(c.r), to_byte(c.g), to_byte(c.b), to_byte(c.a))
}

fn tile_aspectos(render: &RenderModel, module_id: &str, theme: &Theme) -> View<Msg> {
    let mut asps: Vec<&AspectSummary> = render
        .aspect_summary
        .iter()
        .filter(|a| a.module_id == module_id)
        .collect();
    // Orden: primero los mayores; dentro de cada grupo, por orbe
    // ascendente (los más exactos arriba).
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
        .take(20)
        .map(|a| {
            let from = simbolo_cuerpo(&a.from_body);
            let to = simbolo_cuerpo(&a.to_body);
            let kind = simbolo_aspecto(&a.kind);
            let dms = fmt_dms(a.orb_deg);
            // ◂ ▸ (U+25C2/B8) no están en LiberationSans — usamos
            // ◄ ► (U+25C4/BA) que sí, y son visualmente idénticos.
            let dir = match a.applying {
                Some(true) => " ◄",
                Some(false) => " ►",
                None => "",
            };
            // El kind va coloreado por la paleta de aspectos; el
            // resto en fg_text — así la columna de tipo se distingue
            // visualmente de los nombres de cuerpo.
            aspect_row(kind, from, to, &dms, dir, a.kind.as_str(), theme)
        })
        .collect();
    if rows.is_empty() {
        return tile_container(
            vec![line(
                rimay_localize::t("cosmos-empty"),
                11.0,
                theme.fg_muted,
            )],
            theme,
        );
    }
    tile_container(rows, theme)
}

// ----- Uraniano (grupos del dial de 90°) -----

fn tile_uraniano(groups: &[UranianGroup], theme: &Theme) -> View<Msg> {
    if groups.is_empty() {
        return tile_container(
            vec![line(
                rimay_localize::t("cosmos-empty"),
                11.0,
                theme.fg_muted,
            )],
            theme,
        );
    }
    let rows: Vec<View<Msg>> = groups
        .iter()
        .take(16)
        .map(|g| {
            let bodies: Vec<String> = g.bodies.iter().map(|b| simbolo_cuerpo(b).into()).collect();
            line(
                format!("{:.1}°  {}", g.mod90_deg, bodies.join(" ")),
                11.0,
                theme.fg_text,
            )
        })
        .collect();
    tile_container(rows, theme)
}

// ----- Cualidades (elementos + modalidades + polaridad) -----

fn tile_cualidades(render: &RenderModel, theme: &Theme) -> View<Msg> {
    let bodies: Vec<(&str, f32)> = render
        .layers
        .iter()
        .filter(|l| l.module_id == "natal" && matches!(l.kind, LayerKind::Bodies))
        .flat_map(|l| l.glyphs.iter())
        .map(|g| (g.symbol.as_str(), g.deg))
        .collect();

    // Elementos: sign_idx % 4 → 0=Fuego, 1=Tierra, 2=Aire, 3=Agua.
    let mut elementos: [Vec<&'static str>; 4] = Default::default();
    // Modalidades: sign_idx % 3 → 0=Cardinal, 1=Fijo, 2=Mutable.
    let mut modalidades: [Vec<&'static str>; 3] = Default::default();
    // Polaridad: sign_idx % 2 → 0=Yang (fuego/aire), 1=Yin (tierra/agua).
    let mut polaridad: [Vec<&'static str>; 2] = Default::default();

    for (name, deg) in &bodies {
        let sign_idx = ((deg.rem_euclid(360.0) / 30.0) as usize) % 12;
        let glyph = simbolo_cuerpo(name);
        elementos[sign_idx % 4].push(glyph);
        modalidades[sign_idx % 3].push(glyph);
        polaridad[sign_idx % 2].push(glyph);
    }

    let elem_labels = [
        rimay_localize::t("cosmos-elem-fuego"),
        rimay_localize::t("cosmos-elem-tierra"),
        rimay_localize::t("cosmos-elem-aire"),
        rimay_localize::t("cosmos-elem-agua"),
    ];
    let mod_labels = [
        rimay_localize::t("cosmos-mod-cardinal"),
        rimay_localize::t("cosmos-mod-fijo"),
        rimay_localize::t("cosmos-mod-mutable"),
    ];
    let pol_labels = [
        rimay_localize::t("cosmos-pol-yang"),
        rimay_localize::t("cosmos-pol-yin"),
    ];

    let mut rows: Vec<View<Msg>> = Vec::new();
    rows.push(seccion_label(rimay_localize::t("cosmos-elementos"), theme));
    for (i, label) in elem_labels.iter().enumerate() {
        rows.push(fila_cualidad(label, &elementos[i], theme));
    }
    rows.push(seccion_label(rimay_localize::t("cosmos-modalidades"), theme));
    for (i, label) in mod_labels.iter().enumerate() {
        rows.push(fila_cualidad(label, &modalidades[i], theme));
    }
    rows.push(seccion_label(rimay_localize::t("cosmos-polaridad"), theme));
    for (i, label) in pol_labels.iter().enumerate() {
        rows.push(fila_cualidad(label, &polaridad[i], theme));
    }
    tile_container(rows, theme)
}

fn seccion_label(text: String, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(14.0_f32),
        },
        flex_shrink: 0.0,
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(2.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text, 10.0, theme.fg_muted, Alignment::Start)
}

fn fila_cualidad(label: &str, glyphs: &[&str], theme: &Theme) -> View<Msg> {
    let count = glyphs.len();
    let bar_len = count.min(10);
    // █ y ░ existen en LiberationSans/AdwaitaSans; ▰/▱ no, por eso
    // antes salían como cajitas .notdef.
    let bar: String = "█".repeat(bar_len) + &"░".repeat(10_usize.saturating_sub(bar_len));
    let glyph_str = glyphs.join(" ");
    let txt = format!("{label:>9}  {bar}  {count}  {glyph_str}");
    line(txt, 11.0, theme.fg_text)
}

// ----- Box graph (aspectarian triangular) -----

fn tile_box_graph(render: &RenderModel, theme: &Theme) -> View<Msg> {
    // 1. cuerpos natales en orden de longitud (estable porque la layer ya
    //    los emite en el orden canónico).
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
                11.0,
                theme.fg_muted,
            )],
            theme,
        );
    }
    // 2. mapa (par ordenado) → símbolo de aspecto.
    let mut aspects: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    for a in &render.aspect_summary {
        if a.module_id != "natal" {
            continue;
        }
        let key = sorted_pair(&a.from_body, &a.to_body);
        aspects.insert(key, a.kind.clone());
    }
    // 3. filas triangulares: fila i = etiqueta cuerpo i + i celdas.
    const CELL: f32 = 22.0;
    const LBL: f32 = 24.0;
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
    .text_aligned(text.to_string(), 11.0, fg, Alignment::Center);
    if let Some(c) = bg {
        v = v.fill(c).radius(2.0);
    }
    v
}

// ----- Tile genérico: lista de glyphs de una layer -----

fn tile_layer_glyphs(
    render: &RenderModel,
    kind: LayerKind,
    module_id: &str,
    theme: &Theme,
) -> View<Msg> {
    let glyphs: Vec<&cosmos_render::Glyph> = render
        .layers
        .iter()
        .filter(|l| l.module_id == module_id && std::mem::discriminant(&l.kind) == std::mem::discriminant(&kind))
        .flat_map(|l| l.glyphs.iter())
        .collect();
    if glyphs.is_empty() {
        return tile_container(
            vec![line(
                rimay_localize::t("cosmos-empty"),
                11.0,
                theme.fg_muted,
            )],
            theme,
        );
    }
    let rows: Vec<View<Msg>> = glyphs
        .into_iter()
        .take(20)
        .map(|g| {
            // Para lots viene "lot:Fo" — recortamos el prefijo y usamos annotation.
            let label = if g.symbol.starts_with("lot:") {
                g.annotation.clone().unwrap_or_else(|| g.symbol.clone())
            } else if g.symbol.starts_with("✦") {
                g.annotation.clone().unwrap_or_else(|| g.symbol.clone())
            } else {
                simbolo_cuerpo(&g.symbol).to_string()
            };
            let casa = g.house.map(|h| format!(" h{h}")).unwrap_or_default();
            let dms = fmt_dms(g.deg.rem_euclid(30.0) as f64);
            let sign = signo_de_longitud(g.deg);
            line(
                format!("{label}  {dms} {sign}{casa}"),
                11.0,
                theme.fg_text,
            )
        })
        .collect();
    tile_container(rows, theme)
}

fn sorted_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    }
}

// ----- Corpus (pasajes interpretativos) -----

fn tile_corpus(render: &RenderModel, corpus: &Corpus, theme: &Theme) -> View<Msg> {
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
            11.0,
            theme.fg_muted,
        ));
    } else {
        for p in pasajes.iter().take(8) {
            rows.push(line(p.combinacion.to_string(), 10.0, theme.fg_muted));
            // Texto del pasaje: lo cortamos a ~140 chars para que la fila
            // siga siendo de una sola línea visible en el sidebar.
            let txt = recortar(&p.texto, 140);
            rows.push(line(txt, 11.0, theme.fg_text));
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
