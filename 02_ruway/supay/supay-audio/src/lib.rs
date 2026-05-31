//! `supay-audio` — Fase 4.0 del proyecto supay: efectos de sonido.
//!
//! El motor (doomgeneric) ya llama `I_StartSound` en los momentos
//! correctos del gameplay; `supay-core` intercepta esos eventos en un
//! ring buffer (`audio_stubs.c`) que el host drena con
//! `DoomEngine::poll_sounds`. Este crate cierra la cadena:
//!
//! ```text
//! poll_sounds() -> SoundEvent{name,vol,sep}
//!     └─ AudioEngine::play("DS"+name) -> Wad::sound (DMX → f32)
//!         └─ DoomMixer (AudioSource) -> media-audio-cpal AudioSink -> speakers
//! ```
//!
//! El [`DoomMixer`] resamplea cada sfx de su rate nativo (11025 Hz) al
//! rate del dispositivo con interpolación lineal y aplica balance L/R
//! desde el `sep` estéreo de Doom. La música (MUS/MIDI) queda para
//! Fase 4.1+.
//!
//! El sink se reutiliza de `02_ruway/media` (regla #2: las UIs y los
//! drivers son intercambiables; no reimplementamos cpal acá).

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;

use media_audio_cpal::{AudioSink, OpenError};
use media_core::AudioSource;
use parking_lot::Mutex;
use supay_wad::Wad;

/// Máximo de voces simultáneas. Doom raramente supera ~8 canales; 32
/// da margen sin riesgo de saturar el mix.
const MAX_VOICES: usize = 32;

/// Una voz activa: muestras del sfx + cursor de reproducción (en
/// unidades de muestra *source*) + ganancias por canal.
struct Voice {
    samples: Arc<[f32]>,
    src_rate: f32,
    /// Posición fraccional dentro de `samples` (avanza `src_rate/dev_rate`
    /// por frame del dispositivo).
    cursor: f64,
    gain_l: f32,
    gain_r: f32,
}

/// Mixer de SFX. Implementa [`AudioSource`]: en cada callback del sink
/// mezcla todas las voces activas en el buffer interleaved del
/// dispositivo, resampleando linealmente y descartando las voces que
/// llegaron al final.
pub struct DoomMixer {
    voices: Vec<Voice>,
    /// Ganancia maestra para evitar clipping al sumar varias voces.
    master: f32,
}

impl Default for DoomMixer {
    fn default() -> Self {
        Self::new()
    }
}

impl DoomMixer {
    pub fn new() -> Self {
        Self {
            voices: Vec::new(),
            master: 0.6,
        }
    }

    /// Encola una voz nueva. Si se alcanzó [`MAX_VOICES`], dropea la más
    /// vieja (probablemente casi terminada).
    fn add(&mut self, samples: Arc<[f32]>, src_rate: f32, gain_l: f32, gain_r: f32) {
        if samples.is_empty() {
            return;
        }
        if self.voices.len() >= MAX_VOICES {
            self.voices.remove(0);
        }
        self.voices.push(Voice {
            samples,
            src_rate: src_rate.max(1.0),
            cursor: 0.0,
            gain_l,
            gain_r,
        });
    }

    /// Cantidad de voces sonando ahora (útil en tests).
    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }
}

impl AudioSource for DoomMixer {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        for s in buf.iter_mut() {
            *s = 0.0;
        }
        let ch = channels.max(1) as usize;
        let frames = buf.len() / ch;
        let dev_rate = sample_rate.max(1) as f64;
        let master = self.master;

        for v in self.voices.iter_mut() {
            let step = v.src_rate as f64 / dev_rate;
            let n = v.samples.len();
            for f in 0..frames {
                let i0 = v.cursor.floor() as usize;
                if i0 >= n {
                    break; // voz agotada en mitad del buffer → resto silencio
                }
                let frac = (v.cursor - i0 as f64) as f32;
                let s0 = v.samples[i0];
                let s1 = if i0 + 1 < n { v.samples[i0 + 1] } else { s0 };
                let s = (s0 + (s1 - s0) * frac) * master;
                let base = f * ch;
                if ch >= 2 {
                    buf[base] += s * v.gain_l;
                    buf[base + 1] += s * v.gain_r;
                    // Canales extra (5.1 etc.) quedan en silencio — MVP estéreo.
                } else {
                    buf[base] += s * (v.gain_l + v.gain_r) * 0.5;
                }
                v.cursor += step;
            }
        }

        // Descartar voces que ya pasaron el final.
        self.voices.retain(|v| (v.cursor.floor() as usize) < v.samples.len());
    }
}

/// Motor de audio: abre el sink cpal, cachea los sfx decodificados del
/// WAD y reproduce eventos del motor. Mantené una instancia viva
/// mientras corra el juego — al dropearla, el stream cpal se cierra.
pub struct AudioEngine {
    mixer: Arc<Mutex<DoomMixer>>,
    // El sink debe vivir tanto como el engine (al dropearlo se corta el
    // stream). No lo tocamos tras abrirlo.
    _sink: AudioSink,
    wad: Wad,
    /// Cache `lump DS* → (muestras, rate)`. `None` = lump ausente (no
    /// reintenta). Evita re-decodificar el mismo sfx en cada disparo.
    cache: HashMap<String, Option<(Arc<[f32]>, f32)>>,
}

