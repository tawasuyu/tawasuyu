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
    AlignItems, FlexWrap, Rect,
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

/// Alto de la barra de tono (px).
const HUE_BAR_H: f32 = 16.0;

/// Alto fijo del picker (px): swatch + paleta (hasta 2 filas) + barra de tono +
/// 4 sliders. Útil para que un contenedor con scroll estime el alto del control.
pub fn color_picker_height() -> f32 {
    16.0 + 54.0 + (HUE_BAR_H + 6.0) + 4.0 * 24.0
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
    let mut rows: Vec<View<Msg>> = Vec::with_capacity(7);
    rows.push(swatch_view(rgba));
    rows.push(swatch_palette(rgba, swatches, palette, &on_change));
    rows.push(hue_bar(rgba, palette, on_change.clone()));
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

/// La **barra de tono** (HSV): un degradé del arcoíris arrastrable. Mover el
/// cursor cambia sólo el tono (H), conservando saturación, valor y alfa. Un
/// thumb marca el tono actual. Si el color es un gris (S≈0) se asume S=1 al
/// pintar para que de un gris se pueda "entrar" a un color.
fn hue_bar<Msg, F>(rgba: [u8; 4], palette: &ColorPickerPalette, on_change: F) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn([u8; 4]) -> Msg + Clone + Send + Sync + 'static,
{
    let width = palette.slider.track_width.max(1.0);
    let (h, mut s, mut v) = rgb_to_hsv([rgba[0], rgba[1], rgba[2]]);
    if s < 0.02 {
        // Desde un gris/blanco, dejar entrar a un tono saturado.
        s = 1.0;
        if v < 0.02 {
            v = 1.0;
        }
    }
    let alpha = rgba[3];
    let thumb_ratio = h / 360.0;

    // Handler de drag: dx → dh (proporcional al ancho), nuevo tono.
    let handler = move |phase: DragPhase, dx: f32, _dy: f32| -> Option<Msg> {
        match phase {
            DragPhase::Move => {
                let dh = dx / width * 360.0;
                let nh = (h + dh).rem_euclid(360.0);
                let [r, g, b] = hsv_to_rgb(nh, s, v);
                Some(on_change([r, g, b, alpha]))
            }
            DragPhase::End => None,
        }
    };

    View::new(Style {
        size: Size {
            width: length(width),
            height: length(HUE_BAR_H),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .radius(4.0)
    .draggable(handler)
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, RoundedRect};
        use llimphi_ui::llimphi_raster::peniko::{Fill, Gradient};
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let x0 = rect.x as f64;
        let y0 = rect.y as f64;
        let x1 = (rect.x + rect.w) as f64;
        let y1 = (rect.y + rect.h) as f64;
        let rr = RoundedRect::new(x0, y0, x1, y1, 4.0);
        // Degradé del arcoíris: 7 paradas (rojo→amarillo→verde→cian→azul→
        // magenta→rojo) distribuidas parejo.
        let stops = [
            Color::from_rgba8(255, 0, 0, 255),
            Color::from_rgba8(255, 255, 0, 255),
            Color::from_rgba8(0, 255, 0, 255),
            Color::from_rgba8(0, 255, 255, 255),
            Color::from_rgba8(0, 0, 255, 255),
            Color::from_rgba8(255, 0, 255, 255),
            Color::from_rgba8(255, 0, 0, 255),
        ];
        let g = Gradient::new_linear(Point::new(x0, y0), Point::new(x1, y0))
            .with_stops(stops.as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rr);
        // Thumb: línea vertical blanca en la posición del tono.
        let tx = x0 + (x1 - x0) * thumb_ratio as f64;
        let thumb = RoundedRect::new(tx - 1.5, y0 - 1.0, tx + 1.5, y1 + 1.0, 1.5);
        let white = Color::from_rgba8(255, 255, 255, 230);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &white, None, &thumb);
    })
}

/// Convierte RGB (`[u8;3]`) a HSV: `(h en 0..360, s en 0..1, v en 0..1)`.
fn rgb_to_hsv(rgb: [u8; 3]) -> (f32, f32, f32) {
    let r = rgb[0] as f32 / 255.0;
    let g = rgb[1] as f32 / 255.0;
    let b = rgb[2] as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let v = max;
    let s = if max <= 0.0 { 0.0 } else { d / max };
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h.rem_euclid(360.0), s, v)
}

/// Convierte HSV (`h en 0..360, s,v en 0..1`) a RGB (`[u8;3]`).
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to_u8 = |t: f32| ((t + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [to_u8(r1), to_u8(g1), to_u8(b1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_roundtrip_colores_puros() {
        for rgb in [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0], [0, 255, 255]] {
            let (h, s, v) = rgb_to_hsv(rgb);
            assert_eq!(hsv_to_rgb(h, s, v), rgb, "roundtrip {rgb:?}");
        }
    }

    #[test]
    fn gris_tiene_saturacion_cero() {
        let (_, s, v) = rgb_to_hsv([128, 128, 128]);
        assert!(s < 0.01);
        assert!((v - 128.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn tono_rojo_es_cero_grados() {
        let (h, _, _) = rgb_to_hsv([255, 0, 0]);
        assert!(h < 0.5 || h > 359.5);
    }
}
