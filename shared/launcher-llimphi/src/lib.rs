//! `launcher-llimphi` — el frontend Llimphi del motor de launcher único.
//!
//! Renderiza un [`launcher_core::Surface`] a `View<Msg>`: barras ancladas
//! por borde (estilo eww), la barra de menú global (estilo mac, con
//! dropdown vía `context-menu`), docks (con tear-off) y, para los módulos
//! dinámicos del host (reloj, cpu, widgets propios), un hook
//! [`LauncherSpec::render_module`] que el host inyecta. El lanzamiento real
//! lo resuelve el host contra un `app_bus::Launcher`; este crate sólo emite
//! los `Msg` (no sabe spawnear procesos ni instanciar WASM).
//!
//! Es un widget *sin estado*, al estilo del resto de Llimphi: el `Model`
//! del host lleva qué menú está abierto y la lista de flotantes; el widget
//! aplana la `Surface` en vistas y emite `Msg` en cada interacción.
//!
//! Dos entradas: [`launcher_view`] (el árbol principal, para `App::view`) y
//! [`launcher_overlay`] (el dropdown del menú abierto, para
//! `App::view_overlay`).

#![forbid(unsafe_code)]

pub mod host;

use std::sync::Arc;

use app_bus::{AppMenu, AppRegistry};
use launcher_core::{Bar, Dock, Edge, Module, Surface};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style},
    Rect,
};
use llimphi_icons::app_icons::{app_icon_view, AppIcon};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};

/// Closure que mapea un dato (app_id / command / índice de menú) al `Msg`
/// del host. `Send + Sync` porque el dropdown lo exige (context-menu).
type MsgFromStr<Msg> = Arc<dyn Fn(&str) -> Msg + Send + Sync>;
type MsgFromMenu<Msg> = Arc<dyn Fn(Option<usize>) -> Msg + Send + Sync>;
/// Mapea un índice (de tarjeta flotante) al `Msg` del host.
type MsgFromUsize<Msg> = Arc<dyn Fn(usize) -> Msg + Send + Sync>;
/// Hook del host para módulos dinámicos: `clock`, `cpu`, widgets propios…
type ModuleRenderer<Msg> = Arc<dyn Fn(&Module) -> Option<View<Msg>> + Send + Sync>;

/// Todo lo que el render necesita. El host arma esto en cada `view()`.
pub struct LauncherSpec<'a, Msg: Clone + 'static> {
    pub surface: &'a Surface,
    pub registry: &'a AppRegistry,
    pub theme: &'a Theme,
    /// Tamaño de la ventana (para clampear el dropdown).
    pub viewport: (f32, f32),
    /// Menú global de la app focuseada. `None` = sin foco → la barra de
    /// menú queda vacía (sólo sus módulos `trailing`).
    pub focused_menu: Option<&'a AppMenu>,
    /// Índice del menú raíz abierto (estado del host). `None` = ninguno.
    pub open_menu: Option<usize>,
    /// app_id → Msg (típicamente `BusEvent::LaunchRequested`).
    pub on_launch: MsgFromStr<Msg>,
    /// Abrir/cerrar un menú raíz por índice (`None` = cerrar).
    pub on_open_menu: MsgFromMenu<Msg>,
    /// command id → Msg (lo dispara un ítem del menú global).
    pub on_command: MsgFromStr<Msg>,
    /// app_id → Msg: arrancar un ítem del dock como tarjeta flotante.
    pub on_tear_off: MsgFromStr<Msg>,
    /// índice → Msg: cerrar la tarjeta flotante `n` (la × de su cabecera).
    pub on_close: MsgFromUsize<Msg>,
    /// Render de módulos que este crate no conoce (reloj, cpu, custom).
    pub render_module: ModuleRenderer<Msg>,
}

// =====================================================================
// Paleta
// =====================================================================

fn btn_palette(theme: &Theme) -> ButtonPalette {
    ButtonPalette::from_theme(theme)
}

