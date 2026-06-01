//! `llimphi-widget-context-menu` — menú contextual con look gioser.
//!
//! Distintivo y minimalista:
//!
//! ```text
//!   ┃ B5                          ← header (uppercase tiny)
//!   ┃ ✂  Cortar          Ctrl+X
//!   ┃ ⧉  Copiar          Ctrl+C   ← gutter de íconos + barra accent (3px)
//!   ┃ ⎘  Pegar           Ctrl+V
//!   ┃ ─────────────────────
//!   ┃ ◐  Tema             ▸       ← submenú (flyout a la derecha)
//! ```
//!
//! Cada fila: barra accent vertical (firma) · gutter de ícono · label
//! (centrado vertical) · atajo o chevron de submenú. Sin radios, sin
//! sombras: color sólido + tipografía + la barra accent.
//!
//! Se monta como `View<Msg>` que se devuelve desde
//! [`llimphi_ui::App::view_overlay`]. Internamente arma:
//! 1. Un **scrim** full-screen con `on_click = on_dismiss` que cierra
//!    el menú al click-fuera.
//! 2. Un **panel** absoluto (clampeado al viewport).
//! 3. Si hay un submenú abierto ([`ContextMenuSpec::open_sub`]), un
//!    segundo panel-flyout a la derecha del item padre.
//!
//! Animación: [`ContextMenuSpec::appear`] (0..1) controla un fade + un
//! leve desplazamiento vertical de entrada. La app que quiera animarlo
//! guarda un `Tween` y lo va subiendo; pasar `1.0` lo muestra fijo.

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_panel::{panel_signature_painter, PanelStyle};

/// Paleta del menú — estilo "webpage" elegante derivado del theme:
/// panel redondeado con borde hairline, filas como píldoras con hover
/// suave (`bg_hover`) y resaltado de teclado (`bg_active`, más un
/// indicador accent a la izquierda). Defaults dark; override por la app.
#[derive(Debug, Clone, Copy)]
pub struct ContextMenuPalette {
    pub bg_panel: Color,
    /// Fila bajo el cursor (hover) — tinte suave.
    pub bg_hover: Color,
    /// Fila activa por teclado (flechas) — algo más marcado que el hover.
    pub bg_active: Color,
    pub fg_text: Color,
    /// Texto de la fila activa/hover (legible sobre el tinte suave).
    pub fg_active: Color,
    pub fg_shortcut: Color,
    pub fg_disabled: Color,
    pub fg_destructive: Color,
    pub fg_header: Color,
    /// Ícono en gutter (estado normal) — algo más apagado que el texto.
    pub fg_icon: Color,
    pub accent: Color,
    pub border: Color,
    pub separator: Color,
    pub scrim: Color,
    /// Radio de las esquinas del panel.
    pub radius: f64,
    pub panel: PanelStyle,
}

impl ContextMenuPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        // El panel se eleva sobre el fondo: usa `bg_panel` (no `bg_app`)
        // con su gradiente sutil + esquinas redondeadas.
        let mut panel = PanelStyle::neutral(t);
        panel.bg_base = t.bg_panel;
        panel.radius = PANEL_RADIUS as f64;
        Self {
            bg_panel: t.bg_panel,
            bg_hover: t.bg_row_hover,
            bg_active: t.bg_selected,
            fg_text: t.fg_text,
            fg_active: t.fg_text,
            fg_shortcut: t.fg_muted,
            fg_disabled: t.fg_muted,
            fg_destructive: t.fg_destructive,
            fg_header: t.fg_muted,
            fg_icon: t.fg_muted,
            accent: t.accent,
            border: t.border,
            separator: t.border,
            scrim: Color::from_rgba8(0, 0, 0, 64),
            radius: PANEL_RADIUS as f64,
            panel,
        }
    }
}

