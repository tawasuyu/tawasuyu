//! multimedia-app — primer reproductor del dominio.
//!
//! Pipeline video: una fuente [`FrameSource`] genera RGBA, lo empuja
//! a un [`llimphi_surface::ExternalSurface`], y la UI Llimphi lo
//! expone en un canvas central vía `View::gpu_paint_with`. Con
//! argumento es un GIF en disco (loop infinito); sin argumento cae
//! al [`TestCard`] sintético.
//!
//! Pipeline audio: junto al video se abre un sink cpal sobre el
//! default output device, alimentado por un [`ToneSource`] (A4 a
//! -12 dB). Si el sink no puede abrir el device, se loguea y se
//! sigue sólo con video — la falta de audio no aborta la app.
//!
//! Visor de audio: la fuente sale envuelta en [`ProbedAudioSource`],
//! que duplica cada bloque a un ring buffer compartido. Debajo del
//! canvas de video se pinta una franja con la forma de onda del
//! último tramo del stream (vía `paint_with`). Cuando el audio está
//! muteado, la franja queda en silencio (línea recta) — el visor no
//! depende del sink.
//!
//! Captura: dos botones en el row del título toman fotos del estado
//! actual. `rec` arma/cierra una grabación WAV (PCM 16) del stream
//! audio en el cwd; `snap` escribe un PNG con el frame de video
//! pendiente. Pausa silencia/congela ambos taps a la vez.
//!
//! Corre con:
//!   `cargo run -p multimedia-app --release`
//!   `cargo run -p multimedia-app --release -- /ruta/al/anim.gif`
//!   `cargo run -p multimedia-app --release -- /ruta/foto.png`
//!   `MULTIMEDIA_WAV=/ruta/clip.wav cargo run -p multimedia-app --release`
//!   `MULTIMEDIA_MP3=/ruta/cancion.mp3 cargo run -p multimedia-app --release`
//!   `MULTIMEDIA_MUTE=1 cargo run -p multimedia-app --release`
//!
//! El primer argumento posicional es el video; la extensión decide
//! la fuente (`.gif` → anim, `.png/.jpg/.webp/.bmp/.tiff/.jpeg` →
//! imagen fija). La pista de audio se elige con `MULTIMEDIA_WAV` o
//! `MULTIMEDIA_MP3` — sin ninguna, suena un tono A4 sintético.
//!
//! `MULTIMEDIA_MIX_TONE=0.25` (rango 0..1) superpone un tono A4 a esa
//! ganancia sobre la fuente principal vía MixerAudio — demo del
//! mezclador con cualquier fuente.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use llimphi_surface::ExternalSurface;
use llimphi_ui::llimphi_hal::wgpu;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{self, TextBlock};
use llimphi_ui::{App, Handle, View};
use multimedia_audio_cpal::AudioSink;
use multimedia_core::{
    AudioProbe, AudioSource, FrameSource, Levels, MixerAudio, Pause, PausableAudio,
    PausableVideo, ProbedAudioSource, Seekable, TestCard, ToneSource, Volume, VolumeAudio,
    Waterfall,
};
use multimedia_recorder_wav::{default_recording_path, RecordedAudioSource, WavRecorder};
use multimedia_source_gif::GifSource;
use multimedia_source_image::ImageSource;
use multimedia_source_mp3::Mp3Source;
use multimedia_source_wav::WavSource;
use parking_lot::Mutex;

const TESTCARD_W: u32 = 480;
const TESTCARD_H: u32 = 270;
const TESTCARD_FPS: f32 = 30.0;
const TICK_MS: u64 = 33;
/// Capacidad del ring del probe. ~85 ms a 48 kHz · 2 ch — suficiente
/// para una franja de visor responsiva sin meter latencia ni RAM.
const PROBE_CAPACITY: usize = 8192;

#[derive(Clone)]
enum Msg {
    Tick,
    TogglePause,
    ToggleRecord,
    Snapshot,
    VolDown,
    VolUp,
    SeekBack,
    SeekFwd,
}

const VOLUME_STEP: f32 = 0.1;
const SEEK_STEP_SECS: u64 = 5;

struct Model {
    frames: u64,
    started_at: Instant,
}

struct Pipeline {
    surface: ExternalSurface,
    source: Mutex<Box<dyn FrameSource + Send>>,
    buf: Mutex<Vec<u8>>,
    /// Última dimensión `(w, h)` que emitió la fuente. `(0, 0)` hasta
    /// el primer tick exitoso. Lo lee el handler de Snapshot para
    /// armar el `ImageBuffer`.
    last_dim: Mutex<(u32, u32)>,
    last_tick: Mutex<Instant>,
}

