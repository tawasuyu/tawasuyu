//! Render del chasis: topbar, tabs, área principal, monitores.

use super::*;

use llimphi_widget_select::{
    select_menu_view, select_trigger_view, SelectItem, SelectMenuSpec, SelectPalette, SelectPhase,
};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};

/// Alto del disparador del select (debe seguir a `llimphi-widget-select`).
const TRIGGER_H: f32 = 34.0;

/// Ítems del dropdown de engine de aislamiento. Sólo muestra los que el
/// sistema tiene en `PATH` — si falta uno, no aparece (no podés elegirlo
/// y romperte). Orden de presentación = orden de preferencia.
pub(crate) fn engine_items() -> Vec<SelectItem> {
    let mut out: Vec<SelectItem> = Vec::new();
    if super::unshare_disponible() {
        out.push(
            SelectItem::new("unshare".to_string())
                .with_sublabel("util-linux + chroot — sin instalar nada (recomendado)"),
        );
    }
    if super::bwrap_disponible() {
        out.push(
            SelectItem::new("bwrap".to_string()).with_sublabel("bubblewrap — sandbox liviano"),
        );
    }
    if super::podman_disponible() {
        out.push(
            SelectItem::new("podman".to_string()).with_sublabel("OCI completo (con storage.conf)"),
        );
    }
    if out.is_empty() {
        out.push(
            SelectItem::new("(ninguno)".to_string()).with_sublabel(
                "instalá util-linux + coreutils, bubblewrap o podman",
            ),
        );
    }
    out
}

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
        DropKind::Engine => iso_y + 50.0,
        DropKind::Distro => iso_y + 98.0,
        DropKind::Container => iso_y + 98.0 + 64.0,
        // Host nunca usa este anclaje del panel (se expande inline en el canvas).
        DropKind::Host => iso_y,
    }
}

