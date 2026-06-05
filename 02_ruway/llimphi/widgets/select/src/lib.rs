//! `llimphi-widget-select` — control *select* / dropdown moderno.
//!
//! Modelado como un **gradiente de complejidad**: la misma pieza sirve
//! desde el select más tonto hasta uno con carga asíncrona y badges, sin
//! cambiar de widget — sólo se van encendiendo capas.
//!
//! ```text
//!   ┌──────────────────────────────┐
//!   │ ◷  Pendiente            ●  ▾ │   ← disparador (cerrado)
//!   └──────────────────────────────┘
//!        │ click → view_overlay
//!        ▼
//!   ┃ ⌕ Buscar…                     ← caja de búsqueda (capa 2)
//!   ┃ ✓ ◷  Pendiente        12      ← ítem rico: check · icono · label · badge (capa 3)
//!   ┃   ◔  En curso          3
//!   ┃   ◉  Hecho             ·       ← badge dot
//!   ┃ ─────────────────────
//!   ┃ ⟳  Cargando…                  ← estado async (capa 4): Loading / Error+reintento / vacío
//! ```
//!
//! ## Las cuatro capas
//!
//! 1. **Select simple.** [`SelectItem::new`] + [`select_trigger_view`] (el
//!    botón cerrado que muestra lo elegido) + [`select_menu_view`] con
//!    [`SelectPhase::Ready`] montado en [`App::view_overlay`]. `on_pick(idx)`
//!    devuelve el índice **original** del ítem elegido; la app lo guarda y
//!    cierra el menú. Eso es todo.
//! 2. **Buscable (combobox).** Poné `searchable: true`. La caja de arriba
//!    pinta `query`; el tecleo lo rutea la app por `on_key` a un
//!    `TextInputState` y re-filtra con [`filter`]. [`step_active`] mueve el
//!    resaltado con ↑/↓ saltando deshabilitados; [`resolve`] traduce la
//!    posición resaltada al índice original para Enter.
//! 3. **Ítems ricos + badges.** Cada [`SelectItem`] lleva `icon`, `sublabel`
//!    y un [`SelectBadge`] (conteo `12`, chip de texto `beta`, o dot de
//!    estado) — reusa `llimphi-widget-badge`. La selección múltiple sale
//!    sola: pasá varios índices en `selected` y el menú pinta el check ✓ en
//!    cada uno (la semántica toggle/cerrar la decide la app).
//! 4. **Carga asíncrona.** El menú no exige tener los datos: [`SelectPhase`]
//!    distingue `Loading` (fila "Cargando…"), `Error` (mensaje + acción
//!    `on_retry`) y `Ready`. El patrón: la app abre el menú, lanza
//!    `Handle::spawn`, pinta `Loading`, y al volver el worker cambia a
//!    `Ready`. Una *generación* (`u64`) descarta respuestas viejas si el
//!    usuario reabrió/retecleó — ver el ejemplo `select_demo`.
//!
//! El look es el de `context-menu`: panel redondeado con borde hairline,
//! filas como píldoras con hover suave, indicador accent en la fila activa,
//! scrim full-screen que dismissa al click-fuera.

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_badge::{count_badge_view, dot_badge_view};
use llimphi_widget_panel::{panel_signature_painter, PanelStyle};

/// Re-export: el badge semántico se elige con `BadgeKind`, así el consumidor
/// no necesita depender de `llimphi-widget-badge` directamente.
pub use llimphi_widget_badge::BadgeKind;

// ─────────────────────────────────────────────────────────────────────────
// Paleta
// ─────────────────────────────────────────────────────────────────────────