fn config_slot() -> &'static OnceLock<Config> {
    static SLOT: OnceLock<Config> = OnceLock::new();
    &SLOT
}

fn pipeline_slot() -> &'static OnceLock<Pipeline> {
    static SLOT: OnceLock<Pipeline> = OnceLock::new();
    &SLOT
}

struct Config {
    label: String,
    kind: VideoKind,
}

#[derive(Clone, Copy)]
enum VideoKind {
    Testcard,
    Gif,
    Image,
}

/// Path del archivo de video (GIF o imagen estática) cuando aplica.
/// Vacío para Testcard.
fn video_path_slot() -> &'static OnceLock<PathBuf> {
    static SLOT: OnceLock<PathBuf> = OnceLock::new();
    &SLOT
}

/// Probe del stream de audio que `audio_source_from_env` instaló.
/// `None` cuando no hay audio (MULTIMEDIA_MUTE o el sink no abrió) —
/// el visor entonces pinta una franja en silencio.
fn audio_probe_slot() -> &'static OnceLock<Option<AudioProbe>> {
    static SLOT: OnceLock<Option<AudioProbe>> = OnceLock::new();
    &SLOT
}

/// Handle de pausa compartido por audio y video. Se materializa antes
/// de armar las fuentes para poder pasarlo a los wrappers Pausable*.
fn pause() -> &'static Pause {
    static SLOT: OnceLock<Pause> = OnceLock::new();
    SLOT.get_or_init(Pause::new)
}

/// Handle compartido del recorder WAV. Cuando `is_recording()` es
/// false, el wrapper `RecordedAudioSource` es transparente; al
/// armarlo desde la UI empieza a copiar cada bloque del stream a
/// disco.
fn recorder() -> &'static WavRecorder {
    static SLOT: OnceLock<WavRecorder> = OnceLock::new();
    SLOT.get_or_init(WavRecorder::new)
}

/// Ganancia lineal compartida con el wrapper [`VolumeAudio`]. 1.0 =
/// passthrough; los botones suben/bajan en pasos de 0.1.
fn volume() -> &'static Volume {
    static SLOT: OnceLock<Volume> = OnceLock::new();
    SLOT.get_or_init(|| Volume::new(1.0))
}

/// Handle a la fuente audio cuando es `Seekable` (WAV o MP3). `None`
/// si la activa es tono A4 o no hay sink — en ese caso los botones
/// de seek quedan apagados.
fn seekable_handle_slot() -> &'static OnceLock<Option<Arc<Mutex<dyn Seekable + Send>>>> {
    static SLOT: OnceLock<Option<Arc<Mutex<dyn Seekable + Send>>>> = OnceLock::new();
    &SLOT
}

/// Adapter que comparte una fuente vía `Arc<Mutex<T>>` sin moverla.
/// El cpal sink ve un `AudioSource` normal; otros consumidores (la UI
/// para seek/position) pueden seguir hablando con el inner por la
/// otra punta del Arc.
struct SharedAudio<T> {
    inner: Arc<Mutex<T>>,
}

impl<T: AudioSource> AudioSource for SharedAudio<T> {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.inner.lock().fill(buf, sample_rate, channels);
    }
}

/// Mueve la posición de la fuente audio Seekable activa en
/// `delta_secs` (negativo = atrás) con wrap módulo duration. No-op si
/// la fuente actual no es seekable (tono A4).
fn seek_audio_by(delta_secs: i64) {
    let Some(handle) = seekable_handle_slot().get().and_then(|o| o.as_ref()) else {
        return;
    };
    let mut src = handle.lock();
    let dur = src.duration().unwrap_or(Duration::from_secs(1));
    let dur_s = dur.as_secs_f64().max(0.001);
    let cur_s = src.position().as_secs_f64();
    let new_s = (cur_s + delta_secs as f64).rem_euclid(dur_s);
    src.seek_to(Duration::from_secs_f64(new_s));
}

/// Formatea una duración como `M:SS`. Para tracks de menos de una
/// hora — más allá rolls over y se ve raro, pero MVP.
fn fmt_secs(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

/// Path de snapshot único por segundo, en el cwd: `multimedia-snap-N.png`.
fn default_snapshot_path() -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    PathBuf::from(format!("multimedia-snap-{secs}.png"))
}

fn new_testcard() -> Box<dyn FrameSource + Send> {
    Box::new(PausableVideo::new(
        TestCard::new(TESTCARD_W, TESTCARD_H, TESTCARD_FPS),
        pause().clone(),
    ))
}