fn btn_palette_active(theme: &Theme) -> ButtonPalette {
    let base = ButtonPalette::from_theme(theme);
    ButtonPalette {
        bg: theme.accent,
        bg_hover: theme.accent,
        fg: theme.bg_panel,
        radius: base.radius,
    }
}

// =====================================================================
// Vista principal
// =====================================================================

/// El árbol raíz del launcher. Coloca cada barra (y la barra de menú
/// global) en su borde; el centro es un relleno transparente (el launcher
/// suele convivir con el contenido de las apps detrás). Las tarjetas
/// flotantes van como hijos absolutos.
pub fn launcher_view<Msg: Clone + 'static>(spec: &LauncherSpec<Msg>) -> View<Msg> {
    let theme = spec.theme;

    let top = edge_stack(spec, Edge::Top);
    let bottom = edge_stack(spec, Edge::Bottom);
    let left = edge_views(spec, Edge::Left);
    let right = edge_views(spec, Edge::Right);

    // Fila del medio: barras izquierda + relleno que crece + barras derecha.
    let mut middle_children: Vec<View<Msg>> = Vec::new();
    middle_children.extend(left);
    middle_children.push(grow_filler());
    middle_children.extend(right);
    let middle = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        flex_direction: FlexDirection::Row,
        ..Default::default()
    })
    .children(middle_children);

    let mut root_children: Vec<View<Msg>> = Vec::new();
    root_children.extend(top);
    root_children.push(middle);
    root_children.extend(bottom);

    // Tarjetas flotantes (tear-off / conky) como hijos absolutos.
    for (i, card) in spec.surface.floating.iter().enumerate() {
        root_children.push(floating_view(i, card, spec));
    }

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(root_children)
}

/// Las barras de un borde horizontal (top/bottom), incluida la barra de
/// menú global si su borde coincide. Devuelve un Vec para insertar en la
/// columna raíz en orden.
fn edge_stack<Msg: Clone + 'static>(spec: &LauncherSpec<Msg>, edge: Edge) -> Vec<View<Msg>> {
    edge_views(spec, edge)
}

/// Barras (y menú global) ancladas a `edge`.
fn edge_views<Msg: Clone + 'static>(spec: &LauncherSpec<Msg>, edge: Edge) -> Vec<View<Msg>> {
    let mut out: Vec<View<Msg>> = Vec::new();
    // La barra de menú global primero si vive en este borde.
    if let Some(mb) = &spec.surface.app_menu {
        if mb.edge == edge {
            out.push(app_menu_bar_view(spec, mb));
        }
    }
    for bar in spec.surface.bars.iter().filter(|b| b.edge == edge) {
        out.push(bar_view(bar, spec));
    }
    out
}

// =====================================================================
// Barra genérica (slots start/center/end)
// =====================================================================

fn bar_view<Msg: Clone + 'static>(bar: &Bar, spec: &LauncherSpec<Msg>) -> View<Msg> {
    let horizontal = bar.edge.is_horizontal();
    let dir = if horizontal {
        FlexDirection::Row
    } else {
        FlexDirection::Column
    };

    let mut children: Vec<View<Msg>> = Vec::new();
    for m in &bar.start {
        children.push(module_view(m, spec));
    }
    children.push(grow_filler());
    for m in &bar.center {
        children.push(module_view(m, spec));
    }
    children.push(grow_filler());
    for m in &bar.end {
        children.push(module_view(m, spec));
    }

    let size = if horizontal {
        Size {
            width: percent(1.0_f32),
            height: length(bar.thickness),
        }
    } else {
        Size {
            width: length(bar.thickness),
            height: percent(1.0_f32),
        }
    };

    View::new(Style {
        size,
        flex_shrink: 0.0,
        flex_direction: dir,
        align_items: Some(AlignItems::Center),
        padding: pad_xy(bar.padding),
        gap: gap_sz(bar.gap, horizontal),
        ..Default::default()
    })
    .fill(spec.theme.bg_panel)
    .children(children)
}

// =====================================================================
// Módulos
// =====================================================================

