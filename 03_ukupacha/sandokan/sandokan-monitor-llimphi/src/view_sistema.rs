//! Vista del modo Sistema: tabla de procesos del SO leída de `/proc`.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Rect, Size, Style},
    AlignItems, FlexWrap, JustifyContent,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{draw_layout, measurement, Alignment};
use llimphi_ui::View;
use llimphi_theme::Theme;

use crate::modelo::{Model, Msg, SysProc};
use crate::procfs::Sig;
use crate::sistema::render_list;
use crate::widgets::{
    action_btn, empty_state, fmt_dur, fmt_mem, meter, name_color, pad, seg_btn, spacer,
    state_color, usage_color,
};

// ---------------------------------------------------------------------------
// Anchos de columna (px); la última (comando) crece.
// ---------------------------------------------------------------------------

pub(crate) const W_PID: f32 = 62.0;
pub(crate) const W_CPU: f32 = 58.0;
pub(crate) const W_MEM: f32 = 58.0;
pub(crate) const W_RSS: f32 = 78.0;
pub(crate) const W_ST: f32 = 28.0;
pub(crate) const W_THR: f32 = 46.0;
pub(crate) const W_UID: f32 = 54.0;
pub(crate) const W_TIME: f32 = 66.0;
pub(crate) const ROW_H: f32 = 21.0;

// ---------------------------------------------------------------------------
// Cuerpo principal del modo Sistema.
// ---------------------------------------------------------------------------

pub(crate) fn system_body(model: &Model) -> View<Msg> {
    let t = &model.theme;
    if model.system.is_empty() {
        return empty_state(t, "Leyendo /proc…", "Barriendo los procesos del sistema.");
    }

    let rows = render_list(model);
    let total = rows.len();
    let start = model.sys_scroll.min(total.saturating_sub(1));
    let end = (start + crate::modelo::SYS_ROWS).min(total);

    let mut table: Vec<View<Msg>> = Vec::with_capacity(end - start + 2);
    table.push(sys_header_row(model));
    for r in &rows[start..end] {
        let p = &model.system[r.idx];
        let node = model.sys_tree.then_some((r.depth, r.has_kids, model.collapsed.contains(&p.pid)));
        table.push(sys_row(t, p, model.sys_sel == Some(p.pid), node));
    }
    if end < total {
        table.push(
            View::new(Style {
                padding: pad(10.0, 4.0),
                ..Default::default()
            })
            .text(
                &format!("… {} filas más abajo (rueda / ↑↓)", total - end),
                10.5,
                t.fg_muted,
            ),
        );
    }

    let sel = model
        .sys_sel
        .and_then(|pid| model.system.iter().find(|p| p.pid == pid));

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
    .children(vec![
        sys_graphs(model),
        sys_action_bar(model, sel),
        sys_filter_bar(model, total),
        View::new(Style {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0),
                height: auto(),
            },
            padding: Rect {
                left: length(12.0),
                right: length(12.0),
                top: length(0.0),
                bottom: length(8.0),
            },
            ..Default::default()
        })
        .clip(true)
        .children(table),
    ])
}

// ---------------------------------------------------------------------------
// Gráficos de CPU/Memoria en la cabecera del panel Sistema.
// ---------------------------------------------------------------------------

/// Fila de gráficos del tope: un gráfico de %uso por **core** + uno de memoria,
/// en FlexWrap (en ventanas angostas los cores bajan de fila).
pub(crate) fn sys_graphs(model: &Model) -> View<Msg> {
    let t = &model.theme;

    let mut items: Vec<View<Msg>> = Vec::with_capacity(model.core_hist.len() + 1);
    for (i, hist) in model.core_hist.iter().enumerate() {
        let id = model.core_ids.get(i).copied().unwrap_or(i as u32);
        let now = hist.back().copied().unwrap_or(0.0);
        // El valor de la cabecera toma el color del nivel actual; la línea se
        // colorea por tramo según el uso (verde→ámbar→rojo).
        items.push(meter(t, &format!("CPU{id}"), &format!("{now:.0}%"), hist, usage_color(now), 126.0, true));
    }
    let mem_now = model.mem_hist.back().copied().unwrap_or(0.0);
    let used_kb = model.mem_total_kb.saturating_sub(model.mem_avail_kb);
    items.push(meter(
        t,
        "Memoria",
        &format!("{} / {} · {mem_now:.0}%", fmt_mem(used_kb * 1024), fmt_mem(model.mem_total_kb * 1024)),
        &model.mem_hist,
        t.accent,
        236.0,
        false,
    ));

    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        gap: Size {
            width: length(10.0),
            height: length(8.0),
        },
        padding: pad(16.0, 10.0),
        ..Default::default()
    })
    .fill(t.bg_panel)
    .children(items)
}

