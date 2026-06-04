//! Render del chasis: topbar, tabs, área principal, monitores.

use super::*;

// ─── Render de cada slot ────────────────────────────────────────────

pub(crate) fn render_topbar(model: &Model, theme: &Theme) -> View<Msg> {
    match &model.topbar {
        Some(inst) => match (inst.kind, &inst.state) {
            (Kind::Launcher, ModuleState::Launcher(state)) => {
                shuma_module_launcher::view::<Msg>(state, theme, |m| {
                    Msg::Module(Slot::TopBar, ModuleMsg::Launcher(m))
                })
            }
            _ => empty_bar(theme, 40.0),
        },
        None => empty_bar(theme, 40.0),
    }
}

pub(crate) fn render_bottombar(model: &Model, theme: &Theme) -> View<Msg> {
    match &model.bottombar {
        Some(inst) => match (inst.kind, &inst.state) {
            (Kind::CommandBar, ModuleState::CommandBar(state)) => {
                shuma_module_commandbar::view::<Msg>(state, theme, |m| {
                    Msg::Module(Slot::BottomBar, ModuleMsg::CommandBar(m))
                })
            }
            _ => empty_bar(theme, 28.0),
        },
        None => empty_bar(theme, 28.0),
    }
}

pub(crate) fn empty_bar(theme: &Theme, height: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(height),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
}

/// Área central. Si el shumarc declara `[main]`, ese módulo ocupa todo
/// el espacio (sin tabs ni monitores). Si no, se renderizan las tabs +
/// monitor stack a la derecha vía splitter.
pub(crate) fn render_main_area(model: &Model, theme: &Theme) -> View<Msg> {
    let body = match &model.main {
        Some(inst) => render_main_full(inst, theme),
        None => render_tabs_with_monitors(model, theme),
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![body])
}

/// Render full-bleed del slot `main` cuando el shumarc lo configura.
/// Sin tabs ni monitores — útil para wrappers de una sola app.
pub(crate) fn render_main_full(inst: &Instance, theme: &Theme) -> View<Msg> {
    match (inst.kind, &inst.state) {
        (Kind::Shell, ModuleState::Shell(state)) => shuma_module_shell::view::<Msg>(
            state,
            theme,
            |m| Msg::Module(Slot::Main, ModuleMsg::Shell(m)),
        ),
        (Kind::Matilda, ModuleState::Matilda(state)) => {
            shuma_module_matilda::view::<Msg>(state.as_ref(), theme, |m| {
                Msg::Module(Slot::Main, ModuleMsg::Matilda(m))
            })
        }
        (Kind::Minga, ModuleState::Minga(state)) => {
            shuma_module_minga::view::<Msg>(state, theme, |m| {
                Msg::Module(Slot::Main, ModuleMsg::Minga(m))
            })
        }
        (Kind::Canvas, ModuleState::Canvas(state)) => {
            shuma_module_canvas::view::<Msg>(state, theme, |m| {
                Msg::Module(Slot::Main, ModuleMsg::Canvas(m))
            })
        }
        _ => placeholder(theme, &rimay_localize::t("shuma-empty-main-incompat")),
    }
}

/// Layout normal: tira de tabs arriba con toolbar de shortcuts del
/// tab activo, splitter horizontal con (contenido | monitores).
/// Ancho de la franja de dientes a la derecha (px).
const RAIL_W: f32 = 44.0;

pub(crate) fn render_tabs_with_monitors(model: &Model, theme: &Theme) -> View<Msg> {
    let splitter_palette = SplitterPalette::from_theme(theme);

    let toolbar = tabs_toolbar(model, theme);
    let content = tab_content(model, theme);

    // El panel de monitores se togglea con su diente (`monitors_visible`).
    // Oculto → el contenido toma todo el ancho, sin splitter (puro lienzo).
    let tab_body = if model.monitors_visible {
        splitter_two(
            Direction::Row,
            content,
            PaneSize::Flex,
            monitor_stack(model, theme),
            PaneSize::Fixed(model.monitors_width),
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizeMonitors(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        )
    } else {
        content
    };

    // Las tabs ya no son una tira horizontal: ahora son dientes en el rail
    // vertical de la derecha. La columna principal es toolbar + cuerpo.
    let body_wrap = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![tab_body]);

    let main_col = View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: length(0.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![toolbar, body_wrap]);

    // Rail derecho: un diente por VISTA de la sesión activa + el sentinela
    // de medidores. Los dientes nunca reemplazan el espacio: eligen qué vista
    // de la sesión lo llena.
    let rail = view_dock_rail(model, theme);

    // Historial de la sesión activa a la IZQUIERDA (columna fija). El centro
    // es la vista activa; el rail de dientes va a la derecha.
    let history = history_column(model, theme);

    let cuerpo = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![history, main_col, rail]);

    // Tira de sesiones arriba: una tab por sesión (cambia todo el ambiente) +
    // el botón «+» que crea una sesión local nueva.
    let strip = session_strip(model, theme);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![strip, cuerpo])
}

