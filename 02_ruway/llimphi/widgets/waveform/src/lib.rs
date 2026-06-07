//! `llimphi-widget-waveform` — visor de **forma de onda en vivo**.
//!
//! Pattern análogo a [`llimphi-widget-timeline`](
//! https://docs.rs/llimphi-widget-timeline): el widget **no mantiene
//! estado** del audio (ni cpal, ni `AudioProbe`, ni ringbuffer). El caller
//! le pasa un closure `Fn(&mut Vec<f32>) -> u16` que rellena un buffer con
//! los últimos samples y devuelve cuántos **canales** intercalados trae;
//! el widget hace el fold a mono y dibuja un **envelope min/max por
//! columna** (polígono cerrado con relleno tenue + stroke por arriba y
//! por abajo) sobre una **línea central** que siempre está presente como
//! "ground" del visor. Sin handlers de mouse — paint-only.
//!
//! ```text
//!   ┌─────────────────────────────────────────────────┐
//!   │   ▄▄▄    ▄  ▄▄▄  ▄▄    ▄▄  ▄                    │
//!   │  ▄███▄  ▄█▄▄███▄▄██▄▄▄▄██▄▄█▄                   │
//!   │──█████──███████████████████████─── centro ──────│
//!   │  ▀███▀  ▀█▀▀███▀▀██▀▀▀▀██▀▀█▀                   │
//!   │   ▀▀▀    ▀  ▀▀▀  ▀▀    ▀▀  ▀                    │
//!   └─────────────────────────────────────────────────┘
//! ```
//!
//! Uso típico (reproductor con audio probe):
//!
//! ```ignore
//! use std::sync::Arc;
//! let probe = audio_probe();        // Arc<AudioProbe> propia de la app
//! let palette = WaveformPalette::default();
//! waveform_view(
//!     move |out| {
//!         let (_sr, ch) = probe.snapshot(out);
//!         ch                         // canales intercalados
//!     },
//!     &palette,
//! )
//! ```
//!
//! Si el closure devuelve `0` canales (o no llena el buffer), el widget
//! pinta sólo la línea central — útil para mostrar "visor vivo, sin
//! señal" cuando el dispositivo de captura todavía no levantó.

#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use llimphi_ui::llimphi_layout::taffy::prelude::{auto, percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::View;

/// Paleta + dimensiones del visor de waveform.
///
/// El `bg`/`radius` se usan como `fill`/`radius` del nodo contenedor; el
/// `center`/`stroke`/`fill` se pintan dentro de `paint_with`. Los paddings
/// definen el margen interior — la onda no toca los bordes redondeados.
#[derive(Debug, Clone, Copy)]
pub struct WaveformPalette {
    /// Fondo del recuadro (se pinta como `fill` del nodo).
    pub bg: Color,
    /// Color de la línea central (ground del visor).
    pub center: Color,
    /// Color del contorno top/bot del envelope.
    pub stroke: Color,
    /// Color del relleno del envelope (típicamente `stroke` con alfa bajo).
    pub fill: Color,
    /// Radio de las esquinas del recuadro.
    pub radius: f64,
    /// Padding horizontal interior (px) — margen entre el borde y la onda.
    pub pad_x: f32,
    /// Padding vertical interior (px).
    pub pad_y: f32,
    /// Grosor del stroke del envelope (px).
    pub stroke_w: f32,
}

impl Default for WaveformPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl WaveformPalette {
    /// Construye la paleta desde un `Theme` semántico. El relleno del
    /// envelope se deriva del `accent` con alfa bajo para que se vea como
    /// "halo" sin pelear con el stroke.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        let accent = t.accent;
        let [r, g, b, _] = accent.components;
        Self {
            bg: t.bg_panel_alt,
            center: t.fg_muted,
            stroke: accent,
            // Mismo color que el stroke pero con alfa bajo (≈0.27) para
            // que el envelope se vea como un halo sin pelear con el contorno.
            fill: Color::from_rgba8(
                (r * 255.0) as u8,
                (g * 255.0) as u8,
                (b * 255.0) as u8,
                70,
            ),
            radius: 8.0,
            pad_x: 12.0,
            pad_y: 8.0,
            stroke_w: 1.2,
        }
    }
}