fn module_view<Msg: Clone + 'static>(m: &Module, spec: &LauncherSpec<Msg>) -> View<Msg> {
    match m.kind.as_str() {
        "spacer" => grow_filler(),
        "launch" => {
            let app_id = m.str_prop("app_id", "");
            launch_button(app_id, m, spec)
        }
        "dock" => {
            let id = m.str_prop("id", "");
            match spec.surface.dock(id) {
                Some(d) => dock_view(d, spec),
                None => placeholder_chip(&format!("dock?{id}"), spec.theme),
            }
        }
        "app_menu" => menu_titles_view(spec),
        // Todo lo demás (clock/cpu/ram/volume/custom) lo provee el host.
        _ => (spec.render_module)(m).unwrap_or_else(|| placeholder_chip(&m.kind, spec.theme)),
    }
}

/// Botón que lanza una app. Resuelve label/ícono del registro; los props
/// del módulo (`label`, `icon`) los pisan.
fn launch_button<Msg: Clone + 'static>(
    app_id: &str,
    m: &Module,
    spec: &LauncherSpec<Msg>,
) -> View<Msg> {
    let entry = spec.registry.get(app_id);
    let label = m
        .props
        .get("label")
        .and_then(prop_str)
        .map(str::to_string)
        .or_else(|| entry.map(|e| e.label.clone()))
        .unwrap_or_else(|| app_id.to_string());
    let icon = m
        .props
        .get("icon")
        .and_then(prop_str)
        .map(str::to_string)
        .or_else(|| entry.and_then(|e| e.icon.clone()));
    let on_launch = spec.on_launch.clone();
    let id_owned = app_id.to_string();
    app_chip(app_id, label, icon, spec.theme, (on_launch)(&id_owned))
}

