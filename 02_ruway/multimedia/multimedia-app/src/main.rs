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
//! Corre con:
//!   `cargo run -p multimedia-app --release`
//!   `cargo run -p multimedia-app --release -- /ruta/al/anim.gif`
//!   `MULTIMEDIA_MUTE=1 cargo run -p multimedia-app --release`

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
    AudioProbe, AudioSource, FrameSource, Levels, Pause, PausableAudio, PausableVideo,
    ProbedAudioSource, Spectrum, TestCard, ToneSource,
};
use multimedia_source_gif::GifSource;
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
}

struct Model {
    frames: u64,
    started_at: Instant,
}

struct Pipeline {
    surface: ExternalSurface,
    source: Mutex<Box<dyn FrameSource + Send>>,
    buf: Mutex<Vec<u8>>,
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
    init_source: fn() -> Box<dyn FrameSource + Send>,
    // El path del GIF (si aplica) viaja por otro static — fn pointers
    // no capturan, así que `init_source` lo lee de `gif_path_slot`.
}

fn gif_path_slot() -> &'static OnceLock<PathBuf> {
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

fn new_testcard() -> Box<dyn FrameSource + Send> {
    Box::new(PausableVideo::new(
        TestCard::new(TESTCARD_W, TESTCARD_H, TESTCARD_FPS),
        pause().clone(),
    ))
}

fn new_gif() -> Box<dyn FrameSource + Send> {
    let path = gif_path_slot().get().expect("gif path set");
    match GifSource::from_path(path) {
        Ok(s) => Box::new(PausableVideo::new(s, pause().clone())),
        Err(e) => {
            eprintln!("multimedia-app: error abriendo GIF {path:?}: {e} — caigo a testcard");
            new_testcard()
        }
    }
}

