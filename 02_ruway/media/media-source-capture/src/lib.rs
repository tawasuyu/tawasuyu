//! media-source-capture — captura **en vivo** como [`FrameSource`].
//!
//! El lado INPUT del dominio: mientras los `media-source-*` de archivo
//! reproducen bytes en disco, este produce frames de un dispositivo en
//! tiempo real (hoy cámara v4l2; mañana captura de pantalla, sin crate
//! nuevo). Es la fuente que alimenta a `media-recorder-webm` para
//! grabar la cámara a un `.webm` AV1+Opus nativo, sin ffmpeg.
//!
//! ## Las dos piezas
//!
//! - [`LiveSource`] / [`LiveSink`] — núcleo agnóstico de hardware. Un
//!   **slot de último frame** (`Arc<Mutex>` + versión atómica): el
//!   productor empuja frames desde su propio hilo/timing, el consumidor
//!   los lee en `tick` **sin bloquear** — si no hay frame nuevo desde
//!   la última lectura, `tick` devuelve `None`. Es la disciplina
//!   correcta para una fuente en vivo dentro de un bucle de render: el
//!   render nunca se frena esperando al dispositivo, y un frame viejo
//!   nunca se re-emite. Reusable por cualquier grabber (cámara,
//!   pantalla, compute shader, red).
//! - [`CameraSource`] — backend v4l2 (feature `camera`, on por
//!   default). Abre `/dev/videoN`, negocia formato, y corre un hilo que
//!   convierte cada frame a RGBA y lo empuja al `LiveSink`. Se detiene
//!   solo al dropearse.
//!
//! La conversión de pixel-formats ([`convert`]) es pura y testeable sin
//! ningún dispositivo — vive separada del backend.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use media_core::FrameSource;

pub mod convert;
pub use convert::PixelFormat;

#[cfg(feature = "camera")]
mod camera;
#[cfg(feature = "camera")]
pub use camera::{CameraOptions, CameraSource, CaptureError};

/// Estado compartido entre [`LiveSink`] (escribe) y [`LiveSource`]
/// (lee). El `version` se incrementa cada vez que el sink deja un
/// frame; el source recuerda la última versión vista para no re-emitir.
struct Shared {
    /// `(width, height, rgba)` del último frame, o `None` hasta el
    /// primero.
    frame: Mutex<Option<(u32, u32, Vec<u8>)>>,
    version: AtomicU64,
}

/// Crea un par sink↔source conectados. El productor se queda con el
/// [`LiveSink`] (clonable, `Send`) y empuja frames desde donde quiera;
/// el consumidor conecta el [`LiveSource`] al pipeline como cualquier
/// otro [`FrameSource`].
pub fn live_channel() -> (LiveSink, LiveSource) {
    let shared = Arc::new(Shared {
        frame: Mutex::new(None),
        version: AtomicU64::new(0),
    });
    (
        LiveSink {
            shared: shared.clone(),
        },
        LiveSource {
            shared,
            last_seen: 0,
        },
    )
}

/// Extremo productor de un canal en vivo. Clonable y `Send`: varios
/// grabbers o un hilo de dispositivo lo usan para publicar el frame más
/// reciente. Sólo se conserva el último — si el consumidor va más lento
/// que el productor, los frames intermedios se descartan (lo correcto
/// para "en vivo": queremos el ahora, no la cola).
#[derive(Clone)]
pub struct LiveSink {
    shared: Arc<Shared>,
}

impl LiveSink {
    /// Publica un frame RGBA8 ya convertido (`width*height*4` bytes).
    /// Toma posesión del `Vec` para evitar copia.
    pub fn push_rgba(&self, width: u32, height: u32, rgba: Vec<u8>) {
        if rgba.len() != (width as usize) * (height as usize) * 4 {
            return; // tamaño inconsistente: descartar antes que corromper.
        }
        {
            let mut slot = self.shared.frame.lock().unwrap();
            *slot = Some((width, height, rgba));
        }
        self.shared.version.fetch_add(1, Ordering::Release);
    }

