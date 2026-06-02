// =============================================================================
//  uya-app::audio — captura de micrófono, mezcla remota y reproducción.
// -----------------------------------------------------------------------------
//  Tres piezas:
//    · `MezclaRemota` — un `AudioSource` que el `AudioSink` (cpal) drena en su
//      callback. Acumula el audio entrante de cada par (downmix a mono), lo
//      resamplea linealmente al formato del dispositivo de salida y suma todos
//      los pares en el buffer. Lo alimenta el lector de red (ver `enlace`).
//    · `iniciar_microfono` — un hilo que tira del `MicSource` (o un tono
//      sintético si no hay micro) y difunde `Paquete::Audio` a los pares.
//    · `iniciar_reproduccion` — abre el `AudioSink` sobre la `MezclaRemota`.
//
//  Formato de cable: PCM `f32` intercalado en el formato NATIVO del emisor
//  (`sample_rate`/`canales` viajan en el paquete). Toda la conversión ocurre
//  en recepción, en `MezclaRemota`, para no resamplear de más en captura.
// =============================================================================

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use media_audio_cpal::AudioSink;
use media_core::{AudioSource, ToneSource};
use media_source_capture::MicSource;
use uya_core::{Paquete, ParticipanteId};

use crate::Enlace;

/// El audio pendiente de un par, en mono y a su sample-rate nativo.
struct ColaRemota {
    sr: u32,
    /// Muestras mono pendientes de reproducir (jitter buffer).
    muestras: VecDeque<f32>,
    /// Posición de lectura fraccionaria (en frames de entrada) para el
    /// resampleo lineal hacia el sample-rate de salida.
    frac: f64,
}

/// Mezclador de las voces remotas. Es el `AudioSource` que alimenta el sink.
#[derive(Default)]
pub struct MezclaRemota {
    remotas: HashMap<ParticipanteId, ColaRemota>,
    /// Total de muestras de audio recibidas (diagnóstico).
    recibidas: u64,
}

impl MezclaRemota {
    /// Encola un bloque de audio entrante de un par. `inter` viene intercalado
    /// a `(sr, ch)`; lo bajamos a mono y lo guardamos a su sr nativo.
    pub fn empujar(&mut self, id: ParticipanteId, sr: u32, ch: u16, inter: &[f32]) {
        let ch = ch.max(1) as usize;
        self.recibidas += inter.len() as u64;
        let cola = self.remotas.entry(id).or_insert_with(|| ColaRemota {
            sr: sr.max(1),
            muestras: VecDeque::new(),
            frac: 0.0,
        });
        cola.sr = sr.max(1);
        let frames = inter.len() / ch;
        for f in 0..frames {
            let mut acc = 0.0f32;
            for c in 0..ch {
                acc += inter[f * ch + c];
            }
            cola.muestras.push_back(acc / ch as f32);
        }
        // Jitter buffer acotado a ~1 s: si nos pasamos, soltamos lo más viejo.
        let tope = cola.sr as usize;
        while cola.muestras.len() > tope {
            cola.muestras.pop_front();
            cola.frac = 0.0;
        }
    }

    /// Saca a un par (colgó o se desconectó): su cola deja de sonar.
    pub fn quitar(&mut self, id: &ParticipanteId) {
        self.remotas.remove(id);
    }

    /// Total de muestras recibidas hasta ahora (para diagnóstico/CLI).
    pub fn recibidas(&self) -> u64 {
        self.recibidas
    }
}

