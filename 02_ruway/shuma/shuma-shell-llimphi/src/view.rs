//! Render del chasis: topbar, tabs, área principal, monitores.

use super::*;

use llimphi_widget_select::{
    select_menu_view, select_trigger_view, SelectItem, SelectMenuSpec, SelectPalette, SelectPhase,
};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};

/// Alto del disparador del select (debe seguir a `llimphi-widget-select`).
const TRIGGER_H: f32 = 34.0;

/// Ítems del dropdown de aislamiento (orden = `Isolation::ALL`).
fn iso_items() -> Vec<SelectItem> {
    vec![
        SelectItem::new("Local").with_sublabel("Directo en esta máquina."),
        SelectItem::new("Remoto (SSH)").with_sublabel("En otra máquina por SSH."),
    ]
}
fn distro_items() -> Vec<SelectItem> {
    Distro::ALL.iter().map(|d| SelectItem::new(d.label())).collect()
}
fn iso_index(iso: Isolation) -> usize {
    Isolation::ALL.iter().position(|x| *x == iso).unwrap_or(0)
}
fn distro_index(d: Distro) -> usize {
    Distro::ALL.iter().position(|x| *x == d).unwrap_or(0)
}

/// `y` aproximado del disparador de un dropdown dentro del panel de sesión —
/// para anclar su menú. Sigue el orden de `session_panel` (padding+secciones).
/// Aproximado: el menú flota, no va pegado al pixel.
fn cfg_trigger_y(is_draft: bool, kind: DropKind) -> f32 {
    // Orden: title, conn, [note], label-aislamiento, ISO-trigger, header-cont,
    // [abierto] label-distro, DISTRO-trigger, label-cont, CONT-trigger.
    let iso_y = if is_draft { 134.0 } else { 92.0 };
    match kind {
        DropKind::Isolation => iso_y,
        DropKind::Distro => iso_y + 98.0,
        DropKind::Container => iso_y + 98.0 + 64.0,
    }
}

/// El menú del dropdown de config abierto (para `App::view_overlay`).
pub(crate) fn dropdown_overlay(model: &Model) -> Option<View<Msg>> {
    let kind = model.dropdown_open?;
    let session = model.active()?;
    let is_draft = session.kind == SessionKind::Draft;
    let pal = SelectPalette::from_theme(&model.theme);

    let (items, selected_vec): (Vec<SelectItem>, Vec<usize>) = match kind {
        DropKind::Isolation => (iso_items(), vec![iso_index(session.isolation)]),
        DropKind::Distro => (distro_items(), vec![distro_index(session.distro)]),
        DropKind::Container => {
            // Contenedores existentes + la opción de crear uno nuevo al final.
            let mut its: Vec<SelectItem> = model
                .containers
                .iter()
                .map(|c| SelectItem::new(c.clone()))
                .collect();
            its.push(SelectItem::new(format!(
                "+ Crear nuevo ({})",
                session.distro.label()
            )));
            let sel = session
                .container
                .as_ref()
                .and_then(|c| model.containers.iter().position(|x| x == c))
                .map(|i| vec![i])
                .unwrap_or_default();
            (its, sel)
        }
    };
    let visible: Vec<usize> = (0..items.len()).collect();
    let anchor = (12.0, cfg_trigger_y(is_draft, kind) + TRIGGER_H + 4.0);
    let width = (model.session_w - 24.0).max(140.0);

    let n_containers = model.containers.len();
    let on_pick: std::sync::Arc<dyn Fn(usize) -> Msg + Send + Sync> = match kind {
        DropKind::Isolation => {
            std::sync::Arc::new(|i| Msg::SetIsolation(Isolation::ALL[i.min(1)]))
        }
        DropKind::Distro => std::sync::Arc::new(|i| Msg::SetDistro(Distro::ALL[i.min(3)])),
        DropKind::Container => std::sync::Arc::new(move |i| {
            if i < n_containers {
                Msg::SubscribeContainer(i)
            } else {
                Msg::CreateContainer
            }
        }),
    };

    Some(select_menu_view(SelectMenuSpec {
        anchor,
        viewport: (1280.0, 800.0),
        width,
        phase: SelectPhase::Ready(&items),
        visible: &visible,
        active: usize::MAX,
        selected: &selected_vec,
        query: "",
        searchable: false,
        empty_text: "",
        appear: 1.0,
        on_pick,
        on_hover: None,
        on_dismiss: Msg::DismissDropdown,
        on_retry: None,
        palette: &pal,
    }))
}

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
/// Ancho del rail de herramientas (derecha) y de sesiones (izquierda), px.
const RAIL_W: f32 = 44.0;
const SESSION_RAIL_W: f32 = 50.0;