    /// Convierte un buffer crudo del dispositivo ([`PixelFormat`]) a
    /// RGBA y lo publica. Devuelve `false` si la conversión falló
    /// (frame truncado/corrupto) — en ese caso no se publica nada.
    pub fn push_raw(
        &self,
        fmt: PixelFormat,
        width: u32,
        height: u32,
        src: &[u8],
    ) -> bool {
        let mut rgba = Vec::new();
        if !convert::to_rgba(fmt, width, height, src, &mut rgba) {
            return false;
        }
        self.push_rgba(width, height, rgba);
        true
    }

    /// `true` si ya no queda ningún consumidor — el grabber puede
    /// pararse para no quemar CPU contra una pared.
    pub fn is_orphan(&self) -> bool {
        Arc::strong_count(&self.shared) <= 1
    }
}

/// Extremo consumidor: un [`FrameSource`] que entrega el último frame
/// publicado por su [`LiveSink`]. `tick` ignora el `dt` (el timing lo
/// marca el productor) y devuelve `Some` sólo cuando hay un frame más
/// nuevo que el ya leído.
pub struct LiveSource {
    shared: Arc<Shared>,
    last_seen: u64,
}

impl LiveSource {
    /// Dimensiones del último frame publicado, si hubo alguno.
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        self.shared
            .frame
            .lock()
            .unwrap()
            .as_ref()
            .map(|(w, h, _)| (*w, *h))
    }
}

impl FrameSource for LiveSource {
    fn tick(&mut self, _dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        let v = self.shared.version.load(Ordering::Acquire);
        if v == self.last_seen {
            return None; // sin frame nuevo: no tocar `buf`.
        }
        let slot = self.shared.frame.lock().unwrap();
        let (w, h, rgba) = slot.as_ref()?;
        buf.clear();
        buf.extend_from_slice(rgba);
        self.last_seen = v;
        Some((*w, *h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_empieza_vacio() {
        let (_sink, mut src) = live_channel();
        let mut buf = Vec::new();
        assert_eq!(src.tick(Duration::from_millis(16), &mut buf), None);
        assert!(buf.is_empty());
    }

    #[test]
    fn emite_solo_frames_nuevos() {
        let (sink, mut src) = live_channel();
        let mut buf = Vec::new();

        sink.push_rgba(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(src.tick(Duration::ZERO, &mut buf), Some((2, 1)));
        assert_eq!(buf, vec![1, 2, 3, 4, 5, 6, 7, 8]);

        // Sin frame nuevo → None, buf intacto.
        assert_eq!(src.tick(Duration::ZERO, &mut buf), None);
        assert_eq!(buf, vec![1, 2, 3, 4, 5, 6, 7, 8]);

        // Nuevo frame → se emite.
        sink.push_rgba(1, 1, vec![9, 9, 9, 9]);
        assert_eq!(src.tick(Duration::ZERO, &mut buf), Some((1, 1)));
        assert_eq!(buf, vec![9, 9, 9, 9]);
    }

    #[test]
    fn solo_el_ultimo_frame_sobrevive() {
        // Productor más rápido que consumidor: se descartan intermedios.
        let (sink, mut src) = live_channel();
        sink.push_rgba(1, 1, vec![1, 1, 1, 1]);
        sink.push_rgba(1, 1, vec![2, 2, 2, 2]);
        sink.push_rgba(1, 1, vec![3, 3, 3, 3]);
        let mut buf = Vec::new();
        assert_eq!(src.tick(Duration::ZERO, &mut buf), Some((1, 1)));
        assert_eq!(buf, vec![3, 3, 3, 3]); // el más reciente, no el 1.
    }

    #[test]
    fn push_raw_convierte() {
        let (sink, mut src) = live_channel();
        assert!(sink.push_raw(PixelFormat::Rgb24, 1, 1, &[10, 20, 30]));
        let mut buf = Vec::new();
        assert_eq!(src.tick(Duration::ZERO, &mut buf), Some((1, 1)));
        assert_eq!(buf, vec![10, 20, 30, 255]);
    }

    #[test]
    fn rgba_inconsistente_se_descarta() {
        let (sink, mut src) = live_channel();
        sink.push_rgba(2, 2, vec![0; 3]); // debería ser 16 bytes
        let mut buf = Vec::new();
        assert_eq!(src.tick(Duration::ZERO, &mut buf), None);
    }

    #[test]
    fn orphan_cuando_no_hay_source() {
        let (sink, src) = live_channel();
        assert!(!sink.is_orphan());
        drop(src);
        assert!(sink.is_orphan());
    }
}
