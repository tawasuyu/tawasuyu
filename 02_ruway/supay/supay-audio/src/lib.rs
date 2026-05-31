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

/// Canal de percusión MIDI/MUS — sus "notas" son índices de drum kit.
/// Con banco GENMIDI (Fase 4.2) lo mapeamos a instrumentos de percusión
/// OPL; sin banco lo saltamos (sonaría como nota pitcheada rara).
const MUS_PERCUSSION_CHANNEL: u8 = 15;

const MUS_TICK_HZ: f64 = 140.0;
/// Ganancia global de la música antes del soft-limit.
const MUSIC_VOICE_GAIN: f32 = 0.11;
/// Polifonía máxima del synth de música.
const MAX_MUSIC_VOICES: usize = 32;
/// Profundidad de modulación FM (en turnos de fase de la portadora por
/// unidad de salida del modulador). Tuneado a ojo — sin banco real para
/// validar de oído en el entorno de dev.
const FM_DEPTH: f32 = 0.55;
/// Escala del feedback del modulador sobre sí mismo.
const FB_SCALE: f32 = 0.5;

/// Frecuencia de una nota MIDI (A4=69 → 440 Hz).
fn midi_to_freq(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
}

/// Total level OPL (0-63, atenuación de ~0.75 dB/paso) → ganancia lineal.
fn tl_to_gain(tl: u8) -> f32 {
    10f32.powf(-(tl as f32) * 0.0375)
}
/// Sustain level OPL (0-15, ~3 dB/paso) → ganancia lineal.
fn sustain_to_gain(sl: u8) -> f32 {
    10f32.powf(-(sl as f32) * 0.15)
}
/// Rate OPL (0-15) → segundos de la fase de envolvente (aprox; mayor
/// rate = más rápido). Rate 0 = larguísimo.
fn rate_seconds(rate: u8) -> f32 {
    if rate == 0 {
        6.0
    } else {
        0.30 * 0.5f32.powi(rate as i32 - 1)
    }
}

/// Una de las cuatro formas de onda del OPL2 (`wf` 0-3; 4-7 mapean a la
/// más cercana). `phase` en turnos [0,1).
fn opl_wave(phase: f32, wf: u8) -> f32 {
    let mut p = phase.fract();
    if p < 0.0 {
        p += 1.0;
    }
    let s = (p * std::f32::consts::TAU).sin();
    match wf & 0x03 {
        0 => s,                                       // seno completo
        1 => if p < 0.5 { s } else { 0.0 },           // medio seno
        2 => s.abs(),                                 // abs-seno
        _ => if (p % 0.5) < 0.25 { s.abs() } else { 0.0 }, // cuarto de seno
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvState {
    Attack,
    Decay,
    Sustain,
    Release,
    Done,
}

/// Envolvente ADSR aproximada de un operador OPL.
#[derive(Clone, Copy)]
struct Env {
    state: EnvState,
    level: f32,
    attack_inc: f32,
    decay_dec: f32,
    sustain: f32,
    release_dec: f32,
    /// `true` = sostiene en `sustain` hasta note-off; `false` = percusivo
    /// (sigue decayendo a cero tras el decay, sin esperar note-off).
    sustaining: bool,
}

impl Env {
    fn from_op(op: &supay_wad::GenMidiOp, sr: f32) -> Self {
        let sustain = sustain_to_gain(op.sustain_level());
        Env {
            state: EnvState::Attack,
            level: 0.0,
            attack_inc: 1.0 / (rate_seconds(op.attack_rate()) * sr).max(1.0),
            decay_dec: (1.0 - sustain).max(0.0) / (rate_seconds(op.decay_rate()) * sr).max(1.0),
            sustain,
            release_dec: 1.0 / (rate_seconds(op.release_rate()) * sr).max(1.0),
            sustaining: op.sustaining(),
        }
    }

    /// Envolvente fija para el fallback seno (sin banco GENMIDI).
    fn default_sine(sr: f32) -> Self {
        Env {
            state: EnvState::Attack,
            level: 0.0,
            attack_inc: 1.0 / (0.005 * sr).max(1.0),
            decay_dec: 0.0,
            sustain: 0.8,
            release_dec: 1.0 / (0.04 * sr).max(1.0),
            sustaining: true,
        }
    }

    fn step(&mut self) -> f32 {
        match self.state {
            EnvState::Attack => {
                self.level += self.attack_inc;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.state = EnvState::Decay;
                }
            }
            EnvState::Decay => {
                self.level -= self.decay_dec;
                if self.level <= self.sustain {
                    self.level = self.sustain;
                    // Percusivo: sigue cayendo a 0; sostenido: mantiene.
                    self.state = if self.sustaining {
                        EnvState::Sustain
                    } else {
                        EnvState::Release
                    };
                }
            }
            EnvState::Sustain => {
                if self.sustain <= 1e-4 {
                    self.state = EnvState::Done;
                }
            }
            EnvState::Release => {
                self.level -= self.release_dec;
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.state = EnvState::Done;
                }
            }
            EnvState::Done => {}
        }
        self.level
    }

    fn note_off(&mut self) {
        if self.state != EnvState::Done {
            self.state = EnvState::Release;
        }
    }

    fn done(&self) -> bool {
        self.state == EnvState::Done
    }
}

