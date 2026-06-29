//! Vista del modo Mapa: treemap jerárquico (fractal) de procesos por CPU o
//! memoria.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill, Gradient};
use llimphi_ui::llimphi_text::{draw_layout, measurement, Alignment};
use llimphi_ui::View;

use super::modelo::{Model, Msg};
use super::procfs::Sig;
use super::sistema::subtree_pids;
use super::widgets::{action_btn, empty_state, fmt_mem, name_color, pad, seg_btn};

// ---------------------------------------------------------------------------
// Cuerpo del modo Mapa.
// ---------------------------------------------------------------------------

pub(crate) fn map_body(model: &Model) -> View<Msg> {
    let t = &model.theme;
    if model.system.is_empty() {
        return empty_state(
            t,
            &rimay_localize::t("sandokan-mon-sys-empty-title"),
            &rimay_localize::t("sandokan-mon-map-empty-body"),
        );
    }
    let cpu = model.map_cpu;
    // Con zoom, restringe al subárbol de la raíz (incluida).
    let subtree = model
        .map_root
        .filter(|r| model.system.iter().any(|p| p.pid == *r))
        .map(|r| subtree_pids(&model.system, r));

    // Datos para el painter (owned → Send + Sync + 'static).
    let items: Vec<super::treemap::Item> = model
        .system
        .iter()
        .filter(|p| subtree.as_ref().map(|s| s.contains(&p.pid)).unwrap_or(true))
        .map(|p| super::treemap::Item {
            pid: p.pid,
            ppid: p.ppid,
            weight: if cpu { p.cpu_pct as f64 } else { p.rss_kb as f64 },
            cpu: p.cpu_pct,
            mem_kb: p.rss_kb,
            label: p.name.clone(),
        })
        .collect();

    let border = t.bg_app;
    let label_col = Color::from_rgba8(0x0d, 0x10, 0x14, 0xff);
    let accent = t.accent;
    let sel = model.sys_sel;
    let hit_items = items.clone();

    let canvas = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_app)
    .clip(true)
    .paint_with(move |scene, ts, rect| {
        let cells = super::treemap::layout(&items, (rect.x, rect.y, rect.w, rect.h), 15.0, 3.0);
        for c in &cells {
            let r = llimphi_ui::llimphi_raster::kurbo::Rect::new(
                c.x as f64,
                c.y as f64,
                (c.x + c.w) as f64,
                (c.y + c.h) as f64,
            );
            // Color categórico por proceso; la opacidad sube con el uso de CPU
            // y baja con la profundidad (sensación fractal). Contenedor: tenue.
            let base = name_color(&c.label);
            let a = if c.leaf {
                (0.60 + c.cpu / 100.0 * 0.34 - c.depth as f32 * 0.05).clamp(0.5, 0.95)
            } else {
                0.14
            };
            // Gradiente vertical leve: arriba un toque más claro, abajo el base
            // — da volumen sin estridencia.
            let top = base.map_lightness(|l| (l + 0.07).min(1.0)).with_alpha(a);
            let bot = base.map_lightness(|l| (l - 0.05).max(0.0)).with_alpha(a);
            let grad = Gradient::new_linear((c.x as f64, c.y as f64), (c.x as f64, (c.y + c.h) as f64))
                .with_stops([top, bot]);
            scene.fill(Fill::NonZero, Affine::IDENTITY, &grad, None, &r);
            if sel == Some(c.pid) {
                scene.stroke(&Stroke::new(2.5), Affine::IDENTITY, accent, None, &r);
            } else {
                scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, border, None, &r);
            }

            // Etiqueta: nombre arriba y, si hay alto, %CPU · RAM debajo.
            if c.w > 46.0 && c.h > 15.0 {
                let name = ts.layout(&c.label, 11.0, None, Alignment::Start, 1.2, false, None, 400.0, false, false, 0.0, 0.0);
                if measurement(&name).width <= c.w - 6.0 {
                    draw_layout(scene, &name, label_col, ((c.x + 3.0) as f64, (c.y + 2.0) as f64));
                }
                if c.h > 30.0 {
                    let stats = format!("{:.0}% · {}", c.cpu, fmt_mem(c.mem_kb * 1024));
                    let sl = ts.layout(&stats, 9.5, None, Alignment::Start, 1.2, false, None, 400.0, false, false, 0.0, 0.0);
                    if measurement(&sl).width <= c.w - 6.0 {
                        let sc = label_col.with_alpha(0.72);
                        draw_layout(scene, &sl, sc, ((c.x + 3.0) as f64, (c.y + 15.0) as f64));
                    }
                }
            }
        }
    })
    .on_click_at(move |x, y, w, h| {
        // Recomputa el layout en coords LOCALES (0,0,w,h) —las mismas que
        // entrega `on_click_at`— y resuelve el rect más profundo (último
        // dibujado) que contiene el punto.
        let cells = super::treemap::layout(&hit_items, (0.0, 0.0, w, h), 15.0, 3.0);
        cells
            .iter()
            .rev()
            .find(|c| x >= c.x && x <= c.x + c.w && y >= c.y && y <= c.y + c.h)
            .map(|c| Msg::MapClick(c.pid))
    });

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_app)
    .children(vec![map_toolbar(model), canvas])
}

