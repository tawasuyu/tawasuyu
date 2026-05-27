//! `llimphi-widget-context-menu` — menú contextual con look gioser.
//!
//! Distintivo y minimalista:
//!
//! ```text
//!   ┃ B5                          ← header (uppercase tiny)
//!   ┃ Copiar          Ctrl+C
//!   ┃ Cortar          Ctrl+X
//!   ┃ Pegar           Ctrl+V        ← barra accent (3px) toda la altura
//!   ┃ ─────────────────────
//!   ┃ Limpiar         Del
//! ```
//!
//! Sin radios, sin sombras, sin gradientes. Color sólido + tipografía
//! + una barra vertical accent a la izquierda que recorre toda la
//! altura del panel — la firma visual del control.
//!
//! Se monta como `View<Msg>` que se devuelve desde
//! [`llimphi_ui::App::view_overlay`]. Internamente arma:
//! 1. Un **scrim** full-screen con `on_click = on_dismiss` que cierra
//!    el menú al hacer click fuera.
//! 2. Un **panel** posicionado de forma absoluta (clampeado al
//!    viewport para no overflowear).
//!
//! Los items son `on_click` clásicos; la app es responsable de
//! cerrar el menú dentro del handler de cada Msg (o dejar que el
//! click viaje y el modelo lo cierre como side-effect). Soporte de
//! keyboard navigation (flechas + Enter) se maneja desde el `on_key`
//! de la app, leyendo `spec.active` y emitiendo el Msg correspondiente.

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Paleta del menú. Pensada para una app dark; los defaults usan
/// negro + blanco + un acento (override por la app). Las apps con
/// tema claro deberían armarse su propia paleta a mano — el menú
/// queda elegante en cualquier fondo siempre que `bg_panel` contraste
/// con `accent`.
#[derive(Debug, Clone, Copy)]
pub struct ContextMenuPalette {
    /// Fondo del panel.
    pub bg_panel: Color,
    /// Fila activa (resaltada por hover o por keyboard nav).
    pub bg_active: Color,
    /// Texto del label en una fila normal.
    pub fg_text: Color,
    /// Texto del label en la fila activa (suele ser oscuro porque la
    /// fila se pinta del color accent).
    pub fg_active: Color,
    /// Texto del atajo de teclado a la derecha — más apagado.
    pub fg_shortcut: Color,
    /// Texto de un item deshabilitado.
    pub fg_disabled: Color,
    /// Texto de un item destructivo (eliminar, borrar, etc).
    pub fg_destructive: Color,
    /// Texto del header (caption uppercase muy pequeño arriba).
    pub fg_header: Color,
    /// Color de la barra accent vertical (3px a la izquierda) — la
    /// firma visual del control.
    pub accent: Color,
    /// Línea de 1px alrededor del panel (top/right/bottom; la
    /// izquierda la cubre la barra accent).
    pub border: Color,
    /// Línea horizontal de separación entre grupos de items.
    pub separator: Color,
    /// Tinte semi-transparente que cubre el fondo del app mientras
    /// el menú está abierto. Usualmente RGBA con alpha bajo (~0.25)
    /// para apagar suavemente el árbol principal sin ocultarlo.
    pub scrim: Color,
}

impl ContextMenuPalette {
    /// Paleta desde un [`llimphi_theme::Theme`]. La barra accent toma
    /// `theme.accent`; el fondo del panel es `bg_panel` un grado más
    /// oscuro; el texto invertido en la fila activa asume que el
    /// accent es claro respecto al panel.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_panel: t.bg_app,
            bg_active: t.accent,
            fg_text: t.fg_text,
            fg_active: t.bg_app,
            fg_shortcut: t.fg_muted,
            fg_disabled: t.fg_muted,
            fg_destructive: t.fg_destructive,
            fg_header: t.fg_muted,
            accent: t.accent,
            border: t.border,
            separator: t.border,
            scrim: Color::from_rgba8(0, 0, 0, 64),
        }
    }
}

/// Un item del menú. Si `separator` es `true`, los demás campos se
/// ignoran y la fila se renderiza como una línea horizontal. Items
/// `enabled = false` se pintan a media tinta y NO son clicables —
/// útil para "Pegar" cuando no hay clipboard, "Undo" cuando no hay
/// historial, etc.
#[derive(Debug, Clone)]
pub struct ContextMenuItem {
    pub label: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub separator: bool,
    pub destructive: bool,
}

impl ContextMenuItem {
    pub fn action(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            shortcut: None,
            enabled: true,
            separator: false,
            destructive: false,
        }
    }

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
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

    pub fn separator() -> Self {
        Self {
            label: String::new(),
            shortcut: None,
            enabled: false,
            separator: true,
            destructive: false,
        }
    }
}

