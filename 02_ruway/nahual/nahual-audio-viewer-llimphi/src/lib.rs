//! `nahual-audio-viewer-llimphi` — reproductor/visor de audio.
//!
//! Quinto visor del shell meta-app (tras texto/imagen/video/card). Abre
//! un archivo de audio (WAV/MP3/FLAC/Opus/Vorbis vía `media-source-*`),
//! lo reproduce por el sink cpal (`media-audio-cpal`) y pinta un
//! **espectro en vivo** — bandas log-espaciadas calculadas con el
//! `Spectrum` (Goertzel) de `media-core` sobre los samples que un
//! `AudioProbe` tapa del stream realtime.
//!
//! ## Cómo se sostiene el stream
//!
//! El [`AudioSink`] envuelve un `cpal::Stream` que es `!Send`/`!Sync` —
//! por eso vive **dentro** del estado del visor (que la app guarda en su
//! `Model`, sólo `'static`, no `Send`). Soltar el `AudioViewerState`
//! (cambiar de archivo, navegar a otra cosa) dropea el sink y para el
//! audio. No hay statics ni leaks: un visor = un stream.
//!
//! ## Posición
//!
//! La cadena `AudioSource → sink` está type-erased detrás de un
//! `Arc<Mutex<dyn AudioSource>>`, así que el visor NO lee `Seekable` de
//! la fuente: estima el playhead con su propio reloj (acumula `dt` en
//! [`AudioViewerState::tick`], como el video viewer). Es suficiente para
//! un meter; el seek real llegará cuando la cadena exponga `Seekable`.

#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KurboRect};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

use media_audio_cpal::AudioSink;
use media_core::{AudioProbe, AudioSource, Pause, PausableAudio, ProbedAudioSource, Spectrum};

/// Cantidad de bandas del espectro y su rango (Hz). 48 bandas entre
/// 40 Hz y 16 kHz es un compromiso legible: cubre el grueso musical sin
/// barras tan finas que no se distingan.
const SPECTRUM_BANDS: usize = 48;
const SPECTRUM_FMIN: f32 = 40.0;
const SPECTRUM_FMAX: f32 = 16_000.0;
/// Capacidad del ring del probe (samples intercalados). ≈ 8k da una
/// ventana de ~85 ms a 48 kHz stereo — responsivo sin titilar.
const PROBE_CAPACITY: usize = 8 * 1024;

/// Estado del reproductor de audio. No es `Clone` (ni `Send`): contiene
/// el `AudioSink`/`cpal::Stream`. La app lo guarda en su `Model`.
pub struct AudioViewerState {
    /// Mantiene vivo el stream cpal. `None` si no se pudo abrir.
    _sink: Option<AudioSink>,
    /// Tap de samples del stream para el espectro.
    probe: AudioProbe,
    /// Analizador log-band (Goertzel + release suave).
    spectrum: Spectrum,
    /// Buffer reusado para el snapshot del probe (evita realloc/tick).
    scratch: Vec<f32>,
    /// Handle de pausa compartido con el `PausableAudio` de la cadena.
    pause: Pause,
    name: String,
    sample_rate: u32,
    channels: u16,
    duration: Option<Duration>,
    position: Duration,
    playing: bool,
    error: Option<String>,
}

impl Default for AudioViewerState {
    fn default() -> Self {
        Self {
            _sink: None,
            probe: AudioProbe::new(PROBE_CAPACITY),
            spectrum: Spectrum::log_bands(SPECTRUM_BANDS, SPECTRUM_FMIN, SPECTRUM_FMAX),
            scratch: Vec::new(),
            pause: Pause::new(),
            name: String::new(),
            sample_rate: 0,
            channels: 0,
            duration: None,
            position: Duration::ZERO,
            playing: false,
            error: None,
        }
    }
}

/// Una fuente de audio ya decodificada + sus metadatos de presentación.
/// La fuente se boxea para borrar el tipo concreto antes de entrar al
/// `Arc<Mutex<dyn AudioSource>>` que el sink consume.
struct DecodedAudio {
    source: Box<dyn AudioSource + Send>,
    channels: u16,
    sample_rate: u32,
    duration: Duration,
}