/// Paleta del select, derivada del `Theme`. El disparador se lee como un
/// input; el menú flotante como un panel elevado (igual que `context-menu`).
#[derive(Debug, Clone, Copy)]
pub struct SelectPalette {
    pub bg_trigger: Color,
    pub bg_trigger_hover: Color,
    pub bg_panel: Color,
    pub bg_hover: Color,
    pub bg_active: Color,
    pub fg_text: Color,
    pub fg_active: Color,
    pub fg_muted: Color,
    pub fg_placeholder: Color,
    pub fg_destructive: Color,
    pub accent: Color,
    pub border: Color,
    pub border_focus: Color,
    pub separator: Color,
    pub scrim: Color,
    pub radius: f64,
    pub panel: PanelStyle,
}

impl SelectPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        let mut panel = PanelStyle::neutral(t);
        panel.bg_base = t.bg_panel;
        panel.radius = PANEL_RADIUS as f64;
        Self {
            bg_trigger: t.bg_input,
            bg_trigger_hover: t.bg_input_focus,
            bg_panel: t.bg_panel,
            bg_hover: t.bg_row_hover,
            bg_active: t.bg_selected,
            fg_text: t.fg_text,
            fg_active: t.fg_text,
            fg_muted: t.fg_muted,
            fg_placeholder: t.fg_placeholder,
            fg_destructive: t.fg_destructive,
            accent: t.accent,
            border: t.border,
            border_focus: t.border_focus,
            separator: t.border,
            scrim: Color::from_rgba8(0, 0, 0, 64),
            radius: PANEL_RADIUS as f64,
            panel,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Badges
// ─────────────────────────────────────────────────────────────────────────

/// Badge opcional al final de un ítem. Tres formas, todas sobre la paleta
/// semántica de `llimphi-widget-badge`:
/// - [`SelectBadge::count`] — chip ovalado con número (`12`, `99+`).
/// - [`SelectBadge::label`] — chip de texto corto (`beta`, `nuevo`).
/// - [`SelectBadge::dot`] — punto de estado sin texto (online/idle/…).
#[derive(Debug, Clone)]
pub struct SelectBadge {
    pub kind: BadgeKind,
    /// Texto del chip. `Some` → chip de texto; `None` con `count` → conteo;
    /// ambos `None` → dot.
    pub label: Option<String>,
    pub count: Option<u32>,
}

impl SelectBadge {
    pub fn count(n: u32, kind: BadgeKind) -> Self {
        Self { kind, label: None, count: Some(n) }
    }
    pub fn label(text: impl Into<String>, kind: BadgeKind) -> Self {
        Self { kind, label: Some(text.into()), count: None }
    }
    pub fn dot(kind: BadgeKind) -> Self {
        Self { kind, label: None, count: None }
    }

    fn view<Msg: Clone + 'static>(&self) -> View<Msg> {
        match (&self.label, self.count) {
            (Some(text), _) => label_chip_view(text, self.kind),
            (None, Some(n)) => count_badge_view(n, self.kind),
            (None, None) => dot_badge_view(self.kind),
        }
    }
}