/// El menú del dropdown de config abierto (para `App::view_overlay`).
pub(crate) fn dropdown_overlay(model: &Model) -> Option<View<Msg>> {
    let kind = model.dropdown_open?;
    let session = model.active()?;
    // El form de sesión nueva (canvas) expande sus selects INLINE — no
    // anclamos un overlay flotante ahí (no sabemos su Y exacto). Sólo el
    // panel lateral usa este overlay.
    if session.pending {
        return None;
    }
    let is_draft = session.kind == SessionKind::Draft;
    let pal = SelectPalette::from_theme(&model.theme);

    let (items, selected_vec): (Vec<SelectItem>, Vec<usize>) = match kind {
        DropKind::Isolation => (iso_items(), vec![iso_index(session.isolation)]),
        // El resto (Host, Contenedor, Distro, Engine) se elige con pickers
        // INLINE (host_picker/container_picker); este overlay flotante ya no
        // los maneja. Antes pintaba `model.containers` + un "+ Crear nuevo"
        // ENCIMA de la lista inline, tapándola y creando un ubuntu random.
        DropKind::Host | DropKind::Container | DropKind::Distro | DropKind::Engine => {
            return None
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
        DropKind::Engine => {
            let items_clone = engine_items();
            std::sync::Arc::new(move |i| {
                let label = items_clone
                    .get(i)
                    .map(|it| it.label.clone())
                    .unwrap_or_default();
                Msg::SetEngine(label)
            })
        }
        DropKind::Container => std::sync::Arc::new(move |i| {
            if i < n_containers {
                Msg::SubscribeContainer(i)
            } else {
                Msg::CreateContainer
            }
        }),
        DropKind::Host => std::sync::Arc::new(|_| Msg::DismissDropdown),
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
            _ => status_bar(model, theme),
        },
        None => status_bar(model, theme),
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

/// Barra de estado inferior cuando el slot no tiene módulo CommandBar.
/// Si hay un diente de sesión bajo el cursor, muestra su nombre completo
/// (los dientes sólo pintan icono + número); si no, es la barra vacía.
/// El nombre aparece al `on_pointer_enter` y se borra al `on_pointer_leave`
/// — el ciclo de hover end-to-end, mismo patrón que el hover-link de puriy.
pub(crate) fn status_bar(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;
    let bar = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel);
    match model.hovered_session.and_then(|i| model.sessions.get(i)) {
        Some(s) => {
            let label = match s.number {
                Some(n) => format!("#{n}  {}", s.name),
                None => s.name.clone(),
            };
            bar.text_aligned(label, 12.0, theme.fg_text, Alignment::Center)
        }
        None => bar,
    }
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

/// Selector de **host remoto**: un único select de hosts guardados + un botón
/// que abre el gestor (donde se crean/editan/borran). NO edita host/usuario/
/// puerto sueltos — esos viven sólo en el gestor. Lo comparten el form de
/// creación (sesión pending) y el sidebar (sesión viva), para que no vuelvan a
/// divergir en dos UIs distintas. Elegir un host lo aplica a la sesión activa
/// (y conecta, si ya no es pending) vía `Msg::HostApply`.
fn host_select(model: &Model, session: &Session, theme: &Theme) -> Vec<View<Msg>> {
    let pal = SelectPalette::from_theme(theme);
    let mut out: Vec<View<Msg>> = vec![panel_label("Host", theme)];
    // Etiqueta del host actual: Local o el host remoto elegido.
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

/// Selector de **contenedor**: un único select (rootfs en disco para
/// unshare/bwrap + containers podman) + un botón que abre el gestor (donde se
/// crean/arrancan/paran/borran). Sin select de distro suelto ni botón «crear»
/// fuera del gestor — la distro se elige al crear, dentro del gestor. Compartido
/// por form y sidebar. Elegir uno lo liga a la sesión activa (`PickRootfs` /
/// `SubscribeContainer`).
fn container_picker(model: &Model, session: &Session, theme: &Theme) -> Vec<View<Msg>> {
    let pal = SelectPalette::from_theme(theme);
    let mut out: Vec<View<Msg>> = vec![panel_label("Contenedor", theme)];
    let cont_sel = session.container.as_ref().map(|c| {
        // Basename, no el path completo del rootfs.
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
    // El contenedor pertenece al HOST de la sesión: en Local son los rootfs en
    // disco + podman; en un host remoto, los que devuelve `<engine> ps -a` por
    // SSH (poblados en `RemoteContainersLoaded`).
    let es_local = session.host_key() == "local";
    if model.dropdown_open == Some(DropKind::Container) {
        if !es_local {
            // Host remoto: lista los contenedores descubiertos allá.
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
                if super::rootfs_listo(*distro) {
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

    // Mientras se configura una sesión nueva (pending) el form vive en el
    // canvas grande; el panel sólo recuerda al usuario que está en config.
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

    // Estado de conexión de la sesión (en espera / conectado / desconectado).
    children.push(conn_pill(session.conn, theme));

    // Botón Conectar/Reconectar: rearma el shell con el Source que toca. En una
    // sesión real (no draft) siempre disponible — "Conectar" si está caída,
    // "Reconectar" para forjar un shell fresco aunque esté viva.
    if !es_draft {
        let label = match session.conn {
            ConnState::Connected => "Reconectar",
            _ => "Conectar",
        };
        children.push(action_button_small(label, Msg::ReconnectSession(idx), theme));
    }

    // Host: un único select (Local + remotos guardados) + botón al gestor. El
    // contenedor de abajo pertenece a ESTE host.
    children.extend(host_select(model, session, theme));

    // Contenedor: capa OPCIONAL (encima de Local o Remoto). Mismo selector que
    // el form de creación: toggle + un único select + botón al gestor.
    children.push(container_toggle(session.use_container, theme));
    if session.use_container {
        children.extend(container_picker(model, session, theme));
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
        // Start quedaba pegado arriba.
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

    let mut teeth: Vec<View<Msg>> = model
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
            // Hover: publica el índice para que la barra de estado muestre el
            // nombre de la sesión (el leave lo limpia — patrón de puriy con su
            // hover-link). Independiente del click/drag de abajo.
            .on_pointer_enter(Msg::HoverSession(Some(i)))
            .on_pointer_leave(Msg::HoverSession(None))
            .children(vec![icon, num]);
            if i > 0 {
                // El nodo es draggable → en Released, `draggable_at`
                // toma precedencia sobre `on_click` y el click nunca
                // dispara. `on_click_at` (en press) sí coexiste; ignoro
                // las coords y devuelvo el Msg de selección.
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
    // Botón `+` al final del rail: dispara el form grande en el canvas.
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
        align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
        justify_content: Some(llimphi_ui::llimphi_layout::taffy::JustifyContent::Center),
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
            // Draft: cuadro lleno como Local — es local funcional. La diferencia
            // visible está en la insignia (sin número = scratch, con número = real).
            SessionKind::Draft => {
                let sq = RoundedRect::new(cx - r, cy - r, cx + r, cy + r, 2.5);
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &sq);
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

/// La columna de **historial** del rail derecho: los comandos corridos en la
/// sesión activa (líneas `Prompt` del shell), el más reciente arriba. Repeticiones
/// idénticas se **agrupan** en una sola fila con un contador `×N` (abstrae el
/// ruido de las ráfagas). Click en una fila la **carga** en el input
/// (`Msg::RunFromHistory`, el usuario confirma con Enter); el botón ▶ la
/// **re-ejecuta ya** (`Msg::RunFromHistoryNow`).
fn history_column(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    // Comandos de la sesión activa, en orden de ejecución. Cada `Prompt` tiene
    // la forma "$ <cmd>".
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
    // Agrupar: del más nuevo al más viejo, conservar la primera aparición de
    // cada comando y contar cuántas veces se repitió.
    comandos.reverse();
    let mut grupos: Vec<(String, usize)> = Vec::new();
    for c in comandos {
        if let Some(g) = grupos.iter_mut().find(|(t, _)| *t == c) {
            g.1 += 1;
        } else {
            grupos.push((c, 1));
        }
    }
    grupos.truncate(60); // sin scroll todavía: el tope cabe en pantalla

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
    if grupos.is_empty() {
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
        for (cmd, count) in grupos {
            children.push(history_row(&cmd, count, theme));
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

/// Una fila del historial: `[ comando…  ×N  ▶ ]`. El cuerpo (comando + contador)
/// carga la línea en el input; el botón ▶ la re-ejecuta al instante.
fn history_row(cmd: &str, count: usize, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{prelude::Dimension, AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;

    // Cuerpo clickeable: el comando (flex-grow) + el contador ×N si se repitió.
    let mut cuerpo_hijos: Vec<View<Msg>> = vec![View::new(Style {
        size: Size { width: length(0.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(cmd.to_string(), 12.0, theme.fg_text, Alignment::Start)];
    if count > 1 {
        cuerpo_hijos.push(
            View::new(Style {
                size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                flex_shrink: 0.0,
                margin: Rect { left: length(6.0_f32), right: length(0.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
                ..Default::default()
            })
            .text_aligned(format!("×{count}"), 10.0, theme.fg_muted, Alignment::End),
        );
    }
    let cuerpo = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: length(0.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(cuerpo_hijos)
    .on_click(Msg::RunFromHistory(cmd.to_string()));

    // Botón ▶ — re-ejecuta YA. Click propio (gana sobre el del cuerpo por estar
    // en un nodo hermano, no contenedor).
    let run = View::new(Style {
        size: Size { width: length(22.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .hover_fill(theme.bg_button_hover)
    .text_aligned("▶".to_string(), 10.0, theme.accent, Alignment::Center)
    .on_click(Msg::RunFromHistoryNow(cmd.to_string()));

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        padding: Rect { left: length(12.0_f32), right: length(4.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .children(vec![cuerpo, run])
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
    if session.pending {
        return new_session_form(model, session, theme);
    }
    let idx = model.active_session;
    match &session.shell.state {
        ModuleState::Shell(state) => shuma_module_shell::view::<Msg>(state, theme, move |m| {
            Msg::Module(Slot::Session(idx, Which::Shell), ModuleMsg::Shell(m))
        }),
        _ => placeholder(theme, ""),
    }
}

/// Checkbox "Aislar en contenedor" para el form de creación. Pinta un cuadrito
/// `[x]`/`[ ]` + label, click togglea.
fn container_toggle(on: bool, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
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
            // Tilde blanco.
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
        size: Size {
            width: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
            height: length(20.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned("Aislar con rootfs propio (sin instalar nada)".to_string(), 13.0, fg, Alignment::Start);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        margin: Rect { left: length(0.0_f32), right: length(0.0_f32), top: length(10.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .on_click(Msg::ToggleUseContainer)
    .hover_fill(theme.bg_row_hover)
    .children(vec![box_view, label])
}

/// Botones radio horizontales para el aislamiento. Cada botón fija
/// `session.isolation` al click; el actual queda resaltado con el accent.
fn iso_radio_row(current: Isolation, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;
    let mk = |label: &str, iso: Isolation| {
        let active = iso == current;
        View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
                height: length(32.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(if active { theme.accent } else { theme.bg_button })
        .hover_fill(if active { theme.accent } else { theme.bg_button_hover })
        .radius(4.0)
        .text_aligned(
            label.to_string(),
            12.0,
            if active { theme.bg_app } else { theme.fg_text },
            Alignment::Center,
        )
        .on_click(Msg::SetIsolation(iso))
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        mk("Local", Isolation::Local),
        mk("Remoto", Isolation::Remote),
    ])
}

/// Form grande de creación de sesión, ocupa el canvas mientras `session.pending`.
/// Aislamiento (Local/Remote) + Distro + Mount + opción de container. Confirma
/// con Enter / botón "Crear". Cancela con Esc / "Cancelar".
fn new_session_form(model: &Model, session: &Session, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent};
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

    // Host: un único select (Local + remotos). El contenedor pertenece a él.
    let mut children: Vec<View<Msg>> = vec![titulo, sub];
    children.extend(host_select(model, session, theme));

    // Aislar en contenedor: toggle + el mismo selector único que el sidebar.
    children.push(container_toggle(session.use_container, theme));
    if session.use_container {
        children.extend(container_picker(model, session, theme));
    }

    // Botones: Cancelar | Crear.
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
        margin: Rect { left: length(0.0_f32), right: length(0.0_f32), top: length(16.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![cancelar, crear]);
    children.push(botones);

    // Frame del form: ancho cómodo, alineado a la izquierda con padding generoso.
    let form = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
            height: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
        },
        padding: Rect { left: length(24.0_f32), right: length(24.0_f32), top: length(24.0_f32), bottom: length(24.0_f32) },
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
        padding: Rect { left: length(24.0_f32), right: length(24.0_f32), top: length(24.0_f32), bottom: length(24.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![View::new(Style {
        size: Size {
            width: length(520.0_f32),
            height: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
        },
        ..Default::default()
    })
    .children(vec![form])])
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

// ─── Ventanas secundarias: gestor de containers / hosts ─────────────

/// Diálogo bloqueante de containers (modal centrado, abierto por
/// `Msg::OpenContainersWindow`). Form de alta arriba (engine + distro +
/// mount) + lista de `podman ps -a` con acciones por fila (start/stop/rm).
/// La lista se carga al abrir y tras cada acción.
pub(crate) fn containers_modal(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_widget_modal::{modal_view, ModalButton, ModalPalette, ModalSpec};
    modal_view(ModalSpec {
        title: "Containers".to_string(),
        body: containers_modal_body(model, theme),
        buttons: vec![ModalButton::cancel("Listo", Msg::CloseContainersModal)],
        size: (560.0, 600.0),
        viewport: model.viewport,
        // Bloqueante: clic afuera NO cierra (evita perder el draft a medias).
        // Se cierra con «Listo» o Esc.
        on_dismiss: Msg::Noop,
        palette: ModalPalette::from_theme(theme),
    })
}

fn containers_modal_body(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    let sub = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        "Elegí uno de la lista para editarlo, o «Nuevo». Los mounts se aplican al correr.".to_string(),
        11.0, theme.fg_muted, Alignment::Start,
    );

    // "Nuevo" arriba (deselecciona la lista y activa engine/distro) + el editor
    // si hay un draft abierto.
    let nuevo_btn = action_button_small("+ Nuevo", Msg::ContainerDraftNew, theme);
    let editor: Option<View<Msg>> = model.container_draft.as_ref().map(|d| container_draft_form(d, theme));
    let refresh = action_button_small("⟳ Refrescar lista", Msg::RefreshContainersFull, theme);

    // Nombre que se está editando, para remarcar su fila.
    let editing_name: Option<&str> = model
        .container_draft
        .as_ref()
        .and_then(|d| d.editing.as_deref());

    let mut rows: Vec<View<Msg>> = Vec::new();
    if !model.containers_full.is_empty() {
        rows.push(panel_label("Existentes", theme));
        for (i, c) in model.containers_full.iter().enumerate() {
            let selected = editing_name == Some(c.name.as_str());
            rows.push(container_row(i, c, selected, theme));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
        },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        ..Default::default()
    })
    .children({
        let mut all = vec![sub, nuevo_btn];
        if let Some(ed) = editor {
            all.push(ed);
        }
        all.push(refresh);
        all.extend(rows);
        all
    })
}

/// Editor de contenedor: engine + distro (readonly al editar uno existente) +
/// N directorios montados (host → destino, ro/rw). "Guardar" persiste.
fn container_draft_form(d: &ContainerDraft, theme: &Theme) -> View<Msg> {
    use super::MountCol;
    use llimphi_ui::llimphi_layout::taffy::prelude::Dimension;
    use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;
    let tpal = TextInputPalette::from_theme(theme);
    let editing = d.editing.is_some();

    // Radio readonly-aware: al editar, engine/distro quedan fijos (sin click).
    let mk_radio = |label: String, active: bool, msg: Msg| {
        let v = View::new(Style {
            flex_grow: 1.0,
            size: Size { width: Dimension::auto(), height: length(28.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(if active { theme.accent } else { theme.bg_button })
        .radius(4.0)
        .text_aligned(
            label,
            11.0,
            if active { theme.bg_app } else { theme.fg_muted },
            Alignment::Center,
        );
        if editing {
            v
        } else {
            v.hover_fill(if active { theme.accent } else { theme.bg_button_hover })
                .on_click(msg)
        }
    };

    let mut engine_btns: Vec<View<Msg>> = Vec::new();
    for (avail, name) in [
        (super::unshare_disponible(), "unshare"),
        (super::bwrap_disponible(), "bwrap"),
        (super::podman_disponible(), "podman"),
    ] {
        if avail && (!editing || d.engine == name) {
            engine_btns.push(mk_radio(
                name.to_string(),
                d.engine == name,
                Msg::ContainerDraftSetEngine(name.to_string()),
            ));
        }
    }
    if engine_btns.is_empty() {
        engine_btns.push(
            View::new(Style {
                flex_grow: 1.0,
                size: Size { width: Dimension::auto(), height: length(28.0_f32) },
                ..Default::default()
            })
            .text_aligned("—".to_string(), 11.0, theme.fg_muted, Alignment::Center),
        );
    }
    let engine_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(engine_btns);

    let distro_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(
        [Distro::Ubuntu, Distro::Debian, Distro::Alpine, Distro::Arch]
            .into_iter()
            .filter(|dd| !editing || d.distro == *dd)
            .map(|dd| {
                mk_radio(dd.label().to_string(), d.distro == dd, Msg::ContainerDraftSetDistro(dd))
            })
            .collect::<Vec<_>>(),
    );

    // Filas de mount: host → destino + ro/rw + borrar.
    let mut mount_rows: Vec<View<Msg>> = Vec::new();
    for (i, md) in d.mounts.iter().enumerate() {
        let host_in = text_input_view(
            &md.host,
            "/home/usuario/proyecto",
            d.focus == Some((i, MountCol::Host)),
            &tpal,
            Msg::ContainerDraftFocusMount(i, MountCol::Host),
        );
        let arrow = View::new(Style {
            size: Size { width: length(16.0_f32), height: length(28.0_f32) },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .text_aligned("→".to_string(), 12.0, theme.fg_muted, Alignment::Center);
        let tgt_in = text_input_view(
            &md.target,
            "/work",
            d.focus == Some((i, MountCol::Target)),
            &tpal,
            Msg::ContainerDraftFocusMount(i, MountCol::Target),
        );
        let ro_label = if md.readonly { "ro" } else { "rw" };
        let ro_btn = View::new(Style {
            size: Size { width: length(34.0_f32), height: length(28.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(if md.readonly { theme.bg_button } else { theme.accent })
        .hover_fill(theme.bg_button_hover)
        .radius(4.0)
        .text_aligned(
            ro_label.to_string(),
            11.0,
            if md.readonly { theme.fg_text } else { theme.bg_app },
            Alignment::Center,
        )
        .on_click(Msg::ContainerDraftToggleMountRo(i));
        let rm_btn = action_button_small("🗑", Msg::ContainerDraftRemoveMount(i), theme);
        mount_rows.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
                align_items: Some(AlignItems::Center),
                gap: Size { width: length(5.0_f32), height: length(0.0_f32) },
                ..Default::default()
            })
            .children(vec![host_in, arrow, tgt_in, ro_btn, rm_btn]),
        );
    }
    let add_mount = action_button_small("+ agregar directorio", Msg::ContainerDraftAddMount, theme);

    let save_label = if editing { "Guardar (Enter)" } else { "Crear (Enter)" };
    let buttons = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(36.0_f32) },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        margin: Rect { left: length(0.0_f32), right: length(0.0_f32), top: length(10.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        action_button_small(save_label, Msg::ContainerDraftSave, theme),
        action_button_small("Cancelar (Esc)", Msg::ContainerDraftCancel, theme),
    ]);

    let titulo = panel_label(
        if editing { "Editar contenedor" } else { "Nuevo contenedor" },
        theme,
    );
    let host_lbl = if d.host == "local" {
        "Host: Local".to_string()
    } else {
        format!("Host: {}", d.host)
    };
    let mut children = vec![
        titulo,
        panel_note(&host_lbl, theme),
        panel_label("Engine", theme),
        engine_row,
        panel_label("Distro", theme),
        distro_row,
        panel_label("Directorios montados (host → destino)", theme),
    ];
    children.extend(mount_rows);
    children.push(add_mount);
    children.push(buttons);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        padding: Rect { left: length(12.0_f32), right: length(12.0_f32), top: length(12.0_f32), bottom: length(12.0_f32) },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        margin: Rect { left: length(0.0_f32), right: length(0.0_f32), top: length(6.0_f32), bottom: length(6.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(6.0)
    .children(children)
}

fn container_row(idx: usize, c: &ContainerInfo, selected: bool, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    let running = c.status.starts_with("Up");
    let name_view = View::new(Style {
        size: Size { width: length(180.0_f32), height: length(18.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(c.name.clone(), 12.0, theme.fg_text, Alignment::Start);
    let status_view = View::new(Style {
        size: Size {
            width: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
            height: length(18.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        format!("{} · {}", c.status, c.image),
        11.0,
        if running { theme.accent } else { theme.fg_muted },
        Alignment::Start,
    );
    // Rootfs (unshare/bwrap): no hay daemon que arrancar/parar — sólo borrar.
    // Podman: start/stop/rm completos.
    let mut children = vec![name_view, status_view];
    if c.rootfs {
        children.push(action_button_small("🗑", Msg::RemoveRootfs(c.name.clone()), theme));
    } else {
        children.push(action_button_small("▶", Msg::StartContainer(c.name.clone()), theme));
        children.push(action_button_small("■", Msg::StopContainer(c.name.clone()), theme));
        children.push(action_button_small("🗑", Msg::RemoveContainer(c.name.clone()), theme));
    }
    let mut row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(32.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect { left: length(8.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .children(children);
    // Remarcado cuando está seleccionado para editar.
    if selected {
        row = row.fill(theme.bg_panel_alt);
    }
    // Sólo los rootfs se editan (engine/distro + mounts); click en la fila los
    // selecciona. Los botones de acción (🗑/▶/■) consumen su click aparte.
    if c.rootfs {
        row = row.on_click(Msg::ContainerEdit(idx));
    }
    row
}

/// Diálogo bloqueante de hosts (modal centrado, abierto por
/// `Msg::OpenHostsWindow`). Form de alta + lista de hosts guardados en
/// `hosts.json` con borrar por fila.
pub(crate) fn hosts_modal(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_widget_modal::{modal_view, ModalButton, ModalPalette, ModalSpec};
    modal_view(ModalSpec {
        title: "Hosts remotos".to_string(),
        body: hosts_modal_body(model, theme),
        buttons: vec![ModalButton::cancel("Listo", Msg::CloseHostsModal)],
        size: (520.0, 560.0),
        viewport: model.viewport,
        // Bloqueante: clic afuera NO cierra. Se cierra con «Listo» o Esc.
        on_dismiss: Msg::Noop,
        palette: ModalPalette::from_theme(theme),
    })
}

fn hosts_modal_body(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;

    let sub = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        "Se guardan en ~/.config/shuma/hosts.json.".to_string(),
        11.0, theme.fg_muted, Alignment::Start,
    );

    // "Nuevo" arriba (deselecciona la lista) + el editor si hay draft.
    let nuevo_btn = action_button_small("+ Nuevo", Msg::HostDraftStart, theme);
    let editor: Option<View<Msg>> = model.host_draft.as_ref().map(|d| host_draft_form(d, theme));
    let editing_name: Option<&str> = model
        .host_draft
        .as_ref()
        .and_then(|d| d.editing.as_deref());

    let mut rows: Vec<View<Msg>> = Vec::new();
    if !model.hosts.is_empty() {
        rows.push(panel_label("Guardados", theme));
        for (i, h) in model.hosts.iter().enumerate() {
            let selected = editing_name == Some(h.name.as_str());
            rows.push(host_row(i, h, selected, theme));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
        },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        ..Default::default()
    })
    .children({
        let mut all = vec![sub, nuevo_btn];
        if let Some(ed) = editor {
            all.push(ed);
        }
        all.extend(rows);
        all
    })
}

fn host_row(idx: usize, h: &hosts::RemoteHost, selected: bool, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    let display = View::new(Style {
        size: Size {
            width: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
            height: length(18.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        format!("{} · {}", h.display(), h.auth.label()),
        12.0, theme.fg_text, Alignment::Start,
    );
    // Asignar a una sesión se hace en el select del panel, no acá (CRUD puro).
    let rm_btn = action_button_small("🗑", Msg::HostDelete(idx), theme);
    let mut row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect { left: length(8.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .children(vec![display, rm_btn])
    .on_click(Msg::HostEdit(idx));
    if selected {
        row = row.fill(theme.bg_panel_alt);
    }
    row
}

/// Diálogo bloqueante de **disposiciones** (estilo sesiones de tmux): guardar
/// el espacio de trabajo actual con un nombre + lista de las guardadas con
/// Restaurar/Borrar por fila. Abierto por `Msg::OpenLayoutsModal`.
pub(crate) fn layouts_modal(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_widget_modal::{modal_view, ModalButton, ModalPalette, ModalSpec};
    modal_view(ModalSpec {
        title: "Disposiciones".to_string(),
        body: layouts_modal_body(model, theme),
        buttons: vec![ModalButton::cancel("Listo", Msg::CloseLayoutsModal)],
        size: (520.0, 520.0),
        viewport: model.viewport,
        // Bloqueante: clic afuera NO cierra. Se cierra con «Listo» o Esc.
        on_dismiss: Msg::Noop,
        palette: ModalPalette::from_theme(theme),
    })
}

fn layouts_modal_body(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::prelude::Dimension;
    use llimphi_ui::llimphi_text::Alignment;
    let tpal = TextInputPalette::from_theme(theme);

    let sub = View::new(Style {
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        ..Default::default()
    })
    .text_aligned(
        "Una disposición guarda tus sesiones y la geometría de los paneles. Se guardan en ~/.config/shuma/layouts.json.".to_string(),
        11.0, theme.fg_muted, Alignment::Start,
    );

    let name_input = text_input_view(
        &model.layout_name,
        "nombre de la disposición",
        model.layout_name_focused,
        &tpal,
        Msg::LayoutNameFocus,
    );
    let save_btn = action_button_small("Guardar disposición actual", Msg::SaveLayout, theme);

    let mut rows: Vec<View<Msg>> = Vec::new();
    if !model.layouts.is_empty() {
        rows.push(panel_label("Guardadas", theme));
        for (i, l) in model.layouts.iter().enumerate() {
            rows.push(layout_row(i, &l.name, l.sessions.len(), theme));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        ..Default::default()
    })
    .children({
        let mut all = vec![sub, panel_label("Guardar la actual", theme), name_input, save_btn];
        all.extend(rows);
        all
    })
}

/// Una fila de la lista de disposiciones: nombre + N sesiones + Restaurar + 🗑.
fn layout_row(idx: usize, name: &str, n_sessions: usize, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{prelude::Dimension, AlignItems};
    use llimphi_ui::llimphi_text::Alignment;
    let plural = if n_sessions == 1 { "sesión" } else { "sesiones" };
    let display = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(18.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        format!("{name} · {n_sessions} {plural}"),
        12.0, theme.fg_text, Alignment::Start,
    );
    let restore_btn = action_button_small("Restaurar", Msg::RestoreLayout(idx), theme);
    let rm_btn = action_button_small("🗑", Msg::DeleteLayout(idx), theme);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: Rect { left: length(8.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .children(vec![display, restore_btn, rm_btn])
}

fn host_draft_form(d: &HostDraft, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    let tpal = TextInputPalette::from_theme(theme);
    let mut rows: Vec<View<Msg>> = Vec::new();
    rows.push(panel_label(
        if d.editing.is_some() { "Editar host" } else { "Nuevo host" },
        theme,
    ));
    rows.push(panel_label("Nombre", theme));
    rows.push(text_input_view(
        &d.name, "ejemplo",
        d.focused == Some(HostDraftField::Name),
        &tpal,
        Msg::HostDraftFocus(HostDraftField::Name),
    ));
    rows.push(panel_label("Host", theme));
    rows.push(text_input_view(
        &d.host, "1.2.3.4 o ejemplo.com",
        d.focused == Some(HostDraftField::Host),
        &tpal,
        Msg::HostDraftFocus(HostDraftField::Host),
    ));
    rows.push(panel_label("Usuario", theme));
    rows.push(text_input_view(
        &d.user, "root",
        d.focused == Some(HostDraftField::User),
        &tpal,
        Msg::HostDraftFocus(HostDraftField::User),
    ));
    rows.push(panel_label("Puerto", theme));
    rows.push(text_input_view(
        &d.port, "22",
        d.focused == Some(HostDraftField::Port),
        &tpal,
        Msg::HostDraftFocus(HostDraftField::Port),
    ));
    // Toggle de auth.
    let auth_label = if d.use_password { "Contraseña (askpass al conectar)" } else { "Clave PEM" };
    rows.push(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
            align_items: Some(AlignItems::Center),
            padding: Rect { left: length(4.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
            ..Default::default()
        })
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::HostDraftToggleAuth)
        .text_aligned(format!("· Auth: {auth_label} (click cambia)"), 11.0, theme.fg_text, Alignment::Start),
    );
    if !d.use_password {
        rows.push(panel_label("Path PEM", theme));
        rows.push(text_input_view(
            &d.pem_path, "/home/usuario/.ssh/id_rsa",
            d.focused == Some(HostDraftField::Pem),
            &tpal,
            Msg::HostDraftFocus(HostDraftField::Pem),
        ));
    }
    // Botones Guardar/Crear · Cancelar.
    let save_label = if d.editing.is_some() { "Guardar (Enter)" } else { "Crear (Enter)" };
    let save = action_button_small(save_label, Msg::HostDraftSave, theme);
    let cancel = action_button_small("Cancelar (Esc)", Msg::HostDraftCancel, theme);
    let buttons = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(36.0_f32) },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        align_items: Some(AlignItems::Center),
        margin: Rect { left: length(0.0_f32), right: length(0.0_f32), top: length(10.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![save, cancel]);
    rows.push(buttons);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
        },
        padding: Rect { left: length(12.0_f32), right: length(12.0_f32), top: length(12.0_f32), bottom: length(12.0_f32) },
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(6.0)
    .children(rows)
}

/// Fila clickeable de un select expandido inline (form de sesión nueva).
/// El menú flotante del widget select se anclaría mal en el canvas
/// centrado, así que el form expande su lista en flujo, debajo del trigger.
fn pick_row(label: String, msg: Msg, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .hover_fill(theme.bg_row_hover)
    .radius(3.0)
    .text_aligned(label, 11.0, theme.fg_text, llimphi_ui::llimphi_text::Alignment::Start)
    .on_click(msg)
}

/// Columna de `pick_row`s — el cuerpo expandido de un select inline.
fn inline_list(rows: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
        },
        gap: Size { width: length(0.0_f32), height: length(3.0_f32) },
        ..Default::default()
    })
    .children(rows)
}

fn action_button_small(label: &str, msg: Msg, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size {
            width: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect { left: length(10.0_f32), right: length(10.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_button)
    .hover_fill(theme.bg_button_hover)
    .radius(4.0)
    .text_aligned(label.to_string(), 11.0, theme.fg_text, Alignment::Center)
    .on_click(msg)
}
