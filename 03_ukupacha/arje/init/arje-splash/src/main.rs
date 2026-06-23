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

mod drm_present;
mod render;

use std::process::ExitCode;

const DEFAULT_DEVICE: &str = "/dev/dri/card0";
const DEFAULT_MAX_MS: u64 = 8000;
const DEFAULT_FPS: u64 = 30;

fn main() -> ExitCode {
    let device = std::env::args()
        .nth(1)
        .filter(|a| !a.starts_with('-'))
        .or_else(|| std::env::var("ARJE_SPLASH_DEVICE").ok())
        .unwrap_or_else(|| DEFAULT_DEVICE.to_string());
    let max_ms = env_u64("ARJE_SPLASH_MAX_MS", DEFAULT_MAX_MS);
    let fps = env_u64("ARJE_SPLASH_FPS", DEFAULT_FPS).clamp(1, 240);

    eprintln!("[arje-splash] device={device} max_ms={max_ms} fps={fps}");

    drm_present::install_signal_handlers();
    drm_present::run(&drm_present::Opts { device, max_ms, fps });

    // Siempre salimos 0: el splash es decorativo, su fallo no debe marcar el
    // Ente como CRASHED ni disparar back-off en el supervisor.
    ExitCode::SUCCESS
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
