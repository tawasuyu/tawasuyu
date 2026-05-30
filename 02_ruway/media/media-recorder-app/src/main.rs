//! Grabador de pantalla Llimphi — la integración UI del lado INPUT de
//! `media`. Un botón Rec/Stop, un timer y el estado de la grabación;
//! por debajo, el loop `ScreenSource (X11) + MicSource (cpal) →
//! media-recorder-webm → .webm AV1+Opus nativo`, sin ffmpeg.
//!
//! El bucle Elm de Llimphi (`update`/`view`) corre en el hilo de la UI
//! y **no debe bloquear**. La grabación es trabajo largo y pesado
//! (encode AV1 por frame), así que vive en un hilo de fondo lanzado con
//! [`Handle::spawn`]: la clausura corre el loop hasta que el flag de
//! stop se levanta y, al terminar, **devuelve** un `Msg::Finished` que
//! el bucle Elm recibe en `update` — cero locks compartidos con la UI
//! salvo el handle clonable del recorder.
//!
//! Corre con: `cargo run -p media-recorder-app --release`
//! (necesita `$DISPLAY`; el micrófono es opcional — sin él graba
//! video-solo).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, View};

use media_core::{AudioSource, FrameSource};
use media_recorder_webm::{
    default_recording_path, RecordedAudioSource, RecordedFrameSource, WebmRecorder,
    WebmRecorderSettings,
};
use media_source_capture::{MicSource, ScreenOptions, ScreenSource};

const FPS: u32 = 30;

/// Resumen liviano (Clone) de una grabación cerrada, para viajar en el
/// `Msg`.
#[derive(Clone)]
struct RecLite {
    path: String,
    frames: usize,
    audio_packets: usize,
    kib: f64,
}

#[derive(Clone)]
enum Msg {
    Start,
    Stop,
    Tick,
    Finished(Result<RecLite, String>),
}

enum RecState {
    Idle,
    Recording { since: Instant, path: String },
    Stopping,
    Saved(RecLite),
    Failed(String),
}

struct Model {
    state: RecState,
    rec: WebmRecorder,
    stop: Arc<AtomicBool>,
    /// Segundos transcurridos, refrescados por `Tick` mientras graba.
    elapsed_secs: u64,
}

struct RecorderApp;