pub(crate) fn render_tabs_with_monitors(model: &Model, theme: &Theme) -> View<Msg> {
    // Estándar del dock-rail: el rail de dientes va PEGADO al canvas y su panel
    // se despliega hacia AFUERA (resizable, drag del divisor). Nunca el rail a la
    // derecha de su panel. Orden:
    //   panel-sesión(resizable) | rail-sesión | CANVAS | rail-tool | panel-tool(resizable)
    let sp = SplitterPalette::from_theme(theme);

    // Núcleo: rail-sesión | canvas | rail-tool (los rails pegados al canvas).
    let inner = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size { width: length(0.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![
        session_rail(model, theme),
        canvas_view(model, theme),
        tool_rail(model, theme),
    ]);

    // Panel de herramienta a la derecha del rail-tool, resizable.
    let mut core = inner;
    if let Some(tool) = model.active_tool {
        core = splitter_two(
            Direction::Row,
            core,
            PaneSize::Flex,
            tool_panel(model, tool, theme),
            PaneSize::Fixed(model.monitors_width),
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::SetToolWidth(dx)),
                DragPhase::End => None,
            },
            &sp,
        );
    }

    // Panel de sesión a la izquierda del rail-sesión, resizable.
    if model.session_panel_open {
        core = splitter_two(
            Direction::Row,
            session_panel(model, theme),
            PaneSize::Fixed(model.session_w),
            core,
            PaneSize::Flex,
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::SetSessionWidth(dx)),
                DragPhase::End => None,
            },
            &sp,
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![core])
}

