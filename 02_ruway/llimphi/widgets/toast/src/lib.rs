//! `llimphi-widget-toast` — notificaciones efímeras apiladas.
//!
//! Cuatro severidades (Info / Success / Warning / Error) con color
//! semántico hardcoded — un Error debe leerse rojo aunque la app esté
//! en tema "sunset". Cada toast lleva un icono de `llimphi-icons` y
//! un texto corto.
//!
//! El widget es **render-only**: recibe una lista de [`Toast`]s ya
//! filtrados por la app (los que aún no expiraron) y los apila en la
//! esquina bottom-right. El ciclo de vida (push, auto-dismiss tras
//! `duration`, dismiss manual al click) lo maneja la app desde su
//! `update`/`spawn`.
//!
//! Patrón típico:
//! 1. App tiene `Vec<Toast>` en el modelo + `next_id: u64`.
//! 2. Para pushear: agregar Toast con `expires_at = Instant::now() + dur`
//!    + `handle.spawn(move || { sleep(dur); Msg::ToastExpire(id) })`.
//! 3. `view_overlay` filtra los no expirados y los pasa a `toast_stack_view`.

#![forbid(unsafe_code)]

use std::time::Instant;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Shadow, View};
use llimphi_icons::{icon_view, Icon};
use llimphi_theme::{elevation, motion, radius};

/// Severidad del toast — define color e icono.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastKind {
    /// Color de fondo (semántico, no dependiente del theme).
    pub fn bg(self) -> Color {
        match self {
            ToastKind::Info => Color::from_rgba8(28, 56, 88, 245),
            ToastKind::Success => Color::from_rgba8(28, 72, 44, 245),
            ToastKind::Warning => Color::from_rgba8(88, 64, 20, 245),
            ToastKind::Error => Color::from_rgba8(96, 32, 32, 245),
        }
    }

    /// Color del trazo y del texto principal.
    pub fn fg(self) -> Color {
        match self {
            ToastKind::Info => Color::from_rgba8(180, 220, 250, 255),
            ToastKind::Success => Color::from_rgba8(180, 240, 200, 255),
            ToastKind::Warning => Color::from_rgba8(250, 220, 160, 255),
            ToastKind::Error => Color::from_rgba8(250, 200, 200, 255),
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            ToastKind::Info => Icon::Info,
            ToastKind::Success => Icon::Check,
            ToastKind::Warning => Icon::Warning,
            ToastKind::Error => Icon::Error,
        }
    }
}

/// Un toast en cola. La app mantiene `Vec<Toast>` y descarta los
/// expirados antes de pasarlos al render.
#[derive(Debug, Clone)]
pub struct Toast {
    /// Id estable para que la app pueda correlacionar con su Msg de
    /// dismiss (`Msg::ToastDismiss(u64)`).
    pub id: u64,
    pub kind: ToastKind,
    pub text: String,
    /// Cuándo expira. El render no chequea esto — sólo apila lo que
    /// recibe; la app filtra antes.
    pub expires_at: Instant,
}

const TOAST_W: f32 = 320.0;
const TOAST_H: f32 = 44.0;
const ICON_BOX: f32 = 24.0;
const GAP: f32 = 8.0;
const MARGIN: f32 = 16.0;
/// Ancho del "rail" de severidad en el edge izquierdo. 3px es el sweet
/// spot — visible al pasar sin chocar con el icono. Look Linear/Slack.
const RAIL_W: f32 = 3.0;

/// Apila los toasts en la esquina bottom-right del viewport. `on_click`
/// se construye por toast vía `make_dismiss(id)`. Devuelve un `View`
/// para colgar de `view_overlay`.
pub fn toast_stack_view<Msg, F>(
    toasts: &[Toast],
    viewport: (f32, f32),
    make_dismiss: F,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(u64) -> Msg,
{
    let n = toasts.len() as f32;
    let stack_h = n * TOAST_H + (n - 1.0).max(0.0) * GAP;
    let stack_y = (viewport.1 - stack_h - MARGIN).max(MARGIN);
    let stack_x = (viewport.0 - TOAST_W - MARGIN).max(MARGIN);

    let children: Vec<View<Msg>> = toasts
        .iter()
        .map(|t| single_toast_view(t, make_dismiss(t.id)))
        .collect();

    let stack = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(stack_x),
            top: length(stack_y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(TOAST_W),
            height: length(stack_h.max(0.0)),
        },
        flex_direction: FlexDirection::Column,
        gap: Size {
            width: length(0.0_f32),
            height: length(GAP),
        },
        ..Default::default()
    })
    .children(children);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![stack])
}

fn single_toast_view<Msg: Clone + 'static>(toast: &Toast, on_dismiss: Msg) -> View<Msg> {
    let bg = toast.kind.bg();
    let fg = toast.kind.fg();
    let icon = toast.kind.icon();

    // Rail de severidad: stripe del color fg semántico (más brillante
    // que el bg) en el edge izquierdo. Visible al pasar el ojo sin
    // chocar con el icono — refuerza la severidad para usuarios que ya
    // están mirando a otra parte de la UI.
    let rail = View::new(Style {
        size: Size {
            width: length(RAIL_W),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(fg);

    let icon_cell = View::new(Style {
        size: Size {
            width: length(ICON_BOX),
            height: length(ICON_BOX),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![icon_view(icon, fg, 1.6)]);

    let text = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(toast.text.clone(), 12.0, fg, Alignment::Start);

    // Sombra E3 + entrada/salida animada (key estable = id del toast):
    // el toast aparece con fade-in suave y, al expirar/dismiss, su
    // subescena se reproduce con fade-out — sin necesidad de tween
    // manual en la app.
    let (alpha, blur, dy) = elevation::E3;
    let shadow = Shadow {
        color: Color::from_rgba8(0, 0, 0, alpha),
        blur,
        dx: 0.0,
        dy,
        spread: 0.0,
    };

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(TOAST_H),
        },
        align_items: Some(AlignItems::Center),
        // El rail vive en el edge — sin padding-left propio para que
        // pegue al borde; el padding del contenido arranca después.
        padding: Rect {
            left: length(0.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size {
            width: length(10.0_f32),
            height: length(0.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(bg)
    .radius(radius::MD)
    .shadow(shadow)
    .animated_inout(toast.id, motion::NORMAL)
    .clip(true)
    .on_click(on_dismiss)
    .children(vec![rail, icon_cell, text])
}

/// Helper de construcción para uso inmediato:
/// `Toast::info(1, "guardado", Duration::from_secs(3))`.
impl Toast {
    pub fn info(id: u64, text: impl Into<String>, dur: std::time::Duration) -> Self {
        Self { id, kind: ToastKind::Info, text: text.into(), expires_at: Instant::now() + dur }
    }
    pub fn success(id: u64, text: impl Into<String>, dur: std::time::Duration) -> Self {
        Self { id, kind: ToastKind::Success, text: text.into(), expires_at: Instant::now() + dur }
    }
    pub fn warning(id: u64, text: impl Into<String>, dur: std::time::Duration) -> Self {
        Self { id, kind: ToastKind::Warning, text: text.into(), expires_at: Instant::now() + dur }
    }
    pub fn error(id: u64, text: impl Into<String>, dur: std::time::Duration) -> Self {
        Self { id, kind: ToastKind::Error, text: text.into(), expires_at: Instant::now() + dur }
    }

    pub fn is_alive(&self, now: Instant) -> bool {
        now < self.expires_at
    }
}
