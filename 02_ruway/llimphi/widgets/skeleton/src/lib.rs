//! `llimphi-widget-skeleton` — placeholder de carga con shimmer.
//!
//! Cuando una pantalla está cargando contenido cuya forma es predecible
//! (ej. una lista de 5 cards, un avatar+nombre+timestamp), un skeleton
//! es más informativo que un spinner: el usuario ya ve QUÉ vendrá,
//! sólo no tiene los valores reales todavía.
//!
//! El brillo (shimmer) viene de una **banda de gradiente que cruza** el
//! rect de izquierda a derecha cíclicamente. Los stops son
//! `[low, high, low]` sobre una franja del ~50% del ancho, con `Extend::Pad`
//! por default — fuera de la banda el rect queda en `low`, dentro el
//! `high` pinta el destello. Es el patrón canónico de Material/Apple/
//! sistemas modernos, más legible que la oscilación uniforme previa.
//!
//! Como `spinner`, requiere que la app fuerce redraws periódicos para
//! que la animación corra (típico: `Handle::spawn_periodic(50ms, …)`
//! mientras hay skeletons visibles).

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

/// Período del shimmer en segundos — un ciclo completo de la banda
/// cruzando el rect. 1.4s es el sweet spot: rápido para señalar
/// "esto se está cargando", lento para no marear.
const SHIMMER_CYCLE_SECS: f32 = 1.4;
/// Ancho de la banda como fracción del ancho del rect. 50% da una
/// transición suave; bajar a 30% da un destello más puntual.
const SHIMMER_BAND_FRAC: f64 = 0.5;
/// Ancho mínimo absoluto de la banda — evita que en skeletons cortos
/// (avatares chicos, line skeletons de ~80px) el destello sea un
/// pixel apretado.
const SHIMMER_BAND_MIN_PX: f64 = 40.0;

/// Bloque rectangular animado. La altura y forma viene del `Style`
/// que pasa el caller — el skeleton sólo aporta el `fill` animado.
///
/// Devuelve un `View` con `paint_with` que pinta una banda de
/// gradiente atravesando el rect. Para usarlo dentro de un layout
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
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, RoundedRect};
        use llimphi_ui::llimphi_raster::peniko::{Fill, Gradient};

        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }

        // Progress del ciclo en [0, 1).
        let elapsed = started.elapsed().as_secs_f32();
        let progress = (elapsed / SHIMMER_CYCLE_SECS).fract() as f64;

        // Banda: ancho relativo al rect (con floor mínimo) que arranca
        // a la izquierda del rect y termina a la derecha. Distancia
        // total recorrida = rect.w + band_w, así el destello entra y
        // sale por completo.
        let rect_w = rect.w as f64;
        let band_w = (rect_w * SHIMMER_BAND_FRAC).max(SHIMMER_BAND_MIN_PX);
        let travel = rect_w + band_w;
        let band_left = rect.x as f64 - band_w + progress * travel;
        let band_right = band_left + band_w;
        let cy = (rect.y + rect.h * 0.5) as f64;

        // Single fill: gradient lineal con stops [low, high, low]. Fuera
        // de [band_left, band_right] el Extend::Pad (default de peniko)
        // extiende los stops endpoint — ambos `low` — así el resto del
        // rect queda en `low` sin necesidad de un fill base separado.
        let rr = RoundedRect::new(
            rect.x as f64,
            rect.y as f64,
            (rect.x + rect.w) as f64,
            (rect.y + rect.h) as f64,
            radius::SM,
        );
        let gradient = Gradient::new_linear(
            Point::new(band_left, cy),
            Point::new(band_right, cy),
        )
        .with_stops([p.low, p.high, p.low].as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &gradient, None, &rr);
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