impl AudioSource for MezclaRemota {
    fn fill(&mut self, buf: &mut [f32], out_sr: u32, out_ch: u16) {
        for s in buf.iter_mut() {
            *s = 0.0;
        }
        let out_ch = out_ch.max(1) as usize;
        let frames = buf.len() / out_ch;
        let out_sr = out_sr.max(1) as f64;

        for cola in self.remotas.values_mut() {
            // Frames de entrada por cada frame de salida.
            let paso = cola.sr as f64 / out_sr;
            for i in 0..frames {
                let idx = cola.frac.floor() as usize;
                // Necesitamos idx e idx+1 para interpolar; si no hay, silencio.
                if idx + 1 >= cola.muestras.len() {
                    break;
                }
                let a = cola.muestras[idx];
                let b = cola.muestras[idx + 1];
                let t = (cola.frac - idx as f64) as f32;
                let v = a + (b - a) * t;
                for c in 0..out_ch {
                    buf[i * out_ch + c] += v;
                }
                cola.frac += paso;
            }
            // Descartar las muestras enteras ya consumidas, conservar la fracción.
            let consumidas = (cola.frac.floor() as usize).min(cola.muestras.len());
            for _ in 0..consumidas {
                cola.muestras.pop_front();
            }
            cola.frac -= consumidas as f64;
        }

        // Varias voces sumadas pueden pasarse de rango: recorte suave.
        for s in buf.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

/// Arranca el hilo de captura de micrófono: tira del `MicSource` (o de un tono
/// sintético si no hay micro y `UYA_TONO` está puesto) y difunde el audio a los
/// pares mientras el micrófono esté encendido.
pub fn iniciar_microfono(enlace: Arc<Enlace>) {
    std::thread::Builder::new()
        .name("uya-mic".into())
        .spawn(move || {
            let Some((mut fuente, sr, ch)) = construir_fuente_audio() else {
                return;
            };
            // `fuente` (p. ej. MicSource con su Stream cpal) es !Send; vive y
            // muere dentro de este hilo, así que no exigimos `Send` en el box.
            // Bloque de ~20 ms a (sr, ch).
            let bloque = ((sr as usize / 50).max(1)) * ch as usize;
            let mut buf = vec![0.0f32; bloque];
            loop {
                fuente.fill(&mut buf, sr, ch);
                if enlace.microfono_encendido() {
                    enlace.emitir(&Paquete::Audio {
                        sample_rate: sr,
                        canales: ch as u8,
                        muestras: buf.clone(),
                    });
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        })
        .expect("uya: spawn hilo de micrófono");
}

/// Elige la fuente de captura: micrófono real, o un tono sintético si no hay
/// micro y el humano puso `UYA_TONO` (útil para probar el pipeline sin
/// hardware). Sin micro y sin `UYA_TONO`, no captura (audio mudo).
fn construir_fuente_audio() -> Option<(Box<dyn AudioSource>, u32, u16)> {
    match MicSource::open_default() {
        Ok(mic) => {
            let sr = mic.sample_rate();
            let ch = mic.channels();
            Some((Box::new(mic), sr, ch))
        }
        Err(e) => {
            if std::env::var("UYA_TONO").is_ok() {
                eprintln!("uya: sin micrófono ({e}); uso tono sintético (UYA_TONO)");
                Some((Box::new(ToneSource::new(330.0, 0.2)), 48_000, 1))
            } else {
                eprintln!("uya: sin micrófono ({e}); audio mudo (probá UYA_TONO=1)");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> ParticipanteId {
        [n; 32]
    }

    #[test]
    fn downmix_estereo_a_mono() {
        let mut m = MezclaRemota::default();
        // Estéreo intercalado: L=1.0 R=0.0 → mono 0.5.
        m.empujar(id(1), 48_000, 2, &[1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0]);
        assert_eq!(m.recibidas(), 8);
        let mut buf = vec![0.0f32; 4]; // out: 48k mono, mismo rate → sin resampleo
        m.fill(&mut buf, 48_000, 1);
        // Interpolando entre muestras iguales (0.5) da 0.5.
        assert!((buf[0] - 0.5).abs() < 1e-6, "buf[0]={}", buf[0]);
    }

    #[test]
    fn mezcla_suma_dos_pares() {
        let mut m = MezclaRemota::default();
        m.empujar(id(1), 48_000, 1, &[0.3, 0.3, 0.3, 0.3]);
        m.empujar(id(2), 48_000, 1, &[0.4, 0.4, 0.4, 0.4]);
        let mut buf = vec![0.0f32; 2];
        m.fill(&mut buf, 48_000, 1);
        // 0.3 + 0.4 = 0.7 (sin recorte, < 1.0).
        assert!((buf[0] - 0.7).abs() < 1e-6, "buf[0]={}", buf[0]);
    }

    #[test]
    fn salir_silencia_al_par() {
        let mut m = MezclaRemota::default();
        m.empujar(id(1), 48_000, 1, &[0.5, 0.5, 0.5, 0.5]);
        m.quitar(&id(1));
        let mut buf = vec![0.0f32; 2];
        m.fill(&mut buf, 48_000, 1);
        assert_eq!(buf, vec![0.0, 0.0]);
    }
}

/// Abre la reproducción sobre la mezcla remota. Devuelve el `AudioSink`, que el
/// llamador DEBE conservar vivo (al soltarlo, el stream de salida se cierra).
pub fn iniciar_reproduccion(mezcla: Arc<Mutex<MezclaRemota>>) -> Option<AudioSink> {
    match AudioSink::open(mezcla) {
        Ok(sink) => Some(sink),
        Err(e) => {
            eprintln!("uya: sin salida de audio ({e})");
            None
        }
    }
}
