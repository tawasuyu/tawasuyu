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
//! Opcionalmente un diente lleva un **distintivo** ("bubble number") en su
//! esquina externa: [`dock_rail_view_badged`] toma `make_badge(id)` que
//! devuelve un [`DockBadge`] (chip con número o punto de estado, reusando
//! [`llimphi_widget_badge`]). Como se deriva del `id`, basta cambiar el modelo
//! y re-pintar para que el contador cambie en vivo.
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
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{DragPhase, View};
use llimphi_theme::Theme;

pub use llimphi_widget_badge::BadgeKind;
use llimphi_widget_badge::{count_badge_view, dot_badge_view};

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

/// Distintivo ("bubble number") que se superpone en la esquina externa de un
/// diente — la esquina hacia la que el diente *sobresale*. Reusa el widget
/// [`llimphi_widget_badge`]: un chip con número (`Count`) o un punto de estado
/// (`Dot`). Es `Copy` para encajar en cierres `Fn(u64) -> Option<DockBadge>`
/// sin clonar; usa [`dock_rail_view_badged`] para pintarlos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockBadge {
    /// Chip ovalado con un número adentro (`>= 100` se muestra como "99+").
    Count(u32, BadgeKind),
    /// Punto sólido sin número — "hay algo" / estado de conexión.
    Dot(BadgeKind),
}

impl DockBadge {
    fn view<Msg: Clone + 'static>(self) -> View<Msg> {
        match self {
            DockBadge::Count(n, kind) => count_badge_view(n, kind),
            DockBadge::Dot(kind) => dot_badge_view(kind),
        }
    }
}

/// A qué borde se pega el rail — decide hacia dónde **sobresalen** los dientes.
/// `InnerLeft` (default) es para un sidebar a la **izquierda**: la barra de acento
/// queda a la izquierda y el diente abre hacia la derecha (hacia el centro).
/// `InnerRight` espeja todo para un sidebar a la **derecha**: barra a la derecha,
/// el diente abre hacia la izquierda (también hacia el centro). El uso canónico de
/// cosmos usa el default; las apps con dientes a ambos lados pasan el lado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockRailSide {
    /// Sidebar izquierdo: dientes sobresalen hacia la derecha.
    #[default]
    InnerLeft,
    /// Sidebar derecho: dientes sobresalen hacia la izquierda (espejado).
    InnerRight,
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
    dock_rail_view_side(
        items,
        width,
        DockRailSide::InnerLeft,
        palette,
        make_icon,
        on_activate,
        on_drop,
    )
}

/// Como [`dock_rail_view`] pero eligiendo el [`DockRailSide`]: en `InnerRight` el
/// diente se **espeja** (barra de acento a la derecha, esquinas redondeadas a la
/// izquierda) para un sidebar a la derecha. `dock_rail_view` es el atajo a
/// `InnerLeft`.
pub fn dock_rail_view_side<Msg, FIcon, FAct, FDrop>(
    items: &[DockRailItem],
    width: f32,
    side: DockRailSide,
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
    dock_rail_view_core(
        items,
        width,
        side,
        palette,
        make_icon,
        |_| None,
        on_activate,
        on_drop,
    )
}