/// La tira superior de **sesiones de trabajo**: una tab por sesión (la activa
/// resaltada) más un `+` que crea una sesión local nueva. Cambiar de sesión
/// switchea todo el ambiente (shell/historial/inventario).
fn session_strip(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{prelude::Dimension, AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;

    let mut row: Vec<View<Msg>> = model
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let activa = i == model.active_session;
            let fill = if activa { theme.bg_selected } else { theme.bg_panel };
            let fg = if activa { theme.fg_text } else { theme.fg_muted };
            View::new(Style {
                size: Size { width: Dimension::auto(), height: length(28.0_f32) },
                padding: Rect {
                    left: length(14.0_f32),
                    right: length(14.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(fill)
            .hover_fill(theme.bg_row_hover)
            .text_aligned(s.name.clone(), 12.0, fg, Alignment::Center)
            .on_click(Msg::SelectSession(i))
        })
        .collect();

    // Botón «+»: crea una sesión local nueva.
    row.push(
        View::new(Style {
            size: Size { width: length(30.0_f32), height: length(28.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .hover_fill(theme.bg_button_hover)
        .text_aligned("+".to_string(), 16.0, theme.accent, Alignment::Center)
        .on_click(Msg::NewSession),
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(2.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(row)
}

/// Ancho de la columna de historial a la izquierda (px).
const HISTORY_W: f32 = 220.0;

/// La columna de **historial** a la izquierda: los comandos corridos en la
/// sesión activa (líneas `Prompt` del shell), el más reciente arriba. Clickear
/// una línea la recarga en el input del shell (`Msg::RunFromHistory`).
fn history_column(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    // Comandos de la sesión activa (en orden de ejecución; los invertimos para
    // mostrar el más nuevo arriba). Cada `Prompt` tiene la forma "$ <cmd>".
    let mut comandos: Vec<String> = Vec::new();
    if let Some(s) = model.active() {
        if let ModuleState::Shell(sh) = &s.shell.state {
            comandos = sh
                .output
                .iter()
                .filter(|l| l.kind == shuma_module_shell::OutputKind::Prompt)
                .map(|l| l.text.trim_start_matches("$ ").to_string())
                .collect();
        }
    }
    comandos.reverse();
    comandos.truncate(60); // sin scroll todavía: el tope cabe en pantalla

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: Rect { left: length(12.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned("Historial".to_string(), 11.0, theme.fg_muted, Alignment::Start);

    let mut children = vec![header];
    if comandos.is_empty() {
        children.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
                padding: Rect { left: length(12.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
                align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
                ..Default::default()
            })
            .text_aligned("(sin comandos aún)".to_string(), 11.0, theme.fg_muted, Alignment::Start),
        );
    } else {
        for cmd in comandos {
            let label = cmd.clone();
            children.push(
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
                    padding: Rect { left: length(12.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
                    align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .hover_fill(theme.bg_row_hover)
                .text_aligned(label, 12.0, theme.fg_text, Alignment::Start)
                .on_click(Msg::RunFromHistory(cmd)),
            );
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(HISTORY_W), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(children)
}

/// Dibuja el icono **vectorial** de un diente (no texto: la fuente no trae los
/// glyphs y salían cuadritos «tofu»). `kind = None` → el diente de medidores.
/// Mismo enfoque que el rail de pata (`paint_with` + kurbo).
fn rail_icon(view: Option<SessionView>, size: f32, color: Color) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent};
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
        use llimphi_ui::llimphi_raster::peniko::Fill;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let r = (rect.w.min(rect.h) as f64 * 0.34).max(2.0);
        let stroke = Stroke::new((r * 0.22).max(1.2));
        match view {
            // Medidores: tres barras verticales (estilo bar-chart).
            None => {
                let heights = [0.55_f64, 0.95, 0.7];
                let bw = r * 0.45;
                let gap = r * 0.32;
                let total = 3.0 * bw + 2.0 * gap;
                let x0 = cx - total / 2.0;
                for (i, h) in heights.iter().enumerate() {
                    let x = x0 + i as f64 * (bw + gap);
                    let top = (cy + r) - 2.0 * r * h;
                    let bar = RoundedRect::new(x, top, x + bw, cy + r, 1.0);
                    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &bar);
                }
            }
            // Shell: marco de terminal + chevron «>».
            Some(SessionView::Shell) => {
                let frame = RoundedRect::new(cx - r, cy - r * 0.78, cx + r, cy + r * 0.78, 2.0);
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &frame);
                let mut p = BezPath::new();
                p.move_to(Point::new(cx - r * 0.42, cy - r * 0.28));
                p.line_to(Point::new(cx - r * 0.02, cy));
                p.line_to(Point::new(cx - r * 0.42, cy + r * 0.28));
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &p);
            }
            // Hosts: tres racks de servidor apilados.
            Some(SessionView::Hosts) => {
                for i in 0..3 {
                    let y = cy - r + i as f64 * (r * 0.78);
                    let rack = RoundedRect::new(cx - r, y, cx + r, y + r * 0.5, 1.5);
                    scene.stroke(&stroke, Affine::IDENTITY, color, None, &rack);
                }
            }
            // Vhosts: globo (círculo + meridiano + ecuador) = dominios.
            Some(SessionView::Vhosts) => {
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &Circle::new((cx, cy), r));
                scene.stroke(
                    &stroke,
                    Affine::IDENTITY,
                    color,
                    None,
                    &Line::new(Point::new(cx - r, cy), Point::new(cx + r, cy)),
                );
                let mut meridiano = BezPath::new();
                meridiano.move_to(Point::new(cx, cy - r));
                meridiano.quad_to(Point::new(cx - r, cy), Point::new(cx, cy + r));
                meridiano.quad_to(Point::new(cx + r, cy), Point::new(cx, cy - r));
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &meridiano);
            }
            // Lienzo/grafo: tres nodos conectados.
            Some(SessionView::Canvas) => {
                let a = Point::new(cx - r * 0.6, cy - r * 0.4);
                let b = Point::new(cx + r * 0.55, cy - r * 0.5);
                let c = Point::new(cx + r * 0.05, cy + r * 0.65);
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &Line::new(a, b));
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &Line::new(a, c));
                for pt in [a, b, c] {
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        color,
                        None,
                        &Circle::new((pt.x, pt.y), r * 0.3),
                    );
                }
            }
        }
    })
}

/// El rail de dientes a la derecha: uno por vista (la tab seleccionada va
/// activa) más un diente sentinela que togglea el panel de medidores. Reusa los
/// mismos `Msg` que el rail hospedado de pata (`SelectTab` / `HostActivate`).
fn view_dock_rail(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};

    // Un diente por vista (id = índice en SessionView::ALL) + el de medidores.
    let mut items: Vec<DockRailItem> = SessionView::ALL
        .iter()
        .enumerate()
        .map(|(i, v)| DockRailItem { id: i as u64, active: *v == model.active_view })
        .collect();
    items.push(DockRailItem {
        id: MONITORS_TOOTH as u64,
        active: model.monitors_visible,
    });

    dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| {
            let view = SessionView::ALL.get(id as usize).copied();
            rail_icon(view, size, color)
        },
        move |id| match SessionView::ALL.get(id as usize) {
            Some(v) => Msg::SelectView(*v),
            None => Msg::HostActivate(MONITORS_TOOTH), // sentinela = toggle medidores
        },
        |_| None, // sin reorder
    )
}

