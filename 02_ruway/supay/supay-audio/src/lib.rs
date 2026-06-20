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

/// Paneo **constante-power** (equal-power): mapea `sep` de Doom (0 =
/// izquierda total, 128 ≈ centro, 255 = derecha total) a ganancias L/R
/// que conservan la energía percibida al barrer el campo estéreo. Un
/// paneo lineal (`1-p` / `p`) cae −3 dB en el centro; con `cos`/`sin`
/// la suma de potencias es constante (`cos²+sin² = 1`).
fn equal_power_pan(sep: u8) -> (f32, f32) {
    let pan = (sep as f32 / 255.0).clamp(0.0, 1.0);
    let angle = pan * std::f32::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

/// Fase 4.5 — coeficiente de un pasa-bajos 1-polo (`y += coef·(x−y)`) para
/// la oclusión `occ ∈ [0,1]`. `occ==0` ⇒ `1.0` (bypass exacto: `y = x`).
/// Al subir, el corte baja de ~`0.45·sr` (transparente) a `700 Hz` (sonido
/// tapado tras la pared). `coef = 1 − exp(−2π·fc/sr)`, independiente del
/// sample rate.
fn occlusion_lp_coef(occ: f32, sr: f32) -> f32 {
    let occ = occ.clamp(0.0, 1.0);
    if occ <= 1e-3 || sr <= 1.0 {
        return 1.0;
    }
    let fc_open = sr * 0.45;
    let fc = 700.0 + (1.0 - occ) * (fc_open - 700.0);
    let coef = 1.0 - (-2.0 * std::f32::consts::PI * fc / sr).exp();
    coef.clamp(0.0, 1.0)
}

/// Distancia (unidades Doom) a la que la absorción de aire satura. Más
/// allá no oscurece más — el `vol` del motor ya bajó por la atenuación
/// vanilla, esto sólo modela el filtro de agudos.
const AIR_FULL_DIST: f32 = 1400.0;

/// Fase 4.7 — coeficiente del pasa-bajos 1-polo de **absorción de aire**
/// para la `distance` fuente→oyente (unidades Doom). `distance≈0` ⇒ `1.0`
/// (bypass exacto). Al alejarse, el corte baja suave de ~`0.45·sr`
/// (transparente) a `2200 Hz` en `AIR_FULL_DIST` — bastante más alto que
/// el de la oclusión (`700 Hz`): el aire se come los brillos, no tapa el
/// sonido como una pared. `coef = 1 − exp(−2π·fc/sr)`, rate-independiente.
fn air_lp_coef(distance: f32, sr: f32) -> f32 {
    if distance <= 1.0 || sr <= 1.0 {
        return 1.0;
    }
    let t = (distance / AIR_FULL_DIST).clamp(0.0, 1.0);
    let fc_open = sr * 0.45;
    let fc = fc_open + t * (2200.0 - fc_open);
    let coef = 1.0 - (-2.0 * std::f32::consts::PI * fc / sr).exp();
    coef.clamp(0.0, 1.0)
}

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
    /// Fase 4.5 — oclusión geométrica `0..1` (cuántas paredes sólidas hay
    /// entre la fuente y el oyente). `0` = sin filtrar; al subir baja el
    /// corte del pasa-bajos de la voz (sonido apagado, "tras la pared").
    occlusion: f32,
    /// Estado del pasa-bajos 1-polo de oclusión (memoria de la muestra
    /// anterior). `0` cuando `occlusion == 0` (filtro inerte).
    lp: f32,
    /// Fase 4.7 — distancia fuente→oyente en unidades Doom. Modela la
    /// **absorción de aire**: a más distancia, más se comen los agudos
    /// (pasa-bajos suave, análogo acústico de la perspectiva atmosférica
    /// del lado visual). `0` = sin filtrar (fuente al ras del oyente o
    /// sonido no posicionado). Comparte el estado `lp` con la oclusión:
    /// el filtro corre al corte más restrictivo de los dos.
    air_distance: f32,
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
    fn add(
        &mut self,
        samples: Arc<[f32]>,
        src_rate: f32,
        gain_l: f32,
        gain_r: f32,
        occlusion: f32,
        air_distance: f32,
    ) {
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
            occlusion: occlusion.clamp(0.0, 1.0),
            lp: 0.0,
            air_distance: air_distance.max(0.0),
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

        let dev_rate_f = dev_rate as f32;
        for v in self.voices.iter_mut() {
            let step = v.src_rate as f64 / dev_rate;
            let n = v.samples.len();
            // Fase 4.5 — coef del pasa-bajos 1-polo de oclusión, constante
            // dentro del callback (sr fijo). `occlusion==0` ⇒ coef 1.0 =
            // bypass bit-exacto (compat 4.4). Al subir, baja el corte:
            // 0→~0.45·sr (sin filtro), 1→700 Hz (apagado, tras la pared).
            // Fase 4.7 — el filtro de la voz corre al corte más restrictivo
            // entre oclusión (pared) y absorción de aire (distancia): el
            // menor coef = el menor corte = el que más oscurece. Un solo
            // pasa-bajos cubre ambos efectos. Ambos en `0`/cerca ⇒ coef 1.0
            // = bypass bit-exacto (compat 4.6).
            let lp_coef =
                occlusion_lp_coef(v.occlusion, dev_rate_f).min(air_lp_coef(v.air_distance, dev_rate_f));
            // La oclusión también atenúa ~−4.5 dB a tope (la pared absorbe).
            // El aire NO atenúa volumen extra — el `vol` del motor ya cayó
            // por la distancia (atenuación vanilla); esto sólo filtra agudos.
            let occ_gain = 1.0 - 0.4 * v.occlusion;
            for f in 0..frames {
                let i0 = v.cursor.floor() as usize;
                if i0 >= n {
                    break; // voz agotada en mitad del buffer → resto silencio
                }
                let frac = (v.cursor - i0 as f64) as f32;
                let s0 = v.samples[i0];
                let s1 = if i0 + 1 < n { v.samples[i0 + 1] } else { s0 };
                let mut s = s0 + (s1 - s0) * frac;
                if lp_coef < 1.0 {
                    v.lp += lp_coef * (s - v.lp);
                    s = v.lp * occ_gain;
                }
                let s = s * master;
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

/// Acústica de sala que el host fija por sector (Fase 4.3). Modela el
/// reverb que rodea al jugador: un cuarto chico suena seco, una caverna
/// larga arrastra cola. `supay-scene` deriva la geometría; el host la
/// traduce a estos parámetros y los pasa con [`AudioEngine::set_ambience`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomAmbience {
    /// Mezcla húmeda 0..1. `0` ⇒ reverb apagado (path seco bit-equivalente
    /// a 4.2, sin costo de CPU).
    pub wet: f32,
    /// Tamaño de la sala 0..1 → realimentación de los combs (cola más
    /// larga al subir).
    pub room_size: f32,
    /// Amortiguación de agudos en la cola 0..1 (1 = muy apagado, piedra;
    /// 0 = brillante, baldosa).
    pub damping: f32,
}

impl Default for RoomAmbience {
    /// Default seco — sin mapa cargado el host no espacializa.
    fn default() -> Self {
        RoomAmbience { wet: 0.0, room_size: 0.5, damping: 0.5 }
    }
}

// Afinaciones Freeverb (Schroeder–Moorer), medidas para 44100 Hz; se
// reescalan al rate real del dispositivo. 8 combs en paralelo + 4
// allpass en serie por canal, con offset estéreo en el canal derecho.
const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const FV_RATE: f32 = 44_100.0;
const ROOM_SCALE: f32 = 0.28;
const ROOM_OFFSET: f32 = 0.7;
const DAMP_SCALE: f32 = 0.4;
const FV_GAIN: f32 = 0.015;
/// Constante de tiempo (segundos) del crossfade de ambiente entre
/// sectores. ~100 ms: rápido para seguir al jugador, lento para no
/// clickear al cruzar una puerta.
const AMB_TAU: f32 = 0.10;

/// Comb con amortiguación pasa-bajos en el lazo de realimentación.
struct Comb {
    buf: Vec<f32>,
    idx: usize,
    filt: f32,
}
impl Comb {
    fn new(len: usize) -> Self {
        Comb { buf: vec![0.0; len.max(1)], idx: 0, filt: 0.0 }
    }
    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        self.filt = out * (1.0 - damp) + self.filt * damp;
        self.buf[self.idx] = input + self.filt * feedback;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

/// Allpass de Schroeder (difusor).
struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}
impl Allpass {
    fn new(len: usize) -> Self {
        Allpass { buf: vec![0.0; len.max(1)], idx: 0 }
    }
    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buf[self.idx];
        let out = -input + buffered;
        self.buf[self.idx] = input + buffered * 0.5; // feedback fijo 0.5
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

/// Reverb estéreo estilo Freeverb. Toma una entrada mono (la suma del
/// mix) y produce una cola húmeda L/R. Los buffers se dimensionan al
/// rate del dispositivo la primera vez (o si cambia).
struct Reverb {
    combs_l: Vec<Comb>,
    combs_r: Vec<Comb>,
    aps_l: Vec<Allpass>,
    aps_r: Vec<Allpass>,
    sr: f32,
    /// Acústica **actual** (suavizada). Persigue a `target` con una
    /// constante de tiempo de [`AMB_TAU`] para que un cambio de cuarto no
    /// reasiente el reverb de golpe (Fase 4.4).
    amb: RoomAmbience,
    /// Acústica destino que fija el host por sector.
    target: RoomAmbience,
}

impl Reverb {
    fn new() -> Self {
        Reverb {
            combs_l: Vec::new(),
            combs_r: Vec::new(),
            aps_l: Vec::new(),
            aps_r: Vec::new(),
            sr: 0.0,
            amb: RoomAmbience::default(),
            target: RoomAmbience::default(),
        }
    }

    /// Mueve `amb` un paso hacia `target` (lerp exponencial 1-polo).
    /// `coef ∈ (0,1]` = fracción del gap cerrada por frame.
    fn smooth(&mut self, coef: f32) {
        let l = |a: f32, b: f32| a + (b - a) * coef;
        self.amb.wet = l(self.amb.wet, self.target.wet);
        self.amb.room_size = l(self.amb.room_size, self.target.room_size);
        self.amb.damping = l(self.amb.damping, self.target.damping);
    }

    /// (Re)construye las líneas de delay para `sr`. Escala las afinaciones
    /// 44100 al rate real para mantener el carácter de la sala.
    fn rebuild(&mut self, sr: f32) {
        let scale = (sr / FV_RATE).max(0.1);
        let len = |n: usize, off: usize| (((n + off) as f32) * scale) as usize;
        self.combs_l = COMB_TUNING.iter().map(|&n| Comb::new(len(n, 0))).collect();
        self.combs_r = COMB_TUNING.iter().map(|&n| Comb::new(len(n, STEREO_SPREAD))).collect();
        self.aps_l = ALLPASS_TUNING.iter().map(|&n| Allpass::new(len(n, 0))).collect();
        self.aps_r = ALLPASS_TUNING.iter().map(|&n| Allpass::new(len(n, STEREO_SPREAD))).collect();
        self.sr = sr;
    }

    /// Procesa una muestra mono → (wet_l, wet_r). `feedback`/`damp` ya
    /// mapeados desde [`RoomAmbience`].
    fn process(&mut self, input: f32) -> (f32, f32) {
        let feedback = self.amb.room_size * ROOM_SCALE + ROOM_OFFSET;
        let damp = self.amb.damping * DAMP_SCALE;
        let inp = input * FV_GAIN;
        let mut l = 0.0;
        let mut r = 0.0;
        for c in self.combs_l.iter_mut() {
            l += c.process(inp, feedback, damp);
        }
        for c in self.combs_r.iter_mut() {
            r += c.process(inp, feedback, damp);
        }
        for a in self.aps_l.iter_mut() {
            l = a.process(l);
        }
        for a in self.aps_r.iter_mut() {
            r = a.process(r);
        }
        (l, r)
    }
}

/// Fuente de audio combinada: SFX (one-shots) + música (synth MUS) +
/// reverb por sector (Fase 4.3). El `fill` deja que el sfx mixer escriba
/// el buffer, suma la música, y al final inyecta la cola húmeda del
/// reverb sobre el mix seco.
struct DoomAudio {
    sfx: DoomMixer,
    music: Option<MusicSynth>,
    reverb: Reverb,
}

impl AudioSource for DoomAudio {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        // sfx.fill zeroea el buffer y escribe los one-shots.
        self.sfx.fill(buf, sample_rate, channels);
        if let Some(m) = self.music.as_mut() {
            m.render_add(buf, sample_rate, channels);
        }

        // Reverb por sector con crossfade de ambiente (Fase 4.4). Corre
        // mientras la mezcla actual *o* el destino sean audibles — así la
        // cola se desvanece suave al entrar a un cuarto seco. Seco de los
        // dos lados ⇒ bypass sin costo (compat 4.2).
        if self.reverb.amb.wet > 1e-4 || self.reverb.target.wet > 1e-4 {
            let sr = sample_rate.max(1) as f32;
            if (self.reverb.sr - sr).abs() > 0.5 {
                self.reverb.rebuild(sr);
            }
            // coef 1-polo: fracción del gap cerrada por frame para AMB_TAU.
            let coef = 1.0 - (-1.0 / (AMB_TAU * sr)).exp();
            let ch = channels.max(1) as usize;
            let frames = buf.len() / ch;
            for f in 0..frames {
                self.reverb.smooth(coef);
                let wet = self.reverb.amb.wet;
                let base = f * ch;
                let dry = if ch >= 2 {
                    (buf[base] + buf[base + 1]) * 0.5
                } else {
                    buf[base]
                };
                let (wl, wr) = self.reverb.process(dry);
                if ch >= 2 {
                    buf[base] += wl * wet;
                    buf[base + 1] += wr * wet;
                } else {
                    buf[base] += (wl + wr) * 0.5 * wet;
                }
            }
        }
    }
}

/// Colapsa un `AudioBuffer` de takiy (posiblemente estéreo/multicanal,
/// interleaved) a un stream mono promediando los canales de cada cuadro. El
/// [`DoomMixer`] trabaja con voces mono (panea él mismo a L/R), así que el
/// puente con takiy reduce primero a mono.
fn mono_samples(buf: &takiy_synth::AudioBuffer) -> Vec<f32> {
    let ch = buf.channels.max(1) as usize;
    if ch == 1 {
        return buf.samples.clone();
    }
    buf.samples
        .chunks(ch)
        .map(|frame| frame.iter().copied().sum::<f32>() / ch as f32)
        .collect()
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
            reverb: Reverb::new(),
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
    /// `DSPISTOL`). `vol` 0..127, `sep` 0..255 (128 ≈ centro). `occlusion`
    /// `0..1` (Fase 4.5): cuántas paredes sólidas hay entre la fuente y el
    /// oyente — `0` suena seco/directo, `1` apagado tras la pared.
    /// `distance` (Fase 4.7): distancia fuente→oyente en unidades Doom para
    /// la absorción de aire (los agudos se apagan a la distancia); `0` para
    /// sonidos no posicionados (UI, emitidos por el jugador). Si el lump no
    /// existe, no hace nada.
    pub fn play(&mut self, name: &str, vol: u8, sep: u8, occlusion: f32, distance: f32) {
        let lump = format!("DS{}", name.to_uppercase());
        let resolved = self.resolve(&lump);
        if let Some((samples, rate)) = resolved {
            let g = vol as f32 / 127.0;
            // Paneo constante-power (Fase 4.3): sin caída de loudness al
            // centro, a diferencia del balance lineal de 4.0.
            let (pl, pr) = equal_power_pan(sep);
            self.audio
                .lock()
                .sfx
                .add(samples, rate, g * pl, g * pr, occlusion, distance);
        }
    }

    /// Reproduce una partitura de **takiy** como una voz más del mixer de
    /// SFX (puente supay↔takiy). Renderiza el `Score` con el `OscRenderer`
    /// de takiy al sample-rate del dispositivo, lo colapsa a mono y lo encola
    /// con volumen `vol` (0..127) y paneo `sep` (0..255, 128 = centro) —
    /// mismo contrato que [`play`](Self::play). Permite que el motor dispare
    /// jingles o sonidos sintéticos sin que vivan en el WAD.
    pub fn play_takiy_score(&mut self, score: &takiy_core::Score, vol: u8, sep: u8) {
        use takiy_synth::renderer::{OscRenderer, Renderer};
        let dev_rate = self._sink.sample_rate().max(1);
        let buf = OscRenderer {
            sample_rate: dev_rate,
            ..Default::default()
        }
        .render(score);
        let mono = mono_samples(&buf);
        if mono.is_empty() {
            return;
        }
        let g = vol as f32 / 127.0;
        let (pl, pr) = equal_power_pan(sep);
        // Jingle no posicionado → sin oclusión ni absorción de aire.
        self.audio
            .lock()
            .sfx
            .add(mono.into(), dev_rate as f32, g * pl, g * pr, 0.0, 0.0);
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

    /// Fija la acústica **destino** de la sala (Fase 4.3). El host la
    /// recalcula por tick desde el sector del jugador; el reverb la
    /// persigue con un crossfade de ~100 ms (Fase 4.4) en vez de saltar.
    /// `wet=0` lleva el mix a seco (desvaneciendo la cola en vuelo).
    pub fn set_ambience(&mut self, amb: RoomAmbience) {
        self.audio.lock().reverb.target = amb;
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
        m.add(voice_samples(), 100.0, 0.0, 1.0, 0.0, 0.0);
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
        m.add(voice_samples(), 50.0, 1.0, 1.0, 0.0, 0.0);
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
        m.add(voice_samples(), 100.0, 1.0, 1.0, 0.0, 0.0);
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

    #[test]
    fn occlusion_lp_coef_passthrough_at_zero() {
        // Sin oclusión: coef 1.0 ⇒ el pasa-bajos es inerte (y = x).
        assert_eq!(occlusion_lp_coef(0.0, 44100.0), 1.0);
        // Con oclusión el coef baja de 1 (filtra).
        assert!(occlusion_lp_coef(1.0, 44100.0) < 1.0);
    }

    #[test]
    fn occlusion_muffles_high_frequencies() {
        // Señal a Nyquist (alterna ±1): el pasa-bajos de oclusión la mata.
        let nyquist: Arc<[f32]> =
            Arc::from((0..64).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect::<Vec<_>>());

        // Sample rate realista: el corte de oclusión (700 Hz a tope) sólo
        // tiene sentido por debajo de Nyquist. A 44.1 kHz, la onda Nyquist
        // queda muy por encima del corte → fuertemente atenuada.
        let energy = |occ: f32| {
            let mut m = DoomMixer::new();
            m.master = 1.0;
            m.add(nyquist.clone(), 44100.0, 1.0, 1.0, occ, 0.0);
            let mut buf = vec![0.0f32; 64];
            m.fill(&mut buf, 44100, 1);
            buf.iter().map(|x| x.abs()).sum::<f32>()
        };

        let clear = energy(0.0);
        let occluded = energy(1.0);
        // La voz seca pasa la onda completa; la ocluida queda muy atenuada.
        assert!(clear > 50.0, "señal directa conserva energía: {clear}");
        assert!(
            occluded < clear * 0.25,
            "oclusión total apaga los agudos: clear={clear} occluded={occluded}"
        );
    }

    #[test]
    fn air_lp_coef_passthrough_at_zero() {
        // Fase 4.7 — distancia ~0: coef 1.0 ⇒ el pasa-bajos de aire es
        // inerte (bypass exacto). Al alejarse el coef baja (filtra agudos).
        assert_eq!(air_lp_coef(0.0, 44100.0), 1.0);
        assert_eq!(air_lp_coef(1.0, 44100.0), 1.0);
        assert!(air_lp_coef(AIR_FULL_DIST, 44100.0) < 1.0);
        // Monótono: más distancia ⇒ corte más bajo ⇒ coef menor.
        assert!(air_lp_coef(AIR_FULL_DIST, 44100.0) < air_lp_coef(AIR_FULL_DIST * 0.5, 44100.0));
    }

    #[test]
    fn air_absorption_muffles_distant_high_frequencies() {
        // Fase 4.7 — la misma onda a Nyquist suena llena de cerca y
        // apagada de lejos, sin oclusión de por medio (distancia pura).
        let nyquist: Arc<[f32]> =
            Arc::from((0..64).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect::<Vec<_>>());
        let energy = |dist: f32| {
            let mut m = DoomMixer::new();
            m.master = 1.0;
            m.add(nyquist.clone(), 44100.0, 1.0, 1.0, 0.0, dist);
            let mut buf = vec![0.0f32; 64];
            m.fill(&mut buf, 44100, 1);
            buf.iter().map(|x| x.abs()).sum::<f32>()
        };
        let near = energy(0.0);
        let far = energy(AIR_FULL_DIST);
        assert!(near > 50.0, "fuente cercana conserva energía: {near}");
        assert!(
            far < near * 0.6,
            "la distancia se come los agudos: near={near} far={far}"
        );
    }

    #[test]
    fn air_absorption_does_not_change_volume() {
        // El aire filtra agudos pero NO atenúa volumen extra (el `vol` del
        // motor ya cayó por la distancia). Una señal DC (sin agudos) pasa
        // intacta aun a distancia máxima.
        let dc: Arc<[f32]> = Arc::from(vec![1.0f32; 64]);
        let sum = |dist: f32| {
            let mut m = DoomMixer::new();
            m.master = 1.0;
            m.add(dc.clone(), 44100.0, 1.0, 1.0, 0.0, dist);
            let mut buf = vec![0.0f32; 64];
            m.fill(&mut buf, 44100, 1);
            // Promedio del estado estacionario (saltea el ramp del 1-polo).
            buf[32..].iter().copied().sum::<f32>() / 32.0
        };
        let near = sum(0.0);
        let far = sum(AIR_FULL_DIST);
        assert!((near - 1.0).abs() < 1e-3, "DC cercano pasa intacto: {near}");
        assert!(
            (far - near).abs() < 1e-2,
            "DC lejano no pierde volumen (sólo agudos): near={near} far={far}"
        );
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
    fn equal_power_pan_center_is_minus_3db() {
        // Centro (sep=128): ambos canales ≈ cos(π/4)=0.707, no 0.5.
        let (l, r) = equal_power_pan(128);
        assert!((l - 0.707).abs() < 0.01 && (r - 0.707).abs() < 0.01, "l={l} r={r}");
        // Conserva potencia: l²+r² ≈ 1 en todo el barrido.
        for sep in [0u8, 64, 128, 192, 255] {
            let (l, r) = equal_power_pan(sep);
            assert!((l * l + r * r - 1.0).abs() < 1e-3, "sep={sep}: {}", l * l + r * r);
        }
        // Extremos: 0 = todo izquierda, 255 = todo derecha.
        assert!(equal_power_pan(0).0 > 0.99 && equal_power_pan(0).1 < 0.01);
        assert!(equal_power_pan(255).1 > 0.99 && equal_power_pan(255).0 < 0.01);
    }

    #[test]
    fn reverb_dry_when_wet_zero() {
        // wet=0 ⇒ el reverb no toca el buffer (compat 4.2 bit-equivalente).
        let mut da = DoomAudio {
            sfx: DoomMixer::new(),
            music: None,
            reverb: Reverb::new(),
        };
        da.sfx.add(Arc::from(vec![1.0f32, 1.0, 1.0, 1.0]), 100.0, 1.0, 1.0, 0.0, 0.0);
        let mut buf = vec![0.0f32; 8];
        da.fill(&mut buf, 100, 2);
        // Sin cola: las muestras son exactamente el dry del mixer (master 0.6).
        assert!((buf[0] - 0.6).abs() < 1e-6, "buf0={}", buf[0]);
    }

    #[test]
    fn reverb_adds_tail_when_wet() {
        // Con wet>0 y un impulso, el reverb sigue sonando después de que
        // la voz seca se agotó — eso es la cola.
        let mut da = DoomAudio {
            sfx: DoomMixer::new(),
            music: None,
            reverb: Reverb::new(),
        };
        let amb = RoomAmbience { wet: 0.8, room_size: 0.9, damping: 0.2 };
        da.reverb.amb = amb;
        da.reverb.target = amb; // ya asentado: sin crossfade en este test.
        // Impulso corto: 2 muestras, luego silencio seco.
        da.sfx.add(Arc::from(vec![1.0f32, 1.0]), 1000.0, 1.0, 1.0, 0.0, 0.0);
        let mut buf = vec![0.0f32; 2000]; // 1000 frames estéreo a 1000 Hz
        da.fill(&mut buf, 1000, 2);
        // Pasados los primeros frames (donde la voz seca ya terminó) debe
        // haber energía de la cola.
        let tail: f32 = buf[400..].iter().map(|x| x.abs()).sum();
        assert!(tail > 1e-3, "el reverb debe dejar cola audible, tail={tail}");
    }

    #[test]
    fn reverb_crossfades_toward_target() {
        // Arranca seco (default). Fijamos un target húmedo: la mezcla
        // actual debe ramplear hacia él, no saltar.
        let mut da = DoomAudio {
            sfx: DoomMixer::new(),
            music: None,
            reverb: Reverb::new(),
        };
        da.reverb.target = RoomAmbience { wet: 0.5, room_size: 0.8, damping: 0.3 };
        // Un buffer corto (mucho menor que AMB_TAU): no debe llegar al target.
        let mut buf = vec![0.0f32; 20]; // 10 frames a 1000 Hz = 0.01 s ≪ 0.1 s
        da.fill(&mut buf, 1000, 2);
        assert!(da.reverb.amb.wet > 0.0, "el wet arrancó a subir");
        assert!(da.reverb.amb.wet < 0.5, "no debe saltar al target en 0.01 s: {}", da.reverb.amb.wet);
        // Tras muchos frames converge al target.
        let mut buf2 = vec![0.0f32; 4000]; // 2000 frames = 2 s ≫ 0.1 s
        da.fill(&mut buf2, 1000, 2);
        assert!((da.reverb.amb.wet - 0.5).abs() < 1e-3, "converge al target: {}", da.reverb.amb.wet);
        assert!((da.reverb.amb.room_size - 0.8).abs() < 1e-3);
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

    // === Puente supay↔takiy (device-free) ===

    #[test]
    fn mono_samples_promedia_canales_estereo() {
        let buf = takiy_synth::AudioBuffer {
            sample_rate: 44_100,
            channels: 2,
            // 2 cuadros estéreo: (1.0,0.0) → 0.5 ; (-1.0,1.0) → 0.0
            samples: vec![1.0, 0.0, -1.0, 1.0],
        };
        assert_eq!(mono_samples(&buf), vec![0.5, 0.0]);
    }

    #[test]
    fn takiy_score_se_encola_como_voz_mono() {
        use takiy_core::{Pitch, Score, ScoreNote, Track};
        use takiy_synth::renderer::{OscRenderer, Renderer};

        // Una nota La4 de 1 beat a 60 bpm → ~1 s de señal audible.
        let mut score = Score::new(60.0);
        let mut track = Track::new("a");
        track.add(ScoreNote::new(Pitch::A4, 0.0, 1.0, 127));
        score.add_track(track);

        let buf = OscRenderer {
            sample_rate: 44_100,
            ..Default::default()
        }
        .render(&score);
        let mono = mono_samples(&buf);

        assert!(!mono.is_empty(), "el render debe producir muestras");
        assert!(
            mono.iter().copied().map(f32::abs).fold(0.0, f32::max) > 0.1,
            "la voz mono debe ser audible (no silencio)"
        );

        // Esa voz mono se encola en el mixer como cualquier SFX.
        let mut m = DoomMixer::new();
        m.add(mono.into(), 44_100.0, 0.7, 0.7, 0.0, 0.0);
        assert_eq!(m.active_voices(), 1);
    }
}
