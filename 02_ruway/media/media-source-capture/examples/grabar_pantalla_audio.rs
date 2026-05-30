//! Screencast **con audio**: graba pantalla (X11) + micrófono (cpal) a
//! un único `.webm` AV1+Opus **nativo**, sin ffmpeg. Cierra el caso de
//! uso real del lado INPUT del dominio.
//!
//! ```text
//! ScreenSource (X11) ─▶ RecordedFrameSource ─┐
//!                                            ├─▶ media-recorder-webm ─▶ .webm AV1+Opus
//! MicSource   (cpal) ─▶ RecordedAudioSource ─┘
//! ```
//!
//! Necesita las features `screen` + `mic` y un `$DISPLAY` + un input
//! device. Un loop único drena ambas fuentes: tickea el video y, según
//! el tiempo transcurrido, pide al audio las muestras acumuladas (el
//! callback del micrófono llena el ring en realtime; `fill` lo drena y
//! rellena con silencio si hubo underrun).
//!
//! ```bash
//! cargo run -p media-source-capture --example grabar_pantalla_audio \
//!     --features "screen mic" --release -- [segundos] [salida.webm] [fps]
//! ```
//!
//! El recorder encodea Opus si el micrófono da un rate Opus-able
//! (8/12/16/24/48 kHz — `MicSource` pide 48 kHz); si el device sólo da
//! 44.1 kHz, la grabación degrada limpio a video-solo.

use std::time::{Duration, Instant};

use media_core::{AudioSource, FrameSource};
use media_recorder_webm::{
    RecordedAudioSource, RecordedFrameSource, WebmRecorder, WebmRecorderSettings,
};
use media_source_capture::{MicSource, ScreenOptions, ScreenSource};

fn main() {
    let mut args = std::env::args().skip(1);
    let secs: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let path = args.next().unwrap_or_else(|| "pantalla.webm".to_string());
    let fps: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30).max(1);

    // --- Video ---
    let screen = match ScreenSource::open(ScreenOptions {
        fps,
        ..Default::default()
    }) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("no se pudo abrir la captura de pantalla: {e}");
            std::process::exit(1);
        }
    };
    let (w, h) = (screen.width(), screen.height());

    // --- Audio ---
    let mic = match MicSource::open_default() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("no se pudo abrir el micrófono: {e}");
            std::process::exit(1);
        }
    };
    let (a_sr, a_ch) = (mic.sample_rate(), mic.channels());
    println!(
        "video {w}×{h} {:?} @ {fps} fps · audio {a_sr} Hz × {a_ch}ch · {secs:.1}s → {path}",
        screen.format()
    );
    if !matches!(a_sr, 8000 | 12000 | 16000 | 24000 | 48000) {
        eprintln!(
            "aviso: {a_sr} Hz no es Opus-able → la grabación quedará video-solo (probá 48 kHz)"
        );
    }

    let rec = WebmRecorder::with_settings(WebmRecorderSettings {
        fps_num: fps,
        fps_den: 1,
        ..Default::default()
    });
    let mut recorded_v = RecordedFrameSource::new(screen, rec.clone());
    let mut recorded_a = RecordedAudioSource::new(mic, rec.clone());

    let dt = Duration::from_micros(1_000_000 / fps as u64);
    let mut vbuf = Vec::new();
    let mut abuf: Vec<f32> = Vec::new();

    // Cebar dimensiones del video antes de `start()` (NoFormatYet).
    let prime_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if recorded_v.tick(dt, &mut vbuf).is_some() {
            break;
        }
        if Instant::now() >= prime_deadline {
            eprintln!("no llegó ningún frame de la pantalla en 3s — ¿servidor X vivo?");
            std::process::exit(1);
        }
        std::thread::sleep(dt / 2);
    }

    if let Err(e) = rec.start(&path) {
        eprintln!("no se pudo iniciar la grabación: {e}");
        std::process::exit(1);
    }

    // Loop único: video por frame + audio por tiempo transcurrido.
    let start = Instant::now();
    let total = Duration::from_secs_f64(secs);
    let mut last_audio = Instant::now();
    while start.elapsed() < total {
        let _ = recorded_v.tick(dt, &mut vbuf);

        // Audio: pedir las muestras que deberían haber entrado desde el
        // último pull, para que la duración del audio siga al reloj.
        let elapsed = last_audio.elapsed();
        let frames = (a_sr as f64 * elapsed.as_secs_f64()) as usize;
        if frames > 0 {
            abuf.clear();
            abuf.resize(frames * a_ch as usize, 0.0);
            recorded_a.fill(&mut abuf, a_sr, a_ch);
            last_audio = Instant::now();
        }

        std::thread::sleep(dt / 2);
    }

    match rec.stop() {
        Ok((out, summary)) => {
            let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            let audio = if summary.audio_packets > 0 {
                format!(
                    "{} paquetes Opus ({} Hz × {}ch)",
                    summary.audio_packets, summary.audio_sample_rate, summary.audio_channels
                )
            } else {
                "sin audio (video-solo)".to_string()
            };
            println!(
                "listo: {} · {} frames de video · {audio} · {} descartados · {:.1} KiB",
                out.display(),
                summary.video_frames,
                rec.dropped_frames(),
                size as f64 / 1024.0,
            );
        }
        Err(e) => {
            eprintln!("no se pudo cerrar la grabación: {e}");
            std::process::exit(1);
        }
    }
}
