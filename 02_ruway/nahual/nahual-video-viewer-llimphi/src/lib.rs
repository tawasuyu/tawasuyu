//! `nahual-video-viewer-llimphi` — visor/reproductor de video sobre Llimphi.
//!
//! Análogo Llimphi del `nahual-image-viewer-llimphi`, pero para video: la
//! carga vive en [`VideoViewerState::open_av1`] (abre un `.ivf` con el
//! decoder **AV1 nativo** de `media-source-av1` — puro-Rust, sin ffmpeg),
//! el avance en [`VideoViewerState::tick`], y el render en
//! [`video_viewer_view`] (header con tiempo + cuerpo con el frame).
//!
//! ## Render por frame vs. llimphi-surface
//!
//! Este visor pinta cada frame reconstruyendo un `peniko::Image` y
//! usando [`llimphi_ui::View::image`] (aspect-fit centrado). Es simple,
//! reusable y devuelve un `View<Msg>` sin plumbing de wgpu — ideal hasta
//! ~1080p. Para 4K@60 fps el camino de cero-copia es `llimphi-surface`
//! (textura GPU persistente que el decoder escribe sin pasar por la CPU),
//! como hace `media-app`; eso exige acceso directo al device/queue y no
//! cabe en un componente que sólo retorna `View<Msg>`.
//!
//! Para formatos ajenos (H.264/H.265…), la app puede construir su propio
//! `Box<dyn FrameSource>` con `shared/foreign-av` y pasarlo a
//! [`VideoViewerState::from_source`] — el viewer no sabe de códecs.

#![forbid(unsafe_code)]