/// Especificación completa del menú. La app la construye y devuelve
/// `context_menu_view(spec)` desde `App::view_overlay`.
pub struct ContextMenuSpec<Msg: Clone + 'static> {
    /// Esquina top-left deseada del panel, en coords de la ventana
    /// (px). Suele ser la posición del click. El widget la clampea
    /// para que el panel no se salga del `viewport`.
    pub anchor: (f32, f32),
    /// Tamaño actual de la ventana (`w`, `h`). Usado para clamping.
    pub viewport: (f32, f32),
    /// Caption uppercase tiny en la parte superior del panel — ideal
    /// para mostrar a qué objeto se invocó el menú ("B5", "Selección
    /// 3×4"). `None` omite el header.
    pub header: Option<String>,
    pub items: Vec<ContextMenuItem>,
    /// Índice del item resaltado por keyboard. `usize::MAX` = ninguno
    /// (estado típico al recién abrir el menú; el usuario aún no se
    /// movió con flechas). Si está fuera de rango o cae en un
    /// separator, no se resalta nada.
    pub active: usize,
    /// Construye el Msg al elegir un item por click. La app decide
    /// si cierra el menú dentro del handler o por side-effect.
    pub on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync>,
    /// Msg al hacer click fuera del menú (scrim) o por Esc.
    pub on_dismiss: Msg,
    pub palette: ContextMenuPalette,
}

/// Ancho del panel. Fijo para mantener la silueta característica
/// — los menús con ancho variable rompen la consistencia visual.
const PANEL_W: f32 = 240.0;
/// Altura de cada item (no-separator).
const ITEM_H: f32 = 28.0;
/// Altura de un separator.
const SEP_H: f32 = 10.0;
/// Altura del header tiny.
const HEADER_H: f32 = 22.0;
/// Ancho de la barra accent vertical — la firma visual.
const ACCENT_BAR_W: f32 = 3.0;
/// Padding interno del panel (entre la barra accent y los items).
const ITEM_PAD_LEFT: f32 = 12.0;
const ITEM_PAD_RIGHT: f32 = 14.0;

/// Compone el menú como un `View<Msg>`. Para usarlo, devolvelo desde
/// [`llimphi_ui::App::view_overlay`].
pub fn context_menu_view<Msg: Clone + 'static>(spec: ContextMenuSpec<Msg>) -> View<Msg> {
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

    let header_h = if header.is_some() { HEADER_H } else { 0.0 };
    let items_h: f32 = items
        .iter()
        .map(|it| if it.separator { SEP_H } else { ITEM_H })
        .sum();
    let panel_h = header_h + items_h + 2.0; // +2 por los bordes top/bottom

    // Clamping al viewport con un pequeño margen para no pegarse al
    // borde de la ventana — queda más respirado.
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
        children.push(header_view(text, &palette));
    }
    for (i, item) in items.iter().enumerate() {
        let is_active = i == active;
        children.push(item_view(i, item, is_active, &on_pick, &palette));
    }

    // Panel = barra accent (3px) + columna de items. El borde 1px
    // top/right/bottom lo aporta el contenedor exterior (paint
    // del scrim) y el panel se inserta encima.
    let panel = View::new(Style {
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
        flex_direction: FlexDirection::Row,
        ..Default::default()
    })
    .fill(palette.border)
    .children(vec![
        // Barra accent vertical — sin separación con el panel,
        // visualmente forma parte del borde izquierdo.
        View::new(Style {
            size: Size {
                width: length(ACCENT_BAR_W),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(palette.accent),
        // Contenedor de los items (deja 1px al borde derecho/inf
        // /superior para que el `palette.border` del padre se vea).
        View::new(Style {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            padding: Rect {
                left: length(0.0_f32),
                right: length(1.0_f32),
                top: length(1.0_f32),
                bottom: length(1.0_f32),
            },
            ..Default::default()
        })
        .fill(palette.bg_panel)
        .children(children),
    ]);

    // Scrim full-screen. on_click = on_dismiss: cualquier click que
    // NO atrape el panel (= cualquier click "fuera") cierra el menú.
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.scrim)
    .on_click(on_dismiss)
    .children(vec![panel])
}

fn header_view<Msg: Clone + 'static>(text: String, palette: &ContextMenuPalette) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_H),
        },
        padding: Rect {
            left: length(ITEM_PAD_LEFT),
            right: length(ITEM_PAD_RIGHT),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.bg_panel)
    // Header en uppercase tiny — la convención brutalista de "esto
    // no es una opción, es el contexto". Letter-spacing lo simulamos
    // expandiendo a mayúsculas: parley no soporta letter-spacing
    // todavía, pero el efecto de "sección" se logra con tamaño +
    // mayúsculas + apagado.
    .text_aligned(text.to_uppercase(), 9.5, palette.fg_header, Alignment::Start)
}