/// Un item del menú. `separator = true` ignora el resto y pinta una
/// línea. `children` no vacío → es un submenú (muestra chevron ▸ y, si
/// está abierto, despliega un flyout). `icon` es un glifo opcional que
/// se pinta en el gutter izquierdo.
#[derive(Debug, Clone)]
pub struct ContextMenuItem {
    pub label: String,
    pub shortcut: Option<String>,
    pub icon: Option<String>,
    pub enabled: bool,
    pub separator: bool,
    pub destructive: bool,
    /// Items del submenú. Vacío = acción simple.
    pub children: Vec<ContextMenuItem>,
}

impl ContextMenuItem {
    pub fn action(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            shortcut: None,
            icon: None,
            enabled: true,
            separator: false,
            destructive: false,
            children: Vec::new(),
        }
    }

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Glifo del gutter izquierdo (unicode; no acopla a `llimphi-icons`).
    pub fn icon(mut self, glyph: impl Into<String>) -> Self {
        self.icon = Some(glyph.into());
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// Convierte el item en submenú con estos hijos.
    pub fn submenu(mut self, children: Vec<ContextMenuItem>) -> Self {
        self.children = children;
        self
    }

    pub fn has_submenu(&self) -> bool {
        !self.children.is_empty()
    }

    pub fn separator() -> Self {
        Self {
            label: String::new(),
            shortcut: None,
            icon: None,
            enabled: false,
            separator: true,
            destructive: false,
            children: Vec::new(),
        }
    }
}

/// Especificación del menú. Mantiene los 8 campos clásicos para no
/// romper los call-sites por literal; las capacidades nuevas (submenús,
/// animación, hover) viajan aparte en [`ContextMenuExtras`] vía
/// [`context_menu_view_ex`].
pub struct ContextMenuSpec<Msg: Clone + 'static> {
    pub anchor: (f32, f32),
    pub viewport: (f32, f32),
    pub header: Option<String>,
    pub items: Vec<ContextMenuItem>,
    /// Índice resaltado por keyboard. `usize::MAX` = ninguno.
    pub active: usize,
    /// Click en un item de nivel raíz (índice).
    pub on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync>,
    /// Msg al click-fuera (scrim) o Esc.
    pub on_dismiss: Msg,
    pub palette: ContextMenuPalette,
}

/// Capacidades extra opcionales para [`context_menu_view_ex`]: submenús
/// (flyout), animación de aparición y hover. Su `Default` reproduce el
/// menú clásico (sin animación ni submenús).
pub struct ContextMenuExtras<Msg: Clone + 'static> {
    /// Índice del item-submenú desplegado (flyout). La app lo guarda y lo
    /// actualiza vía `on_hover`.
    pub open_sub: Option<usize>,
    /// Progreso de aparición 0..1 (fade + leve slide). `1.0` = fijo.
    pub appear: f32,
    /// Click en un item de submenú: `(parent_idx, child_idx)`.
    pub on_pick_sub: Option<Arc<dyn Fn(usize, usize) -> Msg + Send + Sync>>,
    /// Hover sobre un item raíz: `Some(idx)` si es submenú (abrir flyout),
    /// `None` si es item normal (cerrar). La app guarda el resultado en
    /// `open_sub`.
    pub on_hover: Option<Arc<dyn Fn(Option<usize>) -> Msg + Send + Sync>>,
}

impl<Msg: Clone + 'static> Default for ContextMenuExtras<Msg> {
    fn default() -> Self {
        Self {
            open_sub: None,
            appear: 1.0,
            on_pick_sub: None,
            on_hover: None,
        }
    }
}