fn pipeline_for(device: &wgpu::Device, queue: &wgpu::Queue) -> &'static Pipeline {
    pipeline_slot().get_or_init(|| {
        let cfg = config_slot().get().expect("config set");
        Pipeline {
            surface: ExternalSurface::new(device, queue),
            source: Mutex::new((cfg.init_source)()),
            buf: Mutex::new(Vec::new()),
            last_tick: Mutex::new(Instant::now()),
        }
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
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let cfg = config_slot().get().expect("config set");
        let secs = model.started_at.elapsed().as_secs_f32().max(0.001);
        let fps = model.frames as f32 / secs;

        let paused = pause().is_paused();
        let (glyph, btn_bg, btn_fg) = if paused {
            (
                "play",
                Color::from_rgba8(60, 140, 90, 255),
                Color::from_rgba8(235, 250, 240, 255),
            )
        } else {
            (
                "pause",
                Color::from_rgba8(55, 65, 80, 255),
                Color::from_rgba8(220, 230, 245, 255),
            )
        };
        let pause_btn = View::new(Style {
            size: Size {
                width: length(72.0_f32),
                height: length(36.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(btn_bg)
        .hover_fill(Color::from_rgba8(80, 100, 130, 255))
        .radius(8.0)
        .text(glyph.to_string(), 16.0, btn_fg)
        .on_click(Msg::TogglePause);

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
        .children(vec![pause_btn, title_text, meters_panel()]);

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
        .children(vec![waveform_panel(), spectrum_panel()]);

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
            format!("ticks {} · ui ≈ {fps:.1} fps", model.frames),
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

        // Bucket por columna: agrupa frames en `cols` columnas tomando
        // el pico absoluto del bucket (más legible que el promedio para
        // formas de onda).
        let cols = inner_w.max(2.0) as usize;
        let cols = cols.min(total_frames);
        let frames_per_col = total_frames / cols.max(1);
        if frames_per_col == 0 {
            return;
        }
        let amp = inner_h * 0.5;

        let mut path = BezPath::new();
        for col in 0..cols {
            let f0 = col * frames_per_col;
            let f1 = ((col + 1) * frames_per_col).min(total_frames);
            let mut peak = 0.0_f32;
            for f in f0..f1 {
                // Mono = promedio simple de canales del frame.
                let mut acc = 0.0_f32;
                for ch in 0..channels {
                    acc += snap[f * channels + ch];
                }
                let v = acc / channels as f32;
                if v.abs() > peak.abs() {
                    peak = v;
                }
            }
            let x = inner_x + (col as f32 / (cols as f32 - 1.0).max(1.0)) * inner_w;
            let y = mid_y - peak.clamp(-1.0, 1.0) * amp;
            if col == 0 {
                path.move_to((x as f64, y as f64));
            } else {
                path.line_to((x as f64, y as f64));
            }
        }
        scene.stroke(
            &Stroke::new(1.5),
            Affine::IDENTITY,
            stroke_color,
            None,
            &path,
        );
    })
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
fn spectrum_panel<Msg: 'static>() -> View<Msg> {
    let probe = audio_probe_slot().get().cloned().flatten();
    let scratch: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let analyzer: Arc<Mutex<Spectrum>> =
        Arc::new(Mutex::new(Spectrum::log_bands(28, 40.0, 16_000.0)));
    let bar_color = Color::from_rgba8(255, 175, 95, 255);
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
        let pad_x: f32 = 12.0;
        let pad_y: f32 = 10.0;
        let inner_x = rect.x + pad_x;
        let inner_y = rect.y + pad_y;
        let inner_w = (rect.w - 2.0 * pad_x).max(1.0);
        let inner_h = (rect.h - 2.0 * pad_y).max(1.0);
        let baseline = inner_y + inner_h;

        let Some(probe) = probe.as_ref() else {
            // Sin probe: dibuja una línea de base apagada como hint
            // de que el visor está vivo.
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
        let mut analyzer = analyzer.lock();
        analyzer.analyze(&snap, channels, sr);
        let mags = analyzer.magnitudes();
        let n_bands = mags.len();
        if n_bands == 0 {
            return;
        }
        let gap: f32 = 2.0;
        let total_gap = gap * (n_bands as f32 + 1.0);
        let bar_w = ((inner_w - total_gap) / n_bands as f32).max(1.0);
        for (i, &m) in mags.iter().enumerate() {
            let h = (m.clamp(0.0, 1.0) * inner_h).max(1.0);
            let x0 = inner_x + gap + i as f32 * (bar_w + gap);
            let y0 = baseline - h;
            let rect = KurboRect::new(
                x0 as f64,
                y0 as f64,
                (x0 + bar_w) as f64,
                baseline as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, bar_color, None, &rect);
        }
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match args.first() {
        Some(path) => {
            let path = PathBuf::from(path);
            let label = format!("gif {}", path.display());
            gif_path_slot().set(path).ok();
            Config {
                label,
                init_source: new_gif,
            }
        }
        None => Config {
            label: format!("testcard {TESTCARD_W}×{TESTCARD_H} @ {TESTCARD_FPS:.0} fps"),
            init_source: new_testcard,
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
    let inner: Box<dyn AudioSource + Send> = if let Ok(path) = std::env::var("MULTIMEDIA_WAV") {
        match WavSource::from_path(&path) {
            Ok(wav) => {
                eprintln!(
                    "multimedia-app: wav {path} · {} ch · {} Hz · {:.1}s",
                    wav.source_channels(),
                    wav.source_sample_rate(),
                    wav.duration_seconds(),
                );
                Box::new(wav)
            }
            Err(e) => {
                eprintln!("multimedia-app: no pude abrir WAV {path}: {e} — caigo a tono A4");
                Box::new(ToneSource::a4())
            }
        }
    } else {
        Box::new(ToneSource::a4())
    };
    // Orden: Pausable envuelve al productor (silencio cuando pausado),
    // y luego Probed lo tapea — así el visor también queda en silencio
    // mientras dura la pausa.
    let pausable = PausableAudio::new(inner, pause().clone());
    let probed = ProbedAudioSource::new(pausable, probe.clone());
    (Arc::new(Mutex::new(probed)), probe)
}