fn build_video_source() -> Box<dyn FrameSource + Send> {
    let cfg = config_slot().get().expect("config set");
    let p = pause().clone();
    match cfg.kind {
        VideoKind::Testcard => new_testcard(),
        VideoKind::Gif => {
            let path = video_path_slot().get().expect("video path set");
            match GifSource::from_path(path) {
                Ok(s) => Box::new(PausableVideo::new(s, p)),
                Err(e) => {
                    eprintln!(
                        "multimedia-app: error abriendo GIF {path:?}: {e} — caigo a testcard"
                    );
                    new_testcard()
                }
            }
        }
        VideoKind::Image => {
            let path = video_path_slot().get().expect("video path set");
            match ImageSource::from_path(path) {
                Ok(s) => Box::new(PausableVideo::new(s, p)),
                Err(e) => {
                    eprintln!(
                        "multimedia-app: error abriendo imagen {path:?}: {e} — caigo a testcard"
                    );
                    new_testcard()
                }
            }
        }
    }
}

fn pipeline_for(device: &wgpu::Device, queue: &wgpu::Queue) -> &'static Pipeline {
    pipeline_slot().get_or_init(|| Pipeline {
        surface: ExternalSurface::new(device, queue),
        source: Mutex::new(build_video_source()),
        buf: Mutex::new(Vec::new()),
        last_dim: Mutex::new((0, 0)),
        last_tick: Mutex::new(Instant::now()),
    })
}

struct MultimediaApp;