impl App for RecorderApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "media · grabador de pantalla"
    }

    fn initial_size() -> (u32, u32) {
        (560, 380)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        // Timer de refresco del cronómetro (no-op cuando no graba).
        handle.spawn_periodic(Duration::from_millis(500), || Msg::Tick);
        Model {
            state: RecState::Idle,
            rec: WebmRecorder::with_settings(WebmRecorderSettings {
                fps_num: FPS,
                fps_den: 1,
                ..Default::default()
            }),
            stop: Arc::new(AtomicBool::new(false)),
            elapsed_secs: 0,
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Start => {
                if matches!(model.state, RecState::Recording { .. } | RecState::Stopping) {
                    return model; // ya grabando.
                }
                model.stop.store(false, Ordering::Release);
                let path = default_recording_path(std::env::current_dir().unwrap_or_default());
                let path_str = path.display().to_string();

                let rec = model.rec.clone();
                let stop = model.stop.clone();
                // Trabajo pesado en background; devuelve Msg::Finished al cerrar.
                handle.spawn(move || record_loop(rec, stop, path));

                model.elapsed_secs = 0;
                model.state = RecState::Recording {
                    since: Instant::now(),
                    path: path_str,
                };
            }
            Msg::Stop => {
                if let RecState::Recording { .. } = model.state {
                    model.stop.store(true, Ordering::Release);
                    model.state = RecState::Stopping;
                }
            }
            Msg::Tick => {
                if let RecState::Recording { since, .. } = &model.state {
                    model.elapsed_secs = since.elapsed().as_secs();
                }
            }
            Msg::Finished(res) => {
                model.state = match res {
                    Ok(lite) => RecState::Saved(lite),
                    Err(e) => RecState::Failed(e),
                };
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        // --- Estado / cabecera ---
        let (status, status_color) = match &model.state {
            RecState::Idle => ("listo para grabar".to_string(), rgb(170, 180, 195)),
            RecState::Recording { .. } => (
                format!("● REC  {}", fmt_mmss(model.elapsed_secs)),
                rgb(240, 90, 90),
            ),
            RecState::Stopping => ("guardando…".to_string(), rgb(230, 200, 90)),
            RecState::Saved(l) => (
                format!("✓ {} frames · {:.0} KiB", l.frames, l.kib),
                rgb(120, 210, 150),
            ),
            RecState::Failed(_) => ("error".to_string(), rgb(240, 110, 110)),
        };
        let header = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(status, 40.0, status_color);

        // --- Sub-línea: path / detalle de audio / mensaje de error ---
        let detail = match &model.state {
            RecState::Idle => "pantalla + micrófono → .webm AV1+Opus".to_string(),
            RecState::Recording { path, .. } => path.clone(),
            RecState::Stopping => "muxeando AV1+Opus…".to_string(),
            RecState::Saved(l) => {
                let audio = if l.audio_packets > 0 {
                    format!("{} paquetes Opus", l.audio_packets)
                } else {
                    "video-solo".to_string()
                };
                format!("{}  ·  {}", l.path, audio)
            }
            RecState::Failed(e) => e.clone(),
        };
        let subline = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(detail, 15.0, rgb(150, 160, 175));

        // --- Botón Rec/Stop ---
        let recording = matches!(model.state, RecState::Recording { .. });
        let stopping = matches!(model.state, RecState::Stopping);
        let (label, fill, msg) = if recording {
            ("■ Detener", rgb(220, 70, 70), Msg::Stop)
        } else if stopping {
            ("…", rgb(120, 120, 130), Msg::Stop)
        } else {
            ("● Grabar", rgb(70, 200, 130), Msg::Start)
        };
        let mut button = View::new(Style {
            size: Size {
                width: length(220.0_f32),
                height: length(64.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(fill)
        .radius(14.0)
        .text(label, 26.0, rgb(12, 24, 18));
        if !stopping {
            button = button.on_click(msg);
        }
        let button_row = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(64.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .children(vec![button]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(20.0_f32),
            },
            padding: Rect {
                left: length(28.0_f32),
                right: length(28.0_f32),
                top: length(28.0_f32),
                bottom: length(28.0_f32),
            },
            ..Default::default()
        })
        .fill(rgb(18, 22, 30))
        .children(vec![header, subline, button_row])
    }
}

/// Loop de grabación, en hilo de fondo. Devuelve el `Msg` que el bucle
/// Elm recibe al cerrar.
fn record_loop(rec: WebmRecorder, stop: Arc<AtomicBool>, path: PathBuf) -> Msg {
    let screen = match ScreenSource::open(ScreenOptions {
        fps: FPS,
        ..Default::default()
    }) {
        Ok(s) => s,
        Err(e) => return Msg::Finished(Err(format!("pantalla: {e}"))),
    };

    // Micrófono opcional: sin él, grabación video-solo.
    let mic = MicSource::open_default().ok();
    let (a_sr, a_ch) = mic
        .as_ref()
        .map(|m| (m.sample_rate(), m.channels()))
        .unwrap_or((0, 0));

    let mut recorded_v = RecordedFrameSource::new(screen, rec.clone());
    let mut recorded_a = mic.map(|m| RecordedAudioSource::new(m, rec.clone()));

    let dt = Duration::from_micros(1_000_000 / FPS as u64);
    let mut vbuf = Vec::new();
    let mut abuf: Vec<f32> = Vec::new();

    // Cebar dimensiones (start() las exige).
    let prime_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if recorded_v.tick(dt, &mut vbuf).is_some() {
            break;
        }
        if Instant::now() >= prime_deadline {
            return Msg::Finished(Err("no llegaron frames de la pantalla".into()));
        }
        std::thread::sleep(dt / 2);
    }

    if let Err(e) = rec.start(&path) {
        return Msg::Finished(Err(format!("start: {e}")));
    }

    let mut last_audio = Instant::now();
    while !stop.load(Ordering::Acquire) {
        let _ = recorded_v.tick(dt, &mut vbuf);
        if let Some(ra) = recorded_a.as_mut() {
            let frames = (a_sr as f64 * last_audio.elapsed().as_secs_f64()) as usize;
            if frames > 0 {
                abuf.clear();
                abuf.resize(frames * a_ch.max(1) as usize, 0.0);
                ra.fill(&mut abuf, a_sr, a_ch);
                last_audio = Instant::now();
            }
        }
        std::thread::sleep(dt / 2);
    }

    match rec.stop() {
        Ok((out, summary)) => Msg::Finished(Ok(RecLite {
            path: out.display().to_string(),
            frames: summary.video_frames,
            audio_packets: summary.audio_packets,
            kib: std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) as f64 / 1024.0,
        })),
        Err(e) => Msg::Finished(Err(format!("stop: {e}"))),
    }
}

/// `mm:ss` desde segundos.
fn fmt_mmss(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

#[inline]
fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

fn main() {
    llimphi_ui::run::<RecorderApp>();
}
