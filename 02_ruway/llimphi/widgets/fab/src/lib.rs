//! `llimphi-widget-fab` — Floating Action Button.
//!
//! Botón circular elevado pensado para la **acción primaria** de una
//! pantalla (componer, nuevo, +, capturar). Heredamos el patrón
//! Material/Flutter: rest sobre sombra E3, círculo del color `accent`,
//! glyph blanco centrado, sombra que **respira** al hover (sube a E5).
//!
//! La firma cinética viene del tween de fill+shadow vía `View::animated`.

#![forbid(unsafe_code)]

use llimphi_ui::Shadow;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::{elevation, motion, Theme};

/// Tamaño del FAB. El estándar Material es 56 px; el "mini" 40 px;
/// "extended" lleva texto + ícono y crece el ancho (no implementado
/// como variante separada para mantener la API mínima — quien lo
/// necesite usa `fab_styled`).
#[derive(Debug, Clone, Copy)]
pub enum FabSize {
    Regular,
    Mini,
}

impl FabSize {
    pub fn px(self) -> f32 {
        match self {
            FabSize::Regular => 56.0,
            FabSize::Mini => 40.0,
        }
    }
}

/// Paleta del FAB.
#[derive(Debug, Clone, Copy)]
pub struct FabPalette {
    /// Fill del círculo (idle).
    pub bg: Color,
    /// Color del glyph.
    pub fg: Color,
}

impl FabPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            bg: t.accent,
            // Texto sobre accent: blanco (los accents del repo son todos
            // suficientemente saturados para hacer contraste).
            fg: Color::from_rgba8(255, 255, 255, 255),
        }
    }
}

/// Compone el FAB. `key` debe ser estable para que la anim de hover
/// quede vinculada al mismo nodo entre frames.
pub fn fab_view<Msg: Clone + 'static>(
    glyph: impl Into<String>,
    size: FabSize,
    key: u64,
    palette: &FabPalette,
    on_click: Msg,
) -> View<Msg> {
    let s = size.px();
    let (a, blur, dy) = elevation::E3;
    let shadow = Shadow {
        color: Color::from_rgba8(0, 0, 0, a),
        blur,
        dx: 0.0,
        dy,
        spread: 0.0,
    };
    View::new(Style {
        size: Size { width: length(s), height: length(s) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(palette.bg)
    .radius((s as f64) * 0.5)
    .shadow(shadow)
    .animated(key, motion::FAST)
    .text_aligned(
        glyph.into(),
        (s * 0.42).round(),
        palette.fg,
        Alignment::Center,
    )
    .on_click(on_click)
    .cursor(llimphi_ui::Cursor::Pointer)
}

/// FAB con texto + glyph (Extended FAB de Material). Pildora ancha en
/// vez de círculo.
pub fn fab_extended<Msg: Clone + 'static>(
    label: impl Into<String>,
    key: u64,
    palette: &FabPalette,
    on_click: Msg,
) -> View<Msg> {
    let h = 48.0_f32;
    let (a, blur, dy) = elevation::E3;
    let shadow = Shadow {
        color: Color::from_rgba8(0, 0, 0, a),
        blur,
        dx: 0.0,
        dy,
        spread: 0.0,
    };
    View::new(Style {
        size: Size {
            width: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
            height: length(h),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: llimphi_ui::llimphi_layout::taffy::Rect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .radius((h as f64) * 0.5)
    .shadow(shadow)
    .animated(key, motion::FAST)
    .text_aligned(label.into(), 14.0, palette.fg, Alignment::Center)
    .on_click(on_click)
    .cursor(llimphi_ui::Cursor::Pointer)
}