const PANEL_W: f32 = 252.0;
/// Altura de cada item (no-separator).
const ITEM_H: f32 = 32.0;
const SEP_H: f32 = 11.0;
const HEADER_H: f32 = 26.0;
/// Gutter del ícono a la izquierda del label.
const ICON_W: f32 = 24.0;
const ITEM_PAD_LEFT: f32 = 10.0;
const ITEM_PAD_RIGHT: f32 = 12.0;
/// Radio de las esquinas del panel (estilo webpage).
const PANEL_RADIUS: f32 = 10.0;
/// Radio de la píldora de hover/activo de cada fila.
const ITEM_RADIUS: f32 = 6.0;
/// Padding interno del panel (entre el borde y la columna de píldoras).
const PANEL_PAD: f32 = 6.0;
/// Ancho del indicador accent vertical de la fila activa.
const INDICATOR_W: f32 = 3.0;
/// Desplazamiento vertical de entrada (px) cuando `appear` = 0.
const APPEAR_SLIDE: f32 = 8.0;

/// Compone el menú clásico (sin submenús ni animación) como `View<Msg>`
/// para `App::view_overlay`. Íconos, centrado vertical y separadores ya
/// vienen incluidos.
pub fn context_menu_view<Msg: Clone + 'static>(spec: ContextMenuSpec<Msg>) -> View<Msg> {
    context_menu_view_ex(spec, ContextMenuExtras::default())
}

/// Como [`context_menu_view`] pero con [`ContextMenuExtras`]: submenús
/// (flyout en hover), animación de aparición y hover.
pub fn context_menu_view_ex<Msg: Clone + 'static>(
    spec: ContextMenuSpec<Msg>,
    extras: ContextMenuExtras<Msg>,
) -> View<Msg> {
    let ContextMenuSpec {
        anchor,
        viewport,
        header,
        items,
        active,
        on_pick,
        on_dismiss,
        palette,
    } = spec;
    let ContextMenuExtras {
        open_sub,
        appear,
        on_pick_sub,
        on_hover,
    } = extras;

    let appear = appear.clamp(0.0, 1.0);
    let slide = (1.0 - appear) * APPEAR_SLIDE;

    let (panel, panel_x, panel_y) = panel_view(
        anchor,
        viewport,
        &header,
        &items,
        active,
        slide,
        &on_pick,
        on_hover.as_ref(),
        &palette,
    );

    let mut layers: Vec<View<Msg>> = vec![panel];

    // Flyout del submenú abierto (sólo si la app provee `on_pick_sub`).
    if let (Some(pidx), Some(on_pick_sub)) = (open_sub, on_pick_sub.as_ref()) {
        if let Some(parent) = items.get(pidx).filter(|it| it.has_submenu()) {
            let sub_anchor = submenu_anchor(panel_x, panel_y, &header, &items, pidx);
            let flyout = submenu_view(
                sub_anchor,
                viewport,
                pidx,
                &parent.children,
                slide,
                on_pick_sub,
                &palette,
            );
            layers.push(flyout);
        }
    }

    // Scrim full-screen: cualquier click "fuera" dismissa.
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.scrim)
    .alpha(appear)
    .on_click(on_dismiss)
    .children(layers)
}

/// Arma el panel raíz y devuelve `(view, x, y)` ya clampeados.
#[allow(clippy::too_many_arguments)]
fn panel_view<Msg: Clone + 'static>(
    anchor: (f32, f32),
    viewport: (f32, f32),
    header: &Option<String>,
    items: &[ContextMenuItem],
    active: usize,
    slide: f32,
    on_pick: &Arc<dyn Fn(usize) -> Msg + Send + Sync>,
    on_hover: Option<&Arc<dyn Fn(Option<usize>) -> Msg + Send + Sync>>,
    palette: &ContextMenuPalette,
) -> (View<Msg>, f32, f32) {
    let header_h = if header.is_some() { HEADER_H } else { 0.0 };
    let items_h: f32 = items
        .iter()
        .map(|it| if it.separator { SEP_H } else { ITEM_H })
        .sum();
    // borde (1+1) + padding interno (PANEL_PAD ×2) + header + items.
    let panel_h = 2.0 + 2.0 * PANEL_PAD + header_h + items_h;

    let margin = 4.0;
    let x = anchor
        .0
        .min((viewport.0 - PANEL_W - margin).max(margin))
        .max(margin);
    let y = anchor
        .1
        .min((viewport.1 - panel_h - margin).max(margin))
        .max(margin);

    let mut children: Vec<View<Msg>> = Vec::with_capacity(items.len() + 1);
    if let Some(text) = header {
        children.push(header_view(text.clone(), palette));
    }
    for (i, item) in items.iter().enumerate() {
        children.push(item_view(
            i,
            None,
            item,
            i == active,
            on_pick,
            on_hover,
            palette,
        ));
    }

    let panel = panel_container(x, y + slide, panel_h, children, palette);
    (panel, x, y)
}