/// Toolbar de la tira de tabs: pinta los `ShortcutSpec` del tab activo
/// como botones que disparan `Msg::ShortcutClicked`. Si el tab activo
/// no aporta shortcuts, la barra queda vacía (alto 0 — colapsa).
pub(crate) fn tabs_toolbar(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::prelude::Dimension;
    use llimphi_ui::llimphi_text::Alignment;

    let Some(session) = model.active() else {
        return empty_bar(theme, 0.0);
    };
    let inst = session.instance(model.active_view.which());
    let slot = model.active_view_slot();
    let contribs = match &inst.state {
        ModuleState::Launcher(s) => shuma_module_launcher::contributions(s),
        ModuleState::CommandBar(s) => shuma_module_commandbar::contributions(s),
        ModuleState::Shell(s) => shuma_module_shell::contributions(s),
        ModuleState::Matilda(s) => shuma_module_matilda::contributions(s),
        ModuleState::Minga(s) => shuma_module_minga::contributions(s),
        ModuleState::Canvas(s) => shuma_module_canvas::contributions(s),
    };

    if contribs.shortcuts.is_empty() {
        return empty_bar(theme, 0.0);
    }

    let mut buttons: Vec<View<Msg>> = contribs
        .shortcuts
        .into_iter()
        .map(|spec| shortcut_button(slot.clone(), spec, theme))
        .collect();

    // Label izquierdo: «sesión · vista».
    let titulo = format!("{} · {}", session.name, view_label(model.active_view));
    let label = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(titulo, 12.0, theme.fg_text, Alignment::Start);

    let mut row = vec![label];
    row.append(&mut buttons);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(row)
}

