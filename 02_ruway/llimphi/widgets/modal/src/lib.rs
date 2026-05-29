//! `llimphi-widget-modal` — diálogo genérico centrado con scrim.
//!
//! Distinto del `context-menu` (chico, anclado a un click): el modal
//! ocupa una región central de tamaño configurable, presenta un título,
//! un cuerpo arbitrario (lo arma la app) y una barra de botones.
//!
//! Uso típico:
//! 1. La app guarda `Option<ModalState>` en su modelo.
//! 2. `view_overlay` devuelve `Some(modal_view(spec))` cuando hay
//!    state, `None` cuando se cerró.
//! 3. La app captura `Esc` en `on_key` → cierra; `Enter` → primary.
//!
//! Tres severidades de botón:
//! - `Primary` — verde/accent, acción principal.
//! - `Cancel` — neutral, descarta.
//! - `Destructive` — rojo, acción irreversible (eliminar, etc).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::{alpha, radius, Theme};
use llimphi_widget_panel::{panel_signature_painter, PanelStyle};

/// Paleta del modal.
#[derive(Debug, Clone, Copy)]
pub struct ModalPalette {
    /// Color del scrim. El alpha se usa como **promedio** del vignette
    /// radial: el centro (debajo del panel) queda ~25% más claro y las
    /// esquinas ~40% más oscuras, manteniendo la densidad media igual a
    /// lo que pidió el caller. Esto focaliza al modal sin "encerrarlo".
    pub scrim: Color,
    /// Firma visual del panel — gradient sutil + hairline accent en el
    /// top edge. La que vuelve consistente el "look gioser" en todos
    /// los modales y overlays.
    pub panel: PanelStyle,
    pub border: Color,
    pub fg_title: Color,
    pub fg_text: Color,
    pub bg_btn: Color,
    pub bg_btn_hover: Color,
    pub fg_btn: Color,
    pub bg_primary: Color,
    pub fg_primary: Color,
    pub bg_destructive: Color,
    pub fg_destructive: Color,
}

impl ModalPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            scrim: Color::from_rgba8(0, 0, 0, alpha::SCRIM),
            panel: PanelStyle::from_theme_large(t),
            border: t.border,
            fg_title: t.fg_text,
            fg_text: t.fg_muted,
            bg_btn: t.bg_button,
            bg_btn_hover: t.bg_button_hover,
            fg_btn: t.fg_text,
            bg_primary: t.accent,
            fg_primary: t.bg_app,
            bg_destructive: t.fg_destructive,
            fg_destructive: t.bg_app,
        }
    }
}

/// Severidad de un botón del modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Cancel,
    Destructive,
}

/// Spec de un botón. `label` se renderiza; `msg` se dispatcha al click.
#[derive(Clone)]
pub struct ModalButton<Msg> {
    pub label: String,
    pub kind: ButtonKind,
    pub msg: Msg,
}

impl<Msg> ModalButton<Msg> {
    pub fn primary(label: impl Into<String>, msg: Msg) -> Self {
        Self { label: label.into(), kind: ButtonKind::Primary, msg }
    }
    pub fn cancel(label: impl Into<String>, msg: Msg) -> Self {
        Self { label: label.into(), kind: ButtonKind::Cancel, msg }
    }
    pub fn destructive(label: impl Into<String>, msg: Msg) -> Self {
        Self { label: label.into(), kind: ButtonKind::Destructive, msg }
    }
}

/// Spec completo del modal.
pub struct ModalSpec<Msg: Clone + 'static> {
    pub title: String,
    /// Cuerpo libre — la app construye un `View` con lo que quiera
    /// mostrar (texto, form, lista). Se pinta entre título y botones.
    pub body: View<Msg>,
    pub buttons: Vec<ModalButton<Msg>>,
    /// Tamaño del panel (clampea al viewport con margen).
    pub size: (f32, f32),
    pub viewport: (f32, f32),
    /// Msg al hacer click en el scrim o presionar Esc (la app maneja
    /// Esc en su `on_key`; este Msg es el del click).
    pub on_dismiss: Msg,
    pub palette: ModalPalette,
}

const TITLE_H: f32 = 40.0;
const BUTTONS_H: f32 = 56.0;
const TITLE_FONT: f32 = 14.0;
const BTN_FONT: f32 = 12.5;
const PAD: f32 = 16.0;