/// Flyout del submenú: mismo look, posicionado a la derecha del padre.
#[allow(clippy::too_many_arguments)]
fn submenu_view<Msg: Clone + 'static>(
    anchor: (f32, f32),
    viewport: (f32, f32),
    parent_idx: usize,
    children_items: &[ContextMenuItem],
    slide: f32,
    on_pick_sub: &Arc<dyn Fn(usize, usize) -> Msg + Send + Sync>,
    palette: &ContextMenuPalette,
) -> View<Msg> {
    let panel_h: f32 = children_items
        .iter()
        .map(|it| if it.separator { SEP_H } else { ITEM_H })
        .sum::<f32>()
        + 2.0
        + 2.0 * PANEL_PAD;
    let margin = 4.0;
    let x = anchor
        .0
        .min((viewport.0 - PANEL_W - margin).max(margin))
        .max(margin);
    let y = anchor
        .1
        .min((viewport.1 - panel_h - margin).max(margin))
        .max(margin);

    let mut children: Vec<View<Msg>> = Vec::with_capacity(children_items.len());
    for (j, item) in children_items.iter().enumerate() {
        children.push(item_view(
            j,
            Some((parent_idx, on_pick_sub.clone())),
            item,
            false,
            // on_pick raíz no se usa cuando hay parent; pasamos un dummy.
            &dummy_pick(),
            None,
            palette,
        ));
    }
    panel_container(x, y + slide, panel_h, children, palette)
}

/// El contenedor visual: panel redondeado con borde hairline (un nodo
/// exterior del color del borde + uno interior con el gradiente del
/// PanelStyle) y padding interno para que las píldoras de cada fila
/// queden inset — el look de menú de webpage.
fn panel_container<Msg: Clone + 'static>(
    x: f32,
    y: f32,
    panel_h: f32,
    children: Vec<View<Msg>>,
    palette: &ContextMenuPalette,
) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(PANEL_W),
            height: length(panel_h),
        },
        padding: Rect {
            left: length(1.0_f32),
            right: length(1.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.border)
    .radius(palette.radius as f64)
    .children(vec![View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(PANEL_PAD),
            right: length(PANEL_PAD),
            top: length(PANEL_PAD),
            bottom: length(PANEL_PAD),
        },
        ..Default::default()
    })
    .radius((palette.radius - 1.0) as f64)
    .paint_with(panel_signature_painter(palette.panel))
    .children(children)])
}

/// Ancla del flyout: a la derecha del panel padre, alineado al item.
fn submenu_anchor(
    panel_x: f32,
    panel_y: f32,
    header: &Option<String>,
    items: &[ContextMenuItem],
    parent_idx: usize,
) -> (f32, f32) {
    let mut off = if header.is_some() { HEADER_H } else { 0.0 };
    off += 1.0 + PANEL_PAD; // borde + padding interno del contenedor
    for it in items.iter().take(parent_idx) {
        off += if it.separator { SEP_H } else { ITEM_H };
    }
    // pequeño solape para que el flyout se lea continuo con el padre.
    (panel_x + PANEL_W - PANEL_PAD, panel_y + off)
}