/// Una voz FM de 2 operadores (modulador → portadora), programada desde
/// un instrumento GENMIDI o desde el fallback seno.
struct FmVoice {
    channel: u8,
    note: u8,
    base_freq: f32,
    mod_mult: f32,
    car_mult: f32,
    mod_phase: f32,
    car_phase: f32,
    mod_amp: f32,
    car_amp: f32,
    mod_wave: u8,
    car_wave: u8,
    fb_gain: f32,
    last_mod: f32,
    additive: bool,
    mod_env: Env,
    car_env: Env,
    vel: u8,
}

impl FmVoice {
    fn from_instr(
        channel: u8,
        note: u8,
        vel: u8,
        instr: &supay_wad::GenMidiInstr,
        sr: f32,
    ) -> Self {
        let v = &instr.voices[0];
        let pitch_note = if instr.fixed_pitch() {
            instr.fixed_note
        } else {
            note
        };
        let n = (pitch_note as i16 + v.base_note_offset).clamp(0, 127) as u8;
        FmVoice {
            channel,
            note,
            base_freq: midi_to_freq(n),
            mod_mult: v.modulator.mult(),
            car_mult: v.carrier.mult(),
            mod_phase: 0.0,
            car_phase: 0.0,
            mod_amp: tl_to_gain(v.modulator.total_level()),
            car_amp: tl_to_gain(v.carrier.total_level()),
            mod_wave: v.modulator.waveform_select(),
            car_wave: v.carrier.waveform_select(),
            fb_gain: v.feedback_amount() as f32 / 8.0 * FB_SCALE,
            last_mod: 0.0,
            additive: v.additive(),
            mod_env: Env::from_op(&v.modulator, sr),
            car_env: Env::from_op(&v.carrier, sr),
            vel,
        }
    }

    /// Voz seno (sólo portadora) cuando no hay banco GENMIDI.
    fn sine(channel: u8, note: u8, vel: u8, sr: f32) -> Self {
        FmVoice {
            channel,
            note,
            base_freq: midi_to_freq(note),
            mod_mult: 1.0,
            car_mult: 1.0,
            mod_phase: 0.0,
            car_phase: 0.0,
            mod_amp: 0.0, // sin modulación
            car_amp: 1.0,
            mod_wave: 0,
            car_wave: 0,
            fb_gain: 0.0,
            last_mod: 0.0,
            additive: false,
            mod_env: Env::default_sine(sr),
            car_env: Env::default_sine(sr),
            vel,
        }
    }

    fn note_off(&mut self) {
        self.mod_env.note_off();
        self.car_env.note_off();
    }

    fn done(&self) -> bool {
        self.car_env.done()
    }

    /// Una muestra mono (pre channel-volume). `sr` = rate del dispositivo.
    fn step(&mut self, sr: f32) -> f32 {
        let me = self.mod_env.step();
        let ce = self.car_env.step();
        let fb = self.fb_gain * self.last_mod;
        let mod_out = opl_wave(self.mod_phase + fb, self.mod_wave) * self.mod_amp * me;
        let out = if self.additive {
            (opl_wave(self.mod_phase, self.mod_wave) * self.mod_amp * me
                + opl_wave(self.car_phase, self.car_wave) * self.car_amp * ce)
                * 0.5
        } else {
            opl_wave(self.car_phase + FM_DEPTH * mod_out, self.car_wave) * self.car_amp * ce
        };
        self.last_mod = mod_out;
        self.mod_phase = (self.mod_phase + self.base_freq * self.mod_mult / sr).fract();
        self.car_phase = (self.car_phase + self.base_freq * self.car_mult / sr).fract();
        out * (self.vel as f32 / 127.0)
    }
}