pub fn modal_view<Msg: Clone + 'static>(spec: ModalSpec<Msg>) -> View<Msg> {
    let ModalSpec {
        title,
        body,
        buttons,
        size,
        viewport,
        on_dismiss,
        palette,
    } = spec;

    let (w, h) = (
        size.0.min(viewport.0 - 32.0).max(200.0),
        size.1.min(viewport.1 - 32.0).max(140.0),
    );
    let x = ((viewport.0 - w) * 0.5).max(0.0);
    let y = ((viewport.1 - h) * 0.5).max(0.0);

    // Header — título a la izquierda; al borde inferior, una línea
    // separadora se logra con un nodo de 1px.
    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(TITLE_H),
        },
        padding: Rect {
            left: length(PAD),
            right: length(PAD),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(title, TITLE_FONT, palette.fg_title, Alignment::Start);

    let separator = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.border);

    // Body — flex_grow para ocupar todo el espacio sobrante.
    let body_wrap = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(PAD),
            right: length(PAD),
            top: length(PAD),
            bottom: length(PAD),
        },
        ..Default::default()
    })
    .children(vec![body]);

    // Botones — flex-row justify-end con gap.
    let btn_children: Vec<View<Msg>> = buttons
        .into_iter()
        .map(|b| button_view(b, &palette))
        .collect();
    let buttons_row = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(BUTTONS_H),
        },
        flex_direction: FlexDirection::Row,
        justify_content: Some(JustifyContent::FlexEnd),
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(PAD),
            right: length(PAD),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(btn_children);

    let panel = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(w),
            height: length(h),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .paint_with(panel_signature_painter(palette.panel))
    .radius(palette.panel.radius)
    .clip(true)
    .children(vec![header, separator, body_wrap, buttons_row]);

    let scrim_base = palette.scrim;
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, Rect as KurboRect};
        use llimphi_ui::llimphi_raster::peniko::{color::AlphaColor, Fill, Gradient};

        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        // Vignette: el centro toma alpha = base * 0.75 (más translúcido,
        // deja ver lo que hay detrás del modal); las esquinas alpha =
        // base * 1.4 (más sólido, oscurece los bordes). El promedio
        // visual queda cerca de `base` original, así la densidad pedida
        // por el caller se preserva.
        let [r, g, b, base_a] = scrim_base.components;
        let inner: Color =
            AlphaColor::new([r, g, b, (base_a * 0.75).clamp(0.0, 1.0)]);
        let outer: Color =
            AlphaColor::new([r, g, b, (base_a * 1.4).clamp(0.0, 1.0)]);

        let cx = rect.x as f64 + rect.w as f64 * 0.5;
        let cy = rect.y as f64 + rect.h as f64 * 0.5;
        let diag_half = (((rect.w as f64).powi(2) + (rect.h as f64).powi(2)).sqrt() * 0.5) as f32;
        let gradient = Gradient::new_radial(Point::new(cx, cy), diag_half)
            .with_stops([inner, outer].as_slice());
        let full = KurboRect::new(
            rect.x as f64,
            rect.y as f64,
            (rect.x + rect.w) as f64,
            (rect.y + rect.h) as f64,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, &gradient, None, &full);
    })
    .on_click(on_dismiss)
    .children(vec![panel])
}

fn button_view<Msg: Clone + 'static>(btn: ModalButton<Msg>, palette: &ModalPalette) -> View<Msg> {
    let (bg, fg, hover) = match btn.kind {
        ButtonKind::Primary => (palette.bg_primary, palette.fg_primary, brighten(palette.bg_primary, 0.15)),
        ButtonKind::Cancel => (palette.bg_btn, palette.fg_btn, palette.bg_btn_hover),
        ButtonKind::Destructive => (palette.bg_destructive, palette.fg_destructive, brighten(palette.bg_destructive, 0.15)),
    };
    let label = btn.label.clone();
    View::new(Style {
        size: Size {
            width: length(label.chars().count() as f32 * 7.5 + 28.0),
            height: length(32.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(hover)
    .radius(radius::SM)
    .text_aligned(label, BTN_FONT, fg, Alignment::Center)
    .on_click(btn.msg)
}

/// Aclara un color sumando `delta` a cada componente RGB. Útil para
/// derivar un hover state del color base sin tener que definirlo aparte.
fn brighten(c: Color, delta: f32) -> Color {
    let [r, g, b, a] = c.components;
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    AlphaColor::new([
        (r + delta).clamp(0.0, 1.0),
        (g + delta).clamp(0.0, 1.0),
        (b + delta).clamp(0.0, 1.0),
        a,
    ])
}
