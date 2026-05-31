//! `llimphi-widget-menubar` — la barra de menú principal de una app.
//!
//! Toda app Llimphi declara un [`app_bus::AppMenu`] (Archivo / Editar /
//! Ver / Ayuda …) y lo monta in-window con este widget. Es el gemelo de
//! la barra global de [`launcher_llimphi`], pero vive **dentro** de la
//! ventana de la app — para las apps que corren standalone y no bajo el
//! shell del launcher.
//!
//! Sin estado, al estilo Llimphi: el `Model` del host lleva qué menú raíz
//! está abierto (`Option<usize>`); el widget aplana el `AppMenu` y emite
//! `Msg` en cada interacción.
//!
//! Dos entradas:
//! - [`menubar_view`] → la fila de títulos, para el tope de `App::view`.
//! - [`menubar_overlay`] → el dropdown del menú abierto, para
//!   `App::view_overlay` (devolvé `None` si no hay nada abierto).
//!
//! El `command` de cada ítem es el id que la app entiende (convención
//! `menu.<verbo>`, ver [`app_bus::AppMenu::standard`]); el widget lo
//! rebota por `on_command`.

#![forbid(unsafe_code)]

use std::sync::Arc;

use app_bus::AppMenu;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, FlexDirection, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};

type MsgFromMenu<Msg> = Arc<dyn Fn(Option<usize>) -> Msg + Send + Sync>;
type MsgFromStr<Msg> = Arc<dyn Fn(&str) -> Msg + Send + Sync>;

/// Todo lo que el render necesita. El host lo arma en cada `view()`.
pub struct MenuBarSpec<'a, Msg: Clone + 'static> {
    /// El menú a pintar (típicamente `AppMenu::standard()` + menús propios).
    pub menu: &'a AppMenu,
    /// Índice del menú raíz abierto (estado del host). `None` = ninguno.
    pub open: Option<usize>,
    pub theme: &'a Theme,
    /// Tamaño de la ventana — para clampear el dropdown.
    pub viewport: (f32, f32),
    /// Alto de la barra (px). Usar [`DEFAULT_HEIGHT`] si no hay razón.
    pub height: f32,
    /// Abrir/cerrar un menú raíz por índice (`None` = cerrar).
    pub on_open: MsgFromMenu<Msg>,
    /// command id → Msg, al elegir un ítem.
    pub on_command: MsgFromStr<Msg>,
}

/// Alto recomendado de la barra de menú.
pub const DEFAULT_HEIGHT: f32 = 30.0;

fn title_palette(theme: &Theme) -> ButtonPalette {
    ButtonPalette::from_theme(theme)
}

fn title_palette_active(theme: &Theme) -> ButtonPalette {
    let base = ButtonPalette::from_theme(theme);
    ButtonPalette {
        bg: theme.accent,
        bg_hover: theme.accent,
        fg: theme.bg_panel,
        radius: base.radius,
    }
}

/// La fila de títulos (Archivo / Editar / …). Click sobre un título
/// togglea su dropdown vía `on_open`. El abierto se resalta con el accent.
pub fn menubar_view<Msg: Clone + 'static>(spec: &MenuBarSpec<Msg>) -> View<Msg> {
    let pal = title_palette(spec.theme);
    let pal_on = title_palette_active(spec.theme);

    let mut titles: Vec<View<Msg>> = Vec::with_capacity(spec.menu.menus.len());
    for (i, root) in spec.menu.menus.iter().enumerate() {
        let open = spec.open == Some(i);
        let target = if open { None } else { Some(i) };
        titles.push(button_styled(
            root.label.clone(),
            title_style(),
            Alignment::Center,
            if open { &pal_on } else { &pal },
            (spec.on_open)(target),
        ));
    }

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(spec.height),
        },
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size {
            width: length(2.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(spec.theme.bg_panel_alt)
    .children(titles)
}

/// El dropdown del menú abierto, para `App::view_overlay`. `None` si no
/// hay menú abierto.
pub fn menubar_overlay<Msg: Clone + 'static>(spec: &MenuBarSpec<Msg>) -> Option<View<Msg>> {
    let idx = spec.open?;
    let root = spec.menu.menus.get(idx)?;

    // Ancla: bajo el título, desplazada por el ancho aproximado de los
    // títulos previos (+ el padding izquierdo de la barra). El
    // context-menu clampea al viewport igual.
    let mut x = 6.0_f32;
    for prev in spec.menu.menus.iter().take(idx) {
        x += approx_title_width(&prev.label);
    }

    // Una sola pasada: `items` (lo que pinta el context-menu) y `commands`
    // (índice → command, `None` en separadores) quedan alineados.
    let mut items: Vec<ContextMenuItem> = Vec::new();
    let mut commands: Vec<Option<String>> = Vec::new();
    for (k, src) in root.items.iter().enumerate() {
        if src.separator_before && k != 0 {
            items.push(ContextMenuItem::separator());
            commands.push(None);
        }
        let mut cm = ContextMenuItem::action(src.label.clone());
        if let Some(s) = &src.shortcut {
            cm = cm.with_shortcut(s.clone());
        }
        if !src.enabled {
            cm = cm.disabled();
        }
        items.push(cm);
        commands.push(Some(src.command.clone()));
    }

    let on_command = spec.on_command.clone();
    let on_open = spec.on_open.clone();
    let commands = Arc::new(commands);
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        match commands.get(i).and_then(|c| c.clone()) {
            Some(cmd) => (on_command)(&cmd),
            None => (on_open)(None),
        }
    });

    Some(context_menu_view(ContextMenuSpec {
        anchor: (x, spec.height),
        viewport: spec.viewport,
        header: Some(root.label.clone()),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: (spec.on_open)(None),
        palette: ContextMenuPalette::from_theme(spec.theme),
    }))
}

fn title_style() -> Style {
    Style {
        size: Size {
            width: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
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

/// Ancho aproximado de un título — mismo criterio que `launcher-llimphi`
/// para anclar el dropdown sin medir la fuente.
fn approx_title_width(label: &str) -> f32 {
    label.chars().count() as f32 * 8.0 + 22.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_none_si_no_hay_abierto() {
        let menu = AppMenu::standard();
        let spec = MenuBarSpec {
            menu: &menu,
            open: None,
            theme: &Theme::dark(),
            viewport: (800.0, 600.0),
            height: DEFAULT_HEIGHT,
            on_open: Arc::new(|_| 0u8),
            on_command: Arc::new(|_| 1u8),
        };
        assert!(menubar_overlay(&spec).is_none());
    }

    #[test]
    fn overlay_some_si_hay_abierto() {
        let menu = AppMenu::standard();
        let spec = MenuBarSpec {
            menu: &menu,
            open: Some(0),
            theme: &Theme::dark(),
            viewport: (800.0, 600.0),
            height: DEFAULT_HEIGHT,
            on_open: Arc::new(|_| 0u8),
            on_command: Arc::new(|_| 1u8),
        };
        assert!(menubar_overlay(&spec).is_some());
    }
}