impl AudioViewerState {
    /// Abre y reproduce un archivo de audio. La extensión elige el
    /// decoder; el contenido ya fue discernido como audio por el shell.
    /// Si falla el decode o el sink, queda en estado de error (lo
    /// muestra el header) y sin sonido.
    pub fn open(path: &Path) -> Self {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let decoded = match decode(path) {
            Ok(d) => d,
            Err(e) => {
                return Self {
                    name,
                    error: Some(e),
                    ..Default::default()
                };
            }
        };

        let probe = AudioProbe::new(PROBE_CAPACITY);
        let pause = Pause::new();
        // Orden de la cadena: pausa primero (en pausa rellena silencio),
        // probe después → el espectro ve silencio y decae al pausar.
        let pausable = PausableAudio::new(decoded.source, pause.clone());
        let probed = ProbedAudioSource::new(pausable, probe.clone());
        let source: Arc<Mutex<dyn AudioSource + Send>> = Arc::new(Mutex::new(probed));

        match AudioSink::open(source) {
            Ok(sink) => Self {
                _sink: Some(sink),
                probe,
                spectrum: Spectrum::log_bands(SPECTRUM_BANDS, SPECTRUM_FMIN, SPECTRUM_FMAX),
                scratch: Vec::new(),
                pause,
                name,
                sample_rate: decoded.sample_rate,
                channels: decoded.channels,
                duration: Some(decoded.duration),
                position: Duration::ZERO,
                playing: true,
                error: None,
            },
            Err(e) => Self {
                name,
                error: Some(format!("sin salida de audio: {e}")),
                ..Default::default()
            },
        }
    }

    pub fn position(&self) -> Duration {
        self.position
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Play/pausa. Congela el stream (silencio) y detiene el avance del
    /// reloj de posición.
    pub fn toggle_play(&mut self) {
        if self._sink.is_none() {
            return;
        }
        self.pause.toggle();
        self.playing = !self.playing;
    }

    /// Avanza el reloj y refresca el espectro con el último tramo del
    /// stream. Sin efecto si está en pausa o en error.
    pub fn tick(&mut self, dt: Duration) {
        if !self.playing || self._sink.is_none() {
            return;
        }
        let (sr, ch) = self.probe.snapshot(&mut self.scratch);
        if sr > 0 {
            self.spectrum.analyze(&self.scratch, ch, sr);
        }
        let next = self.position.saturating_add(dt);
        self.position = match self.duration {
            Some(d) if next > d => d,
            _ => next,
        };
    }

    /// Magnitudes actuales del espectro (una por banda, [0,1]).
    pub fn magnitudes(&self) -> &[f32] {
        self.spectrum.magnitudes()
    }
}

/// Decodifica el archivo según extensión a una fuente de audio + metadatos.
fn decode(path: &Path) -> Result<DecodedAudio, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);

    macro_rules! build {
        ($src:expr) => {{
            let s = $src.map_err(|e| e.to_string())?;
            DecodedAudio {
                channels: s.source_channels(),
                sample_rate: s.source_sample_rate(),
                duration: Duration::from_secs_f32(s.duration_seconds().max(0.0)),
                source: Box::new(s),
            }
        }};
    }

    let decoded = match ext.as_deref() {
        Some("wav") => build!(media_source_wav::WavSource::from_path(path)),
        Some("mp3") => build!(media_source_mp3::Mp3Source::from_path(path)),
        Some("flac") => build!(media_source_flac::FlacSource::from_path(path)),
        Some("opus") => build!(media_source_opus::OpusSource::from_path(path)),
        Some("ogg" | "oga") => build!(media_source_vorbis::VorbisSource::from_path(path)),
        other => {
            return Err(format!(
                "formato de audio no soportado: .{}",
                other.unwrap_or("?")
            ))
        }
    };
    Ok(decoded)
}

/// Paleta del visor.
#[derive(Debug, Clone, Copy)]
pub struct AudioViewerPalette {
    pub bg: Color,
    pub fg: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
    pub accent: Color,
}

