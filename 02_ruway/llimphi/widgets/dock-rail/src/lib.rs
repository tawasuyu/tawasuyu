//! `llimphi-widget-dock-rail` — rail vertical de **dientes** para
//! sidebars acoplables.
//!
//! Cada diente es una pestaña vertical: una **barra de acento** de 3px
//! pegada al borde interno (encendida cuando el item está activo) + un
//! **icono** centrado. Los dientes apilan en una columna redondeada que
//! se pinta como una franja flotante al borde del centro — el patrón del
//! dock de cosmos, ahora reutilizable.
//!
//! Render-only y agnóstico del `Msg`: el item se identifica por un `u64`
//! opaco. El clic (en el *press*, para no pelear con el arrastre) activa
//! vía `on_activate(id)`; el diente es **arrastrable** con su `id` como
//! payload, y el rail entero es **drop target** (`on_drop(payload)`) —
//! así una app puede mover un diente de un sidebar a otro soltándolo
//! sobre el rail opuesto. El icono lo dibuja el caller (`make_icon`), que
//! recibe el color ya resuelto según el estado activo/inactivo.
//!
//! ```ignore
//! let items = [DockRailItem { id: 0, active: true }, DockRailItem { id: 1, active: false }];
//! dock_rail_view(
//!     &items,
//!     44.0,
//!     &DockRailPalette::from_theme(&theme),
//!     |id, size, color| my_icon_view(id, size, color),
//!     |id| Msg::DockActivate(side, id),
//!     move |payload| Some(Msg::DockDrop(side, payload)),
//! )
//! ```

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{DragPhase, View};
use llimphi_theme::Theme;

/// Alto de cada diente (px).
const TOOTH_H: f32 = 42.0;
/// Alto de la barra de acento (px) — un poco menor que el diente para
/// dejar aire arriba y abajo.
const BAR_H: f32 = 40.0;
/// Tamaño del icono que se le pide al caller (px).
const ICON_PX: f32 = 20.0;

/// Paleta del rail.
#[derive(Debug, Clone, Copy)]
pub struct DockRailPalette {
    /// Fondo de la franja del rail.
    pub bg_rail: Color,
    /// Fondo del diente activo.
    pub bg_active: Color,
    /// Fondo al hover (diente) y al sobrevolar un drop válido (rail).
    pub bg_hover: Color,
    /// Color del acento: barra del diente activo + su icono.
    pub accent: Color,
    /// Color del icono de un diente inactivo.
    pub icon_inactive: Color,
}

impl DockRailPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            bg_rail: t.bg_panel_alt,
            bg_active: t.bg_selected,
            bg_hover: t.bg_row_hover,
            accent: t.accent,
            icon_inactive: t.fg_muted,
        }
    }
}

/// Un diente del rail: su id opaco y si está activo.
#[derive(Debug, Clone, Copy)]
pub struct DockRailItem {
    pub id: u64,
    pub active: bool,
}

/// Construye el rail de dientes.
///
/// - `items`: en el orden en que se quieren mostrar (el widget no
///   reordena).
/// - `width`: ancho de la franja del rail (px).
/// - `make_icon(id, size, color)`: dibuja el icono del item con el color
///   ya resuelto (acento si activo, atenuado si no).
/// - `on_activate(id)`: `Msg` al clickear (en el press).
/// - `on_drop(payload)`: `Msg` opcional cuando se suelta un diente
///   (cualquier `id`) sobre este rail.
pub fn dock_rail_view<Msg, FIcon, FAct, FDrop>(
    items: &[DockRailItem],
    width: f32,
    palette: &DockRailPalette,
    make_icon: FIcon,
    on_activate: FAct,
    on_drop: FDrop,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FIcon: Fn(u64, f32, Color) -> View<Msg>,
    FAct: Fn(u64) -> Msg,
    FDrop: Fn(u64) -> Option<Msg> + Send + Sync + 'static,
{
    let mut teeth: Vec<View<Msg>> = Vec::with_capacity(items.len());
    for item in items {
        let fg = if item.active {
            palette.accent
        } else {
            palette.icon_inactive
        };
        // Barra de acento, pegada al borde interno.
        let accent_bar = {
            let b = View::new(Style {
                size: Size {
                    width: length(3.0_f32),
                    height: length(BAR_H),
                },
                flex_shrink: 0.0,
                ..Default::default()
            });
            if item.active {
                b.fill(palette.accent).radius(2.0)
            } else {
                b
            }
        };
        let icon_box = View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(0.0_f32),
                height: length(TOOTH_H),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .children(vec![make_icon(item.id, ICON_PX, fg)]);

        let id = item.id;
        let msg = on_activate(id);
        let mut tooth = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(TOOTH_H),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .hover_fill(palette.bg_hover)
        // Click en el press activa; arrastrar mueve de rail (payload=id).
        .on_click_at(move |_, _, _, _| Some(msg.clone()))
        .draggable_at(|phase, _, _, _, _| match phase {
            DragPhase::Move | DragPhase::End => None,
        })
        .drag_payload(id)
        .children(vec![accent_bar, icon_box]);
        if item.active {
            // Pestaña activa: además del fill, redondea el lado que sobresale
            // hacia el contenido (la barra de acento marca el borde interno a
            // la izquierda, así que el diente abre a la derecha). Le da el look
            // de pestaña que sale del rail en vez de un rectángulo plano.
            tooth = tooth
                .fill(palette.bg_active)
                .radius_corners(0.0, 8.0, 8.0, 0.0);
        }
        teeth.push(tooth);
    }

    // La franja: sólo del alto de los dientes (el hueco de abajo lo deja
    // libre para que el área central lo aproveche si el rail flota como
    // overlay). Es además el drop target del lado.
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(width),
            height: auto(),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_rail)
    .radius(5.0)
    .on_drop(on_drop)
    .drop_hover_fill(palette.bg_hover)
    .children(teeth)
}