use std::path::Path;
use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{
    Blob, Color, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use media_core::FrameSource;
use media_source_av1::Av1VideoSource;

/// Estado del reproductor. Mantiene la fuente de frames, el último frame
/// decodificado como `peniko::Image`, y el transporte (play/pausa,
/// posición). `Clone` no aplica (la fuente no es clonable); la app lo
/// guarda en su modelo.
pub struct VideoViewerState {
    source: Option<Box<dyn FrameSource + Send>>,
    /// Último frame listo para pintar.
    frame: Option<Image>,
    width: u32,
    height: u32,
    /// Buffer RGBA reusado entre ticks (evita realocs).
    rgba: Vec<u8>,
    playing: bool,
    position: Duration,
    duration: Option<Duration>,
    name: String,
    error: Option<String>,
}

impl Default for VideoViewerState {
    fn default() -> Self {
        Self {
            source: None,
            frame: None,
            width: 0,
            height: 0,
            rgba: Vec::new(),
            playing: false,
            position: Duration::ZERO,
            duration: None,
            name: String::new(),
            error: None,
        }
    }
}

impl VideoViewerState {
    /// Abre un `.ivf` con video AV1 vía el decoder nativo. Arranca
    /// reproduciendo. Si falla, queda en estado de error (lo refleja el
    /// header) y sin fuente.
    pub fn open_av1(path: &Path) -> Self {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        match Av1VideoSource::open(path) {
            Ok(src) => {
                let (w, h) = src.dimensions();
                // `Seekable::duration` antes de boxear (perdemos el trait
                // al borrar el tipo, pero la duración no cambia).
                let duration = {
                    use media_core::Seekable;
                    src.duration()
                };
                Self {
                    source: Some(Box::new(src)),
                    frame: None,
                    width: w,
                    height: h,
                    rgba: Vec::new(),
                    playing: true,
                    position: Duration::ZERO,
                    duration,
                    name,
                    error: None,
                }
            }
            Err(e) => Self {
                name,
                error: Some(e),
                ..Default::default()
            },
        }
    }

    /// Abre un `.webm`/`.mkv` con video AV1 vía el demuxer nativo
    /// (`media-source-webm` → AV1 puro-Rust). Usa sólo el track de video
    /// (el viewer no tiene sink de audio). Si no hay track AV1 o falla el
    /// demux, queda en estado de error.
    pub fn open_webm(path: &Path) -> Self {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        match media_source_webm::WebmMedia::open(path) {
            Ok(media) => match media.video {
                Some(src) => {
                    let (w, h) = (media.width, media.height);
                    Self {
                        source: Some(Box::new(src)),
                        frame: None,
                        width: w,
                        height: h,
                        rgba: Vec::new(),
                        playing: true,
                        position: Duration::ZERO,
                        duration: media.duration,
                        name,
                        error: None,
                    }
                }
                None => Self {
                    name,
                    error: Some("el webm no tiene track de video AV1".to_string()),
                    ..Default::default()
                },
            },
            Err(e) => Self {
                name,
                error: Some(e.to_string()),
                ..Default::default()
            },
        }
    }

    /// Abre un GIF animado vía `media-source-gif` (frames RGBA8
    /// precomputados con sus delays). El visor lo trata como cualquier
    /// `FrameSource`: lo anima en loop. Las dimensiones se corrigen en
    /// el primer `tick` (se pasan en 0 acá; el frame inicial las fija).
    pub fn open_gif(path: &Path) -> Self {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        match media_source_gif::GifSource::from_path(path) {
            Ok(src) => {
                let duration = Some(src.total_duration());
                Self::from_source(Box::new(src), name, 0, 0, duration)
            }
            Err(e) => Self {
                name,
                error: Some(e.to_string()),
                ..Default::default()
            },
        }
    }

    /// Construye el viewer sobre una fuente arbitraria (p.ej. un puente
    /// `foreign-av`). El viewer no decodifica: sólo tickea y pinta.
    pub fn from_source(
        source: Box<dyn FrameSource + Send>,
        name: impl Into<String>,
        width: u32,
        height: u32,
        duration: Option<Duration>,
    ) -> Self {
        Self {
            source: Some(source),
            frame: None,
            width,
            height,
            rgba: Vec::new(),
            playing: true,
            position: Duration::ZERO,
            duration,
            name: name.into(),
            error: None,
        }
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
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

    /// Pausa/reanuda. En pausa, `tick` no avanza la fuente.
    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// `true` si la reproducción terminó (la fuente se agotó).
    pub fn finished(&self) -> bool {
        self.source.is_none() && self.error.is_none() && self.frame.is_some()
    }

    /// Avanza el tiempo. Si hay un frame nuevo, actualiza la imagen a
    /// pintar y devuelve `true`. Cuando la fuente se agota, la suelta
    /// (deja el último frame congelado).
    pub fn tick(&mut self, dt: Duration) -> bool {
        if !self.playing {
            return false;
        }
        let Some(src) = self.source.as_mut() else {
            return false;
        };
        match src.tick(dt, &mut self.rgba) {
            Some((w, h)) => {
                self.width = w;
                self.height = h;
                let blob = Blob::from(self.rgba.clone());
                self.frame = Some(Image::new(ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: w, height: h }));
                self.position = self.position.saturating_add(dt);
                true
            }
            None => {
                // Sin frame este tick: avanzamos el reloj igual mientras
                // haya fuente; si está agotada (varios None seguidos no
                // los distinguimos sin Seekable), seguimos hasta que la
                // app decida. Para no congelar el reloj, avanzamos.
                self.position = self.position.saturating_add(dt);
                false
            }
        }
    }
}

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct VideoViewerPalette {
    pub bg: Color,
    pub fg: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
    pub accent: Color,
}

impl Default for VideoViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl VideoViewerPalette {
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

/// Pinta header (nombre · dims · ▶/⏸ · mm:ss / mm:ss) + body con el
/// frame actual (aspect-fit) o un placeholder.
pub fn video_viewer_view<Msg>(
    state: &VideoViewerState,
    palette: &VideoViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let name = if state.name.is_empty() {
        "(seleccioná un video)".to_string()
    } else {
        state.name.clone()
    };

    let header_text = if let Some(e) = &state.error {
        format!("{name} · error: {e}")
    } else if state.width > 0 {
        let glyph = if state.playing { "▶" } else { "⏸" };
        let time = match state.duration {
            Some(d) => format!("{} / {}", fmt_time(state.position), fmt_time(d)),
            None => fmt_time(state.position),
        };
        format!("{name} · {}×{} · {glyph} {time}", state.width, state.height)
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

    let body = match (&state.error, &state.frame) {
        (Some(e), _) => placeholder_body(&format!("(error: {e})"), palette.fg_error),
        (None, Some(image)) => frame_body(image.clone()),
        (None, None) => placeholder_body("—", palette.fg_muted),
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

fn frame_body<Msg>(image: Image) -> View<Msg>
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
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(6.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .image(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_time_basico() {
        assert_eq!(fmt_time(Duration::from_secs(0)), "00:00");
        assert_eq!(fmt_time(Duration::from_secs(75)), "01:15");
    }

    #[test]
    fn tick_sobre_fuente_produce_frame() {
        // Usamos el TestCard de media-core como fuente sintética: evita
        // acoplar el test a un archivo y prueba el camino tick→frame.
        // (El decode AV1 real se valida en media-source-av1.)
        use media_core::TestCard;
        let mut st = VideoViewerState::from_source(
            Box::new(TestCard::new(64, 48, 30.0)),
            "testcard",
            64,
            48,
            None,
        );
        assert_eq!(st.dimensions(), (64, 48));
        assert!(st.is_playing());
        assert!(st.frame.is_none(), "sin tick todavía no hay frame");

        let got = st.tick(Duration::from_secs(1));
        assert!(got, "el primer tick debería producir un frame");
        assert!(st.frame.is_some());

        // En pausa, tick no avanza.
        st.toggle_play();
        assert!(!st.tick(Duration::from_secs(1)));
    }

    #[test]
    fn open_inexistente_es_error() {
        let st = VideoViewerState::open_av1(Path::new("/no/existe.ivf"));
        assert!(st.error.is_some());
        assert!(st.source.is_none());
    }
}
