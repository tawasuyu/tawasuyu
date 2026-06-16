//! Render del rail de sesiones, panel de sesión, canvas principal y formularios
//! de creación / configuración.

use super::chrome::SESSION_RAIL_W;
use super::super::*;
use super::widgets::*;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Dimension, Style};
use llimphi_ui::llimphi_layout::taffy::{AlignItems, FlexDirection, JustifyContent, Rect, Size};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{DragPhase, View};
use llimphi_theme::Theme;
use llimphi_widget_select::{
    select_trigger_view, SelectItem, SelectPalette,
};

// ─── Rail de sesiones (izquierda) ──────────────────────────────────

/// El rail IZQUIERDO de sesiones: draft primero, luego las creadas.
/// Cada diente es clickeable y arrastrable para reordenar.
pub(super) fn session_rail(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    let mut teeth: Vec<View<Msg>> = model
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let activa = i == model.active_session;
            let fill = if activa { theme.bg_selected } else { theme.bg_panel_alt };
            let icon_color = if activa { theme.accent } else { theme.fg_muted };
            let badge = s.number.map(|n| n.to_string()).unwrap_or_default();
            // A6 — badge de comando largo: sólo en sesiones NO activas (en la
            // activa el usuario ya está mirando). Avisa que algo largo terminó
            // mientras estabas en otro diente.
            let alerta_larga = !activa && s.long_alerts() > 0;
            let icon = session_tooth_icon(s.kind, s.active_data(), alerta_larga, 22.0, icon_color);
            let num = View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(12.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text_aligned(badge, 9.0, theme.fg_muted, Alignment::Center);

            let mut tooth = View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size { width: percent(1.0_f32), height: length(48.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
                ..Default::default()
            })
            .fill(fill)
            .hover_fill(theme.bg_row_hover)
            .on_pointer_enter(Msg::HoverSession(Some(i)))
            .on_pointer_leave(Msg::HoverSession(None))
            .children(vec![icon, num]);

            if i > 0 {
                tooth = tooth
                    .on_click_at(move |_, _, _, _| Some(Msg::SelectSession(i)))
                    .draggable_at(|phase, _, _, _, _| match phase {
                        DragPhase::Move | DragPhase::End => None,
                    })
                    .drag_payload(i as u64)
                    .on_drop(move |payload| Some(Msg::ReorderSession(payload as usize, i)))
                    .drop_hover_fill(theme.bg_row_hover);
            } else {
                tooth = tooth.on_click(Msg::SelectSession(i));
            }
            tooth
        })
        .collect();

    // Botón `+` al final del rail.
    let plus_icon = View::new(Style {
        size: Size { width: length(20.0_f32), height: length(20.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .paint_with(|scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Line, Point, Stroke};
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let r = (rect.w.min(rect.h) as f64 * 0.32).max(4.0);
        let stroke = Stroke::new(1.8);
        let color = llimphi_ui::llimphi_raster::peniko::Color::from_rgb8(0x90, 0x98, 0xa6);
        scene.stroke(
            &stroke,
            Affine::IDENTITY,
            color,
            None,
            &Line::new(Point::new(cx - r, cy), Point::new(cx + r, cy)),
        );
        scene.stroke(
            &stroke,
            Affine::IDENTITY,
            color,
            None,
            &Line::new(Point::new(cx, cy - r), Point::new(cx, cy + r)),
        );
    });
    let plus = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::OpenNewSessionForm)
    .children(vec![plus_icon]);
    teeth.push(plus);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(SESSION_RAIL_W), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(teeth)
}