pub(crate) fn shortcut_button(slot: Slot, spec: ShortcutSpec, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{prelude::Dimension, AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;

    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(26.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        margin: Rect {
            left: length(4.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_button)
    .hover_fill(theme.bg_button_hover)
    .radius(4.0)
    .text_aligned(spec.label.clone(), 12.0, theme.fg_text, Alignment::Center)
    .on_click(Msg::ShortcutClicked(slot, spec.action))
}

pub(crate) fn tab_content(model: &Model, theme: &Theme) -> View<Msg> {
    let Some(session) = model.active() else {
        return placeholder(theme, &rimay_localize::t("shuma-empty-no-tabs"));
    };
    let idx = model.active_session;
    match model.active_view {
        SessionView::Shell => match &session.shell.state {
            ModuleState::Shell(state) => shuma_module_shell::view::<Msg>(state, theme, move |m| {
                Msg::Module(Slot::Session(idx, Which::Shell), ModuleMsg::Shell(m))
            }),
            _ => placeholder(theme, ""),
        },
        SessionView::Canvas => match &session.canvas.state {
            ModuleState::Canvas(state) => {
                shuma_module_canvas::view::<Msg>(state, theme, move |m| {
                    Msg::Module(Slot::Session(idx, Which::Canvas), ModuleMsg::Canvas(m))
                })
            }
            _ => placeholder(theme, ""),
        },
        SessionView::Hosts => match &session.matilda.state {
            ModuleState::Matilda(state) => hosts_view(&state.desired, theme),
            _ => placeholder(theme, ""),
        },
        SessionView::Vhosts => match &session.matilda.state {
            ModuleState::Matilda(state) => vhosts_view(&state.desired, theme),
            _ => placeholder(theme, ""),
        },
    }
}

/// Lista de hosts del inventario de la sesión: nombre · dirección · tags.
fn hosts_view(inv: &matilda_core::Inventory, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let mut filas: Vec<View<Msg>> = inv
        .hosts()
        .map(|h| {
            let tags = if h.tags.is_empty() {
                String::new()
            } else {
                format!("  [{}]", h.tags.join(", "))
            };
            inventory_row(format!("{}", h.name), format!("{}{tags}", h.address), theme)
        })
        .collect();
    if filas.is_empty() {
        filas.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
                padding: Rect { left: length(16.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
                align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
                ..Default::default()
            })
            .text_aligned("sin hosts en el inventario".to_string(), 12.0, theme.fg_muted, Alignment::Start),
        );
    }
    inventory_panel("Hosts", filas, theme)
}

/// Lista de vhosts del inventario: dominio · upstream · TLS.
fn vhosts_view(inv: &matilda_core::Inventory, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    use matilda_core::Upstream;
    let mut filas: Vec<View<Msg>> = inv
        .vhosts()
        .map(|v| {
            let up = match &v.upstream {
                Upstream::Address(a) => a.clone(),
                Upstream::Container { name, port } => format!("{name}:{port}"),
            };
            let tls = if v.tls { "  🔒 TLS" } else { "" };
            inventory_row(v.domain.clone(), format!("→ {up}{tls}"), theme)
        })
        .collect();
    if filas.is_empty() {
        filas.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
                padding: Rect { left: length(16.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
                align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
                ..Default::default()
            })
            .text_aligned("sin vhosts en el inventario".to_string(), 12.0, theme.fg_muted, Alignment::Start),
        );
    }
    inventory_panel("Vhosts", filas, theme)
}

/// Una fila de inventario: título a la izquierda, detalle tenue a la derecha.
fn inventory_row(titulo: String, detalle: String, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        padding: Rect { left: length(16.0_f32), right: length(16.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
        gap: Size { width: length(12.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .children(vec![
        View::new(Style {
            size: Size { width: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(), height: percent(1.0_f32) },
            flex_grow: 1.0,
            align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(titulo, 13.0, theme.fg_text, Alignment::Start),
        View::new(Style {
            size: Size { width: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(), height: percent(1.0_f32) },
            align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(detalle, 12.0, theme.fg_muted, Alignment::End),
    ])
}

/// Marco de un panel de inventario: cabecera + filas en columna.
fn inventory_panel(titulo: &str, filas: Vec<View<Msg>>, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        padding: Rect { left: length(16.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(titulo.to_string(), 12.0, theme.fg_muted, Alignment::Start);

    let mut children = vec![header];
    children.extend(filas);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(children)
}

// ─── Monitor stack ─────────────────────────────────────────────────

pub(crate) fn monitor_stack(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = StatCardPalette::from_theme(theme);

    let (cpu_value, mem_value) = match model.last_snapshot {
        Some(s) if s.valid => (s.cpu_percent, s.mem_percent),
        _ => (0.0, 0.0),
    };

    let cpu_card = monitor_card(
        "CPU",
        format!("{cpu_value:>3.0}%"),
        match model.last_snapshot {
            Some(s) if s.valid => format!(
                "{} de {} muestras",
                model.sysmon.cpu_history().len(),
                HISTORY
            ),
            _ => rimay_localize::t("shuma-empty-no-data-linux"),
        },
        Color::from_rgb8(0x82, 0xCF, 0xF2),
        model.sysmon.cpu_history().values(),
        &palette,
    );

    let mem_card = monitor_card(
        "MEM",
        format!("{mem_value:>3.0}%"),
        match model.last_snapshot {
            Some(s) if s.valid => format!("{} MB de {} MB", s.mem_used_mb, s.mem_total_mb),
            _ => rimay_localize::t("shuma-empty-no-data"),
        },
        Color::from_rgb8(0xF7, 0xC8, 0x7A),
        model.sysmon.mem_history().values(),
        &palette,
    );

    let mut children = vec![cpu_card, mem_card];

    // Stat-cards extra: una por cada `MonitorSpec` aportado por los
    // módulos vivos. El historial vive en `model.extra_history`.
    for (slot, contribs) in collect_contributions(model) {
        for spec in &contribs.monitors {
            let key = monitor_key(&slot, spec);
            let history = model
                .extra_history
                .get(&key)
                .cloned()
                .unwrap_or_default();
            let display = model
                .extra_display
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "—".into());
            let accent = Color::from_rgb8(spec.accent.r, spec.accent.g, spec.accent.b);
            children.push(monitor_card(
                spec.label.as_str(),
                display,
                rimay_localize::t_args(
                    "shuma-stat-samples",
                    &[
                        ("have", history.len().to_string().into()),
                        ("total", HISTORY.to_string().into()),
                    ],
                ),
                accent,
                history,
                &palette,
            ));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(children)
}

pub(crate) fn monitor_card(
    label: &str,
    value: String,
    description: String,
    accent: Color,
    history: Vec<f32>,
    palette: &StatCardPalette,
) -> View<Msg> {
    let card = stat_card_view::<Msg>(label, value, description.as_str(), accent, &[], palette);
    let curve = curve_view(history, accent);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .children(vec![card, curve])
}

pub(crate) fn curve_view(history: Vec<f32>, accent: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(56.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect: PaintRect| {
        if history.len() < 2 {
            return;
        }
        let n = history.len() as f32;
        let dx = if n > 1.0 { rect.w / (n - 1.0) } else { rect.w };
        let mut path = BezPath::new();
        for (i, v) in history.iter().enumerate() {
            let x = rect.x + dx * i as f32;
            let y = rect.y + rect.h - (v.clamp(0.0, 100.0) / 100.0) * rect.h;
            let p = Point::new(x as f64, y as f64);
            if i == 0 {
                path.push(PathEl::MoveTo(p));
            } else {
                path.push(PathEl::LineTo(p));
            }
        }
        scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, accent, None, &path);
    })
}

pub(crate) fn placeholder(theme: &Theme, text: &str) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(20.0_f32),
            bottom: length(20.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .text_aligned(text.to_string(), 13.0, theme.fg_muted, Alignment::Start)
}
