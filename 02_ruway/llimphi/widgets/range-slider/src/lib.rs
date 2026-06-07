//! `llimphi-widget-range-slider` — slider de dos thumbs sobre un track.
//!
//! Análogo a `RangeSlider` de Flutter o `RangeSlider` de Material 3.
//! El caller mantiene `(lo, hi)` en su modelo (fracciones en `[0,1]`);
//! el widget reporta el nuevo valor por `on_change(lo, hi)` cuando el
//! usuario arrastra cualquiera de los dos thumbs.
//!
//! Diseño:
//! - Track de 4 px, franja activa del color `accent` entre los dos thumbs.
//! - Thumbs de 14 px (círculos), borde 2 px del color `accent`,
//!   sombra E1 — la firma de elevación canónica.
//! - Drag con `draggable` por thumb → callback recibe (lo, hi)
//!   normalizado y monotónicamente ordenado (el bajo nunca supera al alto).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Size, Style},
    Position,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{DragPhase, Shadow, View};
use llimphi_theme::{elevation, Theme};

/// Paleta del range slider.
#[derive(Debug, Clone, Copy)]
pub struct RangeSliderPalette {
    pub track_idle: Color,
    pub track_active: Color,
    pub thumb_fill: Color,
    pub thumb_stroke: Color,
}

impl RangeSliderPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            track_idle: t.border,
            track_active: t.accent,
            thumb_fill: t.bg_panel,
            thumb_stroke: t.accent,
        }
    }
}

/// Compone un range slider de ancho `width_px`, alto 28 px.
/// `lo`/`hi` son fracciones en `[0,1]`. `on_change(lo, hi)` se dispara
/// con cada delta de drag.
pub fn range_slider_view<Msg, F>(
    lo: f32,
    hi: f32,
    width_px: f32,
    palette: &RangeSliderPalette,
    on_change: F,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(f32, f32) -> Msg + Clone + Send + Sync + 'static,
{
    let lo = lo.clamp(0.0, 1.0);
    let hi = hi.clamp(0.0, 1.0);
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };

    let (a, blur, dy) = elevation::E1;
    let thumb_shadow = Shadow {
        color: Color::from_rgba8(0, 0, 0, a),
        blur,
        dx: 0.0,
        dy,
        spread: 0.0,
    };

    let track_w = width_px.max(1.0);
    let lo_x = lo * track_w;
    let hi_x = hi * track_w;
    let active_w = (hi_x - lo_x).max(0.0);

    let track_idle = View::new(Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0_f32), height: length(4.0_f32) },
        inset: llimphi_ui::llimphi_layout::taffy::Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(12.0_f32),
            bottom: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
        },
        ..Default::default()
    })
    .fill(palette.track_idle)
    .radius(2.0);

    let track_active = View::new(Style {
        position: Position::Absolute,
        size: Size { width: length(active_w), height: length(4.0_f32) },
        inset: llimphi_ui::llimphi_layout::taffy::Rect {
            left: length(lo_x),
            right: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
            top: length(12.0_f32),
            bottom: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
        },
        ..Default::default()
    })
    .fill(palette.track_active)
    .radius(2.0);

    let mk_thumb = |left_px: f32,
                    is_lo: bool,
                    on_change: F,
                    lo: f32,
                    hi: f32,
                    track_w: f32| {
        View::new(Style {
            position: Position::Absolute,
            size: Size { width: length(14.0_f32), height: length(14.0_f32) },
            inset: llimphi_ui::llimphi_layout::taffy::Rect {
                left: length(left_px - 7.0),
                right: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
                top: length(7.0_f32),
                bottom: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
            },
            ..Default::default()
        })
        .fill(palette.thumb_fill)
        .radius(7.0)
        .border(2.0, palette.thumb_stroke)
        .shadow(thumb_shadow)
        .draggable(move |phase: DragPhase, dx: f32, _dy: f32| match phase {
            DragPhase::Move => {
                let dfrac = dx / track_w;
                let (new_lo, new_hi) = if is_lo {
                    let nl = (lo + dfrac).clamp(0.0, hi);
                    (nl, hi)
                } else {
                    let nh = (hi + dfrac).clamp(lo, 1.0);
                    (lo, nh)
                };
                Some(on_change(new_lo, new_hi))
            }
            DragPhase::End => None,
        })
        .cursor(llimphi_ui::Cursor::Pointer)
    };

    let thumb_lo = mk_thumb(lo_x, true, on_change.clone(), lo, hi, track_w);
    let thumb_hi = mk_thumb(hi_x, false, on_change, lo, hi, track_w);

    View::new(Style {
        size: Size { width: length(width_px), height: length(28.0_f32) },
        ..Default::default()
    })
    .children(vec![track_idle, track_active, thumb_lo, thumb_hi])
}
