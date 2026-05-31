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

/// Canal de percusión MIDI/MUS — sus "notas" son índices de drum kit,
/// no alturas. Sin banco de samples las saltamos (sonarían como notas
/// pitcheadas raras). Fase 4.2 las mapeará a samples reales.
const MUS_PERCUSSION_CHANNEL: u8 = 15;

/// Frecuencia de una nota MIDI (A4=69 → 440 Hz).
fn midi_to_freq(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
}

/// Una nota sonando en el synth de música: oscilador con fase + envolvente
/// de release corta para evitar clicks al soltar.
struct MusicVoice {
    channel: u8,
    note: u8,
    freq: f32,
    phase: f32,
    vel: u8,
    /// `Some(g)` = soltándose, `g` baja de 1→0; al llegar a 0 se descarta.
    release: Option<f32>,
}

/// Synth de música MVP: reproduce un timeline MUS a 140 Hz con osciladores
/// sinusoidales por nota (sin banco GENMIDI — "feo pero suena la melodía").
/// Mezcla **aditiva** sobre el buffer (lo llena el sfx mixer antes).
pub struct MusicSynth {
    steps: Vec<supay_wad::MusStep>,
    looping: bool,
    /// Índice del próximo step a disparar.
    cursor: usize,
    /// Posición de reproducción en ticks de 140 Hz (f64 para precisión).
    tick_pos: f64,
    /// Tiempo (en ticks) en que dispara el step `cursor`.
    next_fire: f64,
    voices: Vec<MusicVoice>,
    /// Volumen por canal MUS (controller #3), 0..127.
    channel_vol: [u8; 16],
    /// `true` cuando llegó al `End` y no es loop — deja de avanzar.
    finished: bool,
}

const MUS_TICK_HZ: f64 = 140.0;
/// Duración del release en segundos (fade-out para matar clicks).
const MUSIC_RELEASE_S: f32 = 0.04;
/// Ganancia por voz antes del soft-limit — bajo porque varias notas suman.
const MUSIC_VOICE_GAIN: f32 = 0.14;

impl MusicSynth {
    pub fn new(song: supay_wad::MusSong, looping: bool) -> Self {
        let first_delay = song.steps.first().map(|s| s.delay).unwrap_or(0) as f64;
        Self {
            steps: song.steps,
            looping,
            cursor: 0,
            tick_pos: 0.0,
            next_fire: first_delay,
            voices: Vec::new(),
            channel_vol: [127; 16],
            finished: false,
        }
    }

    fn apply(&mut self, ev: supay_wad::MusEvent) {
        use supay_wad::MusEvent::*;
        match ev {
            NoteOn { channel, note, vel } => {
                if channel == MUS_PERCUSSION_CHANNEL {
                    return;
                }
                if vel == 0 {
                    self.release_note(channel, note);
                    return;
                }
                // Reemplaza una voz existente de la misma (canal, nota).
                self.voices.retain(|v| !(v.channel == channel && v.note == note));
                self.voices.push(MusicVoice {
                    channel,
                    note,
                    freq: midi_to_freq(note),
                    phase: 0.0,
                    vel,
                    release: None,
                });
            }
            NoteOff { channel, note } => self.release_note(channel, note),
            Volume { channel, vol } => {
                if (channel as usize) < 16 {
                    self.channel_vol[channel as usize] = vol;
                }
            }
            End => {} // lo maneja el loop de avance
        }
    }

    fn release_note(&mut self, channel: u8, note: u8) {
        for v in self.voices.iter_mut() {
            if v.channel == channel && v.note == note && v.release.is_none() {
                v.release = Some(1.0);
            }
        }
    }

    /// Mezcla **aditivamente** la música en `buf` (interleaved por
    /// `channels`). El sfx mixer ya escribió/zeroeó el buffer.
    pub fn render_add(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        let ch = channels.max(1) as usize;
        let frames = buf.len() / ch;
        let dev_rate = sample_rate.max(1) as f32;
        let ticks_per_frame = MUS_TICK_HZ / dev_rate as f64;
        let release_step = 1.0 / (MUSIC_RELEASE_S * dev_rate).max(1.0);

        for f in 0..frames {
            // Avanzar el reloj de eventos y disparar los que vencieron.
            if !self.finished {
                self.tick_pos += ticks_per_frame;
                while self.cursor < self.steps.len() && self.tick_pos >= self.next_fire {
                    let step = self.steps[self.cursor];
                    self.cursor += 1;
                    if step.event == supay_wad::MusEvent::End {
                        if self.looping {
                            self.cursor = 0;
                            self.tick_pos = 0.0;
                            self.next_fire =
                                self.steps.first().map(|s| s.delay).unwrap_or(0) as f64;
                            // Soltar todo al reiniciar.
                            for v in self.voices.iter_mut() {
                                v.release.get_or_insert(1.0);
                            }
                            break;
                        } else {
                            self.finished = true;
                            break;
                        }
                    }
                    self.apply(step.event);
                    if let Some(next) = self.steps.get(self.cursor) {
                        self.next_fire += next.delay as f64;
                    }
                }
            }

            // Sintetizar la muestra mono de este frame.
            let mut sample = 0.0f32;
            for v in self.voices.iter_mut() {
                let env = v.release.unwrap_or(1.0);
                let amp = MUSIC_VOICE_GAIN
                    * (v.vel as f32 / 127.0)
                    * (self.channel_vol[v.channel as usize] as f32 / 127.0)
                    * env;
                sample += (v.phase * std::f32::consts::TAU).sin() * amp;
                v.phase += v.freq / dev_rate;
                if v.phase >= 1.0 {
                    v.phase -= 1.0;
                }
                if let Some(g) = v.release.as_mut() {
                    *g -= release_step;
                }
            }
            // Descartar voces cuyo release terminó.
            self.voices.retain(|v| v.release.map(|g| g > 0.0).unwrap_or(true));

            // Soft-limit (tanh) para que la suma de notas no clipee.
            let s = sample.tanh();
            let base = f * ch;
            // Música va al centro (mono → ambos canales).
            if ch >= 2 {
                buf[base] += s;
                buf[base + 1] += s;
            } else {
                buf[base] += s;
            }
        }
    }

