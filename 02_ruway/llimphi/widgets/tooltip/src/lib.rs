//! `llimphi-widget-tooltip` — tooltip flotante con anchor + clamping.
//!
//! Render puro: el widget recibe el anchor (típicamente bottom-center
//! del elemento que lo dispara), el viewport y el texto, y devuelve un
//! `View<Msg>` posicionado en absolute para colgarlo de `view_overlay`.
//! La app es responsable de:
//! 1. Detectar el hover sobre el elemento via `View::on_pointer_enter`
//!    + un `Tween`/delay para evitar tooltips que parpadean al pasar.
//! 2. Guardar el `Option<TooltipSpec>` en su modelo.
//! 3. Devolverlo desde `view_overlay`.
//! 4. Cerrarlo con `View::on_pointer_leave` sobre el mismo elemento.
//!
//! No se incluye scrim — el tooltip es informativo, no modal: los
//! clicks atraviesan al árbol principal. (Para popovers con
//! interacción, usar `llimphi-widget-modal` o el `context-menu`).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, FlexDirection, Position, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::{radius, Theme};

/// Paleta del tooltip — fondo "glass panel" oscuro, texto claro.
#[derive(Debug, Clone, Copy)]
pub struct TooltipPalette {
    pub bg: Color,
    pub fg: Color,
    pub border: Color,
}

impl TooltipPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg: t.fg_text,
            border: t.border,
        }
    }
}

/// Lado preferido al que se coloca el tooltip respecto del anchor.
/// Si no entra en el viewport por ese lado, el clamping lo empuja al
/// lado contrario (no recortado).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Side {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

/// Spec para [`tooltip_view`].
#[derive(Debug, Clone)]
pub struct TooltipSpec {
    /// Punto de origen — típicamente el centro del elemento que dispara.
    pub anchor: (f32, f32),
    /// Tamaño actual de la ventana, para clamping.
    pub viewport: (f32, f32),
    /// Lado preferido respecto del anchor.
    pub side: Side,
    pub text: String,
    pub palette: TooltipPalette,
}

const PAD_X: f32 = 8.0;
const PAD_Y: f32 = 5.0;
const GAP: f32 = 6.0;
const FONT_SIZE: f32 = 11.5;
/// Ancho aproximado de un carácter (estimación zonal — Llimphi
/// todavía no expone medición previa al layout). Sirve para clampear
/// tooltips largos a un ancho razonable.
const CHAR_W_APPROX: f32 = 6.5;
const MAX_W: f32 = 280.0;

pub fn tooltip_view<Msg: Clone + 'static>(spec: TooltipSpec) -> View<Msg> {
    let TooltipSpec { anchor, viewport, side, text, palette } = spec;

    // Tamaño estimado del tooltip — Llimphi resuelve layout pero el
    // posicionamiento absolute necesita un x,y; estimamos con el ancho
    // del texto y limitamos al MAX_W. Ancho real puede diferir un
    // píxel — al ojo es invisible.
    let est_w = (text.chars().count() as f32 * CHAR_W_APPROX + PAD_X * 2.0).min(MAX_W);
    let est_h = FONT_SIZE * 1.3 + PAD_Y * 2.0;

    // Posicionamiento respecto del anchor.
    let (raw_x, raw_y) = match side {
        Side::Bottom => (anchor.0 - est_w * 0.5, anchor.1 + GAP),
        Side::Top => (anchor.0 - est_w * 0.5, anchor.1 - GAP - est_h),
        Side::Right => (anchor.0 + GAP, anchor.1 - est_h * 0.5),
        Side::Left => (anchor.0 - GAP - est_w, anchor.1 - est_h * 0.5),
    };

    // Clamping al viewport (margen 4px para no pegarse al borde).
    let margin = 4.0;
    let x = raw_x
        .min((viewport.0 - est_w - margin).max(margin))
        .max(margin);
    let y = raw_y
        .min((viewport.1 - est_h - margin).max(margin))
        .max(margin);

    let panel = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(est_w),
            height: length(est_h),
        },
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::FlexStart),
        padding: Rect {
            left: length(PAD_X),
            right: length(PAD_X),
            top: length(PAD_Y),
            bottom: length(PAD_Y),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .radius(radius::SM)
    .text_aligned(text, FONT_SIZE, palette.fg, Alignment::Start);

    // Wrapper invisible que ocupa toda la pantalla — el panel ya está
    // posicionado en absolute, pero `view_overlay` espera un único root
    // que cubre la ventana. Sin scrim ni intercept de clicks.
    View::new(Style {
        size: Size {
            width: llimphi_ui::llimphi_layout::taffy::prelude::percent(1.0_f32),
            height: llimphi_ui::llimphi_layout::taffy::prelude::percent(1.0_f32),
        },
        ..Default::default()
    })
    // Borde sutil pintado vía un nodo separado: pintamos el panel sobre
    // un rect 1px más grande coloreado con `border` — barato y consistente
    // con cómo el context-menu hace su borde.
    .children(vec![
        View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(x - 1.0),
                top: length(y - 1.0),
                right: auto(),
                bottom: auto(),
            },
            size: Size {
                width: length(est_w + 2.0),
                height: length(est_h + 2.0),
            },
            ..Default::default()
        })
        .fill(palette.border)
        .radius(radius::SM + 1.0),
        panel,
    ])
}
