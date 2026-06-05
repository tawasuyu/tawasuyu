//! `llimphi-widget-color-picker` — selector de color RGBA agnóstico.
//!
//! Compone tres piezas, de arriba abajo:
//! 1. un **swatch** del color actual,
//! 2. una **paleta de chips** preestablecidos (clic = fija el RGB conservando
//!    el alfa actual), envuelta si no entra en una fila,
//! 3. cuatro **sliders RGBA** para el ajuste fino.
//!
//! Es agnóstico: no sabe de config ni de `FieldValue`. Recibe el color como
//! `[u8; 4]` y emite el color nuevo por `on_change([u8; 4]) -> Msg`. Cualquier
//! app llimphi lo usa pasando su propio `Msg`.
//!
//! ```ignore
//! color_picker_view(
//!     self.border,
//!     DEFAULT_SWATCHES,
//!     &ColorPickerPalette::from_theme(&theme),
//!     |rgba| Msg::SetBorderColor(rgba),
//! )
//! ```

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    FlexWrap, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{DragPhase, View};
use llimphi_widget_slider::{slider_view, SliderPalette};

/// Paleta del color-picker: la del slider RGBA + los bordes de los chips.
#[derive(Debug, Clone, Copy)]
pub struct ColorPickerPalette {
    /// Paleta de los sliders RGBA.
    pub slider: SliderPalette,
    /// Borde de un chip inactivo.
    pub chip_border: Color,
    /// Borde del chip activo (el que coincide con el color actual).
    pub chip_border_active: Color,
}

impl Default for ColorPickerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl ColorPickerPalette {
    /// Construye la paleta desde un `Theme` semántico.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            slider: SliderPalette::from_theme(t),
            chip_border: t.border,
            chip_border_active: t.accent,
        }
    }
}

/// Paleta de colores preestablecidos: grises + una rampa de tonos saturados,
/// los típicos para marcos/acentos. El caller puede pasar la suya propia.
pub const DEFAULT_SWATCHES: &[[u8; 3]] = &[
    [0xEC, 0xEC, 0xEC],
    [0x9E, 0x9E, 0x9E],
    [0x42, 0x42, 0x42],
    [0x5C, 0x8F, 0xEB],
    [0x00, 0xBC, 0xD4],
    [0x4C, 0xAF, 0x50],
    [0xFF, 0xC1, 0x07],
    [0xFF, 0x98, 0x00],
    [0xF4, 0x43, 0x36],
    [0xE9, 0x1E, 0x63],
    [0x9C, 0x27, 0xB0],
    [0x79, 0x55, 0x48],
];

/// Alto fijo del picker (px): swatch + paleta (hasta 2 filas) + 4 sliders.
/// Útil para que un contenedor con scroll estime el alto del control.
pub fn color_picker_height() -> f32 {
    16.0 + 54.0 + 4.0 * 24.0
}

/// Compone el selector completo. `rgba` es el color actual; `swatches` la paleta
/// de chips (p. ej. [`DEFAULT_SWATCHES`]); `on_change` recibe el color nuevo.
pub fn color_picker_view<Msg, F>(
    rgba: [u8; 4],
    swatches: &[[u8; 3]],
    palette: &ColorPickerPalette,
    on_change: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn([u8; 4]) -> Msg + Clone + Send + Sync + 'static,
{
    let mut rows: Vec<View<Msg>> = Vec::with_capacity(6);
    rows.push(swatch_view(rgba));
    rows.push(swatch_palette(rgba, swatches, palette, &on_change));
    for (ci, name) in [(0usize, "R"), (1, "G"), (2, "B"), (3, "A")] {
        let f = on_change.clone();
        rows.push(slider_view(
            name.to_string(),
            rgba[ci] as f32,
            0.0,
            255.0,
            &palette.slider,
            move |phase, dv| match phase {
                DragPhase::Move => {
                    let nv = (rgba[ci] as f64 + dv as f64).clamp(0.0, 255.0) as u8;
                    let mut c = rgba;
                    c[ci] = nv;
                    Some(f(c))
                }
                DragPhase::End => None,
            },
        ));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(rows)
}

/// El swatch (muestra) del color actual.
fn swatch_view<Msg: Clone + 'static>(rgba: [u8; 4]) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(40.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]))
    .radius(3.0)
}

/// La fila (envuelta) de chips de la paleta.
fn swatch_palette<Msg, F>(
    cur: [u8; 4],
    swatches: &[[u8; 3]],
    palette: &ColorPickerPalette,
    on_change: &F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn([u8; 4]) -> Msg + Clone + Send + Sync + 'static,
{
    let chips: Vec<View<Msg>> = swatches
        .iter()
        .map(|rgb| swatch_chip(*rgb, cur, palette, on_change))
        .collect();
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(5.0_f32),
            height: length(5.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(3.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(chips)
}

/// Un chip de la paleta: cuadrado clickeable. Si su RGB coincide con el color
/// actual, lleva borde de acento.
fn swatch_chip<Msg, F>(
    rgb: [u8; 3],
    cur: [u8; 4],
    palette: &ColorPickerPalette,
    on_change: &F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn([u8; 4]) -> Msg + Clone + Send + Sync + 'static,
{
    let active = cur[0] == rgb[0] && cur[1] == rgb[1] && cur[2] == rgb[2];
    // Conserva el alfa actual al elegir un chip.
    let new_color = [rgb[0], rgb[1], rgb[2], cur[3]];
    View::new(Style {
        size: Size {
            width: length(22.0_f32),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255))
    .radius(5.0)
    .border(
        if active { 2.0 } else { 1.0 },
        if active {
            palette.chip_border_active
        } else {
            palette.chip_border
        },
    )
    .on_click(on_change(new_color))
}