    /// Notas sonando ahora (para tests).
    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }
}

/// Fuente de audio combinada: SFX (one-shots) + música (synth MUS). El
/// `fill` deja que el sfx mixer escriba el buffer y luego suma la música.
struct DoomAudio {
    sfx: DoomMixer,
    music: Option<MusicSynth>,
}

impl AudioSource for DoomAudio {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        // sfx.fill zeroea el buffer y escribe los one-shots.
        self.sfx.fill(buf, sample_rate, channels);
        if let Some(m) = self.music.as_mut() {
            m.render_add(buf, sample_rate, channels);
        }
    }
}

/// Motor de audio: abre el sink cpal, cachea los sfx decodificados del
/// WAD y reproduce eventos del motor. Mantené una instancia viva
/// mientras corra el juego — al dropearla, el stream cpal se cierra.
pub struct AudioEngine {
    audio: Arc<Mutex<DoomAudio>>,
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
        let audio = Arc::new(Mutex::new(DoomAudio {
            sfx: DoomMixer::new(),
            music: None,
        }));
        let source: Arc<Mutex<dyn AudioSource + Send>> = audio.clone();
        let sink = AudioSink::open(source)?;
        Ok(Self {
            audio,
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
            self.audio.lock().sfx.add(samples, rate, gain_l, gain_r);
        }
    }

    /// Empieza a reproducir un lump de música MUS (bytes crudos). Si no
    /// parsea como MUS (e.g. es MIDI), no hace nada y deja sonando lo
    /// anterior. `looping` repite al llegar al final.
    pub fn play_music(&mut self, mus_bytes: &[u8], looping: bool) {
        if let Some(song) = supay_wad::parse_mus(mus_bytes) {
            self.audio.lock().music = Some(MusicSynth::new(song, looping));
        }
    }

    /// Detiene la música actual (silencio inmediato).
    pub fn stop_music(&mut self) {
        self.audio.lock().music = None;
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

    use supay_wad::{MusEvent, MusSong, MusStep};

    fn song(steps: Vec<MusStep>) -> MusSong {
        MusSong { steps }
    }

    #[test]
    fn midi_to_freq_a4() {
        assert!((midi_to_freq(69) - 440.0).abs() < 1e-3);
        assert!((midi_to_freq(81) - 880.0).abs() < 1e-2); // octava arriba
    }

    #[test]
    fn synth_note_on_sounds_and_activates_voice() {
        let mut s = MusicSynth::new(
            song(vec![MusStep {
                delay: 0,
                event: MusEvent::NoteOn { channel: 0, note: 69, vel: 127 },
            }]),
            false,
        );
        let mut buf = vec![0.0f32; 200]; // 100 frames estéreo
        s.render_add(&mut buf, 100, 2);
        assert_eq!(s.active_voices(), 1);
        assert!(buf.iter().any(|&x| x.abs() > 1e-3), "la nota debe sonar");
    }

    #[test]
    fn synth_skips_percussion_channel() {
        let mut s = MusicSynth::new(
            song(vec![MusStep {
                delay: 0,
                event: MusEvent::NoteOn { channel: 15, note: 40, vel: 127 },
            }]),
            false,
        );
        let mut buf = vec![0.0f32; 64];
        s.render_add(&mut buf, 100, 2);
        assert_eq!(s.active_voices(), 0, "percusión (canal 15) se salta");
    }

    #[test]
    fn synth_note_off_releases_voice() {
        let mut s = MusicSynth::new(
            song(vec![
                MusStep { delay: 0, event: MusEvent::NoteOn { channel: 0, note: 60, vel: 127 } },
                MusStep { delay: 0, event: MusEvent::NoteOff { channel: 0, note: 60 } },
            ]),
            false,
        );
        // dev_rate 1000, release 0.04s → ~40 frames para fade-out.
        let mut buf = vec![0.0f32; 400]; // 200 frames mono
        s.render_add(&mut buf, 1000, 1);
        assert_eq!(s.active_voices(), 0, "tras NoteOff + release la voz se descarta");
    }

    #[test]
    fn synth_delayed_note_is_silent_early() {
        // Nota a 140 ticks = 1 s. Con dev_rate 1000, en 100 frames (0.1 s)
        // no debería haber disparado todavía.
        let mut s = MusicSynth::new(
            song(vec![MusStep {
                delay: 140,
                event: MusEvent::NoteOn { channel: 0, note: 69, vel: 127 },
            }]),
            false,
        );
        let mut buf = vec![0.0f32; 100]; // 100 frames mono
        s.render_add(&mut buf, 1000, 1);
        assert_eq!(s.active_voices(), 0, "la nota retardada no suena en los primeros 0.1 s");
        assert!(buf.iter().all(|&x| x == 0.0));
    }
}