/// El **panel de la sesión activa** (a la derecha de su rail): TODA su
/// configuración. La draft trae el aislamiento a elegir → al configurarlo nace
/// una sesión (no hay botón «+»). Una sesión real muestra su aislamiento + el
/// botón para cerrarla.
fn session_panel(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    let Some(session) = model.active() else {
        return View::new(Style::default());
    };
    let idx = model.active_session;
    let es_draft = session.kind == SessionKind::Draft;

    let titulo = if es_draft { "Borrador".to_string() } else { session.name.clone() };

    let pal = SelectPalette::from_theme(theme);
    let mut children: Vec<View<Msg>> = vec![panel_title(&titulo, theme)];

    // Estado de conexión de la sesión (en espera / conectado / desconectado).
    children.push(conn_pill(session.conn, theme));

    if es_draft {
        children.push(panel_note(
            "Trabajás sin guardar. Al configurar abajo, nace una sesión propia.",
            theme,
        ));
    }

    // Aislamiento base: dropdown Local / Remoto.
    children.push(panel_label("Aislamiento", theme));
    let isos = iso_items();
    children.push(select_trigger_view(
        isos.get(iso_index(session.isolation)),
        "Elegí el aislamiento…",
        model.dropdown_open == Some(DropKind::Isolation),
        None,
        &pal,
        Msg::ToggleDropdown(DropKind::Isolation),
    ));

    // Remoto: form de conexión (host/usuario/puerto) + Conectar.
    if session.isolation == Isolation::Remote {
        let tpal = TextInputPalette::from_theme(theme);
        children.push(panel_label("Host", theme));
        children.push(text_input_view(
            &session.host,
            "ejemplo.com",
            model.focused_field == Some(RemoteField::Host),
            &tpal,
            Msg::FocusField(RemoteField::Host),
        ));
        children.push(panel_label("Usuario", theme));
        children.push(text_input_view(
            &session.user,
            "usuario",
            model.focused_field == Some(RemoteField::User),
            &tpal,
            Msg::FocusField(RemoteField::User),
        ));
        children.push(panel_label("Puerto", theme));
        children.push(text_input_view(
            &session.port,
            "22",
            model.focused_field == Some(RemoteField::Port),
            &tpal,
            Msg::FocusField(RemoteField::Port),
        ));
        children.push(action_button("Conectar", Msg::ConnectRemote, theme));
    }

    // Contenedor: capa OPCIONAL (encima de Local o Remoto) en un colapsable.
    children.push(container_header(session, theme));
    if session.container_open {
        children.push(panel_label("Distro", theme));
        let distros = distro_items();
        children.push(select_trigger_view(
            distros.get(distro_index(session.distro)),
            "Elegí la distro…",
            model.dropdown_open == Some(DropKind::Distro),
            None,
            &pal,
            Msg::ToggleDropdown(DropKind::Distro),
        ));
        let sub = session.container.as_ref().map(|c| SelectItem::new(c.clone()));
        children.push(select_trigger_view(
            sub.as_ref(),
            "Crear / conectar contenedor…",
            model.dropdown_open == Some(DropKind::Container),
            None,
            &pal,
            Msg::ToggleDropdown(DropKind::Container),
        ));
    }

    // Estado actual + cerrar (sólo sesiones reales).
    if !es_draft {
        children.push(panel_label("cwd", theme));
        children.push(panel_note(&session_cwd(session), theme));
        children.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
                margin: Rect { left: length(0.0_f32), right: length(0.0_f32), top: length(10.0_f32), bottom: length(0.0_f32) },
                align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
                justify_content: Some(llimphi_ui::llimphi_layout::taffy::JustifyContent::Center),
                ..Default::default()
            })
            .fill(theme.bg_button)
            .hover_fill(theme.bg_button_hover)
            .radius(5.0)
            .text_aligned("Cerrar sesión".to_string(), 12.0, theme.fg_text, Alignment::Center)
            .on_click(Msg::CloseSession(idx)),
        );
    }

    panel_frame(children, theme)
}

/// cwd del shell de una sesión (para el panel de config).
fn session_cwd(session: &Session) -> String {
    match &session.shell.state {
        ModuleState::Shell(sh) => sh.cwd.display().to_string(),
        _ => "-".to_string(),
    }
}