impl App for MultimediaApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "multimedia · player"
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        handle.spawn_periodic(Duration::from_millis(TICK_MS), || Msg::Tick);
        Model {
            frames: 0,
            started_at: Instant::now(),
        }
    }

    fn update(model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => Model {
                frames: model.frames.wrapping_add(1),
                ..model
            },
            Msg::TogglePause => {
                pause().toggle();
                model
            }
            Msg::ToggleRecord => {
                let rec = recorder();
                if rec.is_recording() {
                    match rec.stop() {
                        Ok(p) => eprintln!(
                            "multimedia-app: recording cerrada en {}",
                            p.display()
                        ),
                        Err(e) => eprintln!("multimedia-app: stop recording: {e}"),
                    }
                } else {
                    let path = default_recording_path(".");
                    match rec.start(&path) {
                        Ok(p) => eprintln!("multimedia-app: grabando en {}", p.display()),
                        Err(e) => eprintln!("multimedia-app: start recording: {e}"),
                    }
                }
                model
            }
            Msg::VolDown => {
                volume().update(|v| v - VOLUME_STEP);
                model
            }
            Msg::VolUp => {
                volume().update(|v| v + VOLUME_STEP);
                model
            }
            Msg::SeekBack => {
                seek_audio_by(-(SEEK_STEP_SECS as i64));
                model
            }
            Msg::SeekFwd => {
                seek_audio_by(SEEK_STEP_SECS as i64);
                model
            }
            Msg::Snapshot => {
                if let Some(pipe) = pipeline_slot().get() {
                    let (w, h) = *pipe.last_dim.lock();
                    let buf = pipe.buf.lock().clone();
                    let expected = (w as usize) * (h as usize) * 4;
                    if w == 0 || h == 0 || buf.len() != expected {
                        eprintln!("multimedia-app: no hay frame para snapshot todavía");
                    } else {
                        let path = default_snapshot_path();
                        match image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(w, h, buf) {
                            Some(img) => match img.save(&path) {
                                Ok(()) => eprintln!(
                                    "multimedia-app: snapshot {}×{} guardado en {}",
                                    w,
                                    h,
                                    path.display()
                                ),
                                Err(e) => eprintln!("multimedia-app: save snapshot: {e}"),
                            },
                            None => eprintln!("multimedia-app: buf inconsistente para snapshot"),
                        }
                    }
                } else {
                    eprintln!("multimedia-app: pipeline aún no montada");
                }
                model
            }
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let cfg = config_slot().get().expect("config set");
        let secs = model.started_at.elapsed().as_secs_f32().max(0.001);
        let fps = model.frames as f32 / secs;

        let paused = pause().is_paused();
        let pause_btn = chip_button(
            if paused { "play" } else { "pause" },
            if paused {
                Color::from_rgba8(60, 140, 90, 255)
            } else {
                Color::from_rgba8(55, 65, 80, 255)
            },
            Color::from_rgba8(220, 230, 245, 255),
            Msg::TogglePause,
        );

        let recording = recorder().is_recording();
        let rec_btn = chip_button(
            if recording { "stop" } else { "rec" },
            if recording {
                Color::from_rgba8(200, 65, 65, 255)
            } else {
                Color::from_rgba8(55, 65, 80, 255)
            },
            Color::from_rgba8(245, 235, 235, 255),
            Msg::ToggleRecord,
        );

        let snap_btn = chip_button(
            "snap",
            Color::from_rgba8(55, 65, 80, 255),
            Color::from_rgba8(220, 230, 245, 255),
            Msg::Snapshot,
        );

        let seekable = seekable_handle_slot()
            .get()
            .and_then(|o| o.as_ref())
            .is_some();
        let seek_bg = if seekable {
            Color::from_rgba8(55, 65, 80, 255)
        } else {
            // Apagado: gris oscuro para señalizar "no aplica".
            Color::from_rgba8(40, 46, 56, 255)
        };
        let seek_fg = if seekable {
            Color::from_rgba8(220, 230, 245, 255)
        } else {
            Color::from_rgba8(100, 110, 125, 255)
        };
        let back_btn = chip_button("«5s", seek_bg, seek_fg, Msg::SeekBack);
        let fwd_btn = chip_button("5s»", seek_bg, seek_fg, Msg::SeekFwd);

        let vol_label = format!("vol {:.0}%", (volume().get() * 100.0).round());
        let vol_text = View::new(Style {
            size: Size {
                width: length(82.0_f32),
                height: length(36.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(vol_label, 13.0, Color::from_rgba8(180, 195, 215, 255));
        let vol_dn = chip_button(
            "vol−",
            Color::from_rgba8(55, 65, 80, 255),
            Color::from_rgba8(220, 230, 245, 255),
            Msg::VolDown,
        );
        let vol_up = chip_button(
            "vol+",
            Color::from_rgba8(55, 65, 80, 255),
            Color::from_rgba8(220, 230, 245, 255),
            Msg::VolUp,
        );

        let title_text = View::new(Style {
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            format!("multimedia — {}", cfg.label),
            22.0,
            Color::from_rgba8(220, 230, 245, 255),
        );

        let title = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(44.0_f32),
            },
            gap: Size {
                width: length(12.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![
            pause_btn,
            rec_btn,
            snap_btn,
            back_btn,
            fwd_btn,
            title_text,
            vol_dn,
            vol_text,
            vol_up,
            meters_panel(),
        ]);

        let canvas_style = Style {
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        };

        let canvas = View::new(canvas_style)
            .fill(Color::from_rgba8(10, 12, 18, 255))
            .radius(10.0)
            .gpu_paint_with(move |device, queue, encoder, view, rect, viewport| {
                let pipe = pipeline_for(device, queue);
                let mut last = pipe.last_tick.lock();
                let now = Instant::now();
                let dt = now - *last;
                *last = now;
                let mut buf = pipe.buf.lock();
                if let Some((w, h)) = pipe.source.lock().tick(dt, &mut buf) {
                    pipe.surface.upload(&buf, w, h);
                    *pipe.last_dim.lock() = (w, h);
                }
                drop(buf);
                pipe.surface.blit(queue, encoder, view, rect, viewport);
            });

        let visor_row = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(96.0_f32),
            },
            gap: Size {
                width: length(10.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![waveform_panel(), waterfall_panel()]);

        let time_label = seekable_handle_slot()
            .get()
            .and_then(|o| o.as_ref())
            .map(|h| {
                let s = h.lock();
                let pos = s.position();
                let dur = s.duration().unwrap_or(Duration::ZERO);
                format!(" · {} / {}", fmt_secs(pos), fmt_secs(dur))
            })
            .unwrap_or_default();
        let footer = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            format!("ticks {} · ui ≈ {fps:.1} fps{time_label}", model.frames),
            14.0,
            Color::from_rgba8(150, 165, 185, 255),
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(12.0_f32),
            },
            padding: TaffyRect {
                left: length(24.0_f32),
                right: length(24.0_f32),
                top: length(16.0_f32),
                bottom: length(16.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(22, 26, 34, 255))
        .children(vec![title, canvas, visor_row, footer])
    }
}

/// Panel inferior con la forma de onda del último tramo del stream
/// (mezcla de canales en mono para mostrarse en una sola línea).
/// Cuando no hay probe (audio muteado) muestra una línea de centro
/// con leyenda "audio off".
fn waveform_panel<Msg: 'static>() -> View<Msg> {
    let probe = audio_probe_slot().get().cloned().flatten();
    let scratch: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let stroke_color = Color::from_rgba8(120, 220, 170, 255);
    let center_color = Color::from_rgba8(80, 92, 110, 255);
    let off_label = probe.is_none();

    View::new(Style {
        size: Size {
            width: auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(14, 16, 22, 255))
    .radius(8.0)
    .paint_with(move |scene, ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 {
            return;
        }
        let pad_x: f32 = 12.0;
        let pad_y: f32 = 8.0;
        let inner_x = rect.x + pad_x;
        let inner_y = rect.y + pad_y;
        let inner_w = (rect.w - 2.0 * pad_x).max(1.0);
        let inner_h = (rect.h - 2.0 * pad_y).max(1.0);
        let mid_y = inner_y + inner_h * 0.5;

        // Línea central — siempre presente, hace de "ground" del visor.
        let mut center = BezPath::new();
        center.move_to((inner_x as f64, mid_y as f64));
        center.line_to(((inner_x + inner_w) as f64, mid_y as f64));
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            center_color,
            None,
            &center,
        );

        if off_label {
            // Sin probe: leyenda mínima para que se sepa que el visor
            // está vivo aunque no haya señal.
            let _ = ts;
            return;
        }
        let Some(probe) = probe.as_ref() else {
            return;
        };

        let mut snap = scratch.lock();
        let (_sr, channels) = probe.snapshot(&mut snap);
        let channels = channels.max(1) as usize;
        let total_frames = snap.len() / channels;
        if total_frames < 2 {
            return;
        }

        // Envelope min/max por columna: por cada bucket de frames
        // guardamos el mínimo y el máximo del mono fold y dibujamos
        // la forma como un polígono cerrado (relleno tenue + stroke).
        // Da mucho más "cuerpo" que la línea pico-sólo.
        let cols = inner_w.max(2.0) as usize;
        let cols = cols.min(total_frames);
        let frames_per_col = total_frames / cols.max(1);
        if frames_per_col == 0 {
            return;
        }
        let amp = inner_h * 0.5;

        let mut top = BezPath::new();
        let mut bot = BezPath::new();
        let mut envelope = BezPath::new();
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
            let x = inner_x + (col as f32 / (cols as f32 - 1.0).max(1.0)) * inner_w;
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
        // Cierre del polígono envelope: vuelve por la línea de
        // mínimos en sentido inverso.
        for col in (0..cols).rev() {
            let f0 = col * frames_per_col;
            let f1 = ((col + 1) * frames_per_col).min(total_frames);
            let mut vmin = f32::INFINITY;
            for f in f0..f1 {
                let mut acc = 0.0_f32;
                for ch in 0..channels {
                    acc += snap[f * channels + ch];
                }
                let v = (acc / channels as f32).clamp(-1.0, 1.0);
                if v < vmin {
                    vmin = v;
                }
            }
            let x = inner_x + (col as f32 / (cols as f32 - 1.0).max(1.0)) * inner_w;
            let y_bot = mid_y - vmin * amp;
            envelope.line_to((x as f64, y_bot as f64));
        }
        envelope.close_path();

        let fill_color = Color::from_rgba8(120, 220, 170, 70);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            fill_color,
            None,
            &envelope,
        );
        scene.stroke(
            &Stroke::new(1.2),
            Affine::IDENTITY,
            stroke_color,
            None,
            &top,
        );
        scene.stroke(
            &Stroke::new(1.2),
            Affine::IDENTITY,
            stroke_color,
            None,
            &bot,
        );
    })
}

/// Botón compacto del row del título: tamaño fijo, hover azulado y
/// click manda `msg`. Centra el texto vertical y horizontalmente.
fn chip_button(label: &str, bg: Color, fg: Color, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(64.0_f32),
            height: length(36.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(Color::from_rgba8(80, 100, 130, 255))
    .radius(8.0)
    .text(label.to_string(), 15.0, fg)
    .on_click(msg)
}

/// Strip de medidores peak + RMS para el row del título. Dos barras
/// horizontales apiladas (peak arriba, RMS abajo) con etiqueta corta
/// a la izquierda. El color de la barra desplaza de verde a rojo
/// pasados los -6 dBFS — pista visual de saturación.
fn meters_panel<Msg: 'static>() -> View<Msg> {
    let probe = audio_probe_slot().get().cloned().flatten();
    let scratch: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let levels: Arc<Mutex<Levels>> = Arc::new(Mutex::new(Levels::new()));
    let track_bg = Color::from_rgba8(34, 40, 52, 255);
    let label_color = Color::from_rgba8(150, 165, 185, 255);
    let off_color = Color::from_rgba8(80, 92, 110, 255);

    View::new(Style {
        size: Size {
            width: length(160.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 {
            return;
        }
        let label_w: f32 = 36.0;
        let bar_h: f32 = 8.0;
        let gap_y: f32 = 6.0;
        let inner_x = rect.x;
        let inner_y = rect.y + (rect.h - (bar_h * 2.0 + gap_y)) * 0.5;
        let bars_x = inner_x + label_w;
        let bars_w = (rect.w - label_w).max(1.0);

        // Etiquetas — texto via Typesetter para mantener consistencia.
        let pk_label = TextBlock::simple(
            "PK",
            11.0,
            label_color,
            (inner_x as f64, (inner_y - 3.0) as f64),
        );
        llimphi_text::draw_block(scene, ts, &pk_label);
        let rms_label = TextBlock::simple(
            "RMS",
            11.0,
            label_color,
            (inner_x as f64, (inner_y + bar_h + gap_y - 3.0) as f64),
        );
        llimphi_text::draw_block(scene, ts, &rms_label);

        // Tracks (fondo).
        let pk_track = KurboRect::new(
            bars_x as f64,
            inner_y as f64,
            (bars_x + bars_w) as f64,
            (inner_y + bar_h) as f64,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, track_bg, None, &pk_track);
        let rms_y = inner_y + bar_h + gap_y;
        let rms_track = KurboRect::new(
            bars_x as f64,
            rms_y as f64,
            (bars_x + bars_w) as f64,
            (rms_y + bar_h) as f64,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, track_bg, None, &rms_track);

        let Some(probe) = probe.as_ref() else {
            // Sin probe: marca tenue al fondo de cada barra para que
            // se sepa que está apagado.
            let pk_off = KurboRect::new(
                bars_x as f64,
                (inner_y + bar_h - 1.0) as f64,
                (bars_x + bars_w) as f64,
                (inner_y + bar_h) as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, off_color, None, &pk_off);
            let rms_off = KurboRect::new(
                bars_x as f64,
                (rms_y + bar_h - 1.0) as f64,
                (bars_x + bars_w) as f64,
                (rms_y + bar_h) as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, off_color, None, &rms_off);
            return;
        };

        let mut snap = scratch.lock();
        let (_sr, channels) = probe.snapshot(&mut snap);
        let mut levels = levels.lock();
        levels.analyze(&snap, channels);
        let pk = levels.peak();
        let rms = levels.rms();

        let pk_w = (pk.clamp(0.0, 1.0) * bars_w).max(0.0);
        let rms_w = (rms.clamp(0.0, 1.0) * bars_w).max(0.0);

        if pk_w > 0.0 {
            let pk_fill = KurboRect::new(
                bars_x as f64,
                inner_y as f64,
                (bars_x + pk_w) as f64,
                (inner_y + bar_h) as f64,
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                level_color(pk),
                None,
                &pk_fill,
            );
        }
        if rms_w > 0.0 {
            let rms_fill = KurboRect::new(
                bars_x as f64,
                rms_y as f64,
                (bars_x + rms_w) as f64,
                (rms_y + bar_h) as f64,
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                level_color(rms),
                None,
                &rms_fill,
            );
        }
    })
}

/// Gradiente verde → ámbar → rojo según el nivel. Cambio a ámbar
/// alrededor de 0.5 (-6 dBFS) y a rojo cerca de full scale.
fn level_color(v: f32) -> Color {
    let v = v.clamp(0.0, 1.0);
    if v < 0.5 {
        Color::from_rgba8(110, 220, 140, 255)
    } else if v < 0.85 {
        Color::from_rgba8(230, 200, 90, 255)
    } else {
        Color::from_rgba8(240, 95, 95, 255)
    }
}

/// Panel de espectro: banco Goertzel sobre el probe + barras log
/// espaciadas (40 Hz → 16 kHz). Sin probe queda con la base oscura y
/// las casillas vacías.
/// Panel waterfall (spectrogram histórico): cada fila es un análisis
/// Goertzel sobre el probe; las filas nuevas entran por arriba y
/// empujan a las viejas hacia abajo. Color va de fondo casi negro a
/// ámbar/blanco según magnitud — la "ráfaga" del bajo y los picos
/// quedan visibles ~2-3 segundos antes de desvanecerse.
fn waterfall_panel<Msg: 'static>() -> View<Msg> {
    let probe = audio_probe_slot().get().cloned().flatten();
    let scratch: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let grid_buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    // 28 bandas para tener resolución y a la vez celdas pintables
    // sin amontonar. ~60 filas a 30 fps = 2 segundos de historia.
    let waterfall: Arc<Mutex<Waterfall>> =
        Arc::new(Mutex::new(Waterfall::new(28, 60, 40.0, 16_000.0)));
    let base_color = Color::from_rgba8(46, 36, 28, 255);

    View::new(Style {
        size: Size {
            width: auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(14, 16, 22, 255))
    .radius(8.0)
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 {
            return;
        }
        let pad: f32 = 6.0;
        let inner_x = rect.x + pad;
        let inner_y = rect.y + pad;
        let inner_w = (rect.w - 2.0 * pad).max(1.0);
        let inner_h = (rect.h - 2.0 * pad).max(1.0);

        let Some(probe) = probe.as_ref() else {
            // Sin probe: línea base apagada — mismo lenguaje que los
            // otros visores.
            let mut center = BezPath::new();
            let mid = inner_y + inner_h * 0.5;
            center.move_to((inner_x as f64, mid as f64));
            center.line_to(((inner_x + inner_w) as f64, mid as f64));
            scene.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                base_color,
                None,
                &center,
            );
            return;
        };

        let mut snap = scratch.lock();
        let (sr, channels) = probe.snapshot(&mut snap);
        if sr == 0 {
            return;
        }
        let mut wf = waterfall.lock();
        wf.analyze(&snap, channels, sr);

        let mut grid = grid_buf.lock();
        let (rows, bands) = wf.snapshot(&mut grid);
        let cell_w = inner_w / bands as f32;
        let cell_h = inner_h / rows as f32;
        for r in 0..rows {
            let y0 = inner_y + r as f32 * cell_h;
            for b in 0..bands {
                let m = grid[r * bands + b];
                if m < 0.02 {
                    continue;
                }
                let x0 = inner_x + b as f32 * cell_w;
                let cell = KurboRect::new(
                    x0 as f64,
                    y0 as f64,
                    (x0 + cell_w + 0.5) as f64,
                    (y0 + cell_h + 0.5) as f64,
                );
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    heat_color(m),
                    None,
                    &cell,
                );
            }
        }
    })
}

/// Gradiente "heat" para el waterfall: tinte oscuro → ámbar → claro
/// según magnitud. Bandas vacías no se pintan (fondo del View queda
/// visible).
fn heat_color(v: f32) -> Color {
    let v = v.clamp(0.0, 1.0);
    if v < 0.25 {
        let t = v / 0.25;
        let r = (60.0 + 110.0 * t) as u8;
        let g = (20.0 + 30.0 * t) as u8;
        let b = (20.0 + 10.0 * t) as u8;
        Color::from_rgba8(r, g, b, 255)
    } else if v < 0.6 {
        let t = (v - 0.25) / 0.35;
        let r = (170.0 + 70.0 * t) as u8;
        let g = (50.0 + 110.0 * t) as u8;
        let b = (30.0 + 40.0 * t) as u8;
        Color::from_rgba8(r, g, b, 255)
    } else {
        let t = (v - 0.6) / 0.4;
        let r = (240.0 + 15.0 * t) as u8;
        let g = (160.0 + 80.0 * t) as u8;
        let b = (70.0 + 160.0 * t) as u8;
        Color::from_rgba8(r, g, b, 255.min((180.0 + 75.0 * t) as u8))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match args.first() {
        Some(path) => {
            let path = PathBuf::from(path);
            let kind = match path
                .extension()
                .and_then(|s| s.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("gif") => VideoKind::Gif,
                Some("png" | "jpg" | "jpeg" | "webp" | "bmp" | "tiff") => VideoKind::Image,
                other => {
                    eprintln!(
                        "multimedia-app: extensión {:?} no reconocida — caigo a testcard",
                        other
                    );
                    VideoKind::Testcard
                }
            };
            let label = match kind {
                VideoKind::Gif => format!("gif {}", path.display()),
                VideoKind::Image => format!("img {}", path.display()),
                VideoKind::Testcard => format!(
                    "testcard {TESTCARD_W}×{TESTCARD_H} @ {TESTCARD_FPS:.0} fps"
                ),
            };
            if !matches!(kind, VideoKind::Testcard) {
                video_path_slot().set(path).ok();
            }
            Config { label, kind }
        }
        None => Config {
            label: format!("testcard {TESTCARD_W}×{TESTCARD_H} @ {TESTCARD_FPS:.0} fps"),
            kind: VideoKind::Testcard,
        },
    };
    config_slot().set(cfg).ok();

    // Audio: si MULTIMEDIA_MUTE está set, saltamos. Si no, elegimos
    // fuente — MULTIMEDIA_WAV=path la activa, sino cae al ToneSource
    // (A4). El AudioSink debe vivir hasta el exit — `cpal::Stream` no
    // es `Sync`, así que no puede ir a un static; lo mantenemos en
    // una local de `main` que sólo se dropea cuando el proceso
    // termina.
    let _audio_sink = if std::env::var("MULTIMEDIA_MUTE").is_err() {
        let (source, probe) = audio_source_from_env();
        match AudioSink::open(source) {
            Ok(sink) => {
                eprintln!(
                    "multimedia-app: audio cpal abierto @ {} Hz · {} ch",
                    sink.sample_rate(),
                    sink.channels(),
                );
                audio_probe_slot().set(Some(probe)).ok();
                Some(sink)
            }
            Err(e) => {
                eprintln!("multimedia-app: audio off ({e}) — sigo sin sonido");
                audio_probe_slot().set(None).ok();
                None
            }
        }
    } else {
        audio_probe_slot().set(None).ok();
        None
    };

    llimphi_ui::run::<MultimediaApp>();
}

fn audio_source_from_env() -> (Arc<Mutex<dyn AudioSource + Send>>, AudioProbe) {
    let probe = AudioProbe::new(PROBE_CAPACITY);

    // Prioridad: WAV → MP3 → tono A4.
    let inner: Box<dyn AudioSource + Send> = if let Ok(path) = std::env::var("MULTIMEDIA_WAV") {
        match WavSource::from_path(&path) {
            Ok(wav) => {
                eprintln!(
                    "multimedia-app: wav {path} · {} ch · {} Hz · {:.1}s",
                    wav.source_channels(),
                    wav.source_sample_rate(),
                    wav.duration_seconds(),
                );
                let shared: Arc<Mutex<WavSource>> = Arc::new(Mutex::new(wav));
                let seek_ref: Arc<Mutex<dyn Seekable + Send>> = shared.clone();
                seekable_handle_slot().set(Some(seek_ref)).ok();
                Box::new(SharedAudio { inner: shared })
            }
            Err(e) => {
                eprintln!("multimedia-app: no pude abrir WAV {path}: {e} — caigo a tono A4");
                seekable_handle_slot().set(None).ok();
                Box::new(ToneSource::a4())
            }
        }
    } else if let Ok(path) = std::env::var("MULTIMEDIA_MP3") {
        match Mp3Source::from_path(&path) {
            Ok(mp3) => {
                eprintln!(
                    "multimedia-app: mp3 {path} · {} ch · {} Hz · {:.1}s",
                    mp3.source_channels(),
                    mp3.source_sample_rate(),
                    mp3.duration_seconds(),
                );
                let shared: Arc<Mutex<Mp3Source>> = Arc::new(Mutex::new(mp3));
                let seek_ref: Arc<Mutex<dyn Seekable + Send>> = shared.clone();
                seekable_handle_slot().set(Some(seek_ref)).ok();
                Box::new(SharedAudio { inner: shared })
            }
            Err(e) => {
                eprintln!("multimedia-app: no pude abrir MP3 {path}: {e} — caigo a tono A4");
                seekable_handle_slot().set(None).ok();
                Box::new(ToneSource::a4())
            }
        }
    } else {
        seekable_handle_slot().set(None).ok();
        Box::new(ToneSource::a4())
    };

    // Overlay opcional de tono A4 mezclado a `MULTIMEDIA_MIX_TONE`
    // (0..1) — útil para probar el mixer con cualquier fuente. Si
    // está set y parsea bien, env la fuente principal en un MixerAudio
    // junto a un ToneSource atenuado por su propio Volume.
    let inner: Box<dyn AudioSource + Send> = match std::env::var("MULTIMEDIA_MIX_TONE")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
    {
        Some(g) if g > 0.0 => {
            let g = g.min(1.0);
            eprintln!("multimedia-app: overlay tono A4 a {:.0}%", g * 100.0);
            let tone = VolumeAudio::new(ToneSource::a4(), Volume::new(g));
            let mix = MixerAudio::with_sources(vec![inner, Box::new(tone)]);
            Box::new(mix)
        }
        _ => inner,
    };
    // Orden: Pausable envuelve al productor (silencio cuando pausado);
    // Volume aplica ganancia después de pausar; Recorded captura ese
    // mismo flujo (graba el silencio durante la pausa, igual que lo
    // escucha el sink); Probed tapea afuera para que el visor refleje
    // lo que realmente se reproduce.
    let pausable = PausableAudio::new(inner, pause().clone());
    let voled = VolumeAudio::new(pausable, volume().clone());
    let recorded = RecordedAudioSource::new(voled, recorder().clone());
    let probed = ProbedAudioSource::new(recorded, probe.clone());
    (Arc::new(Mutex::new(probed)), probe)
}