/// Synth de música: reproduce un timeline MUS a 140 Hz con voces FM de
/// 2 operadores programadas desde el banco GENMIDI (Fase 4.2). Sin banco
/// cae a osciladores seno (comportamiento 4.1). Mezcla **aditiva** sobre
/// el buffer (lo llena el sfx mixer antes).
pub struct MusicSynth {
    steps: Vec<supay_wad::MusStep>,
    looping: bool,
    cursor: usize,
    tick_pos: f64,
    next_fire: f64,
    voices: Vec<FmVoice>,
    /// Volumen por canal MUS (controller #3), 0..127.
    channel_vol: [u8; 16],
    /// Instrumento (patch GM) por canal — controller #0 lo cambia.
    channel_patch: [u8; 16],
    /// Banco OPL; `None` ⇒ fallback seno.
    bank: Option<Arc<supay_wad::GenMidi>>,
    /// Rate del dispositivo, fijado al inicio de cada `render_add` para
    /// que los eventos disparados sepan construir las envolventes.
    cur_sr: f32,
    finished: bool,
}

impl MusicSynth {
    pub fn new(
        song: supay_wad::MusSong,
        looping: bool,
        bank: Option<Arc<supay_wad::GenMidi>>,
    ) -> Self {
        let first_delay = song.steps.first().map(|s| s.delay).unwrap_or(0) as f64;
        Self {
            steps: song.steps,
            looping,
            cursor: 0,
            tick_pos: 0.0,
            next_fire: first_delay,
            voices: Vec::new(),
            channel_vol: [127; 16],
            channel_patch: [0; 16],
            bank,
            cur_sr: 44_100.0,
            finished: false,
        }
    }

    fn apply(&mut self, ev: supay_wad::MusEvent) {
        use supay_wad::MusEvent::*;
        match ev {
            NoteOn { channel, note, vel } => {
                if vel == 0 {
                    self.release_note(channel, note);
                    return;
                }
                // Reemplaza una voz existente de la misma (canal, nota).
                self.voices
                    .retain(|v| !(v.channel == channel && v.note == note));
                if self.voices.len() >= MAX_MUSIC_VOICES {
                    return;
                }
                let voice = match &self.bank {
                    Some(bank) => {
                        let instr = if channel == MUS_PERCUSSION_CHANNEL {
                            bank.percussion(note)
                        } else {
                            bank.melodic(self.channel_patch[channel as usize])
                        };
                        match instr {
                            Some(i) => FmVoice::from_instr(channel, note, vel, i, self.cur_sr),
                            // Sin instrumento resuelto: percusión se salta,
                            // melódico cae a seno.
                            None if channel == MUS_PERCUSSION_CHANNEL => return,
                            None => FmVoice::sine(channel, note, vel, self.cur_sr),
                        }
                    }
                    // Sin banco: percusión se salta, melódico es seno.
                    None if channel == MUS_PERCUSSION_CHANNEL => return,
                    None => FmVoice::sine(channel, note, vel, self.cur_sr),
                };
                self.voices.push(voice);
            }
            NoteOff { channel, note } => self.release_note(channel, note),
            Volume { channel, vol } => {
                if (channel as usize) < 16 {
                    self.channel_vol[channel as usize] = vol;
                }
            }
            Program { channel, patch } => {
                if (channel as usize) < 16 {
                    self.channel_patch[channel as usize] = patch;
                }
            }
            End => {} // lo maneja el loop de avance
        }
    }

    fn release_note(&mut self, channel: u8, note: u8) {
        for v in self.voices.iter_mut() {
            if v.channel == channel && v.note == note {
                v.note_off();
            }
        }
    }