// ---------------------------------------------------------------------------
// Barra de acciones del modo Sistema.
// ---------------------------------------------------------------------------

/// Barra de acciones: toggle Lista/Árbol + acciones sobre el seleccionado.
pub(crate) fn sys_action_bar(model: &Model, sel: Option<&SysProc>) -> View<Msg> {
    let t = &model.theme;
    let mut row: Vec<View<Msg>> = vec![
        seg_btn(t, "Árbol", model.sys_tree, Msg::SysTree(true)),
        seg_btn(t, "Lista", !model.sys_tree, Msg::SysTree(false)),
    ];
    match sel {
        Some(p) => {
            row.push(
                View::new(Style {
                    flex_grow: 1.0,
                    ..Default::default()
                })
                .text(&format!("PID {} · {}", p.pid, p.name), 12.5, t.fg_text),
            );
            row.push(action_btn(t, "Terminar", t.bg_button, t.fg_text, Msg::Signal(p.pid, Sig::Term)));
            row.push(action_btn(t, "Matar", t.fg_destructive, t.bg_app, Msg::Signal(p.pid, Sig::Kill)));
            row.push(action_btn(t, "Pausar", t.bg_button, t.fg_text, Msg::Signal(p.pid, Sig::Stop)));
            row.push(action_btn(t, "Seguir", t.bg_button, t.fg_text, Msg::Signal(p.pid, Sig::Cont)));
        }
        None => row.push(
            View::new(Style {
                flex_grow: 1.0,
                ..Default::default()
            })
            .text(
                "Elegí un proceso (click / ↑↓) para terminar, matar, pausar o seguir.",
                12.0,
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

// ---------------------------------------------------------------------------
// Barra de filtro.
// ---------------------------------------------------------------------------

/// Barra de filtro (búsqueda por nombre/comando/PID). Click la enfoca; `/`
/// también. Muestra el texto en vivo con caret, el conteo de coincidencias y
/// una ✕ para limpiar.
pub(crate) fn sys_filter_bar(model: &Model, matches: usize) -> View<Msg> {
    let t = &model.theme;
    let has = !model.sys_filter.is_empty();
    let active = model.filter_mode;

    let (shown, color) = if !has && !active {
        (
            "Filtrar por nombre o PID  ·  «/» o Ctrl+F".to_string(),
            t.fg_placeholder,
        )
    } else {
        let caret = if active { "▏" } else { "" };
        (format!("{}{caret}", model.sys_filter), t.fg_text)
    };

    let mut row: Vec<View<Msg>> = vec![
        View::new(Style {
            size: Size {
                width: length(24.0),
                height: percent(1.0),
            },
            flex_shrink: 0.0,
            justify_content: Some(JustifyContent::Center),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .text("/", 13.0, t.fg_muted),
        View::new(Style {
            flex_grow: 1.0,
            justify_content: Some(JustifyContent::Center),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .clip(true)
        .text(shown, 12.0, color)
        .on_click(Msg::FilterMode(true)),
    ];
    if has {
        row.push(
            View::new(Style::default())
                .text(format!("{matches} coinciden"), 11.0, t.fg_muted),
        );
        row.push(action_btn(t, "✕", t.bg_button, t.fg_text, Msg::FilterClose));
    }

    let bg = if active { t.bg_input_focus } else { t.bg_input };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0),
            height: length(6.0),
        },
        padding: pad(16.0, 6.0),
        ..Default::default()
    })
    .fill(bg)
    .children(row)
}

// ---------------------------------------------------------------------------
// Fila de cabecera de la tabla.
// ---------------------------------------------------------------------------

pub(crate) fn sys_header_row(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let hcell = |label: &str, w: f32, sort: Option<crate::modelo::Sort>| {
        let active = sort.map(|s| s == model.sys_sort).unwrap_or(false);
        let fg = if active { t.accent } else { t.fg_muted };
        let mut v = View::new(Style {
            size: Size {
                width: length(w),
                height: percent(1.0),
            },
            flex_shrink: 0.0,
            justify_content: Some(JustifyContent::Center),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .text(label, 10.5, fg);
        if let Some(s) = sort {
            v = v.on_click(Msg::SysSort(s));
        }
        v
    };
    let cmd = {
        let active = model.sys_sort == crate::modelo::Sort::Name;
        let fg = if active { t.accent } else { t.fg_muted };
        View::new(Style {
            flex_grow: 1.0,
            justify_content: Some(JustifyContent::Center),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .text("COMANDO (nombre↕)", 10.5, fg)
        .on_click(Msg::SysSort(crate::modelo::Sort::Name))
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        size: Size {
            width: percent(1.0),
            height: length(ROW_H + 4.0),
        },
        gap: Size {
            width: length(6.0),
            height: length(0.0),
        },
        padding: Rect {
            left: length(8.0),
            right: length(8.0),
            top: length(0.0),
            bottom: length(0.0),
        },
        ..Default::default()
    })
    .children(vec![
        hcell("PID", W_PID, Some(crate::modelo::Sort::Pid)),
        hcell("%CPU", W_CPU, Some(crate::modelo::Sort::Cpu)),
        hcell("%MEM", W_MEM, Some(crate::modelo::Sort::Mem)),
        hcell("RSS", W_RSS, Some(crate::modelo::Sort::Mem)),
        hcell("S", W_ST, None),
        hcell("HILOS", W_THR, None),
        hcell("UID", W_UID, None),
        hcell("TIEMPO", W_TIME, Some(crate::modelo::Sort::Uptime)),
        cmd,
    ])
}

// ---------------------------------------------------------------------------
// Fila de proceso.
// ---------------------------------------------------------------------------

/// `node = Some((depth, has_kids, collapsed))` en modo árbol; `None` en lista.
pub(crate) fn sys_row(t: &Theme, p: &SysProc, selected: bool, node: Option<(u16, bool, bool)>) -> View<Msg> {
    let cell = |s: String, w: f32, color: Color| {
        View::new(Style {
            size: Size {
                width: length(w),
                height: percent(1.0),
            },
            flex_shrink: 0.0,
            justify_content: Some(JustifyContent::Center),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .text(s, 11.5, color)
    };
    let bg = if selected { t.bg_selected } else { t.bg_app };
    // %CPU coloreado por nivel cuando hay actividad; el comando toma el color
    // categórico del proceso (coherente con el treemap).
    let cpu_col = if p.cpu_pct >= 0.5 {
        usage_color(p.cpu_pct)
    } else {
        t.fg_muted
    };
    let cmd_col = name_color(&p.name);

    // Celda de comando: en árbol lleva sangría por profundidad + triángulo de
    // colapso (dibujado, no glifo de fuente) antes del texto.
    let cmd_cell = {
        let mut parts: Vec<View<Msg>> = Vec::new();
        if let Some((depth, has_kids, collapsed)) = node {
            let indent = depth as f32 * 14.0;
            if indent > 0.0 {
                parts.push(spacer(indent));
            }
            parts.push(tri_node(t, has_kids, collapsed, p.pid));
        }
        parts.push(command_node(&p.cmd, cmd_col));
        View::new(Style {
            flex_grow: 1.0,
            flex_basis: length(0.0),
            min_size: Size {
                width: length(0.0),
                height: auto(),
            },
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .clip(true)
        .children(parts)
    };

    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        size: Size {
            width: percent(1.0),
            height: length(ROW_H),
        },
        gap: Size {
            width: length(6.0),
            height: length(0.0),
        },
        padding: Rect {
            left: length(8.0),
            right: length(8.0),
            top: length(0.0),
            bottom: length(0.0),
        },
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(t.bg_row_hover)
    .on_click(Msg::SysSelect(p.pid))
    .children(vec![
        cell(p.pid.to_string(), W_PID, t.fg_muted),
        cell(format!("{:.1}", p.cpu_pct), W_CPU, cpu_col),
        cell(format!("{:.1}", p.mem_pct), W_MEM, t.fg_muted),
        cell(fmt_mem(p.rss_kb * 1024), W_RSS, t.fg_muted),
        cell(p.state.to_string(), W_ST, state_color(t, p.state)),
        cell(p.threads.to_string(), W_THR, t.fg_muted),
        cell(p.uid.to_string(), W_UID, t.fg_muted),
        cell(fmt_dur(p.uptime_secs), W_TIME, t.fg_muted),
        cmd_cell,
    ])
}

// ---------------------------------------------------------------------------
// Triángulo de colapso del árbol.
// ---------------------------------------------------------------------------

/// Triángulo de colapso del árbol, **dibujado** (no glifo de fuente, que salía
/// tofu): ▶ colapsado / ▼ expandido. Las hojas quedan en blanco. Clickeable.
pub(crate) fn tri_node(t: &Theme, has_kids: bool, collapsed: bool, pid: i32) -> View<Msg> {
    let col = t.fg_muted;
    let mut v = View::new(Style {
        size: Size {
            width: length(15.0),
            height: length(ROW_H),
        },
        flex_shrink: 0.0,
        ..Default::default()
    });
    if has_kids {
        v = v.paint_with(move |scene, _ts, rect| {
            let cx = rect.x + rect.w / 2.0;
            let cy = rect.y + rect.h / 2.0;
            let s = 3.6_f32;
            let mut tri = BezPath::new();
            if collapsed {
                // apunta a la derecha ▶
                tri.move_to(((cx - s) as f64, (cy - s) as f64));
                tri.line_to(((cx - s) as f64, (cy + s) as f64));
                tri.line_to(((cx + s) as f64, cy as f64));
            } else {
                // apunta abajo ▼
                tri.move_to(((cx - s) as f64, (cy - s) as f64));
                tri.line_to(((cx + s) as f64, (cy - s) as f64));
                tri.line_to((cx as f64, (cy + s) as f64));
            }
            tri.close_path();
            scene.fill(Fill::NonZero, Affine::IDENTITY, col, None, &tri);
        });
        v = v.on_click(Msg::SysToggleNode(pid));
    }
    v
}

// ---------------------------------------------------------------------------
// Celda de comando responsive.
// ---------------------------------------------------------------------------

/// Celda de comando: **rellena el espacio disponible** (flex), texto a la
/// izquierda, una sola línea, y se pica con `...` si no entra. Pintado con
/// `paint_with` para medir contra el ancho REAL de la columna (responsive) y
/// elipsar pixel-exacto. Esto evita reservar una columna gigante.
pub(crate) fn command_node(cmd: &str, color: Color) -> View<Msg> {
    let cmd = cmd.chars().take(512).collect::<String>();
    View::new(Style {
        flex_grow: 1.0,
        flex_basis: length(0.0),
        min_size: Size {
            width: length(0.0),
            height: auto(),
        },
        size: Size {
            width: auto(),
            height: length(ROW_H),
        },
        ..Default::default()
    })
    .clip(true)
    .paint_with(move |scene, ts, rect| {
        if cmd.is_empty() {
            return;
        }
        let avail = (rect.w - 4.0).max(1.0);
        let layout = ts.layout(&cmd, 11.5, None, Alignment::Start, 1.2, false, None, 400.0, false, false);
        let m = measurement(&layout);
        let x = (rect.x + 2.0) as f64;
        let y = (rect.y + ((rect.h - m.height) / 2.0).max(0.0)) as f64;
        if m.width <= avail {
            draw_layout(scene, &layout, color, (x, y));
        } else {
            // Picar por estimación (ancho promedio de glifo) + "...".
            let n = cmd.chars().count().max(1);
            let avg = m.width / n as f32;
            let fit = ((avail / avg).floor() as usize).saturating_sub(2).min(n);
            let mut s: String = cmd.chars().take(fit).collect();
            s.push_str("...");
            let lay = ts.layout(&s, 11.5, None, Alignment::Start, 1.2, false, None, 400.0, false, false);
            draw_layout(scene, &lay, color, (x, y));
        }
    })
}