impl AudioEngine {
    /// Abre el dispositivo de salida por defecto y arranca el stream.
    /// Falla si no hay dispositivo o el formato no es soportado — el
    /// host debe degradar a "sin audio" en ese caso.
    pub fn new(wad: Wad) -> Result<Self, OpenError> {
        let mixer = Arc::new(Mutex::new(DoomMixer::new()));
        let source: Arc<Mutex<dyn AudioSource + Send>> = mixer.clone();
        let sink = AudioSink::open(source)?;
        Ok(Self {
            mixer,
            _sink: sink,
            wad,
            cache: HashMap::new(),
        })
    }

    /// Reproduce un sfx por su nombre base (e.g. `"pistol"` → lump
    /// `DSPISTOL`). `vol` 0..127, `sep` 0..255 (128 ≈ centro). Si el
    /// lump no existe, no hace nada.
    pub fn play(&mut self, name: &str, vol: u8, sep: u8) {
        let lump = format!("DS{}", name.to_uppercase());
        let resolved = self.resolve(&lump);
        if let Some((samples, rate)) = resolved {
            let g = vol as f32 / 127.0;
            // sep: 0 = izquierda total, 255 = derecha total, 128 = centro.
            let pan = sep as f32 / 255.0;
            let gain_l = g * (1.0 - pan);
            let gain_r = g * pan;
            self.mixer.lock().add(samples, rate, gain_l, gain_r);
        }
    }

    fn resolve(&mut self, lump: &str) -> Option<(Arc<[f32]>, f32)> {
        if let Some(cached) = self.cache.get(lump) {
            return cached.clone();
        }
        let decoded = self.wad.sound(lump).map(|snd| {
            let arc: Arc<[f32]> = Arc::from(snd.samples);
            (arc, snd.sample_rate as f32)
        });
        self.cache.insert(lump.to_string(), decoded.clone());
        decoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice_samples() -> Arc<[f32]> {
        // Cuatro muestras distintas, sin resample (src==dev).
        Arc::from(vec![1.0f32, 0.5, -0.5, -1.0])
    }

    #[test]
    fn mixer_applies_pan_gains_stereo() {
        let mut m = DoomMixer::new();
        m.master = 1.0;
        // pan a la derecha: gain_l=0, gain_r=1.
        m.add(voice_samples(), 100.0, 0.0, 1.0);
        let mut buf = vec![0.0f32; 8]; // 4 frames estéreo
        m.fill(&mut buf, 100, 2);
        // Canal izquierdo (índices pares) en silencio.
        assert!(buf[0].abs() < 1e-6 && buf[2].abs() < 1e-6);
        // Canal derecho (impares) = las muestras.
        assert!((buf[1] - 1.0).abs() < 1e-6);
        assert!((buf[3] - 0.5).abs() < 1e-6);
        assert!((buf[5] + 0.5).abs() < 1e-6);
    }

    #[test]
    fn mixer_resamples_when_rates_differ() {
        let mut m = DoomMixer::new();
        m.master = 1.0;
        // src_rate = mitad del dev_rate → cada muestra source dura 2 frames.
        m.add(voice_samples(), 50.0, 1.0, 1.0);
        let mut buf = vec![0.0f32; 4]; // 4 frames mono
        m.fill(&mut buf, 100, 1);
        // step = 0.5: cursors 0.0, 0.5, 1.0, 1.5.
        // frame0: s=1.0; frame1: interp(1.0,0.5,0.5)=0.75;
        // frame2: s=0.5; frame3: interp(0.5,-0.5,0.5)=0.0.
        assert!((buf[0] - 1.0).abs() < 1e-6, "buf0={}", buf[0]);
        assert!((buf[1] - 0.75).abs() < 1e-6, "buf1={}", buf[1]);
        assert!((buf[2] - 0.5).abs() < 1e-6, "buf2={}", buf[2]);
        assert!((buf[3] - 0.0).abs() < 1e-6, "buf3={}", buf[3]);
    }

    #[test]
    fn mixer_drops_finished_voices() {
        let mut m = DoomMixer::new();
        m.add(voice_samples(), 100.0, 1.0, 1.0);
        assert_eq!(m.active_voices(), 1);
        // Buffer largo: consume las 4 muestras y deja la voz agotada.
        let mut buf = vec![0.0f32; 16];
        m.fill(&mut buf, 100, 1);
        assert_eq!(m.active_voices(), 0, "la voz agotada debe descartarse");
    }

    #[test]
    fn mixer_silent_without_voices() {
        let mut m = DoomMixer::new();
        let mut buf = vec![0.3f32; 8];
        m.fill(&mut buf, 100, 2);
        assert!(buf.iter().all(|&s| s == 0.0), "sin voces el buffer es silencio");
    }
}