/// Píldora de estado de conexión: punto de color + texto.
fn conn_pill(conn: ConnState, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    let color = match conn {
        ConnState::Connected => llimphi_ui::llimphi_raster::peniko::Color::from_rgb8(0x4a, 0xde, 0x80),
        ConnState::Pending => llimphi_ui::llimphi_raster::peniko::Color::from_rgb8(0xf7, 0xc8, 0x7a),
        ConnState::Disconnected => llimphi_ui::llimphi_raster::peniko::Color::from_rgb8(0xe0, 0x6c, 0x6c),
    };
    let dot = View::new(Style {
        size: Size { width: length(12.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(llimphi_ui::llimphi_layout::taffy::JustifyContent::Center),
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle};
        use llimphi_ui::llimphi_raster::peniko::Fill;
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &Circle::new((cx, cy), 4.0));
    });
    let txt = View::new(Style {
        // height auto → el Row (align Center) lo centra; con 22px fijo el texto
        // Start quedaba pegado arriba. Ver `container_header`.
        size: Size { width: percent(1.0_f32), height: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto() },
        ..Default::default()
    })
    .text_aligned(conn.label().to_string(), 11.0, theme.fg_muted, Alignment::Start);
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

/// Cabecera clickeable del colapsable de contenedor: chevron (vectorial) +
/// "Contenedor" + el nombre suscrito o «ninguno». Click → `Msg::ToggleContainer`.
fn container_header(session: &Session, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{prelude::Dimension, AlignItems};
    use llimphi_ui::llimphi_text::Alignment;
    let open = session.container_open;
    let accent = theme.accent;
    let chevron = View::new(Style {
        size: Size { width: length(16.0_f32), height: length(28.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Point, Stroke};
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let r = 4.0;
        let mut p = BezPath::new();
        if open {
            // ▾
            p.move_to(Point::new(cx - r, cy - r * 0.5));
            p.line_to(Point::new(cx, cy + r * 0.5));
            p.line_to(Point::new(cx + r, cy - r * 0.5));
        } else {
            // ▸
            p.move_to(Point::new(cx - r * 0.5, cy - r));
            p.line_to(Point::new(cx + r * 0.5, cy));
            p.line_to(Point::new(cx - r * 0.5, cy + r));
        }
        scene.stroke(&Stroke::new(1.6), Affine::IDENTITY, accent, None, &p);
    });
    // `height: auto` (no 28px fijo): el texto Start se ancla ARRIBA del nodo
    // (el compositor sólo centra vertical el texto Center), así que un nodo más
    // alto que el texto lo deja descentrado. Con altura = altura del texto, el
    // `align_items: Center` del Row de afuera lo centra en sus 28px.
    let label = View::new(Style {
        size: Size { width: Dimension::auto(), height: Dimension::auto() },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned("Contenedor".to_string(), 12.0, theme.fg_text, Alignment::Start);
    let estado = View::new(Style {
        size: Size { width: Dimension::auto(), height: Dimension::auto() },
        ..Default::default()
    })
    .text_aligned(
        session.container.clone().unwrap_or_else(|| "ninguno".into()),
        11.0,
        theme.fg_muted,
        Alignment::End,
    );
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        margin: Rect { left: length(0.0_f32), right: length(0.0_f32), top: length(6.0_f32), bottom: length(0.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .radius(5.0)
    .on_click(Msg::ToggleContainer)
    .children(vec![chevron, label, estado])
}

/// El **canvas principal**: SÓLO el shell de la sesión activa. Sin barra de
/// tabs/shortcuts encima (los atajos viven en el menú/command-bar).
fn canvas_view(model: &Model, theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: length(0.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![tab_content(model, theme)])
}

/// El rail IZQUIERDO de **sesiones**: la draft primero, luego las creadas (se
/// agregan al frente). Cada diente lleva el icono de su tipo, una insignia
/// numérica y un LED de actividad, y es **arrastrable** para reordenar (la draft
/// queda fija). No hay `+`: la sesión nace al configurar la draft.
fn session_rail(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;

    let teeth: Vec<View<Msg>> = model
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let activa = i == model.active_session;
            let fill = if activa { theme.bg_selected } else { theme.bg_panel_alt };
            let icon_color = if activa { theme.accent } else { theme.fg_muted };
            // Insignia: número para las creadas, vacío para la draft.
            let badge = s.number.map(|n| n.to_string()).unwrap_or_default();
            let icon = session_tooth_icon(s.kind, s.active_data(), 22.0, icon_color);
            let num = View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(12.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text_aligned(badge, 9.0, theme.fg_muted, Alignment::Center);
            // Cada diente: clickeable (selecciona/togglea panel), arrastrable
            // (payload = su índice) y drop-target (soltar otro acá lo reordena).
            // La draft (0) no es arrastrable ni acepta drop (queda fija).
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
            .on_click(Msg::SelectSession(i))
            .children(vec![icon, num]);
            if i > 0 {
                tooth = tooth
                    .draggable_at(|phase, _, _, _, _| match phase {
                        DragPhase::Move | DragPhase::End => None,
                    })
                    .drag_payload(i as u64)
                    .on_drop(move |payload| Some(Msg::ReorderSession(payload as usize, i)))
                    .drop_hover_fill(theme.bg_row_hover);
            }
            tooth
        })
        .collect();
    // Sin botón «+»: la sesión nace al configurar la draft desde su panel.

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(SESSION_RAIL_W), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(teeth)
}

/// Icono vectorial del diente de una sesión según su tipo, con el LED de
/// actividad en la esquina (verde si hay datos moviéndose).
fn session_tooth_icon(kind: SessionKind, active_data: bool, size: f32, color: Color) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent};
    View::new(Style {
        size: Size { width: length(size), height: length(size) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Line, Point, RoundedRect, Stroke};
        use llimphi_ui::llimphi_raster::peniko::{Color as PColor, Fill};
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let r = (rect.w.min(rect.h) as f64 * 0.34).max(2.0);
        let stroke = Stroke::new((r * 0.22).max(1.2));
        match kind {
            // Draft: cuadro punteado (vacío, no toca nada).
            SessionKind::Draft => {
                let sq = RoundedRect::new(cx - r, cy - r, cx + r, cy + r, 2.0);
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &sq);
                let mut pencil = BezPath::new();
                pencil.move_to(Point::new(cx - r * 0.4, cy + r * 0.4));
                pencil.line_to(Point::new(cx + r * 0.4, cy - r * 0.4));
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &pencil);
            }
            // Local: cuadro lleno (caja local).
            SessionKind::Local => {
                let sq = RoundedRect::new(cx - r, cy - r, cx + r, cy + r, 2.5);
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &sq);
            }
            // Remoto: globo (círculo + ecuador + meridiano).
            SessionKind::Remote => {
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &Circle::new((cx, cy), r));
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &Line::new(Point::new(cx - r, cy), Point::new(cx + r, cy)));
                let mut m = BezPath::new();
                m.move_to(Point::new(cx, cy - r));
                m.quad_to(Point::new(cx - r, cy), Point::new(cx, cy + r));
                m.quad_to(Point::new(cx + r, cy), Point::new(cx, cy - r));
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &m);
            }
        }
        // LED de actividad: punto en la esquina superior derecha.
        let led = if active_data {
            PColor::from_rgb8(0x4a, 0xde, 0x80) // verde = datos moviéndose
        } else {
            PColor::from_rgb8(0x55, 0x5a, 0x66) // gris apagado
        };
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            led,
            None,
            &Circle::new((cx + r * 1.05, cy - r * 1.05), (r * 0.32).max(1.5)),
        );
    })
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
/// Icono **vectorial** de una herramienta del rail derecho (`paint_with`+kurbo,
/// no texto — eso daba "tofu").
fn tool_icon(tool: Tool, size: f32, color: Color) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent};
    View::new(Style {
        size: Size { width: length(size), height: length(size) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{
            Affine, BezPath, Circle, Point, RoundedRect, Stroke,
        };
        use llimphi_ui::llimphi_raster::peniko::Fill;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let r = (rect.w.min(rect.h) as f64 * 0.34).max(2.0);
        let stroke = Stroke::new((r * 0.22).max(1.2));
        match tool {
            // Historial: reloj (círculo + manecillas).
            Tool::History => {
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &Circle::new((cx, cy), r));
                let mut h = BezPath::new();
                h.move_to(Point::new(cx, cy));
                h.line_to(Point::new(cx, cy - r * 0.55));
                h.move_to(Point::new(cx, cy));
                h.line_to(Point::new(cx + r * 0.45, cy));
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &h);
            }
            // Monitor: tres barras verticales (bar-chart).
            Tool::Monitor => {
                let heights = [0.55_f64, 0.95, 0.7];
                let bw = r * 0.45;
                let gap = r * 0.32;
                let total = 3.0 * bw + 2.0 * gap;
                let x0 = cx - total / 2.0;
                for (i, h) in heights.iter().enumerate() {
                    let x = x0 + i as f64 * (bw + gap);
                    let top = (cy + r) - 2.0 * r * h;
                    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &RoundedRect::new(x, top, x + bw, cy + r, 1.0));
                }
            }
            // Explorer: carpeta (rect con pestaña).
            Tool::Explorer => {
                let body = RoundedRect::new(cx - r, cy - r * 0.5, cx + r, cy + r * 0.75, 2.0);
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &body);
                let mut tab = BezPath::new();
                tab.move_to(Point::new(cx - r, cy - r * 0.5));
                tab.line_to(Point::new(cx - r * 0.4, cy - r * 0.5));
                tab.line_to(Point::new(cx - r * 0.2, cy - r * 0.85));
                tab.line_to(Point::new(cx - r, cy - r * 0.85));
                tab.close_path();
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &tab);
            }
            // Matilda: tres racks apilados (inventario de servidores).
            Tool::Matilda => {
                for i in 0..3 {
                    let y = cy - r + i as f64 * (r * 0.78);
                    scene.stroke(&stroke, Affine::IDENTITY, color, None, &RoundedRect::new(cx - r, y, cx + r, y + r * 0.5, 1.5));
                }
            }
        }
    })
}