    /// Mezcla **aditivamente** la música en `buf` (interleaved por
    /// `channels`). El sfx mixer ya escribió/zeroeó el buffer.
    pub fn render_add(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        let ch = channels.max(1) as usize;
        let frames = buf.len() / ch;
        let dev_rate = sample_rate.max(1) as f32;
        self.cur_sr = dev_rate;
        let ticks_per_frame = MUS_TICK_HZ / dev_rate as f64;

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
                            for v in self.voices.iter_mut() {
                                v.note_off();
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
                let chvol = self.channel_vol[v.channel as usize] as f32 / 127.0;
                sample += v.step(dev_rate) * chvol;
            }
            self.voices.retain(|v| !v.done());

            // Soft-limit (tanh) para que la suma de notas no clipee.
            let s = (sample * MUSIC_VOICE_GAIN).tanh();
            let base = f * ch;
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
    /// Banco de instrumentos OPL (lump `GENMIDI`) compartido con cada
    /// `MusicSynth`. `None` si el WAD no lo trae → música en seno.
    bank: Option<Arc<supay_wad::GenMidi>>,
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
        // Banco OPL para la música FM (Fase 4.2). Sin GENMIDI → seno.
        let bank = wad.genmidi().map(Arc::new);
        Ok(Self {
            audio,
            _sink: sink,
            wad,
            cache: HashMap::new(),
            bank,
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
            let synth = MusicSynth::new(song, looping, self.bank.clone());
            self.audio.lock().music = Some(synth);
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

    use supay_wad::{
        GenMidi, GenMidiInstr, MusEvent, MusSong, MusStep,
    };

    fn song(steps: Vec<MusStep>) -> MusSong {
        MusSong { steps }
    }

    /// Banco GENMIDI sintético: 175 instrumentos default, con la portadora
    /// del patch `melodic` y de la percusión 130 con attack rápido para
    /// que suenen de inmediato en los tests.
    fn test_bank() -> Arc<GenMidi> {
        let mut instruments = vec![GenMidiInstr::default(); 175];
        for idx in [0usize, 130] {
            // carrier.att_dec = 0xF0 → attack 15 (instantáneo), decay 0.
            instruments[idx].voices[0].carrier.att_dec = 0xF0;
            instruments[idx].voices[0].carrier.am_vib = 0x20; // sustaining
        }
        // 130 = percusión nota 37 (128 + 37-35); fija pitch.
        instruments[130].flags = 0x01;
        instruments[130].fixed_note = 50;
        Arc::new(GenMidi { instruments })
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
            None, // fallback seno
        );
        let mut buf = vec![0.0f32; 200]; // 100 frames estéreo
        s.render_add(&mut buf, 100, 2);
        assert_eq!(s.active_voices(), 1);
        assert!(buf.iter().any(|&x| x.abs() > 1e-3), "la nota debe sonar");
    }

    #[test]
    fn synth_skips_percussion_without_bank() {
        let mut s = MusicSynth::new(
            song(vec![MusStep {
                delay: 0,
                event: MusEvent::NoteOn { channel: 15, note: 40, vel: 127 },
            }]),
            false,
            None,
        );
        let mut buf = vec![0.0f32; 64];
        s.render_add(&mut buf, 100, 2);
        assert_eq!(s.active_voices(), 0, "sin banco, percusión (canal 15) se salta");
    }

    #[test]
    fn synth_note_off_releases_voice() {
        let mut s = MusicSynth::new(
            song(vec![
                MusStep { delay: 0, event: MusEvent::NoteOn { channel: 0, note: 60, vel: 127 } },
                MusStep { delay: 0, event: MusEvent::NoteOff { channel: 0, note: 60 } },
            ]),
            false,
            None,
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
            None,
        );
        let mut buf = vec![0.0f32; 100]; // 100 frames mono
        s.render_add(&mut buf, 1000, 1);
        assert_eq!(s.active_voices(), 0, "la nota retardada no suena en los primeros 0.1 s");
        assert!(buf.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn synth_fm_instrument_sounds_with_bank() {
        // Con banco GENMIDI, un NoteOn melódico programa una voz FM y suena.
        let mut s = MusicSynth::new(
            song(vec![MusStep {
                delay: 0,
                event: MusEvent::NoteOn { channel: 0, note: 60, vel: 127 },
            }]),
            false,
            Some(test_bank()),
        );
        let mut buf = vec![0.0f32; 400]; // 200 frames mono
        s.render_add(&mut buf, 8000, 1);
        assert_eq!(s.active_voices(), 1);
        assert!(buf.iter().any(|&x| x.abs() > 1e-4), "la voz FM debe sonar");
    }

    #[test]
    fn synth_percussion_plays_with_bank() {
        // Con banco, el canal 15 SÍ suena (percusión OPL), a diferencia
        // del fallback sin banco. Nota 37 → instrumento 130.
        let mut s = MusicSynth::new(
            song(vec![MusStep {
                delay: 0,
                event: MusEvent::NoteOn { channel: 15, note: 37, vel: 127 },
            }]),
            false,
            Some(test_bank()),
        );
        let mut buf = vec![0.0f32; 200];
        s.render_add(&mut buf, 8000, 1);
        assert_eq!(s.active_voices(), 1, "con banco, la percusión suena");
    }

    #[test]
    fn synth_program_change_sets_channel_patch() {
        // Program change + NoteOn no deben romper y deben activar la voz.
        let mut s = MusicSynth::new(
            song(vec![
                MusStep { delay: 0, event: MusEvent::Program { channel: 0, patch: 0 } },
                MusStep { delay: 0, event: MusEvent::NoteOn { channel: 0, note: 64, vel: 100 } },
            ]),
            false,
            Some(test_bank()),
        );
        let mut buf = vec![0.0f32; 200];
        s.render_add(&mut buf, 8000, 1);
        assert_eq!(s.active_voices(), 1);
    }
}