fn item_view<Msg: Clone + 'static>(
    idx: usize,
    item: &ContextMenuItem,
    is_active: bool,
    on_pick: &Arc<dyn Fn(usize) -> Msg + Send + Sync>,
    palette: &ContextMenuPalette,
) -> View<Msg> {
    if item.separator {
        return separator_view(palette);
    }

    let (bg, fg, fg_short) = if !item.enabled {
        (palette.bg_panel, palette.fg_disabled, palette.fg_disabled)
    } else if is_active {
        (palette.bg_active, palette.fg_active, palette.fg_active)
    } else if item.destructive {
        (palette.bg_panel, palette.fg_destructive, palette.fg_shortcut)
    } else {
        (palette.bg_panel, palette.fg_text, palette.fg_shortcut)
    };

    // Label a la izquierda — flex-grow 1 para que empuje al shortcut
    // contra el borde derecho. Si no hay shortcut, sigue ocupando todo.
    let label = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        ..Default::default()
    })
    .text_aligned(item.label.clone(), 12.5, fg, Alignment::Start);

    let mut row_children: Vec<View<Msg>> = vec![label];
    if let Some(sh) = &item.shortcut {
        row_children.push(
            View::new(Style {
                size: Size {
                    width: length(70.0_f32),
                    height: percent(1.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(sh.clone(), 11.0, fg_short, Alignment::End),
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
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .children(row_children);

    if item.enabled {
        // Hover = mismo look que active. El usuario nunca se confunde
        // sobre "qué pasa si click acá".
        row = row.hover_fill(palette.bg_active);
        let on_pick = on_pick.clone();
        row = row.on_click_at(move |_, _, _, _| Some(on_pick(idx)));
    }
    row
}

fn separator_view<Msg: Clone + 'static>(palette: &ContextMenuPalette) -> View<Msg> {
    // Container con el bg del panel; adentro, una línea horizontal
    // centrada al 60% del ancho. Margen vertical sobrante = padding
    // arriba/abajo para que la línea quede en el centro óptico.
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(SEP_H),
        },
        flex_direction: FlexDirection::Column,
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .children(vec![View::new(Style {
        size: Size {
            width: percent(0.6_f32),
            height: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.separator)])
}

/// Helper de navegación por teclado. Dado el item activo actual + la
/// dirección (`+1` baja, `-1` sube), devuelve el siguiente índice
/// válido (saltea separators y disabled). Si no hay items elegibles
/// devuelve `usize::MAX` (= ninguno).
pub fn step_active(items: &[ContextMenuItem], current: usize, direction: i32) -> usize {
    if items.is_empty() {
        return usize::MAX;
    }
    let n = items.len() as i32;
    // Si current es MAX, empezamos en el extremo opuesto a la dirección.
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
        let items = vec![
            it("A"),
            ContextMenuItem::separator(),
            it("B"),
            it("C"),
        ];
        // Desde "A" hacia abajo: salteamos el separator y caemos en "B" (idx 2).
        assert_eq!(step_active(&items, 0, 1), 2);
        // Desde "B" hacia arriba: salteamos el separator y caemos en "A" (idx 0).
        assert_eq!(step_active(&items, 2, -1), 0);
    }

    #[test]
    fn step_active_skips_disabled() {
        let items = vec![it("A"), it("B").disabled(), it("C")];
        assert_eq!(step_active(&items, 0, 1), 2); // salta B
        assert_eq!(step_active(&items, 2, -1), 0);
    }

    #[test]
    fn step_active_wraps_around() {
        let items = vec![it("A"), it("B"), it("C")];
        assert_eq!(step_active(&items, 2, 1), 0); // 2 → 0
        assert_eq!(step_active(&items, 0, -1), 2); // 0 → 2
    }

    #[test]
    fn step_active_from_max_picks_endpoint() {
        let items = vec![it("A"), it("B"), it("C")];
        // MAX hacia abajo arranca en idx 0.
        assert_eq!(step_active(&items, usize::MAX, 1), 0);
        // MAX hacia arriba arranca en el último.
        assert_eq!(step_active(&items, usize::MAX, -1), 2);
    }

    #[test]
    fn step_active_all_disabled_returns_max() {
        let items = vec![it("A").disabled(), ContextMenuItem::separator()];
        assert_eq!(step_active(&items, usize::MAX, 1), usize::MAX);
    }
}
