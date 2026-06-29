//! `arje-splash` — el splash nativo del arranque sin parpadeo.
//!
//! Ente génesis de **prioridad alta**: arje-zero lo encarna apenas monta el bus
//! (antes que mirada). Toma el nodo DRM reusando el modo que dejó el GOP del
//! loader (sin re-modeset → sin flash desde el logo de arranque) y pinta un
//! splash animado (respiración del logo de marca + barra de progreso) hasta que
//! es hora de soltar la pantalla: por SIGTERM de arje-zero o por un tope de
//! tiempo (red de seguridad de Fase 1, antes del socket de handoff de Fase 2).
//!
//! Equivalente nativo de Plymouth, en Rust, propiedad nuestra de punta a punta.
//! Es **best-effort**: sin DRM/GPU (CI, dev sin pantalla) loguea y sale 0 — el
//! arranque continúa. Ver `SDD-ARRANQUE-SIN-PARPADEO.md`.
//!
//! ## Configuración (env / argv)
//!
//! - `ARJE_SPLASH_DEVICE` / primer arg posicional — nodo DRM (def `/dev/dri/card0`).
//! - `ARJE_SPLASH_MAX_MS` — tope de duración en ms antes de soltar solo
//!   (def 8000; `0` = sólo por señal).
//! - `ARJE_SPLASH_FPS` — frames por segundo objetivo (def 30).

mod config;
mod drm_present;
mod handoff;
mod image;
mod logs;
mod render;

use std::process::ExitCode;

const DEFAULT_DEVICE: &str = "/dev/dri/card0";

fn main() -> ExitCode {
    bitacora::abrir("arje");
    // Volcado headless del splash a PNG (evidencia visual, sin DRM): rinde el
    // frame real de `render::paint_frame` y lo escribe como PNG. Uso:
    //   arje-splash --dump-chakana <salida.png> [ancho alto t_ms]
    if let Some(i) = std::env::args().position(|a| a == "--dump-chakana") {
        let args: Vec<String> = std::env::args().collect();
        let path = args.get(i + 1).cloned().unwrap_or_else(|| "chakana.png".into());
        let w = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(480usize);
        let h = args.get(i + 3).and_then(|s| s.parse().ok()).unwrap_or(300usize);
        let t_ms = args.get(i + 4).and_then(|s| s.parse().ok()).unwrap_or(1200u64);
        return dump_png(&path, w, h, t_ms);
    }
    // Modo cliente de prueba (`arje-splash --poke`): simula a mirada mandando
    // READY al socket de handoff y esperando RELEASED. Para verificar Fase 2
    // end-to-end en QEMU sin levantar el compositor.
    if std::env::args().any(|a| a == "--poke") {
        match handoff::poke(&handoff::sock_path()) {
            Ok(()) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("[arje-splash --poke] error: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    let greeter_sim = std::env::args().any(|a| a == "--greeter-sim");

    // Config del splash (la escribe wawa-panel; la lee acá). Los env la pueden
    // pisar puntualmente — útil para los seeds de demo/test.
    let cfg = config::SplashCfg::load();
    let device = std::env::args()
        .nth(1)
        .filter(|a| !a.starts_with('-'))
        .or_else(|| std::env::var("ARJE_SPLASH_DEVICE").ok())
        .unwrap_or_else(|| DEFAULT_DEVICE.to_string());
    // La config manda; los env la pisan puntualmente (demos/tests).
    let max_ms = env_u64("ARJE_SPLASH_MAX_MS", cfg.max_ms);
    let fps = env_u64("ARJE_SPLASH_FPS", cfg.fps).clamp(1, 240);

    eprintln!(
        "[arje-splash] device={device} max_ms={max_ms} fps={fps} source={:?} greeter_sim={greeter_sim}",
        cfg.source
    );

    drm_present::install_signal_handlers();
    let opts = drm_present::Opts { device, max_ms, fps, cfg };
    if greeter_sim {
        drm_present::run_greeter(&opts);
    } else {
        drm_present::run(&opts);
    }

    // Siempre salimos 0: el splash es decorativo, su fallo no debe marcar el
    // Ente como CRASHED ni disparar back-off en el supervisor.
    ExitCode::SUCCESS
}

/// Rinde el frame del splash a un PNG (XRGB8888 → RGBA). Headless, sin DRM.
fn dump_png(path: &str, w: usize, h: usize, t_ms: u64) -> ExitCode {
    let pitch = w * 4;
    let mut buf = vec![0u8; pitch * h];
    render::paint_frame(&mut buf, w, h, pitch, t_ms, 0.0);
    // El buffer es XRGB8888 little-endian = bytes [B, G, R, X]; PNG quiere RGBA.
    let mut rgba = Vec::with_capacity(w * h * 4);
    for px in buf.chunks_exact(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    let file = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[arje-splash --dump-chakana] no pude crear {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    match enc.write_header().and_then(|mut wr| wr.write_image_data(&rgba)) {
        Ok(()) => {
            eprintln!("[arje-splash --dump-chakana] {path} ({w}x{h}, t={t_ms}ms)");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[arje-splash --dump-chakana] error PNG: {e}");
            ExitCode::FAILURE
        }
    }
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
