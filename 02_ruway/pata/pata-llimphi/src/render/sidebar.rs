//! Render del **sidebar navegador** (Fase 11c): el rail de dientes pegado al
//! borde y, cuando un diente está activo, el panel con el navegador de
//! Mónadas/archivos.
//!
//! - El **rail** reusa [`llimphi_widget_dock_rail`]: una franja vertical con un
//!   diente por `SidebarTab`. El diente activo (su panel desplegado) va
//!   resaltado. Clic → [`Msg::NavTabActivate`].
//! - El **panel** ([`panel_inner`]) lleva un cabezal con el toggle Árbol/Grafo +
//!   el navegador ([`llimphi_widget_navigator`]) dentro de un área de scroll. El
//!   plano de datos lo provee [`crate::nouser`].
//!
//! Dos backends montan estas piezas distinto:
//! - **winit** ([`sidebar_rail_view`] + [`nav_panel_view`]): cada superficie vive
//!   en la ventana única, posicionada en absoluto sobre la pantalla; el panel
//!   flota como un drawer.
//! - **layer-shell** ([`sidebar_surface_view`]): el rail es su propia layer
//!   surface anclada al borde; al abrir un diente la surface **crece** en ancho y
//!   el panel se pinta junto al rail (el eje libre lo estira el compositor).

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style},
    Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;

use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_navigator::{navigator_view, NavKind, NavMode, NavNode, NavPalette, NavSpec};
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};

use std::collections::HashSet;

use pata_core::config::{Anchor, Surface};
use pata_core::layout::Rect;

use crate::nouser::NavState;
use crate::Msg;

/// Alto del cabezal del panel (título + toggle de modo), en px.
const HEADER_H: f32 = 40.0;
/// Padding interno del panel, en px.
const PAD: f32 = 8.0;
/// Alto estimado de una fila del navegador en modo árbol (igual al `ROW_H`
/// interno del widget). Para dimensionar el scroll.
const TREE_ROW_H: f32 = 24.0;
/// Alto estimado de un nodo del navegador en modo grafo (nodo + separación).
const GRAPH_ROW_H: f32 = 60.0;

// =====================================================================
// Piezas compartidas por ambos backends
// =====================================================================

/// El rail de dientes (sin fondo de franja): un diente por `SidebarTab`. `si`
/// identifica la superficie para el `Msg` del clic.
fn rail_widget(surface: &Surface, si: usize, width: f32, nav: &NavState, theme: &Theme) -> View<Msg> {
    let items: Vec<DockRailItem> = surface
        .tabs
        .iter()
        .enumerate()
        .map(|(ti, _)| DockRailItem {
            id: ti as u64,
            active: nav.is_open(si, ti),
        })
        .collect();
    let icons: Vec<String> = surface.tabs.iter().map(|t| t.icon.clone()).collect();
    dock_rail_view(
        &items,
        width,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| {
            let name = icons.get(id as usize).map(|s| s.as_str()).unwrap_or("");
            tooth_icon(name, size, color)
        },
        move |id| Msg::NavTabActivate(si, id as usize),
        // Mover un diente de un rail a otro: Fase futura (drop entre sidebars).
        |_| None,
    )
}

/// Una franja de rail que **llena su alto**: fondo de panel + el rail de dientes
/// arriba. La usan ambos backends (en winit dentro del rect absoluto, en
/// layer-shell como columna de ancho `thickness` dentro de la surface).
fn rail_strip(surface: &Surface, si: usize, thickness: f32, nav: &NavState, theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(thickness),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![rail_widget(surface, si, thickness, nav, theme)])
}

