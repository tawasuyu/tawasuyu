//! Render de topbar, bottombar y área principal del chasis.

use super::super::*;
use super::session::*;
use super::tools::*;
use super::widgets::*;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Style};
use llimphi_ui::llimphi_layout::taffy::{FlexDirection, Size};
use llimphi_ui::llimphi_raster::kurbo::Affine;
use llimphi_ui::{DragPhase, View};
use llimphi_theme::Theme;
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};

/// Ancho del rail de herramientas (derecha) y sesiones (izquierda), en px.
pub(super) const RAIL_W: f32 = 44.0;
pub(super) const SESSION_RAIL_W: f32 = 50.0;

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
        size: Size { width: percent(1.0_f32), height: length(height) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
}

/// Barra de estado inferior cuando no hay módulo CommandBar.
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

/// Área central: si el shumarc declara `[main]`, ese módulo ocupa todo el
/// espacio. Si no, se renderizan las tabs + monitor stack a la derecha.
pub(crate) fn render_main_area(model: &Model, theme: &Theme) -> View<Msg> {
    let body = match &model.main {
        Some(inst) => render_main_full(inst, theme),
        None => render_tabs_with_monitors(model, theme),
    };
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![body])
}

/// Render full-bleed del slot `main` cuando el shumarc lo configura.
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

/// Layout normal: splitter con (panel-sesión | rail-sesión | canvas | rail-tool | panel-tool).
pub(crate) fn render_tabs_with_monitors(model: &Model, theme: &Theme) -> View<Msg> {
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
        // Transición de escena: al cambiar de herramienta (tab) la `scene_key`
        // cambia y el panel entra con fade + slide-up suave, en vez de saltar.
        let scene = Tool::ALL.iter().position(|x| *x == tool).unwrap_or(0) as u64;
        let panel = tool_panel(model, tool, theme).animated_enter_from(
            scene,
            motion::SLOW,
            Affine::translate((0.0, 24.0)),
        );
        core = splitter_two(
            Direction::Row,
            core,
            PaneSize::Flex,
            panel,
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
