//! Wallpaper en **video** — el worker de decodificación.
//!
//! Un hilo es dueño de una [`foreign_av::FfmpegVideoSource`] (ffmpeg detrás) y
//! decodifica el archivo a frames RGBA a su fps, en **loop** (al EOF rebobina
//! con `seek_to(0)`). Publica el último frame en un buffer compartido; el render
//! (`DrmState`) lo consume con [`VideoWallpaper::take_new_frame`], lo compone al
//! tamaño de cada salida y lo pinta como fondo.
//!
//! **Por qué un hilo y no el bucle de render:** leer del pipe de ffmpeg
//! (`read_exact`) bloquea; hacerlo en el `tick` de 60 Hz congelaría el
//! compositor. El worker corre aparte y el render sólo copia el último frame
//! listo (sin bloquear nunca).
//!
//! **Costo y apagado:** decodificar+subir por frame cuesta; por eso el efecto es
//! opt-in (`wallpaper_source = "video"`). Cuando una ventana a pantalla completa
//! tapa el fondo, o la sesión está en otra VT, el render llama
//! [`VideoWallpaper::set_paused(true)`](VideoWallpaper::set_paused) y el worker
//! deja de decodificar (no se ve, no se gasta).

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use media_core::{FrameSource, Seekable};

/// Frame compartido worker→render. `gen` sube en cada frame nuevo; el render
/// compara contra el último que consumió para no recomponer de gusto.
#[derive(Default)]
struct Shared {
    rgba: Vec<u8>,
    w: u32,
    h: u32,
    gen: u64,
}

/// Estado de control compartido (pausa / parada) — átomos para no tomar el lock
/// del frame sólo para pausar.
struct Control {
    paused: std::sync::atomic::AtomicBool,
    stop: std::sync::atomic::AtomicBool,
}

/// Handle del wallpaper en video: dueño del hilo decodificador y del último
/// frame. Vive en `DrmState`; al soltarse (cambio de fuente/archivo o cierre)
/// para el hilo y lo espera (`Drop`).
pub(crate) struct VideoWallpaper {
    /// Ruta en reproducción — para detectar cambios de config sin reiniciar.
    path: String,
    /// fps efectivo configurado (`0` = el nativo del archivo).
    fps: u32,
    shared: Arc<Mutex<Shared>>,
    ctrl: Arc<Control>,
    handle: Option<JoinHandle<()>>,
    /// Última `gen` que el render consumió.
    last_seen_gen: u64,
}

impl VideoWallpaper {
    /// Arranca el worker para `path`. `fps` `0` = el nativo del archivo.
    pub(crate) fn start(path: &str, fps: u32) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let ctrl = Arc::new(Control {
            paused: std::sync::atomic::AtomicBool::new(false),
            stop: std::sync::atomic::AtomicBool::new(false),
        });
        let handle = spawn_worker(path.to_string(), fps, shared.clone(), ctrl.clone());
        Self {
            path: path.to_string(),
            fps,
            shared,
            ctrl,
            handle: Some(handle),
            last_seen_gen: 0,
        }
    }

    /// ¿Sigue describiendo la misma fuente que la config? Si cambió el archivo o
    /// el fps, el llamante reemplaza el worker.
    pub(crate) fn matches(&self, path: &str, fps: u32) -> bool {
        self.path == path && self.fps == fps
    }

    /// Pausa/reanuda la decodificación (cuando el fondo no se ve).
    pub(crate) fn set_paused(&self, paused: bool) {
        self.ctrl.paused.store(paused, std::sync::atomic::Ordering::Relaxed);
    }

    /// Devuelve el último frame **si es nuevo** desde la última llamada (copia
    /// de los bytes RGBA + dimensiones). `None` si no hay frame nuevo.
    pub(crate) fn take_new_frame(&mut self) -> Option<(Vec<u8>, u32, u32)> {
        let s = self.shared.lock().ok()?;
        if s.gen == self.last_seen_gen || s.w == 0 || s.h == 0 || s.rgba.is_empty() {
            return None;
        }
        self.last_seen_gen = s.gen;
        Some((s.rgba.clone(), s.w, s.h))
    }
}

impl Drop for VideoWallpaper {
    fn drop(&mut self) {
        self.ctrl.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Construye la fuente de video desde la ruta (probe → session → source).
/// `None` (con log único) si el archivo no abre o no tiene video.
fn build_source(path: &str) -> Option<foreign_av::FfmpegVideoSource> {
    let info = match foreign_av::probe(path) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("mirada-compositor · wallpaper-video «{path}» no abre ({e}); fondo fijo.");
            return None;
        }
    };
    let session = foreign_av::MediaSession::open(info).ok()?;
    foreign_av::FfmpegVideoSource::from_session(session).ok()
}

fn spawn_worker(
    path: String,
    fps: u32,
    shared: Arc<Mutex<Shared>>,
    ctrl: Arc<Control>,
) -> JoinHandle<()> {
    use std::sync::atomic::Ordering::Relaxed;
    std::thread::spawn(move || {
        let Some(mut source) = build_source(&path) else {
            return; // sin fuente válida: el render cae al fondo fijo.
        };
        // Cadencia: el fps configurado, o el nativo del archivo.
        let eff_fps = if fps > 0 { fps as f32 } else { source.fps() };
        let interval = Duration::from_secs_f32(1.0 / eff_fps.max(1.0));
        let mut buf: Vec<u8> = Vec::new();
        loop {
            if ctrl.stop.load(Relaxed) {
                break;
            }
            if ctrl.paused.load(Relaxed) {
                std::thread::sleep(Duration::from_millis(80));
                continue;
            }
            std::thread::sleep(interval);
            match source.step_frame(&mut buf) {
                Some((w, h)) => {
                    if let Ok(mut s) = shared.lock() {
                        s.rgba.clear();
                        s.rgba.extend_from_slice(&buf);
                        s.w = w;
                        s.h = h;
                        s.gen = s.gen.wrapping_add(1);
                    }
                }
                // EOF → loop: rebobiná al inicio. El próximo `step_frame`
                // reengancha el pipe nuevo (refresh_if_needed) y sigue.
                None => source.seek_to(Duration::ZERO),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::VideoWallpaper;
    use std::time::Duration;

    #[test]
    fn ruta_invalida_degrada_sin_panico() {
        // Un archivo que no existe: el worker no encuentra fuente, sale solo, y
        // el consumidor nunca ve un frame. `Drop` une el hilo sin colgar.
        let mut vw = VideoWallpaper::start("/no/existe/jamas.mp4", 24);
        assert!(vw.matches("/no/existe/jamas.mp4", 24));
        assert!(!vw.matches("/otro.mp4", 24));
        assert!(!vw.matches("/no/existe/jamas.mp4", 30));
        std::thread::sleep(Duration::from_millis(150));
        assert!(vw.take_new_frame().is_none(), "una ruta inválida no produce frames");
    }
}