/// El contenido del panel (cabezal con toggle de modo + navegador con scroll),
/// dimensionado para llenar su contenedor de alto `panel_h` px. Trae su propio
/// fondo y padding. `ti` es el diente desplegado.
fn panel_inner(surface: &Surface, ti: usize, panel_h: f32, nav: &NavState, theme: &Theme) -> View<Msg> {
    // --- Cabezal: título del diente + toggle Árbol/Grafo ---
    let titulo = surface
        .tabs
        .get(ti)
        .map(|t| t.label.clone())
        .unwrap_or_default();
    let titulo_view = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: length(HEADER_H),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(titulo, 13.0, theme.fg_text);

    let toggle = View::new(Style {
        size: Size {
            width: length(140.0_f32),
            height: length(HEADER_H),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![segmented_view(
        &NavMode::LABELS,
        nav.mode.index(),
        |i| Msg::NavSetMode(NavMode::from_index(i)),
        &SegmentedPalette::from_theme(theme),
    )]);

    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_H),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![titulo_view, toggle]);

    // --- Cuerpo: navegador (o un aviso si no hay datos) ---
    let viewport = (panel_h - HEADER_H - PAD * 2.0).max(0.0);
    let cuerpo = if nav.roots.is_empty() {
        aviso_view(nav, theme, viewport)
    } else {
        let row_h = match nav.mode {
            NavMode::Tree => TREE_ROW_H,
            NavMode::Graph => GRAPH_ROW_H,
        };
        let visibles = count_visible(&nav.roots, &nav.expanded);
        let content_len = visibles as f32 * row_h + 16.0;
        let offset = clamp_offset(nav.scroll, content_len, viewport);

        let navv = navigator_view(
            NavSpec {
                roots: &nav.roots,
                mode: nav.mode,
                selected: nav.selected,
                palette: NavPalette::from_theme(theme),
                guides: true,
            },
            |id| nav.expanded.contains(&id),
            Msg::NavToggle,
            Msg::NavSelect,
            Some(Msg::NavOpen),
        );

        scroll_y(
            offset,
            content_len,
            viewport,
            navv,
            Msg::NavScroll,
            &ScrollPalette::from_theme(theme),
        )
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: TaffyRect {
            left: length(PAD),
            right: length(PAD),
            top: length(PAD),
            bottom: length(PAD),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(PAD),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![header, cuerpo])
}

// =====================================================================
// Backend winit: superficies en absoluto sobre la ventana única
// =====================================================================

/// El rail de un `SurfaceKind::Sidebar`, posicionado en el rect que el layout
/// reservó para él (path winit). `si` es el índice de la superficie.
pub fn sidebar_rail_view(
    surface: &Surface,
    si: usize,
    rect: Rect,
    nav: &NavState,
    theme: &Theme,
) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(rect.x as f32),
            top: length(rect.y as f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(rect.w as f32),
            height: length(rect.h as f32),
        },
        ..Default::default()
    })
    .children(vec![rail_strip(surface, si, rect.w as f32, nav, theme)])
}

/// El panel flotante del diente `ti` desplegado (path winit): flota junto al
/// `rail_rect` (a su derecha si el sidebar está a la izquierda, a su izquierda si
/// está a la derecha).
pub fn nav_panel_view(
    surface: &Surface,
    ti: usize,
    rail_rect: Rect,
    screen: (i32, i32),
    nav: &NavState,
    theme: &Theme,
) -> View<Msg> {
    let pw = surface.panel_width;
    let (_, sh) = screen;
    let h = (rail_rect.h as f32).min(sh as f32);
    let y = rail_rect.y as f32;
    let x = match surface.anchor {
        Anchor::Right => (rail_rect.x as f32 - pw).max(0.0),
        _ => (rail_rect.x + rail_rect.w) as f32,
    };

    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(pw),
            height: length(h),
        },
        ..Default::default()
    })
    .children(vec![panel_inner(surface, ti, h, nav, theme)])
}

// =====================================================================
// Backend layer-shell: la surface del rail crece para alojar el panel
// =====================================================================