/// Compone el visor de waveform.
///
/// `source` se invoca **una vez por frame** dentro del `paint_with`: el
/// caller lo usa para rellenar el buffer con los últimos samples
/// intercalados y devolver cuántos canales trae. Si devuelve `0` (o el
/// buffer queda vacío) el widget pinta sólo la línea central. El widget
/// es stateless: redibujá pasando el mismo closure cada frame y la onda
/// avanza sola con cada snapshot nuevo.
///
/// El widget ocupa el espacio que le dé el padre (`width: auto`, `height:
/// 100%`); para que crezca dentro de una fila/columna del padre, el
/// caller lo envuelve con un `flex_grow: 1.0`.
pub fn waveform_view<Msg, F>(source: F, palette: &WaveformPalette) -> View<Msg>
where
    Msg: 'static,
    F: Fn(&mut Vec<f32>) -> u16 + Send + Sync + 'static,
{
    let pal = *palette;
    // Buffer scratch: se reusa entre frames para no realocar. Es seguro
    // tenerlo en un `Arc<Mutex>` porque `paint_with` corre en el hilo de
    // UI (un sólo painter activo por frame).
    let scratch: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));

    View::new(Style {
        size: Size {
            width: auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(pal.bg)
    .radius(pal.radius)
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 {
            return;
        }
        let inner_x = rect.x + pal.pad_x;
        let inner_y = rect.y + pal.pad_y;
        let inner_w = (rect.w - 2.0 * pal.pad_x).max(1.0);
        let inner_h = (rect.h - 2.0 * pal.pad_y).max(1.0);
        let mid_y = inner_y + inner_h * 0.5;

        // Línea central — siempre presente, hace de "ground" del visor.
        let mut center = BezPath::new();
        center.move_to((inner_x as f64, mid_y as f64));
        center.line_to(((inner_x + inner_w) as f64, mid_y as f64));
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            pal.center,
            None,
            &center,
        );

        let mut snap = scratch.lock().unwrap_or_else(|p| p.into_inner());
        let channels = source(&mut snap).max(1) as usize;
        let total_frames = snap.len() / channels;
        if total_frames < 2 {
            return;
        }

        // Envelope min/max por columna: para cada bucket de frames
        // guardamos el mínimo y el máximo del mono fold y dibujamos la
        // forma como un polígono cerrado (relleno tenue + stroke top/bot).
        // Da mucho más "cuerpo" que la línea pico-sólo.
        let cols = (inner_w.max(2.0) as usize).min(total_frames);
        let frames_per_col = total_frames / cols.max(1);
        if frames_per_col == 0 {
            return;
        }
        let amp = inner_h * 0.5;
        let denom = (cols as f32 - 1.0).max(1.0);

        let mut top = BezPath::new();
        let mut bot = BezPath::new();
        let mut envelope = BezPath::new();
        // Pasada hacia adelante: top + arranca envelope por el borde
        // superior. Cacheamos los mínimos para no recorrer el buffer dos
        // veces.
        let mut mins = Vec::with_capacity(cols);
        for col in 0..cols {
            let f0 = col * frames_per_col;
            let f1 = ((col + 1) * frames_per_col).min(total_frames);
            let mut vmin = f32::INFINITY;
            let mut vmax = f32::NEG_INFINITY;
            for f in f0..f1 {
                let mut acc = 0.0_f32;
                for ch in 0..channels {
                    acc += snap[f * channels + ch];
                }
                let v = (acc / channels as f32).clamp(-1.0, 1.0);
                if v < vmin {
                    vmin = v;
                }
                if v > vmax {
                    vmax = v;
                }
            }
            mins.push(vmin);
            let x = inner_x + (col as f32 / denom) * inner_w;
            let y_top = mid_y - vmax * amp;
            let y_bot = mid_y - vmin * amp;
            if col == 0 {
                top.move_to((x as f64, y_top as f64));
                bot.move_to((x as f64, y_bot as f64));
                envelope.move_to((x as f64, y_top as f64));
            } else {
                top.line_to((x as f64, y_top as f64));
                bot.line_to((x as f64, y_bot as f64));
                envelope.line_to((x as f64, y_top as f64));
            }
        }
        // Cierre del envelope: volvé por la línea de mínimos en sentido
        // inverso (sin recorrer samples otra vez — los mins ya están).
        for col in (0..cols).rev() {
            let x = inner_x + (col as f32 / denom) * inner_w;
            let y_bot = mid_y - mins[col] * amp;
            envelope.line_to((x as f64, y_bot as f64));
        }
        envelope.close_path();

        scene.fill(Fill::NonZero, Affine::IDENTITY, pal.fill, None, &envelope);
        let stroke = Stroke::new(pal.stroke_w as f64);
        scene.stroke(&stroke, Affine::IDENTITY, pal.stroke, None, &top);
        scene.stroke(&stroke, Affine::IDENTITY, pal.stroke, None, &bot);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_theme_usa_colores_semanticos() {
        let t = llimphi_theme::Theme::dark();
        let p = WaveformPalette::from_theme(&t);
        assert_eq!(p.bg, t.bg_panel_alt);
        assert_eq!(p.center, t.fg_muted);
        assert_eq!(p.stroke, t.accent);
        // fill = accent con alfa bajo (~0.27).
        let [r0, g0, b0, _] = t.accent.components;
        let [r1, g1, b1, a1] = p.fill.components;
        // Componentes RGB iguales módulo el roundtrip f32→u8→f32.
        assert!((r0 - r1).abs() < 0.01);
        assert!((g0 - g1).abs() < 0.01);
        assert!((b0 - b1).abs() < 0.01);
        // Alfa ≈ 70/255 ≈ 0.274.
        assert!((a1 - 70.0 / 255.0).abs() < 0.005);
    }

    #[test]
    fn construye_sin_panic_sin_senal() {
        // Closure que reporta 0 canales => sólo pinta la línea central.
        let pal = WaveformPalette::default();
        let _ = waveform_view::<(), _>(|_| 0, &pal);
    }

    #[test]
    fn construye_con_senal_mono() {
        let pal = WaveformPalette::default();
        let _ = waveform_view::<(), _>(
            |out| {
                out.clear();
                for i in 0..1024 {
                    out.push(((i as f32) * 0.01).sin());
                }
                1
            },
            &pal,
        );
    }

    #[test]
    fn construye_con_senal_estereo() {
        let pal = WaveformPalette::default();
        let _ = waveform_view::<(), _>(
            |out| {
                out.clear();
                for i in 0..512 {
                    let t = i as f32 * 0.02;
                    out.push(t.sin());
                    out.push((t * 1.5).sin());
                }
                2
            },
            &pal,
        );
    }
}