/// Icono vectorial del diente de una sesión según su tipo. `alert` (A6) pinta
/// una badge ámbar cuando un comando largo terminó en una sesión no-activa.
fn session_tooth_icon(
    kind: SessionKind,
    active_data: bool,
    alert: bool,
    size: f32,
    color: Color,
) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(size), height: length(size) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{
            Affine, BezPath, Circle, Line, Point, RoundedRect, Stroke,
        };
        use llimphi_ui::llimphi_raster::peniko::{Color as PColor, Fill};
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let r = (rect.w.min(rect.h) as f64 * 0.34).max(2.0);
        let stroke = Stroke::new((r * 0.22).max(1.2));
        match kind {
            SessionKind::Draft | SessionKind::Local => {
                let sq = RoundedRect::new(cx - r, cy - r, cx + r, cy + r, 2.5);
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &sq);
            }
            SessionKind::Remote => {
                scene.stroke(
                    &stroke,
                    Affine::IDENTITY,
                    color,
                    None,
                    &Circle::new((cx, cy), r),
                );
                scene.stroke(
                    &stroke,
                    Affine::IDENTITY,
                    color,
                    None,
                    &Line::new(Point::new(cx - r, cy), Point::new(cx + r, cy)),
                );
                let mut m = BezPath::new();
                m.move_to(Point::new(cx, cy - r));
                m.quad_to(Point::new(cx - r, cy), Point::new(cx, cy + r));
                m.quad_to(Point::new(cx + r, cy), Point::new(cx, cy - r));
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &m);
            }
        }
        // LED de actividad.
        let led = if active_data {
            PColor::from_rgb8(0x4a, 0xde, 0x80)
        } else {
            PColor::from_rgb8(0x55, 0x5a, 0x66)
        };
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            led,
            None,
            &Circle::new((cx + r * 1.05, cy - r * 1.05), (r * 0.32).max(1.5)),
        );
        // A6 — badge de comando largo: punto ámbar en la esquina opuesta al LED
        // (abajo-izquierda), con un halo tenue para que cante un poco más.
        if alert {
            let ambar = PColor::from_rgb8(0xf7, 0xc8, 0x7a);
            let bx = cx - r * 1.05;
            let by = cy + r * 1.05;
            let rad = (r * 0.36).max(1.8);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                ambar.with_alpha(0.30),
                None,
                &Circle::new((bx, by), rad * 1.9),
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, ambar, None, &Circle::new((bx, by), rad));
        }
    })
}

// ─── Panel de sesión (sidebar izquierdo) ───────────────────────────

/// El panel de la sesión activa: toda su configuración.
pub(super) fn session_panel(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    let Some(session) = model.active() else {
        return View::new(Style::default());
    };
    let idx = model.active_session;
    let es_draft = session.kind == SessionKind::Draft;

    if session.pending {
        let children: Vec<View<Msg>> = vec![
            panel_title("Sesión nueva", theme),
            panel_note(
                "Configurá los datos en el canvas. Enter confirma, Esc cancela.",
                theme,
            ),
        ];
        return panel_frame(children, theme);
    }

    let titulo = if es_draft { "local · scratch".to_string() } else { session.name.clone() };
    let mut children: Vec<View<Msg>> = vec![panel_title(&titulo, theme)];
    children.push(conn_pill(session.conn, theme));

    if !es_draft {
        let label = match session.conn {
            ConnState::Connected => "Reconectar",
            _ => "Conectar",
        };
        children.push(action_button_small(label, Msg::ReconnectSession(idx), theme));
    }

    children.extend(host_select(model, session, theme));
    children.push(container_toggle(session.use_container, theme));
    if session.use_container {
        children.extend(container_picker(model, session, theme));
    }

    // Persistencia: el flag guarda el output a disco (cada 5 s + al toggle)
    // y lo restaura al reabrir la app. La draft es scratch — no aplica.
    if !es_draft {
        children.push(toggle_row(
            "Persistir sesión (output al reabrir)",
            session.persist,
            Msg::ToggleSessionPersist(idx),
            theme,
        ));
    }

    // Environment: los grupos de env.json, activables en bloque.
    children.extend(env_section(model, theme));

    if !es_draft {
        children.push(panel_label("cwd", theme));
        children.push(panel_note(&session_cwd(session), theme));
        children.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
                margin: Rect {
                    left: length(0.0_f32),
                    right: length(0.0_f32),
                    top: length(10.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .fill(theme.bg_button)
            .hover_fill(theme.bg_button_hover)
            .radius(5.0)
            .text_aligned(
                "Cerrar sesión".to_string(),
                12.0,
                theme.fg_text,
                Alignment::Center,
            )
            .on_click(Msg::CloseSession(idx)),
        );
    }

    panel_frame(children, theme)
}

fn session_cwd(session: &Session) -> String {
    match &session.shell.state {
        ModuleState::Shell(sh) => sh.cwd.display().to_string(),
        _ => "-".to_string(),
    }
}

/// Píldora de estado de conexión: punto de color + texto.
pub(super) fn conn_pill(conn: ConnState, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let color = match conn {
        ConnState::Connected => {
            llimphi_ui::llimphi_raster::peniko::Color::from_rgb8(0x4a, 0xde, 0x80)
        }
        ConnState::Pending => {
            llimphi_ui::llimphi_raster::peniko::Color::from_rgb8(0xf7, 0xc8, 0x7a)
        }
        ConnState::Disconnected => {
            llimphi_ui::llimphi_raster::peniko::Color::from_rgb8(0xe0, 0x6c, 0x6c)
        }
    };
    let dot = View::new(Style {
        size: Size { width: length(12.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle};
        use llimphi_ui::llimphi_raster::peniko::Fill;
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            color,
            None,
            &Circle::new((cx, cy), 4.0),
        );
    });
    let txt = View::new(Style {
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        ..Default::default()
    })
    .text_aligned(
        conn.label().to_string(),
        11.0,
        theme.fg_muted,
        Alignment::Start,
    );
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![dot, txt])
}

