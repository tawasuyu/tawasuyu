//! Graba la pantalla a un `.webm` AV1+Opus **nativo**, sin ffmpeg —
//! el loop completo del lado INPUT cerrado de punta a punta:
//!
//! ```text
//! ScreenSource (X11 GetImage) ──▶ RecordedFrameSource ──▶ media-recorder-webm ──▶ .webm AV1
//! ```
//!
//! Necesita la feature `screen` (X11 vía x11rb) y un `$DISPLAY` activo.
//!
//! ```bash
//! cargo run -p media-source-capture --example grabar_pantalla \
//!     --features screen --release -- [segundos] [salida.webm] [fps]
//! ```
//!
//! Defaults: 5 segundos, `pantalla.webm` en el cwd, 30 fps. Sin audio
//! (este ejemplo es video puro; para mezclar audio se añade un
//! `RecordedAudioSource` sobre un `AudioSource`, igual que la cámara).

use std::time::{Duration, Instant};

use media_core::FrameSource;
use media_recorder_webm::{RecordedFrameSource, WebmRecorder, WebmRecorderSettings};
use media_source_capture::{ScreenOptions, ScreenSource};

fn main() {
    let mut args = std::env::args().skip(1);
    let secs: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let path = args.next().unwrap_or_else(|| "pantalla.webm".to_string());
    let fps: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30).max(1);

    // Abrir la captura. `open` bloquea hasta negociar geometría/formato;
    // un error aquí (no hay $DISPLAY, etc.) llega sincrónico.
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
    println!(
        "capturando {w}×{h} {:?} @ {fps} fps por {secs:.1}s → {path}",
        screen.format()
    );

    let rec = WebmRecorder::with_settings(WebmRecorderSettings {
        fps_num: fps,
        fps_den: 1,
        ..Default::default()
    });
    let mut recorded = RecordedFrameSource::new(screen, rec.clone());
    let mut buf = Vec::new();

    let dt = Duration::from_micros(1_000_000 / fps as u64);

    // Cebar dimensiones: el recorder rechaza `start()` con `NoFormatYet`
    // hasta ver el primer frame. El hilo de captura empuja al ritmo de
    // `fps`, así que esperamos a lo sumo unos pocos intervalos.
    let prime_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if recorded.tick(dt, &mut buf).is_some() {
            break; // ya hay dimensiones.
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

    // Loop de grabación. Tickeamos algo más rápido que la captura para
    // no perder frames; los ticks sin frame nuevo devuelven `None` y no
    // graban duplicados (garantía del `LiveSource`).
    let start = Instant::now();
    let total = Duration::from_secs_f64(secs);
    while start.elapsed() < total {
        let _ = recorded.tick(dt, &mut buf);
        std::thread::sleep(dt / 2);
    }

    match rec.stop() {
        Ok((out, summary)) => {
            let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            println!(
                "listo: {} · {} frames de video · {} paquetes de audio · {} descartados · {:.1} KiB",
                out.display(),
                summary.video_frames,
                summary.audio_packets,
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