/// Chip de lanzamiento: si la app tiene un icono de marca vectorial
/// ([`AppIcon`]), pinta `[glifo en color de marca]  label`; si no, cae al
/// comportamiento clásico (emoji/glyph del registro embebido en el texto).
fn app_chip<Msg: Clone + 'static>(
    app_id: &str,
    label: String,
    glyph: Option<String>,
    theme: &Theme,
    msg: Msg,
) -> View<Msg> {
    match AppIcon::from_app_id(app_id) {
        Some(icon) => {
            let icon_box = View::new(Style {
                size: Size {
                    width: length(16.0_f32),
                    height: length(16.0_f32),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .children(vec![app_icon_view(icon, 1.8)]);
            let text = View::new(Style {
                size: Size {
                    width: auto(),
                    height: auto(),
                },
                ..Default::default()
            })
            .text(label, 12.0, theme.fg_text);
            View::new(chip_row_style())
                .hover_fill(theme.bg_row_hover)
                .radius(6.0)
                .on_click(msg)
                .children(vec![icon_box, text])
        }
        None => {
            let text = match &glyph {
                Some(ic) => format!("{ic} {label}"),
                None => label,
            };
            button_styled(
                text,
                chip_style(),
                Alignment::Center,
                &btn_palette(theme),
                msg,
            )
        }
    }
}

// =====================================================================
// Dock (con tear-off)
// =====================================================================

fn dock_view<Msg: Clone + 'static>(dock: &Dock, spec: &LauncherSpec<Msg>) -> View<Msg> {
    let horizontal = dock.edge.is_horizontal();
    let dir = if horizontal {
        FlexDirection::Row
    } else {
        FlexDirection::Column
    };
    let mut children: Vec<View<Msg>> = Vec::new();
    for e in &dock.entries {
        let entry = spec.registry.get(&e.app_id);
        let label = e
            .label
            .clone()
            .or_else(|| entry.map(|x| x.label.clone()))
            .unwrap_or_else(|| e.app_id.clone());
        let icon = e
            .icon
            .clone()
            .or_else(|| entry.and_then(|x| x.icon.clone()));
        let launch = (spec.on_launch)(&e.app_id);
        let mut item = app_chip(&e.app_id, label, icon, spec.theme, launch);
        // Tear-off: un grip diminuto que arranca el ítem como flotante.
        if dock.tear_off {
            let grip = View::new(Style {
                size: Size {
                    width: length(12.0_f32),
                    height: length(12.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text_aligned("⤢".to_string(), 9.0, spec.theme.fg_muted, Alignment::Center)
            .hover_fill(spec.theme.bg_row_hover)
            .on_click((spec.on_tear_off)(&e.app_id));
            // Apilamos el botón + grip en una mini-columna.
            item = View::new(Style {
                flex_direction: FlexDirection::Column,
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .children(vec![item, grip]);
        }
        children.push(item);
    }

    View::new(Style {
        flex_direction: dir,
        align_items: Some(AlignItems::Center),
        gap: gap_sz(8.0, horizontal),
        ..Default::default()
    })
    .children(children)
}

// =====================================================================
// Barra de menú global (estilo mac)
// =====================================================================

fn app_menu_bar_view<Msg: Clone + 'static>(
    spec: &LauncherSpec<Msg>,
    mb: &launcher_core::AppMenuBar,
) -> View<Msg> {
    let mut children: Vec<View<Msg>> = Vec::new();
    children.push(menu_titles_view(spec));
    children.push(grow_filler());
    for m in &mb.trailing {
        children.push(module_view(m, spec));
    }
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(mb.thickness),
        },
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        padding: pad_xy(8.0),
        gap: gap_sz(4.0, true),
        ..Default::default()
    })
    .fill(spec.theme.bg_panel_alt)
    .children(children)
}

/// La fila de títulos de menú (Archivo / Editar / Ayuda). Click togglea el
/// dropdown vía `on_open_menu`. El abierto se resalta.
fn menu_titles_view<Msg: Clone + 'static>(spec: &LauncherSpec<Msg>) -> View<Msg> {
    let Some(menu) = spec.focused_menu else {
        return View::new(Style {
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            ..Default::default()
        });
    };
    let pal = btn_palette(spec.theme);
    let pal_on = btn_palette_active(spec.theme);
    let mut titles: Vec<View<Msg>> = Vec::with_capacity(menu.menus.len());
    for (i, root) in menu.menus.iter().enumerate() {
        let open = spec.open_menu == Some(i);
        // Toggle: si ya está abierto, cerrar.
        let target = if open { None } else { Some(i) };
        titles.push(button_styled(
            root.label.clone(),
            menu_title_style(),
            Alignment::Center,
            if open { &pal_on } else { &pal },
            (spec.on_open_menu)(target),
        ));
    }
    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: gap_sz(2.0, true),
        ..Default::default()
    })
    .children(titles)
}

/// El dropdown del menú abierto, para `App::view_overlay`. `None` si no hay
/// menú abierto o no hay app focuseada.
pub fn launcher_overlay<Msg: Clone + 'static>(spec: &LauncherSpec<Msg>) -> Option<View<Msg>> {
    let idx = spec.open_menu?;
    let menu = spec.focused_menu?;
    let root = menu.menus.get(idx)?;

    // Ancla aproximada: bajo el título, desplazada por el ancho de los
    // títulos previos. El context-menu clampea al viewport.
    let menu_bar_h = spec
        .surface
        .app_menu
        .as_ref()
        .map(|mb| mb.thickness)
        .unwrap_or(32.0);
    let mut x = 12.0_f32;
    for prev in menu.menus.iter().take(idx) {
        x += approx_title_width(&prev.label);
    }

    // Una sola pasada: `cm_items` (lo que pinta el context-menu) y
    // `commands` (índice → command, `None` en las filas separador) quedan
    // alineados por índice, así el `on_pick(i)` resuelve sin desfase.
    let mut cm_items: Vec<ContextMenuItem> = Vec::new();
    let mut commands: Vec<Option<String>> = Vec::new();
    for (k, src) in root.items.iter().enumerate() {
        if src.separator_before && k != 0 {
            cm_items.push(ContextMenuItem::separator());
            commands.push(None);
        }
        let mut cm = ContextMenuItem::action(src.label.clone());
        if let Some(s) = &src.shortcut {
            cm = cm.with_shortcut(s.clone());
        }
        if !src.enabled {
            cm = cm.disabled();
        }
        cm_items.push(cm);
        commands.push(Some(src.command.clone()));
    }

    let on_command = spec.on_command.clone();
    let on_open_menu = spec.on_open_menu.clone();
    let commands_for_pick = Arc::new(commands);

    let spec_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        match commands_for_pick.get(i).and_then(|c| c.clone()) {
            Some(cmd) => (on_command)(&cmd),
            None => (on_open_menu)(None), // click en separador → cerrar
        }
    });

    let spec_dismiss = (spec.on_open_menu)(None);

    Some(context_menu_view(ContextMenuSpec {
        anchor: (x, menu_bar_h),
        viewport: spec.viewport,
        header: Some(root.label.clone()),
        items: cm_items,
        active: usize::MAX,
        on_pick: spec_pick,
        on_dismiss: spec_dismiss,
        palette: ContextMenuPalette::from_theme(spec.theme),
    }))
}