// ─── Selectores inline ─────────────────────────────────────────────

/// Selector de host remoto: Local + hosts guardados + botón al gestor.
pub(super) fn host_select(model: &Model, session: &Session, theme: &Theme) -> Vec<View<Msg>> {
    let pal = SelectPalette::from_theme(theme);
    let mut out: Vec<View<Msg>> = vec![panel_label("Host", theme)];
    let cur_label = match &session.host_label {
        None => "Local (esta máquina)".to_string(),
        Some(name) => model
            .hosts
            .iter()
            .find(|h| &h.name == name)
            .map(|h| h.display())
            .unwrap_or_else(|| name.clone()),
    };
    let cur_item = SelectItem::new(cur_label);
    out.push(select_trigger_view(
        Some(&cur_item),
        "Elegí el host…",
        model.dropdown_open == Some(DropKind::Host),
        None,
        &pal,
        Msg::ToggleDropdown(DropKind::Host),
    ));
    if model.dropdown_open == Some(DropKind::Host) {
        let mut rows: Vec<View<Msg>> =
            vec![pick_row("Local (esta máquina)".to_string(), Msg::PickHost(None), theme)];
        for (i, h) in model.hosts.iter().enumerate() {
            rows.push(pick_row(h.display(), Msg::PickHost(Some(i)), theme));
        }
        out.push(inline_list(rows));
    }
    out.push(action_button_small("Gestionar hosts…", Msg::OpenHostsWindow, theme));
    out
}

/// Selector de contenedor: rootfs o podman, inline, + botón al gestor.
pub(super) fn container_picker(model: &Model, session: &Session, theme: &Theme) -> Vec<View<Msg>> {
    let pal = SelectPalette::from_theme(theme);
    let mut out: Vec<View<Msg>> = vec![panel_label("Contenedor", theme)];
    let cont_sel = session.container.as_ref().map(|c| {
        let short = c.rsplit('/').find(|s| !s.is_empty()).unwrap_or(c.as_str());
        SelectItem::new(short.to_string())
    });
    out.push(select_trigger_view(
        cont_sel.as_ref(),
        "Elegí un contenedor…",
        model.dropdown_open == Some(DropKind::Container),
        None,
        &pal,
        Msg::ToggleDropdown(DropKind::Container),
    ));
    let es_local = session.host_key() == "local";
    if model.dropdown_open == Some(DropKind::Container) {
        if !es_local {
            if model.remote_containers.is_empty() {
                out.push(panel_note(
                    "Sin contenedores en el host remoto (o no respondió aún).",
                    theme,
                ));
            } else {
                let mut rows: Vec<View<Msg>> = Vec::new();
                for c in &model.remote_containers {
                    rows.push(pick_row(c.clone(), Msg::PickRemoteContainer(c.clone()), theme));
                }
                out.push(inline_list(rows));
            }
        } else {
            let mut rows: Vec<View<Msg>> = Vec::new();
            for distro in &[Distro::Ubuntu, Distro::Debian, Distro::Alpine, Distro::Arch] {
                if rootfs_listo(*distro) {
                    let d = *distro;
                    rows.push(pick_row(
                        format!("rootfs · {}", d.label()),
                        Msg::PickRootfs(d),
                        theme,
                    ));
                }
            }
            for (i, c) in model.containers.iter().enumerate() {
                rows.push(pick_row(c.clone(), Msg::SubscribeContainer(i), theme));
            }
            if rows.is_empty() {
                out.push(panel_note(
                    "Sin contenedores — usá «Gestionar contenedores».",
                    theme,
                ));
            } else {
                out.push(inline_list(rows));
            }
        }
    }
    out.push(action_button_small(
        "Gestionar contenedores…",
        Msg::OpenContainersWindow,
        theme,
    ));
    out
}

