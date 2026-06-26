//! Gráficas centrales: rueda natal, esfera 3D, cielo del observador
//! y el área de pestañas con el segmented de tipo.
//!
//! Submodules: `dial` (dial uraniano de 90°), `armonico` (rueda armónica).
//! Todas las funciones son puras respecto al estado — reciben `&Model` y
//! devuelven `View<Msg>` (o `Vec<DrawCommand>`).

pub(super) mod armonico;
pub(super) mod dial;

use cosmos_canvas_llimphi::{canvas_view_clickable_ex, ViewTransform};
use cosmos_render::{
    compose_sphere, compose_wheel_with_hits, CompositionOpts, DrawCommand, Palette, Rgba,
    SphereOpts, SphereView, TextAnchor,
};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    style::FlexWrap,
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, PaintRect, View};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};

use crate::glyphs::{self, Icon};
use crate::model::{ChartView, Model, Msg, MENU_BAR_H, TAB_BAR_H, WHEEL_SIZE};

use super::dock::dock_rail_overlay;
use super::impresion::print_view;
use crate::model::DockSide;

// =====================================================================
// Helpers compartidos (usados por submódulos dial y armonico)
// =====================================================================

/// Paleta del lienzo según el tema activo. En modo impresión usa la
/// paleta clara sobre papel blanco (alto contraste para fotocopia).
pub(super) fn graphics_palette(model: &Model) -> Palette {
    if model.cfg.print_mode {
        Palette::print()
    } else if model.cfg.theme_dark {
        Palette::dark()
    } else {
        Palette::light()
    }
}

/// Fondo del lienzo según el tema activo.
pub(super) fn graphics_bg(model: &Model) -> Color {
    if model.cfg.print_mode {
        Color::WHITE
    } else if model.cfg.theme_dark {
        Color::from_rgba8(14, 15, 22, 255)
    } else {
        Color::from_rgba8(245, 246, 250, 255)
    }
}

/// Convierte un `Color` (peniko) a `Rgba` (cosmos-render).
pub(super) fn rgba_of(c: Color) -> Rgba {
    let [r, g, b, a] = c.to_rgba8().to_u8_array();
    Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
}

/// Normaliza alias de cuerpos a un id que `planet_commands` entienda.
pub(super) fn canon_glyph(sym: &str) -> String {
    match sym {
        "ascending_node" | "mean_node" => "north_node",
        "descending_node" => "south_node",
        other => other,
    }
    .to_string()
}

/// Color elemental de un signo por índice, según el tema.
pub(super) fn sign_color_theme(sign_idx: usize, model: &Model) -> Color {
    let pal = graphics_palette(model);
    let ids = crate::glyphs::SIGN_IDS;
    let c = pal.sign(ids[sign_idx % 12]);
    Color::from_rgba8(
        (c.r.clamp(0.0, 1.0) * 255.0) as u8,
        (c.g.clamp(0.0, 1.0) * 255.0) as u8,
        (c.b.clamp(0.0, 1.0) * 255.0) as u8,
        (c.a.clamp(0.0, 1.0) * 255.0) as u8,
    )
}

/// Longitudes eclípticas de los cuerpos natales (símbolo → grados).
pub(super) fn natal_body_lons(render: &cosmos_render::RenderModel) -> Vec<(String, f32)> {
    render
        .layers
        .iter()
        .filter(|l| {
            l.module_id == "natal" && matches!(l.kind, cosmos_render::LayerKind::Bodies)
        })
        .flat_map(|l| l.glyphs.iter())
        .map(|g| (g.symbol.clone(), g.deg))
        .collect()
}

/// Longitudes eclípticas topocéntricas (símbolo → grados), si el overlay
/// topocéntrico está activo. Vacío si no.
pub(super) fn topo_body_lons(render: &cosmos_render::RenderModel) -> Vec<(String, f32)> {
    render
        .layers
        .iter()
        .filter(|l| {
            l.module_id == "topocentric" && matches!(l.kind, cosmos_render::LayerKind::Bodies)
        })
        .flat_map(|l| l.glyphs.iter())
        .map(|g| (g.symbol.clone(), g.deg))
        .collect()
}

// =====================================================================
// Columna de canvas con controles opcionales
// =====================================================================

