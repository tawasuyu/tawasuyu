//! `llimphi-widget-skeleton` — placeholder de carga con shimmer.
//!
//! Cuando una pantalla está cargando contenido cuya forma es predecible
//! (ej. una lista de 5 cards, un avatar+nombre+timestamp), un skeleton
//! es más informativo que un spinner: el usuario ya ve QUÉ vendrá,
//! sólo no tiene los valores reales todavía.
//!
//! El brillo (shimmer) viene de pintar dos colores que se interpolan
//! según un seno del `Instant::now()`. Como `spinner`, requiere que la
//! app fuerce redraws periódicos para que la animación corra (típico:
//! `Handle::spawn_periodic(50ms, …)` mientras hay skeletons visibles).

#![forbid(unsafe_code)]

use std::time::Instant;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Size, Style},
    Position,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_theme::{radius, Theme};

/// Paleta del skeleton — dos tonos entre los que oscila.
#[derive(Debug, Clone, Copy)]
pub struct SkeletonPalette {
    pub low: Color,
    pub high: Color,
}

impl SkeletonPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            low: t.bg_panel_alt,
            high: t.bg_button_hover,
        }
    }
}

/// Bloque rectangular animado. La altura y forma viene del `Style`
/// que pasa el caller — el skeleton sólo aporta el `fill` animado.
///
/// Devuelve un `View` con `paint_with` que pinta un rect del color
/// interpolado encima de su rect. Para usarlo dentro de un layout
/// con tamaño definido, envolvelo en un contenedor con el `Style`
/// adecuado.
pub fn skeleton_view<Msg: Clone + 'static>(palette: &SkeletonPalette) -> View<Msg> {
    let started = Instant::now();
    let p = *palette;
    View::new(Style {
        position: Position::Absolute,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, RoundedRect};
        use llimphi_ui::llimphi_raster::peniko::Fill;

        // Oscilación seno → [0, 1] con período ~1.4s
        let elapsed = started.elapsed().as_secs_f32();
        let t = (elapsed * std::f32::consts::TAU / 1.4).sin() * 0.5 + 0.5;
        let color = lerp_color(p.low, p.high, t);
        let rr = RoundedRect::new(
            rect.x as f64,
            rect.y as f64,
            (rect.x + rect.w) as f64,
            (rect.y + rect.h) as f64,
            radius::SM,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rr);
    })
}

/// Caja con tamaño explícito (ancho + alto en px) + skeleton adentro.
/// Helper para casos comunes: line skeleton (`skeleton_line_view(160)`).
pub fn skeleton_box_view<Msg: Clone + 'static>(
    width_px: f32,
    height_px: f32,
    palette: &SkeletonPalette,
) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(width_px),
            height: length(height_px),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![skeleton_view(palette)])
}

/// Línea horizontal típica para texto en carga (height fijo ~12px).
pub fn skeleton_line_view<Msg: Clone + 'static>(
    width_px: f32,
    palette: &SkeletonPalette,
) -> View<Msg> {
    skeleton_box_view(width_px, 12.0, palette)
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let [r0, g0, b0, a0] = a.components;
    let [r1, g1, b1, a1] = b.components;
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    AlphaColor::new([
        r0 + (r1 - r0) * t,
        g0 + (g1 - g0) * t,
        b0 + (b1 - b0) * t,
        a0 + (a1 - a0) * t,
    ])
}