/// Checkbox "Aislar en contenedor".
pub(super) fn container_toggle(on: bool, theme: &Theme) -> View<Msg> {
    toggle_row(
        "Aislar con rootfs propio (sin instalar nada)",
        on,
        Msg::ToggleUseContainer,
        theme,
    )
}

/// Fila checkbox genérica del panel: cajita + label, click alterna `msg`.
pub(super) fn toggle_row(label: &str, on: bool, msg: Msg, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let accent = theme.accent;
    let fg = theme.fg_text;
    let bg = theme.bg_panel_alt;
    let box_view = View::new(Style {
        size: Size { width: length(18.0_f32), height: length(18.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Point, RoundedRect, Stroke};
        use llimphi_ui::llimphi_raster::peniko::Fill;
        let rr = RoundedRect::new(
            rect.x as f64 + 1.0,
            rect.y as f64 + 1.0,
            rect.x as f64 + rect.w as f64 - 1.0,
            rect.y as f64 + rect.h as f64 - 1.0,
            3.0,
        );
        if on {
            scene.fill(Fill::NonZero, Affine::IDENTITY, accent, None, &rr);
            let mut p = BezPath::new();
            let cx = (rect.x + rect.w * 0.5) as f64;
            let cy = (rect.y + rect.h * 0.5) as f64;
            let r = (rect.w.min(rect.h) as f64) * 0.28;
            p.move_to(Point::new(cx - r, cy));
            p.line_to(Point::new(cx - r * 0.2, cy + r * 0.7));
            p.line_to(Point::new(cx + r, cy - r * 0.6));
            scene.stroke(
                &Stroke::new(2.0),
                Affine::IDENTITY,
                llimphi_ui::llimphi_raster::peniko::Color::from_rgb8(0xff, 0xff, 0xff),
                None,
                &p,
            );
        } else {
            scene.fill(Fill::NonZero, Affine::IDENTITY, bg, None, &rr);
            scene.stroke(
                &Stroke::new(1.2),
                Affine::IDENTITY,
                llimphi_ui::llimphi_raster::peniko::Color::from_rgb8(0x55, 0x5a, 0x66),
                None,
                &rr,
            );
        }
    });
    let label = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(20.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(label.to_string(), 13.0, fg, Alignment::Start);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(10.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .on_click(msg)
    .hover_fill(theme.bg_row_hover)
    .children(vec![box_view, label])
}

// ─── Environment (grupos activables) ────────────────────────────────

/// Cuántas variables se listan por grupo antes de resumir con "+N más".
const ENV_VARS_VISIBLES: usize = 6;

/// Sección «Environment» del panel: cada grupo de `env.json` con su link
/// on/off (click = activar/desactivar el grupo entero) y sus variables
/// listadas debajo. `:env NAME=valor [@grupo]` agrega desde el teclado.
pub(super) fn env_section(model: &Model, theme: &Theme) -> Vec<View<Msg>> {
    use llimphi_ui::llimphi_text::Alignment;
    let mut out: Vec<View<Msg>> = vec![panel_label("Environment", theme)];
    for (i, g) in model.env_groups.iter().enumerate() {
        // Fila del grupo: [on|off] nombre · N — click alterna el grupo.
        let (pill_fill, pill_fg, pill_txt) = if g.active {
            (theme.accent, theme.bg_panel, "on")
        } else {
            (theme.bg_panel_alt, theme.fg_muted, "off")
        };
        let pill = View::new(Style {
            size: Size { width: length(30.0_f32), height: length(16.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(pill_fill)
        .radius(8.0)
        .text_aligned(pill_txt.to_string(), 10.0, pill_fg, Alignment::Center);
        let name_color = if g.active { theme.fg_text } else { theme.fg_muted };
        let nombre = View::new(Style {
            size: Size { width: Dimension::auto(), height: length(16.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(g.name.clone(), 12.0, name_color, Alignment::Start);
        let count = View::new(Style {
            size: Size { width: length(24.0_f32), height: length(16.0_f32) },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .text_aligned(g.vars.len().to_string(), 10.0, theme.fg_muted, Alignment::End);
        out.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
                align_items: Some(AlignItems::Center),
                gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
                margin: Rect {
                    left: length(0.0_f32),
                    right: length(0.0_f32),
                    top: length(4.0_f32),
                    bottom: length(0.0_f32),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .hover_fill(theme.bg_row_hover)
            .radius(4.0)
            .on_click(Msg::ToggleEnvGroup(i))
            .children(vec![pill, nombre, count]),
        );
        // Variables del grupo, indentadas y discretas.
        for (k, v) in g.vars.iter().take(ENV_VARS_VISIBLES) {
            let color = if g.active { theme.fg_muted } else { theme.fg_placeholder };
            out.push(
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(15.0_f32) },
                    padding: Rect {
                        left: length(38.0_f32),
                        right: length(0.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .text_aligned(format!("{k}={v}"), 10.0, color, Alignment::Start)
                .mono()
                .max_lines(1),
            );
        }
        if g.vars.len() > ENV_VARS_VISIBLES {
            out.push(panel_note(
                &format!("    +{} más", g.vars.len() - ENV_VARS_VISIBLES),
                theme,
            ));
        }
        if g.vars.is_empty() {
            out.push(
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(15.0_f32) },
                    padding: Rect {
                        left: length(38.0_f32),
                        right: length(0.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .text_aligned("(vacío)".to_string(), 10.0, theme.fg_placeholder, Alignment::Start),
            );
        }
    }
    out.push(panel_note(":env NAME=valor @grupo agrega una variable", theme));
    out
}

// ─── Canvas principal ───────────────────────────────────────────────

/// El canvas principal: sólo el shell de la sesión activa.
pub(super) fn canvas_view(model: &Model, theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: length(0.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![tab_content(model, theme)])
}

pub(crate) fn tab_content(model: &Model, theme: &Theme) -> View<Msg> {
    let Some(session) = model.active() else {
        return placeholder(theme, &rimay_localize::t("shuma-empty-no-tabs"));
    };
    if session.pending {
        return new_session_form(model, session, theme);
    }
    let idx = model.active_session;
    let lift = move |m| Msg::Module(Slot::Session(idx, Which::Shell), ModuleMsg::Shell(m));
    match &session.shell.state {
        // Hospedado en barra (pata): el input vive en la barra del host, así que
        // el canvas pinta sólo el cuerpo (sin input) para no duplicarlo.
        ModuleState::Shell(state) if model.hosted_bar => {
            shuma_module_shell::body_view::<Msg>(state, theme, lift)
        }
        ModuleState::Shell(state) => shuma_module_shell::view::<Msg>(state, theme, lift),
        _ => placeholder(theme, ""),
    }
}

// ─── Form de nueva sesión ───────────────────────────────────────────

/// Form grande de creación de sesión, ocupa el canvas mientras `session.pending`.
fn new_session_form(model: &Model, session: &Session, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    let titulo = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        format!("Nueva sesión · {}", session.name),
        18.0,
        theme.fg_text,
        Alignment::Start,
    );
    let sub = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        "Elegí dónde corre el shell. Enter confirma · Esc cancela.".to_string(),
        12.0,
        theme.fg_muted,
        Alignment::Start,
    );

    let mut children: Vec<View<Msg>> = vec![titulo, sub];
    children.extend(host_select(model, session, theme));
    children.push(container_toggle(session.use_container, theme));
    if session.use_container {
        children.extend(container_picker(model, session, theme));
    }

    let cancelar = View::new(Style {
        size: Size { width: length(120.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_button)
    .hover_fill(theme.bg_button_hover)
    .radius(5.0)
    .text_aligned("Cancelar".to_string(), 12.0, theme.fg_text, Alignment::Center)
    .on_click(Msg::CancelNewSession);
    let crear = View::new(Style {
        size: Size { width: length(120.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.accent)
    .hover_fill(theme.accent)
    .radius(5.0)
    .text_aligned("Crear".to_string(), 12.0, theme.bg_app, Alignment::Center)
    .on_click(Msg::ConfirmNewSession);
    let botones = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        gap: Size { width: length(10.0_f32), height: length(0.0_f32) },
        align_items: Some(AlignItems::Center),
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(16.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![cancelar, crear]);
    children.push(botones);

    let form = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: Dimension::auto(), height: Dimension::auto() },
        padding: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(24.0_f32),
            bottom: length(24.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(8.0)
    .children(children);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(24.0_f32),
            bottom: length(24.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![View::new(Style {
        size: Size { width: length(520.0_f32), height: Dimension::auto() },
        ..Default::default()
    })
    .children(vec![form])])
}