/// El rail DERECHO de **herramientas** de la sesión activa (historial, monitor,
/// explorer, matilda). La herramienta abierta va resaltada; re-clickear cierra
/// su panel. Reusa el `dock_rail` (mismo look que pata).
fn tool_rail(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};

    let items: Vec<DockRailItem> = Tool::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| DockRailItem { id: i as u64, active: model.active_tool == Some(*t) })
        .collect();

    dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| {
            let t = Tool::ALL.get(id as usize).copied().unwrap_or(Tool::History);
            tool_icon(t, size, color)
        },
        move |id| Msg::SelectTool(Tool::ALL.get(id as usize).copied().unwrap_or(Tool::History)),
        |_| None,
    )
}

/// El panel de la herramienta activa (entre el canvas y el rail derecho).
// ─── Estilo común de paneles (padding, chips, etiquetas) ───────────────

/// Marco de un panel lateral: ancho fijo, **padding** (los márgenes que
/// faltaban), fondo y gap entre secciones.
fn panel_frame(children: Vec<View<Msg>>, theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(children)
}

/// Título de un panel (nombre de la sesión / herramienta).
fn panel_title(t: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(t.to_string(), 13.0, theme.fg_text, Alignment::Start)
}

/// Etiqueta de sección (tenue, chica).
fn panel_label(t: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::prelude::Dimension;
    use llimphi_ui::llimphi_text::Alignment;
    // `height: auto` = altura del texto: con `Start` el texto se ancla arriba,
    // así que un nodo fijo de 18px lo dejaba descentrado. Ver `container_header`.
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        margin: Rect { left: length(0.0_f32), right: length(0.0_f32), top: length(8.0_f32), bottom: length(2.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(t.to_string(), 10.0, theme.fg_muted, Alignment::Start)
}

/// Nota/párrafo tenue dentro de un panel.
fn panel_note(t: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{prelude::Dimension, AlignItems};
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(t.to_string(), 11.0, theme.fg_muted, Alignment::Start)
}


/// Fila de chips, con wrap si no caben en el ancho del panel.
fn chip_row(chips: Vec<View<Msg>>) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{prelude::Dimension, FlexWrap};
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        ..Default::default()
    })
    .children(chips)
}

