//! `llimphi-widget-toolbar` — barra de herramientas moderna.
//!
//! Grupos de botones-ícono planos: hover con fondo redondeado, estado
//! **activo** con fondo de selección + ícono en acento, **deshabilitado**
//! atenuado y sin click. Entre grupos, un separador vertical sutil.
//!
//! El widget es render-only y agnóstico del `Msg`:
//! - los **íconos los dibuja el caller** vía closure `Fn(size, color) ->
//!   View` (mismo contrato que `dock-rail::make_icon`) — el widget resuelve
//!   el color según el estado (activo/normal/deshabilitado);
//! - los **grupos son datos** (`Vec<ToolbarGroup>`): el caller los arma,
//!   reordena o filtra — la barra es componible/configurable por
//!   construcción, sin sistema de config propio.
//!
//! ```ignore
//! let barra = toolbar_view(
//!     vec![
//!         ToolbarGroup::new(vec![
//!             ToolbarItem::new(|s, c| icon_view(Icon::ChevronUp, c, 1.7), Msg::Subir)
//!                 .with_label("subir"),
//!         ]),
//!         ToolbarGroup::new(vec![
//!             ToolbarItem::new(|s, c| icon_view(Icon::Rows, c, 1.7), Msg::Vista(0)).active(lista),
//!             ToolbarItem::new(|s, c| icon_view(Icon::Grid, c, 1.7), Msg::Vista(2)).active(iconos),
//!         ]),
//!     ],
//!     36.0,
//!     &ToolbarPalette::from_theme(&theme),
//! );
//! ```

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;

/// Tamaño del ícono que se le pide al caller (px).
const ICON_PX: f32 = 16.0;

/// Paleta de la barra — subset del `Theme` semántico.
#[derive(Debug, Clone, Copy)]
pub struct ToolbarPalette {
    /// Fondo de la franja.
    pub bg_bar: Color,
    /// Fondo del botón al hover.
    pub bg_hover: Color,
    /// Fondo del botón activo (toggle prendido).
    pub bg_active: Color,
    /// Ícono/label normal.
    pub fg: Color,
    /// Ícono del botón activo.
    pub fg_active: Color,
    /// Ícono/label deshabilitado.
    pub fg_disabled: Color,
    /// Separador vertical entre grupos.
    pub separator: Color,
}

impl ToolbarPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_bar: t.bg_panel_alt,
            bg_hover: t.bg_row_hover,
            bg_active: t.bg_selected,
            fg: t.fg_muted,
            fg_active: t.accent,
            fg_disabled: t.fg_placeholder,
            separator: t.border,
        }
    }
}

impl Default for ToolbarPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

/// Un botón de la barra. El ícono es una closure `(size_px, color) -> View`
/// — el widget la invoca con el color ya resuelto por estado.
pub struct ToolbarItem<Msg> {
    pub icon: Box<dyn Fn(f32, Color) -> View<Msg>>,
    /// Texto corto opcional a la derecha del ícono (la barra es icon-first).
    pub label: Option<String>,
    /// Toggle prendido (fondo de selección + acento).
    pub active: bool,
    /// `false` = atenuado y sin click.
    pub enabled: bool,
    pub on_click: Msg,
}

impl<Msg> ToolbarItem<Msg> {
    pub fn new(
        icon: impl Fn(f32, Color) -> View<Msg> + 'static,
        on_click: Msg,
    ) -> Self {
        Self {
            icon: Box::new(icon),
            label: None,
            active: false,
            enabled: true,
            on_click,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// Un grupo de botones contiguos; entre grupos va un separador.
pub struct ToolbarGroup<Msg> {
    pub items: Vec<ToolbarItem<Msg>>,
}

impl<Msg> ToolbarGroup<Msg> {
    pub fn new(items: Vec<ToolbarItem<Msg>>) -> Self {
        Self { items }
    }
}

/// Compone la barra: franja horizontal de alto `height` con los grupos
/// alineados a la izquierda.
pub fn toolbar_view<Msg: Clone + 'static>(
    groups: Vec<ToolbarGroup<Msg>>,
    height: f32,
    palette: &ToolbarPalette,
) -> View<Msg> {
    let mut kids: Vec<View<Msg>> = Vec::new();
    let n = groups.len();
    for (gi, group) in groups.into_iter().enumerate() {
        for item in group.items {
            kids.push(button(item, height, palette));
        }
        if gi + 1 < n {
            kids.push(separator(height, palette));
        }
    }
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(height),
        },
        flex_shrink: 0.0,
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
    .fill(palette.bg_bar)
    .children(kids)
}

fn button<Msg: Clone + 'static>(
    item: ToolbarItem<Msg>,
    bar_h: f32,
    palette: &ToolbarPalette,
) -> View<Msg> {
    let fg = if !item.enabled {
        palette.fg_disabled
    } else if item.active {
        palette.fg_active
    } else {
        palette.fg
    };
    let mut inner: Vec<View<Msg>> = vec![View::new(Style {
        size: Size {
            width: length(ICON_PX),
            height: length(ICON_PX),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![(item.icon)(ICON_PX, fg)])];
    if let Some(label) = item.label {
        inner.push(
            View::new(Style {
                size: Size { width: auto(), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text(label, 11.5, fg),
        );
    }
    let mut btn = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: length(bar_h - 8.0),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(7.0_f32),
            right: length(7.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size {
            width: length(5.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(6.0)
    .children(inner);
    if item.active {
        btn = btn.fill(palette.bg_active);
    }
    if item.enabled {
        btn = btn.hover_fill(palette.bg_hover).on_click(item.on_click);
    }
    btn
}

fn separator<Msg: Clone + 'static>(bar_h: f32, palette: &ToolbarPalette) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(1.0_f32),
            height: length(bar_h - 16.0),
        },
        flex_shrink: 0.0,
        margin: Rect {
            left: length(5.0_f32),
            right: length(5.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.separator)
}