/// Como [`dock_rail_view_side`] pero con **distintivos** ("bubble numbers"):
/// `make_badge(id)` devuelve un [`DockBadge`] opcional que se superpone en la
/// esquina externa del diente (la esquina hacia la que sobresale). El badge no
/// captura clics — clickear sobre él sigue activando el diente. Igual que
/// `make_icon`, el badge se deriva del `id`, así que basta variar el modelo y
/// re-pintar para que el contador cambie en vivo.
#[allow(clippy::too_many_arguments)]
pub fn dock_rail_view_badged<Msg, FIcon, FBadge, FAct, FDrop>(
    items: &[DockRailItem],
    width: f32,
    side: DockRailSide,
    palette: &DockRailPalette,
    make_icon: FIcon,
    make_badge: FBadge,
    on_activate: FAct,
    on_drop: FDrop,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FIcon: Fn(u64, f32, Color) -> View<Msg>,
    FBadge: Fn(u64) -> Option<DockBadge>,
    FAct: Fn(u64) -> Msg,
    FDrop: Fn(u64) -> Option<Msg> + Send + Sync + 'static,
{
    dock_rail_view_core(
        items,
        width,
        side,
        palette,
        make_icon,
        make_badge,
        on_activate,
        on_drop,
    )
}

#[allow(clippy::too_many_arguments)]
fn dock_rail_view_core<Msg, FIcon, FBadge, FAct, FDrop>(
    items: &[DockRailItem],
    width: f32,
    side: DockRailSide,
    palette: &DockRailPalette,
    make_icon: FIcon,
    make_badge: FBadge,
    on_activate: FAct,
    on_drop: FDrop,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FIcon: Fn(u64, f32, Color) -> View<Msg>,
    FBadge: Fn(u64) -> Option<DockBadge>,
    FAct: Fn(u64) -> Msg,
    FDrop: Fn(u64) -> Option<Msg> + Send + Sync + 'static,
{
    let mirror = side == DockRailSide::InnerRight;
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
        // En un sidebar derecho (mirror) la barra de acento va a la DERECHA y el
        // icono a la izquierda → el diente sobresale hacia el centro (a la izq.).
        let mut tooth_children = if mirror {
            vec![icon_box, accent_bar]
        } else {
            vec![accent_bar, icon_box]
        };
        // Distintivo opcional, superpuesto en la esquina externa superior — la
        // esquina hacia la que el diente sobresale (derecha en InnerLeft, izquierda
        // en InnerRight). Va `Absolute` para flotar sobre el icono sin empujarlo.
        if let Some(badge) = make_badge(id) {
            let inset = if mirror {
                Rect {
                    top: length(2.0_f32),
                    left: length(2.0_f32),
                    right: auto(),
                    bottom: auto(),
                }
            } else {
                Rect {
                    top: length(2.0_f32),
                    right: length(2.0_f32),
                    left: auto(),
                    bottom: auto(),
                }
            };
            tooth_children.push(
                View::new(Style {
                    position: Position::Absolute,
                    inset,
                    ..Default::default()
                })
                .children(vec![badge.view::<Msg>()]),
            );
        }
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
        .children(tooth_children);
        if item.active {
            // Pestaña activa: además del fill, redondea el lado que sobresale hacia
            // el contenido. La barra de acento marca el borde interno; el diente abre
            // hacia el lado opuesto (derecha en InnerLeft, izquierda en InnerRight) →
            // look de pestaña que sale del rail, no un rectángulo plano.
            let corners = if mirror {
                (8.0, 0.0, 0.0, 8.0)
            } else {
                (0.0, 8.0, 8.0, 0.0)
            };
            tooth = tooth
                .fill(palette.bg_active)
                .radius_corners(corners.0, corners.1, corners.2, corners.3);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> DockRailPalette {
        DockRailPalette {
            bg_rail: Color::BLACK,
            bg_active: Color::BLACK,
            bg_hover: Color::BLACK,
            accent: Color::WHITE,
            icon_inactive: Color::WHITE,
        }
    }

    fn empty_icon(_id: u64, _size: f32, _color: Color) -> View<i32> {
        View::new(Style::default())
    }

    /// Un diente con badge gana exactamente un hijo extra (el overlay absoluto)
    /// frente a uno sin badge — certifica que el distintivo se inserta en el árbol.
    #[test]
    fn badge_agrega_hijo_overlay() {
        let items = [
            DockRailItem { id: 0, active: false }, // con badge
            DockRailItem { id: 1, active: false }, // sin badge
        ];
        let rail = dock_rail_view_badged(
            &items,
            44.0,
            DockRailSide::InnerLeft,
            &palette(),
            empty_icon,
            |id| (id == 0).then_some(DockBadge::Count(5, BadgeKind::Error)),
            |_id| 0i32,
            |_p| None,
        );
        // rail.children = dientes; cada diente lleva [accent_bar, icon_box] (+ badge).
        let teeth = &rail.children;
        assert_eq!(teeth.len(), 2);
        assert_eq!(teeth[0].children.len(), 3, "diente con badge: +1 hijo");
        assert_eq!(teeth[1].children.len(), 2, "diente sin badge: solo barra+icono");
    }

    /// Sin `make_badge` (las funciones públicas viejas), ningún diente lleva badge.
    #[test]
    fn sin_badge_dos_hijos() {
        let items = [DockRailItem { id: 0, active: true }];
        let rail = dock_rail_view(&items, 44.0, &palette(), empty_icon, |_id| 0i32, |_p| None);
        assert_eq!(rail.children[0].children.len(), 2);
    }

    /// En `InnerRight` el badge se ancla a la izquierda (esquina externa espejada).
    #[test]
    fn badge_espeja_inset_en_inner_right() {
        use llimphi_ui::llimphi_layout::taffy::style_helpers::TaffyAuto;
        let items = [DockRailItem { id: 0, active: false }];
        let rail = dock_rail_view_badged(
            &items,
            44.0,
            DockRailSide::InnerRight,
            &palette(),
            empty_icon,
            |_id| Some(DockBadge::Dot(BadgeKind::Success)),
            |_id| 0i32,
            |_p| None,
        );
        let badge_overlay = rail.children[0].children.last().expect("overlay presente");
        // Espejo: left fijo, right auto.
        assert_ne!(badge_overlay.style.inset.left, TaffyAuto::AUTO);
        assert_eq!(badge_overlay.style.inset.right, TaffyAuto::AUTO);
    }
}