/// La vista que llena la layer surface de un `SurfaceKind::Sidebar` de tamaño
/// `(w, h)` px. Colapsada es sólo el rail (`w == thickness`); con un diente
/// abierto la surface creció a `thickness + panel_width` y se pinta rail + panel
/// (el orden depende del anclaje: el rail siempre pegado a su borde). `si` es el
/// índice de la superficie.
pub fn sidebar_surface_view(
    surface: &Surface,
    si: usize,
    w: f32,
    h: f32,
    nav: &NavState,
    theme: &Theme,
) -> View<Msg> {
    let thickness = surface.thickness;
    let rail = rail_strip(surface, si, thickness, nav, theme);

    let open_ti = match nav.open {
        Some((s, ti)) if s == si => Some(ti),
        _ => None,
    };

    let children = if let Some(ti) = open_ti {
        let pw = (w - thickness).max(0.0);
        let panel = View::new(Style {
            size: Size {
                width: length(pw),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![panel_inner(surface, ti, h, nav, theme)]);
        // El rail va pegado a su borde: a la izquierda del panel si el sidebar
        // está anclado a la izquierda; a la derecha si está a la derecha.
        match surface.anchor {
            Anchor::Right => vec![panel, rail],
            _ => vec![rail, panel],
        }
    } else {
        vec![rail]
    };

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: length(w),
            height: length(h),
        },
        ..Default::default()
    })
    .children(children)
}

// =====================================================================
// Auxiliares
// =====================================================================

/// Un aviso centrado cuando no hay Mónadas que mostrar (conectando, o error).
fn aviso_view(nav: &NavState, theme: &Theme, viewport: f32) -> View<Msg> {
    let (texto, color) = match &nav.error {
        Some(e) => (e.clone(), theme.fg_muted),
        None => ("Conectando con nouser…".to_string(), theme.fg_muted),
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(viewport.max(40.0)),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(texto, 12.0, color)
}

/// Cuenta los nodos visibles del bosque dado el conjunto de expandidos — para
/// dimensionar el alto del contenido del scroll.
fn count_visible(roots: &[NavNode], expanded: &HashSet<u64>) -> usize {
    fn walk(node: &NavNode, expanded: &HashSet<u64>, acc: &mut usize) {
        *acc += 1;
        if node.has_children() && expanded.contains(&node.id) {
            for c in &node.children {
                walk(c, expanded, acc);
            }
        }
    }
    let mut acc = 0;
    for r in roots {
        walk(r, expanded, &mut acc);
    }
    acc
}

/// El icono de un diente del rail: un glifo vectorial según el nombre declarado
/// en el `SidebarTab` (`monads` → diamante, `files` → cuadrado, otro → círculo),
/// con el color que el rail ya resolvió (acento si activo, atenuado si no).
fn tooth_icon(name: &str, size: f32, color: Color) -> View<Msg> {
    // Reusamos la semántica de iconos del navegador para coherencia visual.
    let kind = match name {
        "monads" | "monadas" | "monad" => NavKind::Monad,
        "files" | "archivos" | "file" | "dir" => NavKind::Dir,
        _ => NavKind::Other,
    };
    View::new(Style {
        size: Size {
            width: length(size),
            height: length(size),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Point, RoundedRect};
        use llimphi_ui::llimphi_raster::peniko::Fill;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let r = (rect.w.min(rect.h) as f64 * 0.38).max(2.0);
        match kind {
            NavKind::Monad => {
                let mut p = BezPath::new();
                p.move_to(Point::new(cx, cy - r));
                p.line_to(Point::new(cx + r, cy));
                p.line_to(Point::new(cx, cy + r));
                p.line_to(Point::new(cx - r, cy));
                p.close_path();
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &p);
            }
            NavKind::Group | NavKind::Dir => {
                let sq = RoundedRect::new(cx - r, cy - r, cx + r, cy + r, 2.0);
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &sq);
            }
            NavKind::File | NavKind::Other => {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    color,
                    None,
                    &Circle::new((cx, cy), r * 0.7),
                );
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_widget_navigator::NavNode;

    fn forest() -> Vec<NavNode> {
        vec![
            NavNode::branch(
                1,
                "m1",
                NavKind::Monad,
                vec![NavNode::leaf(11, "a", NavKind::File), NavNode::leaf(12, "b", NavKind::File)],
            ),
            NavNode::leaf(2, "m2", NavKind::Monad),
        ]
    }

    #[test]
    fn count_visible_respeta_expansion() {
        let roots = forest();
        // Colapsado: sólo las 2 raíces.
        let none = HashSet::new();
        assert_eq!(count_visible(&roots, &none), 2);
        // Expandida la primera: 2 raíces + 2 hijos.
        let mut exp = HashSet::new();
        exp.insert(1u64);
        assert_eq!(count_visible(&roots, &exp), 4);
    }
}