impl Default for AudioViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl AudioViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
            accent: t.accent,
        }
    }
}

fn fmt_time(d: Duration) -> String {
    let total = d.as_secs();
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// Pinta header (nombre · rate/ch · ▶/⏸ · mm:ss/mm:ss) + body con el
/// espectro (o un placeholder si no hay audio / hay error).
pub fn audio_viewer_view<Msg>(
    state: &AudioViewerState,
    palette: &AudioViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let name = if state.name.is_empty() {
        "(seleccioná un audio)".to_string()
    } else {
        state.name.clone()
    };

    let header_text = if let Some(e) = &state.error {
        format!("{name} · error: {e}")
    } else if state._sink.is_some() {
        let glyph = if state.playing { "▶" } else { "⏸" };
        let time = match state.duration {
            Some(d) => format!("{} / {}", fmt_time(state.position), fmt_time(d)),
            None => fmt_time(state.position),
        };
        format!(
            "{name} · {} Hz · {} ch · {glyph} {time}",
            state.sample_rate, state.channels
        )
    } else {
        name
    };

    let header_color = if state.error.is_some() {
        palette.fg_error
    } else {
        palette.fg_muted
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, header_color, Alignment::Start);

    let body = match (&state.error, state._sink.is_some()) {
        (Some(e), _) => placeholder_body(&format!("(error: {e})"), palette.fg_error),
        (None, true) => spectrum_body(state.magnitudes().to_vec(), palette),
        (None, false) => placeholder_body("—", palette.fg_muted),
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(vec![header, body])
}

fn placeholder_body<Msg>(text: &str, color: Color) -> View<Msg>
where
    Msg: Clone + 'static,
{
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), 12.0, color, Alignment::Center)
}

/// Pinta las barras del espectro de abajo hacia arriba. `mags` se mueve
/// al closure (es chico: una banda por float) para que el painter sea
/// `Send + Sync` sin compartir el `Spectrum` mutable.
fn spectrum_body<Msg>(mags: Vec<f32>, palette: &AudioViewerPalette) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let accent = palette.accent;
    let track = palette.fg_muted;
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 || mags.is_empty() {
            return;
        }
        let n = mags.len();
        let slot_w = rect.w / n as f32;
        let bar_w = (slot_w * 0.7).max(1.0);
        let baseline = rect.y + rect.h;
        for (i, &m) in mags.iter().enumerate() {
            let x0 = rect.x + i as f32 * slot_w;
            // Piso tenue de 1 px para que se vea la grilla aun en silencio.
            let h = (m.clamp(0.0, 1.0) * rect.h).max(1.0);
            let bar = KurboRect::new(
                x0 as f64,
                (baseline - h) as f64,
                (x0 + bar_w) as f64,
                baseline as f64,
            );
            // Mezcla accent→track según altura: barras altas más vivas.
            let t = m.clamp(0.0, 1.0);
            let color = lerp_color(track, accent, t);
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &bar);
        }
    })
}

/// Interpola linealmente dos colores en sRGB (suficiente para un meter).
fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let ca = a.to_rgba8();
    let cb = b.to_rgba8();
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color::from_rgba8(
        mix(ca.r, cb.r),
        mix(ca.g, cb.g),
        mix(ca.b, cb.b),
        255,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_time_basico() {
        assert_eq!(fmt_time(Duration::from_secs(0)), "00:00");
        assert_eq!(fmt_time(Duration::from_secs(125)), "02:05");
    }

    #[test]
    fn open_inexistente_es_error() {
        let st = AudioViewerState::open(Path::new("/no/existe.wav"));
        assert!(st.error.is_some());
        assert!(st._sink.is_none());
        assert!(!st.is_playing());
    }

    #[test]
    fn formato_desconocido_es_error() {
        let st = AudioViewerState::open(Path::new("/x.xyz"));
        assert!(st.error.is_some());
    }

    #[test]
    fn estado_default_sin_audio() {
        let st = AudioViewerState::default();
        assert!(st._sink.is_none());
        assert_eq!(st.position(), Duration::ZERO);
        assert_eq!(st.magnitudes().len(), SPECTRUM_BANDS);
    }
}