/// Chip de texto corto (no existe en `llimphi-widget-badge`, que sólo da
/// count y dot). Mismo lenguaje visual: bg sólido semántico + texto claro.
fn label_chip_view<Msg: Clone + 'static>(text: &str, kind: BadgeKind) -> View<Msg> {
    let w = (text.chars().count() as f32 * 6.5 + 14.0).max(18.0);
    View::new(Style {
        size: Size { width: length(w), height: length(16.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(kind.bg())
    .radius(4.0)
    .text_aligned(text.to_string(), 10.0, kind.fg(), Alignment::Center)
}

// ─────────────────────────────────────────────────────────────────────────
// Ítems
// ─────────────────────────────────────────────────────────────────────────

/// Una opción del select. `icon`/`sublabel`/`badge` son opcionales: un
/// select simple sólo setea `label`.
#[derive(Debug, Clone)]
pub struct SelectItem {
    pub label: String,
    pub sublabel: Option<String>,
    pub icon: Option<String>,
    pub badge: Option<SelectBadge>,
    pub enabled: bool,
}

impl SelectItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            sublabel: None,
            icon: None,
            badge: None,
            enabled: true,
        }
    }
    /// Segunda línea, más apagada y chica (ej. descripción, id, ruta).
    pub fn with_sublabel(mut self, sub: impl Into<String>) -> Self {
        self.sublabel = Some(sub.into());
        self
    }
    /// Glifo del gutter izquierdo (unicode; no acopla a `llimphi-icons`).
    pub fn icon(mut self, glyph: impl Into<String>) -> Self {
        self.icon = Some(glyph.into());
        self
    }
    pub fn badge(mut self, badge: SelectBadge) -> Self {
        self.badge = Some(badge);
        self
    }
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    fn row_height(&self) -> f32 {
        if self.sublabel.is_some() {
            ITEM_H_SUB
        } else {
            ITEM_H
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Fase async
// ─────────────────────────────────────────────────────────────────────────

/// En qué punto está el contenido del menú. La app lo deriva de su estado
/// de carga (`Handle::spawn` → resultado). Toma prestados los ítems para no
/// clonar la lista por frame.
pub enum SelectPhase<'a> {
    /// Trabajo en vuelo: el menú pinta una fila "Cargando…".
    Loading,
    /// Falló: mensaje + (si hay `on_retry`) acción de reintento.
    Error(&'a str),
    /// Listo: estos son los ítems (todos; `visible` dice cuáles tras filtro).
    Ready(&'a [SelectItem]),
}

// ─────────────────────────────────────────────────────────────────────────
// Constantes de layout
// ─────────────────────────────────────────────────────────────────────────

const PANEL_RADIUS: f32 = 10.0;
const ITEM_RADIUS: f32 = 6.0;
const PANEL_PAD: f32 = 6.0;
const INDICATOR_W: f32 = 3.0;
const CHECK_W: f32 = 18.0;
const ICON_W: f32 = 22.0;
const ITEM_H: f32 = 34.0;
const ITEM_H_SUB: f32 = 46.0;
const STATUS_H: f32 = 40.0;
const SEARCH_H: f32 = 34.0;
const ITEM_PAD_LEFT: f32 = 8.0;
const ITEM_PAD_RIGHT: f32 = 10.0;
const TRIGGER_H: f32 = 34.0;
const APPEAR_SLIDE: f32 = 8.0;

// ─────────────────────────────────────────────────────────────────────────
// Disparador (estado cerrado)
// ─────────────────────────────────────────────────────────────────────────

/// El botón cerrado del select. Muestra el ítem elegido (icono + label +
/// badge) o el `placeholder` si no hay nada. El chevron ▾/▴ refleja `open`.
/// Click emite `on_toggle` (la app alterna su flag de menú abierto).
pub fn select_trigger_view<Msg: Clone + 'static>(
    selected: Option<&SelectItem>,
    placeholder: &str,
    open: bool,
    width: Option<f32>,
    palette: &SelectPalette,
    on_toggle: Msg,
) -> View<Msg> {
    let size_width = match width {
        Some(w) => length(w),
        None => percent(1.0_f32),
    };

    let mut row: Vec<View<Msg>> = Vec::with_capacity(4);

    if let Some(item) = selected {
        if let Some(icon) = &item.icon {
            row.push(
                View::new(Style {
                    size: Size { width: length(ICON_W), height: auto() },
                    flex_shrink: 0.0,
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Center),
                    ..Default::default()
                })
                .text_aligned(icon.clone(), 13.0, palette.fg_muted, Alignment::Center),
            );
        }
        row.push(
            View::new(Style {
                size: Size { width: auto(), height: auto() },
                flex_grow: 1.0,
                ..Default::default()
            })
            .text_aligned(item.label.clone(), 13.0, palette.fg_text, Alignment::Start),
        );
        if let Some(badge) = &item.badge {
            row.push(badge.view());
        }
    } else {
        row.push(
            View::new(Style {
                size: Size { width: auto(), height: auto() },
                flex_grow: 1.0,
                ..Default::default()
            })
            .text_aligned(
                placeholder.to_string(),
                13.0,
                palette.fg_placeholder,
                Alignment::Start,
            ),
        );
    }

    // Chevron — gira según abierto/cerrado.
    let chevron = if open { "\u{25B4}" } else { "\u{25BE}" }; // ▴ / ▾
    row.push(
        View::new(Style {
            size: Size { width: length(16.0_f32), height: auto() },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned(chevron.to_string(), 11.0, palette.fg_muted, Alignment::Center),
    );

    let (bg, border) = if open {
        (palette.bg_trigger_hover, palette.border_focus)
    } else {
        (palette.bg_trigger, palette.border)
    };

    // Borde de 1px (rect padre coloreado) + relleno interno con la fila.
    View::new(Style {
        size: Size { width: size_width, height: length(TRIGGER_H + 2.0) },
        padding: Rect {
            left: length(1.0_f32),
            right: length(1.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(border)
    .radius(7.0)
    .on_click(on_toggle)
    .children(vec![View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(10.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(bg)
    .radius(6.0)
    .hover_fill(palette.bg_trigger_hover)
    .children(row)])
}

// ─────────────────────────────────────────────────────────────────────────
// Menú flotante (estado abierto) — para App::view_overlay
// ─────────────────────────────────────────────────────────────────────────

/// Especificación del menú desplegado. Todo lo prestado (`phase`, `visible`,
/// `selected`, `query`, `palette`) vive en el `Model` de la app; el menú no
/// clona la lista. Los callbacks son `Arc<dyn Fn>` como en `context-menu`.
pub struct SelectMenuSpec<'a, Msg: Clone + 'static> {
    /// Esquina superior-izquierda deseada (típico: bajo el disparador).
    pub anchor: (f32, f32),
    pub viewport: (f32, f32),
    pub width: f32,
    pub phase: SelectPhase<'a>,
    /// Índices **originales** visibles tras el filtro (ver [`filter`]). En
    /// `Loading`/`Error` se ignora.
    pub visible: &'a [usize],
    /// Posición dentro de `visible` resaltada por teclado/hover.
    /// `usize::MAX` = ninguna.
    pub active: usize,
    /// Índices originales actualmente elegidos (uno para single, varios para
    /// multi). Pintan check ✓.
    pub selected: &'a [usize],
    /// Texto de la caja de búsqueda (si `searchable`).
    pub query: &'a str,
    pub searchable: bool,
    /// Texto cuando `Ready` pero `visible` quedó vacío (filtro sin match).
    pub empty_text: &'a str,
    /// Progreso de aparición 0..1 (fade + leve slide). `1.0` = fijo.
    pub appear: f32,
    /// Click en un ítem: índice **original**.
    pub on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync>,
    /// Hover sobre un ítem: posición dentro de `visible` (para sincronizar
    /// `active` con el mouse). Opcional.
    pub on_hover: Option<Arc<dyn Fn(usize) -> Msg + Send + Sync>>,
    /// Click-fuera (scrim) o Esc.
    pub on_dismiss: Msg,
    /// Acción de la fila de error. Si `None`, el error no es accionable.
    pub on_retry: Option<Msg>,
    pub palette: &'a SelectPalette,
}

/// Compone el menú desplegado como `View<Msg>` para [`App::view_overlay`].
/// Incluye scrim de dismiss, caja de búsqueda (si corresponde) y la fase
/// async resuelta a filas.
pub fn select_menu_view<Msg: Clone + 'static>(spec: SelectMenuSpec<'_, Msg>) -> View<Msg> {
    let SelectMenuSpec {
        anchor,
        viewport,
        width,
        phase,
        visible,
        active,
        selected,
        query,
        searchable,
        empty_text,
        appear,
        on_pick,
        on_hover,
        on_dismiss,
        on_retry,
        palette,
    } = spec;

    let appear = appear.clamp(0.0, 1.0);
    let slide = (1.0 - appear) * APPEAR_SLIDE;

    // Construir los hijos del panel (search + cuerpo) y medir su alto.
    let mut children: Vec<View<Msg>> = Vec::new();
    let mut body_h = 0.0_f32;

    if searchable {
        children.push(search_box_view(query, palette));
        body_h += SEARCH_H + 4.0;
    }

    match phase {
        SelectPhase::Loading => {
            children.push(status_row_view("\u{27F3}", "Cargando…", palette.fg_muted, palette));
            body_h += STATUS_H;
        }
        SelectPhase::Error(msg) => {
            children.push(error_row_view(msg, on_retry.clone(), palette));
            body_h += STATUS_H;
        }
        SelectPhase::Ready(items) => {
            if visible.is_empty() {
                children.push(status_row_view("\u{2205}", empty_text, palette.fg_muted, palette));
                body_h += STATUS_H;
            } else {
                let is_selected = |orig: usize| selected.contains(&orig);
                for (pos, &orig) in visible.iter().enumerate() {
                    if let Some(item) = items.get(orig) {
                        body_h += item.row_height();
                        children.push(item_row_view(
                            pos,
                            orig,
                            item,
                            pos == active,
                            is_selected(orig),
                            &on_pick,
                            on_hover.as_ref(),
                            palette,
                        ));
                    }
                }
            }
        }
    }

    let panel_h = 2.0 + 2.0 * PANEL_PAD + body_h;

    // Clamp al viewport.
    let margin = 4.0;
    let x = anchor
        .0
        .min((viewport.0 - width - margin).max(margin))
        .max(margin);
    let y = anchor
        .1
        .min((viewport.1 - panel_h - margin).max(margin))
        .max(margin);

    let panel = panel_container(x, y + slide, width, panel_h, children, palette);

    // Scrim full-screen: cualquier click fuera dismissa.
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(palette.scrim)
    .alpha(appear)
    .on_click(on_dismiss)
    .children(vec![panel])
}

/// Contenedor visual: panel redondeado con borde hairline (nodo exterior
/// del color de borde + interior con el gradiente del PanelStyle) y padding
/// para que las píldoras queden inset.
fn panel_container<Msg: Clone + 'static>(
    x: f32,
    y: f32,
    width: f32,
    panel_h: f32,
    children: Vec<View<Msg>>,
    palette: &SelectPalette,
) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(width), height: length(panel_h) },
        padding: Rect {
            left: length(1.0_f32),
            right: length(1.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.border)
    .radius(palette.radius)
    .children(vec![View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(PANEL_PAD),
            right: length(PANEL_PAD),
            top: length(PANEL_PAD),
            bottom: length(PANEL_PAD),
        },
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .radius((palette.radius - 1.0).max(0.0))
    .paint_with(panel_signature_painter(palette.panel))
    .children(children)])
}

/// Caja de búsqueda: render-only (el tecleo lo rutea la app por `on_key`).
fn search_box_view<Msg: Clone + 'static>(query: &str, palette: &SelectPalette) -> View<Msg> {
    let is_empty = query.is_empty();
    let (text, color) = if is_empty {
        ("Buscar…".to_string(), palette.fg_placeholder)
    } else {
        (query.to_string(), palette.fg_text)
    };
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(SEARCH_H) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg_trigger)
    .radius(ITEM_RADIUS as f64)
    .children(vec![
        View::new(Style {
            size: Size { width: length(16.0_f32), height: auto() },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned("\u{2315}".to_string(), 13.0, palette.fg_muted, Alignment::Center),
        View::new(Style {
            size: Size { width: auto(), height: auto() },
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(text, 12.5, color, Alignment::Start),
    ])
}

/// Fila de estado (Loading / vacío): glifo + texto centrado-izquierda.
fn status_row_view<Msg: Clone + 'static>(
    glyph: &str,
    text: &str,
    color: Color,
    palette: &SelectPalette,
) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(STATUS_H) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(ITEM_PAD_LEFT),
            right: length(ITEM_PAD_RIGHT),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size { width: length(ICON_W), height: auto() },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned(glyph.to_string(), 14.0, color, Alignment::Center),
        View::new(Style {
            size: Size { width: auto(), height: auto() },
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(text.to_string(), 12.5, color, Alignment::Start),
    ])
    .fill(palette.bg_panel)
    .radius(ITEM_RADIUS as f64)
}

/// Fila de error: mensaje destructivo + "Reintentar" clickeable (si hay
/// `on_retry`). El click consume el evento (no dismissa el menú).
fn error_row_view<Msg: Clone + 'static>(
    msg: &str,
    on_retry: Option<Msg>,
    palette: &SelectPalette,
) -> View<Msg> {
    let mut children: Vec<View<Msg>> = vec![
        View::new(Style {
            size: Size { width: length(ICON_W), height: auto() },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned("\u{26A0}".to_string(), 14.0, palette.fg_destructive, Alignment::Center),
        View::new(Style {
            size: Size { width: auto(), height: auto() },
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(msg.to_string(), 12.5, palette.fg_destructive, Alignment::Start),
    ];

    if let Some(retry) = on_retry {
        children.push(
            View::new(Style {
                size: Size { width: length(74.0_f32), height: length(24.0_f32) },
                flex_shrink: 0.0,
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .fill(palette.bg_active)
            .radius(ITEM_RADIUS as f64)
            .hover_fill(palette.bg_hover)
            // El hit-test interno gana sobre el scrim: el click no dismissa.
            .on_click(retry)
            .text_aligned("Reintentar".to_string(), 11.5, palette.fg_text, Alignment::Center),
        );
    }

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(STATUS_H) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(ITEM_PAD_LEFT),
            right: length(ITEM_PAD_RIGHT),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .radius(ITEM_RADIUS as f64)
    .children(children)
}

/// Una fila de ítem. `is_active` = resaltado por teclado/hover; `is_checked`
/// = está en `selected`.
#[allow(clippy::too_many_arguments)]
fn item_row_view<Msg: Clone + 'static>(
    pos: usize,
    orig: usize,
    item: &SelectItem,
    is_active: bool,
    is_checked: bool,
    on_pick: &Arc<dyn Fn(usize) -> Msg + Send + Sync>,
    on_hover: Option<&Arc<dyn Fn(usize) -> Msg + Send + Sync>>,
    palette: &SelectPalette,
) -> View<Msg> {
    let fg = if !item.enabled {
        palette.fg_muted
    } else if is_active {
        palette.fg_active
    } else {
        palette.fg_text
    };
    let icon_fg = if !item.enabled {
        palette.fg_muted
    } else if is_active {
        palette.accent
    } else {
        palette.fg_muted
    };

    // Indicador accent vertical en la fila activa (reserva ancho siempre).
    let indicator = {
        let v = View::new(Style {
            size: Size { width: length(INDICATOR_W), height: percent(0.55_f32) },
            flex_shrink: 0.0,
            ..Default::default()
        });
        if is_active && item.enabled {
            v.fill(palette.accent).radius(2.0)
        } else {
            v
        }
    };

    // Check de selección (✓ accent cuando elegido; gutter reservado siempre
    // para que multi-select no haga saltar el texto).
    let check = View::new(Style {
        size: Size { width: length(CHECK_W), height: auto() },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(
        if is_checked { "\u{2713}".to_string() } else { String::new() },
        13.0,
        palette.accent,
        Alignment::Center,
    );

    let mut row_children: Vec<View<Msg>> = vec![indicator, check];

    if let Some(icon) = &item.icon {
        row_children.push(
            View::new(Style {
                size: Size { width: length(ICON_W), height: auto() },
                flex_shrink: 0.0,
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text_aligned(icon.clone(), 13.0, icon_fg, Alignment::Center),
        );
    }

    // Columna label + sublabel.
    let mut text_col: Vec<View<Msg>> = vec![View::new(Style {
        size: Size { width: percent(1.0_f32), height: auto() },
        ..Default::default()
    })
    .text_aligned(item.label.clone(), 12.5, fg, Alignment::Start)];
    if let Some(sub) = &item.sublabel {
        text_col.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: auto() },
                ..Default::default()
            })
            .text_aligned(sub.clone(), 11.0, palette.fg_muted, Alignment::Start),
        );
    }
    row_children.push(
        View::new(Style {
            size: Size { width: auto(), height: auto() },
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            justify_content: Some(JustifyContent::Center),
            gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
            ..Default::default()
        })
        .children(text_col),
    );

    if let Some(badge) = &item.badge {
        row_children.push(badge.view());
    }

    let mut row = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(item.row_height()) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(ITEM_PAD_LEFT),
            right: length(ITEM_PAD_RIGHT),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .radius(ITEM_RADIUS as f64)
    .children(row_children);

    if is_active && item.enabled {
        row = row.fill(palette.bg_active);
    }

    if item.enabled {
        row = row.hover_fill(palette.bg_hover);
        let on_pick = on_pick.clone();
        // Consumir el click (no propagar al scrim que dismissaría).
        row = row.on_click_at(move |_, _, _, _| Some(on_pick(orig)));
        if let Some(on_hover) = on_hover {
            let on_hover = on_hover.clone();
            row = row.on_pointer_enter(on_hover(pos));
        }
    }
    row
}

// ─────────────────────────────────────────────────────────────────────────
// Helpers de filtro + navegación (puros, lado app)
// ─────────────────────────────────────────────────────────────────────────

/// Filtra ítems por `query` (subsecuencia case-insensitive sobre
/// label+sublabel), devolviendo los índices **originales** ordenados por
/// score descendente (mejor match primero). `query` vacío → todos en orden.
///
/// El scoring premia matches contiguos y al inicio de palabra — suficiente
/// para un combobox; no pretende ser fzf.
pub fn filter(items: &[SelectItem], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..items.len()).collect();
    }
    let mut scored: Vec<(usize, i32)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let mut hay = item.label.to_lowercase();
        if let Some(sub) = &item.sublabel {
            hay.push(' ');
            hay.push_str(&sub.to_lowercase());
        }
        if let Some(score) = subseq_score(&hay, &q) {
            scored.push((i, score));
        }
    }
    // Orden estable por score desc, conservando orden original ante empate.
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.into_iter().map(|(i, _)| i).collect()
}

/// Score de subsecuencia: `None` si `needle` no es subsecuencia de `hay`.
/// Bonus por chars contiguos y por arrancar tras separador (inicio de
/// palabra).
fn subseq_score(hay: &str, needle: &str) -> Option<i32> {
    let hay: Vec<char> = hay.chars().collect();
    let needle: Vec<char> = needle.chars().collect();
    let mut hi = 0;
    let mut score = 0;
    let mut prev_match = false;
    let mut prev_char: Option<char> = None;
    for &nc in &needle {
        let mut found = false;
        while hi < hay.len() {
            let hc = hay[hi];
            let at_boundary = prev_char.map(|c| c == ' ' || c == '-' || c == '_' || c == '/').unwrap_or(true);
            if hc == nc {
                score += 1;
                if prev_match {
                    score += 2; // contiguo
                }
                if at_boundary {
                    score += 3; // inicio de palabra
                }
                prev_match = true;
                prev_char = Some(hc);
                hi += 1;
                found = true;
                break;
            } else {
                prev_match = false;
                prev_char = Some(hc);
                hi += 1;
            }
        }
        if !found {
            return None;
        }
    }
    Some(score)
}

/// Mueve el resaltado: dada la posición actual en `visible` + dirección
/// (`+1`/`-1`), la siguiente posición **habilitada** (envuelve). Devuelve
/// `usize::MAX` si no hay ninguna elegible. `current == usize::MAX` arranca
/// por el extremo según la dirección.
pub fn step_active(items: &[SelectItem], visible: &[usize], current: usize, direction: i32) -> usize {
    if visible.is_empty() {
        return usize::MAX;
    }
    let n = visible.len() as i32;
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
        if let Some(item) = items.get(visible[i as usize]) {
            if item.enabled {
                return i as usize;
            }
        }
    }
    usize::MAX
}

/// Traduce una posición en `visible` al índice **original** del ítem — lo
/// que la app pasa a `on_pick`/guarda como elegido al pulsar Enter.
pub fn resolve(visible: &[usize], pos: usize) -> Option<usize> {
    visible.get(pos).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<SelectItem> {
        vec![
            SelectItem::new("Pendiente"),
            SelectItem::new("En curso").disabled(),
            SelectItem::new("Hecho"),
            SelectItem::new("Archivado").with_sublabel("guardado en frío"),
        ]
    }

    #[test]
    fn filter_vacio_devuelve_todos_en_orden() {
        let it = items();
        assert_eq!(filter(&it, ""), vec![0, 1, 2, 3]);
        assert_eq!(filter(&it, "   "), vec![0, 1, 2, 3]);
    }

    #[test]
    fn filter_subsecuencia_case_insensitive() {
        let it = items();
        // "hec" sólo es subsecuencia de "Hecho" (ojo: "he" también matchea
        // "arcHivado…En frío", por eso elegimos algo discriminante).
        assert_eq!(filter(&it, "hec"), vec![2]);
        // "EN" matchea "Pendiente" y "En curso" (subsecuencia, case-insensitive).
        let r = filter(&it, "en");
        assert!(r.contains(&0) && r.contains(&1));
    }

    #[test]
    fn filter_busca_en_sublabel() {
        let it = items();
        // "frío" sólo aparece en el sublabel de "Archivado".
        assert_eq!(filter(&it, "frío"), vec![3]);
    }

    #[test]
    fn filter_prefijo_puntua_mas_que_disperso() {
        let it = vec![
            SelectItem::new("abandono"), // 'a' disperso al inicio
            SelectItem::new("banana"),   // "ban" contiguo al inicio
        ];
        // Para "ban", banana (contiguo, inicio de palabra) debe ir primero.
        assert_eq!(filter(&it, "ban")[0], 1);
    }

    #[test]
    fn step_active_salta_deshabilitados_y_envuelve() {
        let it = items();
        let vis = filter(&it, ""); // [0,1,2,3], pos 1 = "En curso" disabled
        assert_eq!(step_active(&it, &vis, 0, 1), 2); // salta el disabled
        assert_eq!(step_active(&it, &vis, 0, -1), 3); // envuelve, salta disabled
        assert_eq!(step_active(&it, &vis, usize::MAX, 1), 0); // arranca al inicio
    }

    #[test]
    fn step_active_lista_vacia() {
        let it = items();
        assert_eq!(step_active(&it, &[], usize::MAX, 1), usize::MAX);
    }

    #[test]
    fn resolve_mapea_posicion_a_indice_original() {
        let vis = vec![3, 0, 2];
        assert_eq!(resolve(&vis, 0), Some(3));
        assert_eq!(resolve(&vis, 2), Some(2));
        assert_eq!(resolve(&vis, 9), None);
    }

    #[test]
    fn badge_constructores() {
        let c = SelectBadge::count(12, BadgeKind::Info);
        assert_eq!(c.count, Some(12));
        let l = SelectBadge::label("beta", BadgeKind::Warning);
        assert_eq!(l.label.as_deref(), Some("beta"));
        let d = SelectBadge::dot(BadgeKind::Success);
        assert!(d.label.is_none() && d.count.is_none());
    }
}