/// Arma la columna `[controles?, lienzo]`. Con `fill` el lienzo crece para
/// ocupar todo el espacio (fondo a sangre, recortado para no pisar los
/// paneles vecinos); sin `fill` queda en una caja de lado `size`.
pub(super) fn canvas_column(
    controls: Option<View<Msg>>,
    canvas: View<Msg>,
    size: f32,
    fill: bool,
) -> View<Msg> {
    let canvas_box = if fill {
        View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(0.0_f32),
            },
            min_size: Size {
                width: length(0.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .clip(true)
        .children(vec![canvas])
    } else {
        View::new(Style {
            size: Size {
                width: length(size),
                height: length(size),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![canvas])
    };
    let mut kids: Vec<View<Msg>> = Vec::new();
    if let Some(c) = controls {
        kids.push(c);
    }
    kids.push(canvas_box);
    let style = if fill {
        Style {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            min_size: Size {
                width: length(0.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(0.0_f32),
                height: length(4.0_f32),
            },
            ..Default::default()
        }
    } else {
        Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: length(size),
                height: auto(),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(0.0_f32),
                height: length(4.0_f32),
            },
            ..Default::default()
        }
    };
    View::new(style).children(kids)
}

// =====================================================================
// Botonera de zoom / encuadre
// =====================================================================

/// Botonera de zoom/encuadre del lienzo de la rueda.
pub(super) fn zoom_controls(model: &Model, theme: &Theme) -> View<Msg> {
    let btn = |icon: Icon, msg: Msg| -> View<Msg> {
        View::new(Style {
            size: Size {
                width: length(26.0_f32),
                height: length(24.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .radius(4.0)
        .fill(theme.bg_panel)
        .hover_fill(theme.bg_row_hover)
        .on_click(msg)
        .children(vec![glyphs::icon_view(icon, 15.0, theme.fg_text)])
    };
    let pct = View::new(Style {
        size: Size {
            width: length(46.0_f32),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(
        format!("{:.0}%", model.wheel_zoom * 100.0),
        11.0,
        theme.fg_muted,
        Alignment::Center,
    );
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(4.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        btn(Icon::ZoomOut, Msg::WheelZoom(0.8)),
        pct,
        btn(Icon::ZoomIn, Msg::WheelZoom(1.25)),
        btn(Icon::Refresh, Msg::WheelResetView),
    ])
}

/// Envuelve un lienzo custom (sin hit-test de cuerpos) en la columna con
/// botonera de zoom + zoom/paneo, igual que la rueda estándar.
pub(super) fn custom_canvas(model: &Model, cmds: Vec<DrawCommand>, size: f32, theme: &Theme, fill: bool) -> View<Msg> {
    let t = ViewTransform {
        zoom: model.wheel_zoom,
        pan: model.wheel_pan,
    };
    let canvas = cosmos_canvas_llimphi::canvas_view_ex::<Msg>(cmds, size, Some(graphics_bg(model)), t)
        .draggable_at(|phase, dx, dy, _lx, _ly| match phase {
            DragPhase::Move => Some(Msg::WheelPan(dx, dy)),
            DragPhase::End => None,
        });
    canvas_column(Some(zoom_controls(model, theme)), canvas, size, fill)
}

// =====================================================================
// Rueda natal estándar
// =====================================================================

/// La rueda natal 2D como canvas clickeable (sólo el gráfico), de la carta
/// cuyo `render` se pasa, al tamaño `size`.
fn wheel_canvas(model: &Model, render: &cosmos_render::RenderModel, size: f32, theme: &Theme, fill: bool) -> View<Msg> {
    let opts = CompositionOpts {
        size,
        rot_offset_deg: model.cfg.rot_offset_deg,
        include_bodies: true,
        palette: graphics_palette(model),
        draw_ascensional_cross: model.cfg.asc_cross,
        show_coord_labels: model.cfg.coord_labels,
        show_minor_aspects: model.cfg.minor_aspects,
        dial_3d: model.cfg.dial_3d,
        selected_body: model.selected_body.clone(),
        // El zoom de la rueda re-dibuja con más detalle (no magnifica el
        // bitmap): se mete como `detail`, no como escala uniforme.
        detail: model.wheel_zoom,
    };
    let (commands, hits) = compose_wheel_with_hits(render, &opts);
    let canvas_bg = graphics_bg(model);
    // Offset del menú contextual: origen del centro ≈ nav (resizable) +
    // barra de menú + cabecera del switcher. (Aprox. en mosaico.)
    let nav_off = model.nav_w + if model.nav_open { 6.0 } else { 0.0 };
    // Sin escala uniforme: el detalle ya lo aplicó `compose_wheel`. Sólo paneo.
    let t = ViewTransform {
        zoom: 1.0,
        pan: model.wheel_pan,
    };
    let canvas = canvas_view_clickable_ex::<Msg, _>(commands, size, Some(canvas_bg), t, move |wx, wy| {
        let picked: Option<String> = hits.pick(wx, wy).map(str::to_string);
        Some(Msg::SelectBody(picked))
    })
    // Drag: paneo del lienzo. Coexiste con el on_click_at (el press
    // selecciona el cuerpo; el movimiento panea). La rueda (zoom/paneo
    // con Ctrl/Alt) la maneja App::on_wheel.
    .draggable_at(|phase, dx, dy, _lx, _ly| match phase {
        DragPhase::Move => Some(Msg::WheelPan(dx, dy)),
        DragPhase::End => None,
    })
    .on_right_click_at(move |lx, ly, _w, _h| {
        Some(Msg::OpenCanvasCtx(nav_off + lx, MENU_BAR_H + TAB_BAR_H + ly))
    });

    canvas_column(Some(zoom_controls(model, theme)), canvas, size, fill)
}

// =====================================================================
// Esfera 3D
// =====================================================================

/// Esfera celeste 3D sobre el motor GPU **`llimphi-3d`** (asimilado
/// 2026-06-24). La geometría (cuentas de la eclíptica/ecuador/cuerpos…) se
/// arma en CPU desde el `RenderModel` y se dibuja en `gpu_paint_with` con
/// depth real y cámara en perspectiva. Drag rota yaw/pitch; la rueda
/// (zoom) acerca/aleja la cámara. La botonera ◀▶▲▼⟳ espeja el drag.
fn sphere_canvas(model: &Model, render: &cosmos_render::RenderModel, size: f32, theme: &Theme, fill: bool) -> View<Msg> {
    let pal = graphics_palette(model);
    let geom = crate::sphere_gpu::sphere_geometry(render, &pal);
    let labels = crate::sphere_gpu::sphere_labels(render, &pal);
    let bg = graphics_bg(model);
    let yaw = model.sphere_yaw;
    let pitch = model.sphere_pitch;
    // Zoom de la rueda → distancia de cámara (acercar = menor distancia).
    let dist = (3.0 / model.wheel_zoom.max(0.25)).clamp(1.7, 9.0);
    let slot = model.sphere_gpu.clone();

    let canvas = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .gpu_paint_with(move |device, queue, encoder, target, rect: PaintRect, vp| {
        crate::sphere_gpu::paint(
            &slot,
            device,
            queue,
            encoder,
            target,
            vp,
            (rect.x, rect.y, rect.w, rect.h),
            &geom,
            yaw,
            pitch,
            dist,
        );
    })
    // Etiquetas (signos, ASC/MC, glifos de cuerpos) en vello ENCIMA del pase GPU:
    // se proyecta cada ancla 3D a pantalla con la misma cámara. Los signos y
    // cuerpos usan glifos VECTORIALES (trazos, sin fuente → sin tofu); los
    // ángulos, texto plano ASCII.
    .paint_over(move |scene, ts, rect: PaintRect| {
        use cosmos_render::glyphs::{planet_commands, sign_commands};
        use crate::sphere_gpu::LabelKind;
        use llimphi_ui::llimphi_raster::peniko::Color as PColor;
        use llimphi_ui::llimphi_text::{draw_layout, layout_block, Alignment, TextBlock};
        let r = (rect.x, rect.y, rect.w, rect.h);
        for lab in &labels {
            let Some((sx, sy)) = crate::sphere_gpu::project_label(lab.world, r, yaw, pitch, dist)
            else {
                continue;
            };
            let col = Rgba { r: lab.color[0], g: lab.color[1], b: lab.color[2], a: lab.color[3] };
            let sw = (lab.size * 0.09).max(1.0);
            match &lab.kind {
                LabelKind::Sign(name) => {
                    let cmds = sign_commands(name, sx, sy, lab.size, col, sw);
                    cosmos_canvas_llimphi::paint_commands(scene, ts, &cmds);
                }
                LabelKind::Planet(name) => {
                    let cmds = planet_commands(name, sx, sy, lab.size, col, sw);
                    cosmos_canvas_llimphi::paint_commands(scene, ts, &cmds);
                }
                LabelKind::Text(txt) => {
                    let c = PColor::new([lab.color[0], lab.color[1], lab.color[2], lab.color[3]]);
                    let approx = lab.size as f64 * txt.chars().count() as f64 * 0.5;
                    let block = TextBlock {
                        text: txt,
                        size_px: lab.size,
                        color: c,
                        origin: (sx as f64 - approx, sy as f64 - lab.size as f64 * 0.5),
                        max_width: Some(approx as f32 * 2.0),
                        alignment: Alignment::Center,
                        line_height: 1.0,
                        italic: false,
                        font_family: None,
                    };
                    let layout = layout_block(ts, &block);
                    draw_layout(scene, &layout, c, block.origin);
                }
            }
        }
    })
    // Agarre tipo "globo": la esfera sigue al cursor (horizontal invertido).
    .draggable_at(|phase, dx, dy, _lx, _ly| match phase {
        DragPhase::Move => Some(Msg::SphereRotate(-dx * 0.3, dy * 0.3)),
        DragPhase::End => None,
    });
    canvas_column(Some(sphere_controls(theme)), canvas, size, fill)
}

/// Botonera de rotación de la esfera 3D.
fn sphere_controls(theme: &Theme) -> View<Msg> {
    let step = 15.0_f32;
    let btn = |icon: Icon, msg: Msg| -> View<Msg> {
        View::new(Style {
            size: Size {
                width: length(30.0_f32),
                height: length(24.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .radius(4.0)
        .fill(theme.bg_panel)
        .hover_fill(theme.bg_row_hover)
        .on_click(msg)
        .children(vec![glyphs::icon_view(icon, 14.0, theme.fg_text)])
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(4.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        btn(Icon::ArrowLeft, Msg::SphereRotate(-step, 0.0)),
        btn(Icon::ArrowRight, Msg::SphereRotate(step, 0.0)),
        btn(Icon::ArrowUp, Msg::SphereRotate(0.0, -step)),
        btn(Icon::ArrowDown, Msg::SphereRotate(0.0, step)),
        btn(Icon::Refresh, Msg::SphereReset),
    ])
}

// =====================================================================
// Esfera 2.5D (la vista de alambre vello — referencia de paridad)
// =====================================================================

/// La esfera celeste "2.5D": `compose_sphere` (alambre vello proyectado a mano)
/// pintado como `DrawCommand`s. Comparte el mismo `yaw`/`pitch` que la 3D GPU
/// (drag rota, la rueda acerca) para comparar lado a lado qué falta parear.
fn sphere25_canvas(model: &Model, render: &cosmos_render::RenderModel, size: f32, theme: &Theme, fill: bool) -> View<Msg> {
    let opts = SphereOpts {
        size,
        palette: graphics_palette(model),
        ..SphereOpts::default()
    };
    let view = SphereView {
        yaw_deg: model.sphere_yaw,
        pitch_deg: model.sphere_pitch,
    };
    let cmds = compose_sphere(render, &view, &opts);
    let t = ViewTransform {
        zoom: model.wheel_zoom,
        pan: model.wheel_pan,
    };
    let canvas = cosmos_canvas_llimphi::canvas_view_ex::<Msg>(cmds, size, Some(graphics_bg(model)), t)
        .draggable_at(|phase, dx, dy, _lx, _ly| match phase {
            DragPhase::Move => Some(Msg::SphereRotate(-dx * 0.3, dy * 0.3)),
            DragPhase::End => None,
        });
    canvas_column(Some(sphere_controls(theme)), canvas, size, fill)
}

// =====================================================================
// Cielo del observador (proyección azimutal)
// =====================================================================

fn pending_view(msg: &str, theme: &Theme) -> View<Msg> {
    crate::view::tile_container(
        vec![crate::view::line(msg.to_string(), 12.0, theme.fg_muted)],
        theme,
    )
}

/// Controles del Cielo: alterna cénit/nadir.
fn sky_controls(nadir: bool, theme: &Theme) -> View<Msg> {
    let label = if nadir { "Ver cénit ↑" } else { "Ver nadir ↓" };
    let btn = View::new(Style {
        size: Size {
            width: auto(),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(4.0)
    .fill(theme.bg_panel)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::ToggleSkyNadir)
    .text_aligned(label.to_string(), 11.0, theme.fg_text, Alignment::Center);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![btn])
}

/// Cielo del observador: proyección azimutal (cénit al centro, horizonte
/// al borde) de los cuerpos en alt/az. Compone `DrawCommand`s y los pinta
/// en el mismo canvas que la rueda. Usa `model.astro` (la lectura
/// astronómica cacheada); si todavía no está, muestra "calculando…".
fn sky_canvas(model: &Model, size: f32, theme: &Theme, fill: bool) -> View<Msg> {
    let Some(astro) = &model.astro else {
        return pending_view("Cielo del observador — calculando…", theme);
    };
    let dark = model.cfg.theme_dark;
    let nadir = model.sky_nadir;
    let zoom = model.wheel_zoom as f64;
    let pan = model.wheel_pan;
    let rect_cell = model.carto_rect.clone();
    let lst = astro.lst_deg;
    let lat = astro.lat_deg;
    let pal = graphics_palette(model);
    // Cuerpos: (nombre canónico, altitud°, azimut°).
    let bodies: Vec<(String, f64, f64)> = astro
        .sky
        .iter()
        .map(|(b, p)| (b.canonical().to_string(), p.altitude_deg, p.azimuth_deg))
        .collect();
    let fg_text = rgba_of(theme.fg_text);
    let fg_muted = rgba_of(theme.fg_muted);
    let border = rgba_of(theme.border);
    let bg = graphics_bg(model);

    let canvas = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .clip(true)
    // Arrastrar panea la cúpula (con zoom hace falta para recorrerla).
    .draggable_at(|phase, dx, dy, _lx, _ly| match phase {
        DragPhase::Move => Some(Msg::WheelPan(dx, dy)),
        DragPhase::End => None,
    })
    .paint_with(move |scene, ts, rect: PaintRect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle as KCircle, Line as KLine, Stroke};
        use llimphi_ui::llimphi_raster::peniko::{Color as PColor, Fill};
        use llimphi_ui::llimphi_text::{draw_layout, layout_block, Alignment, TextBlock};

        // Deja el rect para que `on_wheel` haga zoom hacia el cursor.
        if let Ok(mut g) = rect_cell.lock() {
            *g = Some((rect.x, rect.y, rect.w, rect.h));
        }
        // Centro desplazado por el paneo, radio escalado por el zoom.
        let cx = rect.x as f64 + rect.w as f64 * 0.5 + pan.0 as f64;
        let cy = rect.y as f64 + rect.h as f64 * 0.5 + pan.1 as f64;
        let r = (rect.w.min(rect.h) as f64) * 0.42 * zoom;
        let id = Affine::IDENTITY;
        let col = |c: Rgba| {
            PColor::from_rgba8(
                (c.r * 255.0) as u8,
                (c.g * 255.0) as u8,
                (c.b * 255.0) as u8,
                (c.a.clamp(0.0, 1.0) * 255.0) as u8,
            )
        };
        let disc = |scene: &mut llimphi_ui::llimphi_raster::vello::Scene, x: f64, y: f64, rad: f64, c: PColor| {
            scene.fill(Fill::NonZero, id, c, None, &KCircle::new((x, y), rad));
        };
        let ring = |scene: &mut llimphi_ui::llimphi_raster::vello::Scene, x: f64, y: f64, rad: f64, w: f64, c: PColor| {
            scene.stroke(&Stroke::new(w), id, c, None, &KCircle::new((x, y), rad));
        };
        let seg = |scene: &mut llimphi_ui::llimphi_raster::vello::Scene, a: (f64, f64), b: (f64, f64), w: f64, c: PColor| {
            scene.stroke(&Stroke::new(w), id, c, None, &KLine::new(a, b));
        };
        let text = |scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
                    ts: &mut llimphi_ui::llimphi_text::Typesetter,
                    x: f64,
                    y: f64,
                    s: &str,
                    size_px: f32,
                    c: PColor,
                    center: bool| {
            let approx = size_px as f64 * s.chars().count() as f64 * 0.5;
            let block = TextBlock {
                text: s,
                size_px,
                color: c,
                origin: (if center { x - approx } else { x }, y - size_px as f64 * 0.5),
                max_width: if center { Some(approx as f32 * 2.0) } else { None },
                alignment: if center { Alignment::Center } else { Alignment::Start },
                line_height: 1.0,
                italic: false,
                font_family: None,
            };
            let layout = layout_block(ts, &block);
            draw_layout(scene, &layout, c, block.origin);
        };

        // alt/az del observador para una posición ecuatorial.
        let radec_altaz = move |ra: f64, dec: f64| -> (f64, f64) {
            let h = ((lst - ra).rem_euclid(360.0)).to_radians();
            let decr = dec.to_radians();
            let latr = lat.to_radians();
            let sin_alt = decr.sin() * latr.sin() + decr.cos() * latr.cos() * h.cos();
            let alt = sin_alt.clamp(-1.0, 1.0).asin().to_degrees();
            let a_south = h.sin().atan2(h.cos() * latr.sin() - decr.tan() * latr.cos());
            let az = (a_south.to_degrees() + 180.0).rem_euclid(360.0);
            (alt, az)
        };
        // Cúpula azimutal: (alt°, az°) → (x, y, visible). En modo cénit el
        // centro es el cénit y se ve el hemisferio sobre el horizonte; en
        // nadir el centro es el nadir, el este-oeste se espeja (como mirar
        // hacia abajo) y se ve el hemisferio bajo el horizonte.
        let dome = move |alt: f64, az: f64| -> (f64, f64, bool) {
            let azr = az.to_radians();
            if !nadir {
                let rr = r * (90.0 - alt) / 90.0;
                (cx + rr * azr.sin(), cy - rr * azr.cos(), alt > 0.0)
            } else {
                let rr = r * (90.0 + alt) / 90.0;
                (cx - rr * azr.sin(), cy - rr * azr.cos(), alt < 0.0)
            }
        };

        // --- Disco del cielo ---
        let dome_fill = if dark {
            PColor::from_rgba8(7, 9, 16, 255)
        } else {
            PColor::from_rgba8(232, 238, 246, 255)
        };
        disc(scene, cx, cy, r, dome_fill);

        // --- Malla ecuatorial: meridianos de AR y paralelos de declinación ---
        let polyline = |scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
                        pts: &[(f64, f64)],
                        w: f64,
                        c: PColor| {
            let mut prev: Option<(f64, f64, bool)> = None;
            for &(ra, dec) in pts {
                let (alt, az) = radec_altaz(ra, dec);
                let (x, y, vis) = dome(alt, az);
                if let Some((px, py, pv)) = prev {
                    if vis && pv {
                        seg(scene, (px, py), (x, y), w, c);
                    }
                }
                prev = Some((x, y, vis));
            }
        };
        let grid_eq = col(fg_muted.with_alpha(0.14));
        // Meridianos de AR cada 30° (2 h), de declinación −80° a +80°.
        for h in 0..12 {
            let ra = h as f64 * 30.0;
            let pts: Vec<(f64, f64)> = (-8..=8).map(|j| (ra, j as f64 * 10.0)).collect();
            polyline(scene, &pts, 0.5, grid_eq);
        }
        // Paralelos de declinación; el ecuador celeste (0°) algo más marcado.
        for &d in &[-60.0_f64, -30.0, 0.0, 30.0, 60.0] {
            let pts: Vec<(f64, f64)> = (0..=72).map(|i| (i as f64 * 5.0, d)).collect();
            let w = if d == 0.0 { 0.7 } else { 0.5 };
            let c = if d == 0.0 { col(fg_muted.with_alpha(0.22)) } else { grid_eq };
            polyline(scene, &pts, w, c);
        }

        // --- Eclíptica: el camino del Sol, círculo máximo (tono cálido) ---
        let eps = 23.4393_f64.to_radians();
        let ecl_pts: Vec<(f64, f64)> = (0..=180)
            .map(|i| {
                let lam = (i as f64 * 2.0).to_radians();
                let ra = (lam.sin() * eps.cos()).atan2(lam.cos()).to_degrees().rem_euclid(360.0);
                let dec = (lam.sin() * eps.sin()).asin().to_degrees();
                (ra, dec)
            })
            .collect();
        let ecl_col = col(Rgba { r: 0.93, g: 0.74, b: 0.36, a: 1.0 }.with_alpha(0.55));
        polyline(scene, &ecl_pts, 1.1, ecl_col);

        // --- Figuras de constelaciones (tenues) + sus estrellas como puntos ---
        let cons_col = col(fg_muted.with_alpha(0.34));
        let cstar = if dark {
            Rgba { r: 0.78, g: 0.82, b: 0.95, a: 0.5 }
        } else {
            Rgba { r: 0.20, g: 0.24, b: 0.34, a: 0.5 }
        };
        for fig in cosmos_render::constellations_data::FIGURAS {
            for path in fig.paths {
                for s in path.windows(2) {
                    let (a_alt, a_az) = radec_altaz(s[0].0 as f64, s[0].1 as f64);
                    let (b_alt, b_az) = radec_altaz(s[1].0 as f64, s[1].1 as f64);
                    let (ax, ay, au) = dome(a_alt, a_az);
                    let (bx, by, bu) = dome(b_alt, b_az);
                    if au && bu {
                        seg(scene, (ax, ay), (bx, by), 0.6, cons_col);
                    }
                }
                // Los vértices del trazo son estrellas: puntitos discretos.
                for &(ra, dec) in path.iter() {
                    let (alt, az) = radec_altaz(ra as f64, dec as f64);
                    let (x, y, vis) = dome(alt, az);
                    if vis {
                        disc(scene, x, y, (r * 0.0035).max(0.7), col(cstar));
                    }
                }
            }
        }

        // --- Estrellas brillantes reales: tamaño/brillo por magnitud ---
        for st in cosmos_render::sky_data::BRIGHT_STARS {
            let (alt, az) = radec_altaz(st.ra_deg as f64, st.dec_deg as f64);
            let (x, y, vis) = dome(alt, az);
            if !vis {
                continue;
            }
            // mag −1.5 (Sirio) → brillante; mag 1.65 → tenue.
            let b = (((1.8 - st.mag as f64) / 3.4).clamp(0.12, 1.0)).powf(0.8);
            let rad = r * (0.006 + 0.013 * b);
            let star_c = if dark {
                Rgba { r: 0.86, g: 0.90, b: 1.0, a: (0.55 + 0.45 * b) as f32 }
            } else {
                Rgba { r: 0.10, g: 0.13, b: 0.22, a: (0.55 + 0.45 * b) as f32 }
            };
            disc(scene, x, y, rad, col(star_c));
            // Destello en cruz para las muy brillantes.
            if st.mag < 0.6 {
                let ray = rad * 2.6;
                let rc = col(star_c.with_alpha(star_c.a * 0.6));
                seg(scene, (x - ray, y), (x + ray, y), 0.8, rc);
                seg(scene, (x, y - ray), (x, y + ray), 0.8, rc);
            }
            // Nombre de las más brillantes.
            if st.mag < 1.0 {
                text(scene, ts, x, y - rad - 6.0, st.name, 9.0, col(fg_muted.with_alpha(0.85)), true);
            }
        }

        // --- Anillos de altitud + cruz de cardinales ---
        let grid_c = col(border.with_alpha(0.9));
        ring(scene, cx, cy, r, 1.4, grid_c);
        for alt in [30.0_f64, 60.0] {
            let rr = r * (90.0 - alt) / 90.0;
            ring(scene, cx, cy, rr, 0.6, col(border.with_alpha(0.5)));
            // Etiqueta de altitud sobre el meridiano norte.
            let (lx, ly, _) = dome(alt, 0.0);
            text(scene, ts, lx + 3.0, ly, &format!("{}°", alt as i32), 8.5, col(fg_muted.with_alpha(0.7)), false);
        }
        seg(scene, (cx - r, cy), (cx + r, cy), 0.6, col(border.with_alpha(0.5)));
        seg(scene, (cx, cy - r), (cx, cy + r), 0.6, col(border.with_alpha(0.5)));
        // Cardinales — posición vía la proyección (espeja sola en nadir).
        for (txt, az) in [("N", 0.0_f64), ("E", 90.0), ("S", 180.0), ("O", 270.0)] {
            let (x, y, _) = dome(0.0, az);
            let ux = (x - cx) * 1.06 + cx;
            let uy = (y - cy) * 1.06 + cy;
            text(scene, ts, ux, uy, txt, 13.0, col(fg_muted), true);
        }

        // --- Planetas con personalidad: color propio, tamaño por brillo,
        //     adornos (rayos del Sol, anillo de Saturno) ---
        for (name, alt, az) in &bodies {
            let (x, y, vis) = dome(*alt, *az);
            if !vis {
                continue;
            }
            let pc = pal.planet(name);
            // Presencia aparente de cada cuerpo (no a escala — legibilidad).
            let k = match name.as_str() {
                "sun" => 2.7,
                "moon" => 2.4,
                "jupiter" => 1.9,
                "venus" => 1.8,
                "saturn" => 1.6,
                "mars" => 1.4,
                "mercury" => 1.05,
                "uranus" => 1.1,
                "neptune" => 1.1,
                "pluto" => 0.85,
                _ => 1.2,
            };
            let rad = r * 0.011 * k;
            // Halo suave del color del cuerpo.
            disc(scene, x, y, rad * 1.9, col(pc.with_alpha(0.18)));
            disc(scene, x, y, rad, col(pc));
            ring(scene, x, y, rad, 1.0, col(pc.with_alpha(0.9)));
            match name.as_str() {
                "sun" => {
                    let rc = col(pc.with_alpha(0.85));
                    for k8 in 0..8 {
                        let a = std::f64::consts::PI * k8 as f64 / 4.0;
                        let (s, c) = a.sin_cos();
                        seg(scene, (x + c * rad * 1.4, y + s * rad * 1.4), (x + c * rad * 2.2, y + s * rad * 2.2), 1.0, rc);
                    }
                }
                "saturn" => {
                    // Anillo inclinado.
                    let rc = col(pc.with_alpha(0.9));
                    scene.stroke(
                        &Stroke::new(1.0),
                        Affine::translate((x, y)) * Affine::rotate(-0.5) * Affine::scale_non_uniform(1.0, 0.42),
                        rc,
                        None,
                        &KCircle::new((0.0, 0.0), rad * 1.7),
                    );
                }
                _ => {}
            }
            text(scene, ts, x, y - rad - 7.0, crate::format::simbolo_cuerpo(name), 10.0, col(fg_text), true);
        }

        // --- Encabezado: modo + lugar ---
        let modo = if nadir { "Nadir (hemisferio bajo el horizonte)" } else { "Cénit (cielo visible)" };
        text(scene, ts, rect.x as f64 + 8.0, rect.y as f64 + rect.h as f64 - 10.0, modo, 9.5, col(fg_muted.with_alpha(0.85)), false);
    });

    canvas_column(Some(sky_controls(nadir, theme)), canvas, size, fill)
}

// =====================================================================
// Gráfica por tipo y área central (pestañas + switcher + gráfica)
// =====================================================================

/// La gráfica elegida (según `chart_view`) para una carta/render dados, al
/// tamaño `size`. Reusada por la vista única y por cada celda del mosaico.
/// `fill = true` (vista única): el lienzo ocupa toda el área central.
/// `fill = false` (mosaico): lienzo de lado fijo `size`.
fn graphic_for(
    model: &Model,
    chart: &cosmos_model::Chart,
    render: &cosmos_render::RenderModel,
    size: f32,
    theme: &Theme,
    fill: bool,
) -> View<Msg> {
    match model.chart_view {
        ChartView::Estandar => wheel_canvas(model, render, size, theme, fill),
        ChartView::Uraniano => dial::uranian_dial_canvas(model, render, size, theme, fill),
        ChartView::Armonica => armonico::harmonic_wheel_canvas(model, render, size, theme, fill),
        ChartView::Carto => crate::astrocarto::tile_astrocarto(
            chart,
            render,
            theme,
            model.wheel_zoom,
            model.wheel_pan,
            model.carto_rect.clone(),
        ),
        ChartView::Esfera25D => sphere25_canvas(model, render, size, theme, fill),
        ChartView::Esfera3d => sphere_canvas(model, render, size, theme, fill),
        ChartView::Cielo => sky_canvas(model, size, theme, fill),
        ChartView::Impresion => print_view(model, theme),
    }
}

/// Una celda del mosaico: etiqueta (clickeable → activa la carta) + su
/// gráfica a tamaño reducido.
fn tile_cell(model: &Model, i: usize, tab: &crate::model::OpenTab, theme: &Theme) -> View<Msg> {
    let active = i == model.active_tab;
    let label = View::new(Style {
        size: Size {
            width: length(super::TILE_SIZE),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(if active { theme.bg_selected } else { theme.bg_panel })
    .radius(4.0)
    .text_aligned(tab.label().to_string(), 11.0, theme.fg_text, Alignment::Center)
    .on_click(Msg::ActivateChartTab(i));

    let g = graphic_for(model, &tab.chart, &tab.render, super::TILE_SIZE, theme, false);
    super::impresion::tile_cell_panel(label, g, theme, super::TILE_SIZE)
}

/// Tira de pestañas de cartas abiertas (multi-carta). Cada pestaña: label
/// clickeable + ✕ para cerrar. La activa va resaltada.
fn chart_tabs(model: &Model, theme: &Theme) -> View<Msg> {
    let mut kids: Vec<View<Msg>> = Vec::new();
    for (i, tab) in model.open.iter().enumerate() {
        let active = i == model.active_tab;
        let label = View::new(Style {
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(10.0_f32),
                right: length(6.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(tab.label().to_string(), 12.0, theme.fg_text, Alignment::Center)
        .on_click(Msg::ActivateChartTab(i));
        let close = View::new(Style {
            size: Size {
                width: length(18.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::CloseChartTab(i))
        .children(vec![glyphs::icon_view(Icon::Close, 11.0, theme.fg_muted)]);

        let mut tabv = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            margin: Rect {
                left: length(0.0_f32),
                right: length(2.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![label, close]);
        tabv = if active {
            tabv.fill(theme.bg_app)
        } else {
            tabv.fill(theme.bg_panel)
        };
        kids.push(tabv);
    }

    // Relleno + botón de alternar pestañas/mosaico (a la derecha).
    kids.push(
        View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            ..Default::default()
        }),
    );
    let (toggle_icon, toggle_label) = if model.tile_mode {
        (Icon::Window, "pestañas")
    } else {
        (Icon::Grid, "mosaico")
    };
    kids.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            gap: Size {
                width: length(4.0_f32),
                height: length(0.0_f32),
            },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::ToggleTileMode)
        .children(vec![
            glyphs::icon_view(toggle_icon, 14.0, theme.fg_muted),
            View::new(Style {
                size: Size {
                    width: auto(),
                    height: percent(1.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(toggle_label.to_string(), 11.0, theme.fg_muted, Alignment::Center),
        ]),
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(kids)
}

/// Segmented en la cabecera del centro para alternar el tipo de gráfica.
fn chart_switcher(model: &Model, theme: &Theme) -> View<Msg> {
    let labels: Vec<&str> = ChartView::all().iter().map(|c| c.title()).collect();
    let sel = ChartView::all()
        .iter()
        .position(|c| *c == model.chart_view)
        .unwrap_or(0);
    let seg = segmented_view(
        &labels,
        sel,
        |i| Msg::SetChartView(ChartView::all().get(i).copied().unwrap_or_default()),
        &SegmentedPalette::from_theme(theme),
    );
    let seg_box = View::new(Style {
        size: Size {
            width: length(520.0_f32),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![seg]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(TAB_BAR_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![seg_box])
}

/// El panel central: cabecera con el switch de tipo de gráfica + la
/// gráfica elegida. El centro es **sólo el gráfico**; las tablas viven en
/// el panel de herramientas (derecha).
pub(crate) fn center_view(model: &Model, theme: &Theme) -> View<Msg> {
    let switcher = chart_switcher(model, theme);

    // Mosaico (cartas lado a lado) sólo si hay >1 abierta; si no, la activa.
    let inner = if model.tile_mode && model.open.len() > 1 {
        let tiles: Vec<View<Msg>> = model
            .open
            .iter()
            .enumerate()
            .map(|(i, tab)| tile_cell(model, i, tab, theme))
            .collect();
        View::new(Style {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            gap: Size {
                width: length(10.0_f32),
                height: length(10.0_f32),
            },
            ..Default::default()
        })
        .children(tiles)
    } else {
        // Vista única: el gráfico ocupa toda el área (fondo a sangre).
        graphic_for(model, &model.chart, &model.render, WHEEL_SIZE, theme, true)
    };

    // Los rails de los sidebars flotan como overlay sobre el área gráfica
    // (pegados a los bordes internos), así la rueda usa todo el espacio y
    // los dientes sobresalen sobre ella.
    let mut area_kids = vec![inner];
    if let Some(l) = dock_rail_overlay(DockSide::Left, model, theme) {
        area_kids.push(l);
    }
    if let Some(r) = dock_rail_overlay(DockSide::Right, model, theme) {
        area_kids.push(r);
    }
    let graphic_area = View::new(Style {
        flex_grow: 1.0,
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(0.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(area_kids);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: percent(1.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![chart_tabs(model, theme), switcher, graphic_area])
}