// ---------------------------------------------------------------------------
// Barra de herramientas del mapa.
// ---------------------------------------------------------------------------

fn map_toolbar(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let mut row = vec![
        View::new(Style::default()).text(rimay_localize::t("sandokan-mon-map-area-by"), 12.0, t.fg_muted),
        seg_btn(t, &rimay_localize::t("sandokan-mon-memoria"), !model.map_cpu, Msg::MapMetric(false)),
        // "CPU" es un acrónimo técnico — no se traduce.
        seg_btn(t, "CPU", model.map_cpu, Msg::MapMetric(true)),
    ];
    // Breadcrumb de zoom (si estamos dentro de un subárbol).
    if let Some(r) = model.map_root {
        let name = model
            .system
            .iter()
            .find(|p| p.pid == r)
            .map(|p| p.name.as_str())
            .unwrap_or("?");
        row.push(seg_btn(t, &rimay_localize::t("sandokan-mon-map-up"), false, Msg::MapZoomOut));
        row.push(seg_btn(t, &rimay_localize::t("sandokan-mon-map-all"), false, Msg::MapRoot(None)));
        row.push(
            View::new(Style::default())
                .text(
                    rimay_localize::t_args("sandokan-mon-map-zoom", &[("name", name.to_string().into())]),
                    11.5,
                    name_color(name),
                ),
        );
    }
    match model.sys_sel.and_then(|pid| model.system.iter().find(|p| p.pid == pid)) {
        Some(p) => {
            row.push(
                View::new(Style {
                    flex_grow: 1.0,
                    ..Default::default()
                })
                .text(format!("▸ PID {} · {}", p.pid, p.name), 12.0, name_color(&p.name)),
            );
            row.push(action_btn(t, &rimay_localize::t("sandokan-mon-sys-terminate"), t.bg_button, t.fg_text, Msg::Signal(p.pid, Sig::Term)));
            row.push(action_btn(t, &rimay_localize::t("sandokan-mon-sys-kill"), t.fg_destructive, t.bg_app, Msg::Signal(p.pid, Sig::Kill)));
        }
        None => row.push(
            View::new(Style {
                flex_grow: 1.0,
                ..Default::default()
            })
            .text(
                rimay_localize::t("sandokan-mon-map-hint"),
                11.0,
                t.fg_muted,
            ),
        ),
    }
    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0),
            height: length(6.0),
        },
        padding: pad(16.0, 8.0),
        ..Default::default()
    })
    .fill(t.bg_panel_alt)
    .children(row)
}