fn header_view<Msg: Clone + 'static>(text: String, palette: &ContextMenuPalette) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_H),
        },
        padding: Rect {
            left: length(ITEM_PAD_LEFT + INDICATOR_W + ICON_W + 4.0),
            right: length(ITEM_PAD_RIGHT),
            top: length(2.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text.to_uppercase(), 9.5, palette.fg_header, Alignment::Start)
}

/// Pinta una fila. Si `parent` es `Some((pidx, cb))`, es un item de
/// submenú y clickea vía `cb(pidx, idx)`; si es `None`, es raíz y usa
/// `on_pick(idx)` + (si corresponde) `on_hover` para abrir su flyout.
#[allow(clippy::too_many_arguments)]
fn item_view<Msg: Clone + 'static>(
    idx: usize,
    parent: Option<(usize, Arc<dyn Fn(usize, usize) -> Msg + Send + Sync>)>,
    item: &ContextMenuItem,
    is_active: bool,
    on_pick: &Arc<dyn Fn(usize) -> Msg + Send + Sync>,
    on_hover: Option<&Arc<dyn Fn(Option<usize>) -> Msg + Send + Sync>>,
    palette: &ContextMenuPalette,
) -> View<Msg> {
    if item.separator {
        return separator_view(palette);
    }

    // Color del texto y del atajo según estado.
    let (fg, fg_dim): (Color, Color) = if !item.enabled {
        (palette.fg_disabled, palette.fg_disabled)
    } else if item.destructive {
        (palette.fg_destructive, palette.fg_shortcut)
    } else if is_active {
        (palette.fg_active, palette.fg_active)
    } else {
        (palette.fg_text, palette.fg_shortcut)
    };
    // Ícono: accent cuando la fila está activa (cue del menú), si no
    // apagado.
    let icon_fg = if !item.enabled {
        palette.fg_disabled
    } else if is_active {
        palette.accent
    } else {
        palette.fg_icon
    };

    // Indicador accent vertical a la izquierda — visible sólo en la fila
    // activa; reserva su ancho siempre para que el texto no salte.
    let indicator = View::new(Style {
        size: Size {
            width: length(INDICATOR_W),
            height: percent(0.55_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    });
    let indicator = if is_active && item.enabled {
        indicator.fill(palette.accent).radius(2.0)
    } else {
        indicator
    };

    // Gutter de ícono — auto height para que el row lo centre vertical.
    let icon_cell = View::new(Style {
        size: Size {
            width: length(ICON_W),
            height: auto(),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(item.icon.clone().unwrap_or_default(), 13.0, icon_fg, Alignment::Center);

    // Label — auto height (lo centra el align_items Center del row).
    let label = View::new(Style {
        size: Size {
            width: auto(),
            height: auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(item.label.clone(), 12.5, fg, Alignment::Start);

    // Cola: chevron de submenú o atajo de teclado.
    let trailing_text = if item.has_submenu() {
        Some(("\u{203A}".to_string(), fg)) // ›
    } else {
        item.shortcut.clone().map(|s| (s, fg_dim))
    };
    let mut row_children: Vec<View<Msg>> = vec![indicator, icon_cell, label];
    if let Some((txt, color)) = trailing_text {
        row_children.push(
            View::new(Style {
                size: Size {
                    width: length(64.0_f32),
                    height: auto(),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .text_aligned(txt, 11.0, color, Alignment::End),
        );
    }

    let mut row = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(ITEM_H),
        },
        flex_direction: FlexDirection::Row,
        padding: Rect {
            left: length(ITEM_PAD_LEFT),
            right: length(ITEM_PAD_RIGHT),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size {
            width: length(2.0_f32),
            height: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .radius(ITEM_RADIUS as f64)
    .children(row_children);

    // Fondo: píldora suave en activo (teclado). El hover lo aporta
    // `hover_fill` (tinte aún más suave) para no competir con el activo.
    if is_active && item.enabled {
        row = row.fill(palette.bg_active);
    }

    if item.enabled {
        row = row.hover_fill(palette.bg_hover);
        match &parent {
            Some((pidx, cb)) => {
                let cb = cb.clone();
                let pidx = *pidx;
                row = row.on_click_at(move |_, _, _, _| Some(cb(pidx, idx)));
            }
            None => {
                let on_pick = on_pick.clone();
                row = row.on_click_at(move |_, _, _, _| Some(on_pick(idx)));
                // Hover abre/cierra el flyout según sea submenú o no.
                if let Some(on_hover) = on_hover {
                    let on_hover = on_hover.clone();
                    let target = if item.has_submenu() { Some(idx) } else { None };
                    row = row.on_pointer_enter(on_hover(target));
                }
            }
        }
    }
    row
}

fn separator_view<Msg: Clone + 'static>(palette: &ContextMenuPalette) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(SEP_H),
        },
        flex_direction: FlexDirection::Column,
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(ITEM_PAD_LEFT),
            right: length(ITEM_PAD_RIGHT),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.separator)])
}

/// `on_pick` dummy para los items de submenú (que usan `on_pick_sub`).
/// Nunca se invoca: `item_view` con `parent=Some` ignora `on_pick`.
fn dummy_pick<Msg: Clone + 'static>() -> Arc<dyn Fn(usize) -> Msg + Send + Sync> {
    Arc::new(|_| unreachable!("submenu item usa on_pick_sub, no on_pick"))
}

/// Navegación por teclado: dado el activo + dirección (`+1`/`-1`), el
/// siguiente índice válido (saltea separators y disabled). `usize::MAX`
/// si no hay elegibles.
pub fn step_active(items: &[ContextMenuItem], current: usize, direction: i32) -> usize {
    if items.is_empty() {
        return usize::MAX;
    }
    let n = items.len() as i32;
    let start = if current == usize::MAX {
        if direction >= 0 {
            -1
        } else {
            n
        }
    } else {
        current as i32
    };
    let mut i = start;
    for _ in 0..n {
        i += direction;
        if i < 0 {
            i = n - 1;
        } else if i >= n {
            i = 0;
        }
        let item = &items[i as usize];
        if !item.separator && item.enabled {
            return i as usize;
        }
    }
    usize::MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(label: &str) -> ContextMenuItem {
        ContextMenuItem::action(label)
    }

    #[test]
    fn step_active_skips_separators() {
        let items = vec![it("A"), ContextMenuItem::separator(), it("B"), it("C")];
        assert_eq!(step_active(&items, 0, 1), 2);
        assert_eq!(step_active(&items, 2, -1), 0);
    }

    #[test]
    fn step_active_skips_disabled() {
        let items = vec![it("A"), it("B").disabled(), it("C")];
        assert_eq!(step_active(&items, 0, 1), 2);
        assert_eq!(step_active(&items, 2, -1), 0);
    }

    #[test]
    fn step_active_wraps_around() {
        let items = vec![it("A"), it("B"), it("C")];
        assert_eq!(step_active(&items, 2, 1), 0);
        assert_eq!(step_active(&items, 0, -1), 2);
    }

    #[test]
    fn submenu_y_icono_se_setean() {
        let item = it("Tema")
            .icon("◐")
            .submenu(vec![it("Oscuro"), it("Claro")]);
        assert!(item.has_submenu());
        assert_eq!(item.children.len(), 2);
        assert_eq!(item.icon.as_deref(), Some("◐"));
    }

    #[test]
    fn extras_default_es_menu_clasico() {
        let extras: ContextMenuExtras<u8> = ContextMenuExtras::default();
        assert_eq!(extras.appear, 1.0);
        assert!(extras.open_sub.is_none());
        assert!(extras.on_hover.is_none());
        assert!(extras.on_pick_sub.is_none());
    }
}