// =====================================================================
// Flotantes
// =====================================================================

fn floating_view<Msg: Clone + 'static>(
    index: usize,
    card: &launcher_core::FloatingCard,
    spec: &LauncherSpec<Msg>,
) -> View<Msg> {
    let mut children: Vec<View<Msg>> = Vec::new();
    // Cabecera: título (crece) + botón × que cierra la tarjeta.
    let title = View::new(Style {
        size: Size {
            width: auto(),
            height: length(18.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text(card.title.clone().unwrap_or_default(), 11.0, spec.theme.fg_muted);
    let close = View::new(Style {
        size: Size {
            width: length(16.0_f32),
            height: length(16.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned("×".to_string(), 12.0, spec.theme.fg_muted, Alignment::Center)
    .hover_fill(spec.theme.bg_row_hover)
    .radius(4.0)
    .on_click((spec.on_close)(index));
    children.push(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(18.0_f32),
            },
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![title, close]),
    );
    for m in &card.modules {
        children.push(module_view(m, spec));
    }
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(card.x),
            top: length(card.y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(card.w),
            height: length(card.h),
        },
        flex_direction: FlexDirection::Column,
        padding: pad_xy(10.0),
        gap: gap_sz(6.0, false),
        ..Default::default()
    })
    .fill(spec.theme.bg_panel)
    .radius(10.0)
    .children(children)
}

// =====================================================================
// Helpers de estilo
// =====================================================================

fn grow_filler<Msg: Clone + 'static>() -> View<Msg> {
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: auto(),
            height: auto(),
        },
        ..Default::default()
    })
}

fn placeholder_chip<Msg: Clone + 'static>(kind: &str, theme: &Theme) -> View<Msg> {
    View::new(chip_style())
        .fill(theme.bg_panel_alt)
        .radius(4.0)
        .text_aligned(kind.to_string(), 11.0, theme.fg_muted, Alignment::Center)
}

fn chip_style() -> Style {
    Style {
        size: Size {
            width: auto(),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    }
}

/// Chip con icono de marca + label en fila. Mismo alto que [`chip_style`].
fn chip_row_style() -> Style {
    Style {
        size: Size {
            width: auto(),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    }
}

fn menu_title_style() -> Style {
    Style {
        size: Size {
            width: auto(),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    }
}

fn pad_xy(p: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::prelude::LengthPercentage> {
    Rect {
        left: length(p),
        right: length(p),
        top: length(0.0_f32),
        bottom: length(0.0_f32),
    }
}

fn gap_sz(
    g: f32,
    horizontal: bool,
) -> Size<llimphi_ui::llimphi_layout::taffy::prelude::LengthPercentage> {
    if horizontal {
        Size {
            width: length(g),
            height: length(0.0_f32),
        }
    } else {
        Size {
            width: length(0.0_f32),
            height: length(g),
        }
    }
}

fn approx_title_width(label: &str) -> f32 {
    label.chars().count() as f32 * 8.0 + 22.0
}

fn prop_str(p: &launcher_core::Prop) -> Option<&str> {
    match p {
        launcher_core::Prop::Str(s) => Some(s),
        _ => None,
    }
}

// Re-exports cómodos para el host.
pub use app_bus;
pub use launcher_core;
