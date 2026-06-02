// =============================================================================
//  uya-app::captura — el hilo que produce mis cuadros de video.
// -----------------------------------------------------------------------------
//  Por defecto la "cámara" es una TestCard de `media-core` (gradiente animado +
//  círculo) — el pipeline anda en cualquier máquina, sin hardware ni v4l2. Con
//  la feature `camara` intenta abrir una webcam real (v4l2) y, si no hay, cae a
//  la TestCard. Cada cuadro nuevo: (1) se difunde a los pares vía `Enlace`, y
//  (2) se reinyecta como `EventoUya::Cuadro` con mi propio id para el preview.
// =============================================================================

use std::sync::Arc;
use std::time::{Duration, Instant};

use media_core::{FrameSource, TestCard};
use uya_core::{FormatoCuadro, Paquete};

use crate::{Enlace, EventoUya};

/// Arranca el hilo de captura sobre un `Enlace` ya abierto. `ancho`/`alto` es la
/// resolución objetivo de la TestCard (la webcam real impone la suya). `fps` es
/// la cadencia objetivo.
pub fn iniciar_camara(enlace: Arc<Enlace>, ancho: u16, alto: u16, fps: f32) {
    let eventos = enlace.eventos();
    std::thread::Builder::new()
        .name("uya-camara".into())
        .spawn(move || {
            let mut fuente = construir_fuente(ancho, alto, fps);
            let mut buf: Vec<u8> = Vec::new();
            let mut ultimo = Instant::now();
            let mut seq: u32 = 0;
            let mut reporto_tam = false;
            let yo = enlace.yo();
            loop {
                let ahora = Instant::now();
                let dt = ahora.saturating_duration_since(ultimo);
                ultimo = ahora;

                if let Some((w, h)) = fuente.tick(dt, &mut buf) {
                    if enlace.camara_encendida() {
                        seq = seq.wrapping_add(1);
                        // Para el cable: JPEG comprimido. Si el encode falla
                        // (no debería), caemos a RGBA crudo.
                        let (formato, datos) = match crate::video::encodar_jpeg(
                            &buf,
                            w,
                            h,
                            crate::video::CALIDAD,
                        ) {
                            Some(jpeg) => {
                                if !reporto_tam {
                                    eprintln!(
                                        "uya: cuadro {w}x{h} RGBA={} → JPEG={} ({:.1}%)",
                                        buf.len(),
                                        jpeg.len(),
                                        100.0 * jpeg.len() as f32 / buf.len().max(1) as f32
                                    );
                                    reporto_tam = true;
                                }
                                (FormatoCuadro::Jpeg, jpeg)
                            }
                            None => (FormatoCuadro::Rgba, buf.clone()),
                        };
                        enlace.emitir(&Paquete::Cuadro {
                            ancho: w as u16,
                            alto: h as u16,
                            seq,
                            formato,
                            datos,
                        });
                        // Preview local: RGBA crudo, sin pasar por el códec.
                        let enviado = eventos.send(EventoUya::Cuadro {
                            id: yo,
                            ancho: w as u16,
                            alto: h as u16,
                            rgba: Arc::new(buf.clone()),
                        });
                        // La UI se cerró: no tiene sentido seguir capturando.
                        if enviado.is_err() {
                            break;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        })
        .expect("uya: spawn hilo de cámara");
}

/// Elige la fuente de video: webcam real bajo la feature `camara`, TestCard si
/// no hay cámara o la feature está apagada.
fn construir_fuente(ancho: u16, alto: u16, fps: f32) -> Box<dyn FrameSource + Send> {
    #[cfg(feature = "camara")]
    {
        match media_source_capture::CameraSource::open_default() {
            Ok(cam) => return Box::new(cam),
            Err(e) => eprintln!("uya: sin webcam ({e:?}), uso TestCard"),
        }
    }
    Box::new(TestCard::new(ancho as u32, alto as u32, fps))
}