fn tool_panel(model: &Model, tool: Tool, theme: &Theme) -> View<Msg> {
    let inner = match tool {
        Tool::History => history_column(model, theme),
        Tool::Monitor => monitor_stack(model, theme),
        Tool::Explorer => explorer_panel(model, theme),
        Tool::Matilda => matilda_panel(model, theme),
    };
    panel_frame(vec![inner], theme)
}

/// Panel Explorer/SFTP: lista los archivos del cwd de la sesión (local). El
/// SFTP remoto queda pendiente (fase de aislamiento remoto).
fn explorer_panel(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let cwd = model
        .active()
        .and_then(|s| match &s.shell.state {
            ModuleState::Shell(sh) => Some(sh.cwd.display().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| ".".to_string());

    let mut filas: Vec<View<Msg>> = vec![tool_header(&format!("Explorer · {cwd}"), theme)];
    match std::fs::read_dir(&cwd) {
        Ok(rd) => {
            let mut entradas: Vec<(bool, String)> = rd
                .flatten()
                .map(|e| {
                    let dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    (dir, e.file_name().to_string_lossy().to_string())
                })
                .collect();
            entradas.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            entradas.truncate(200);
            for (dir, name) in entradas {
                let etiqueta = if dir { format!("{name}/") } else { name.clone() };
                // Click: una carpeta carga `cd <dir>` en el input del shell; un
                // archivo carga su nombre. El usuario confirma con Enter.
                let cmd = if dir { format!("cd {name}") } else { name.clone() };
                filas.push(
                    View::new(Style {
                        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
                        padding: Rect { left: length(12.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
                        align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
                        ..Default::default()
                    })
                    .hover_fill(theme.bg_row_hover)
                    .on_click(Msg::RunFromHistory(cmd))
                    .text_aligned(etiqueta, 12.0, if dir { theme.accent } else { theme.fg_text }, Alignment::Start),
                );
            }
        }
        Err(_) => filas.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
                padding: Rect { left: length(12.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
                align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
                ..Default::default()
            })
            .text_aligned("(cwd inaccesible · SFTP remoto pendiente)".to_string(), 11.0, theme.fg_muted, Alignment::Start),
        ),
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(filas)
}

/// Panel Matilda: hosts + vhosts del inventario de la sesión activa.
fn matilda_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let Some(session) = model.active() else {
        return tool_header("Matilda", theme);
    };
    let st = match &session.matilda.state {
        ModuleState::Matilda(st) => st.as_ref(),
        _ => return tool_header("Matilda", theme),
    };
    let slot = Slot::Session(model.active_session, Which::Matilda);

    // Botones de acción del módulo (Discover/Plan/Dry-run/Apply/Reload) — los
    // declara el propio módulo; los disparamos por el puente `handle_shortcut`.
    let acciones = shuma_module_matilda::contributions(st)
        .shortcuts
        .into_iter()
        .map(|spec| action_button(&spec.label, Msg::ShortcutClicked(slot.clone(), spec.action), theme))
        .collect::<Vec<_>>();
    let barra = chip_row(acciones); // wrap si no caben

    let hosts = hosts_view(&st.desired, theme);
    let vhosts = vhosts_view(&st.desired, theme);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        ..Default::default()
    })
    .children(vec![barra, hosts, vhosts])
}

/// Un botón de acción (para el panel de matilda).
fn action_button(label: &str, msg: Msg, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{prelude::Dimension, AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: Dimension::auto(), height: length(26.0_f32) },
        padding: Rect { left: length(10.0_f32), right: length(10.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        margin: Rect { left: length(0.0_f32), right: length(6.0_f32), top: length(0.0_f32), bottom: length(6.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_button)
    .hover_fill(theme.bg_button_hover)
    .radius(5.0)
    .text_aligned(label.to_string(), 11.5, theme.fg_text, Alignment::Center)
    .on_click(msg)
}

/// Cabecera tenue de un panel/sección de herramienta.
fn tool_header(titulo: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: Rect { left: length(12.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(titulo.to_string(), 11.0, theme.fg_muted, Alignment::Start)
}


/// El canvas principal de la sesión activa: su **shell** (terminal). Las demás
/// cosas (historial, monitor, explorer, matilda) viven en los paneles de
/// herramienta a la derecha.
pub(crate) fn tab_content(model: &Model, theme: &Theme) -> View<Msg> {
    let Some(session) = model.active() else {
        return placeholder(theme, &rimay_localize::t("shuma-empty-no-tabs"));
    };
    let idx = model.active_session;
    match &session.shell.state {
        ModuleState::Shell(state) => shuma_module_shell::view::<Msg>(state, theme, move |m| {
            Msg::Module(Slot::Session(idx, Which::Shell), ModuleMsg::Shell(m))
        }),
        _ => placeholder(theme, ""),
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
            let tls = if v.tls { "  TLS" } else { "" };
            inventory_row(v.domain.clone(), format!("-> {up}{tls}"), theme)
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
                .unwrap_or_else(|| "-".into());
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
