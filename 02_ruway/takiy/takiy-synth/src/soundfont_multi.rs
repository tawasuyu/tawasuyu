//! `MultiProgramRenderer` — un preset SF2 distinto por pista.
//!
//! Igual que [`SoundFontRenderer`](crate::soundfont::SoundFontRenderer)
//! pero asigna un canal MIDI a cada pista del `Score` y le manda su
//! propio program change. Así la pista 0 puede ser piano, la 1
//! contrabajo, la 2 strings, etcétera, todo en un solo render.
//!
//! Limitación práctica: MIDI tiene 16 canales y el canal 9 está
//! reservado para drums en GM, así que en la práctica caben 15 pistas
//! con timbres distintos. Si hay más, se envuelven en módulo y
//! comparten canal (y por tanto preset).

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use takiy_core::Score;

use crate::audio::AudioBuffer;
use crate::renderer::Renderer;
use crate::soundfont::LoadError;

/// Canal MIDI 9 está reservado para batería en General MIDI.
const DRUM_CHANNEL: usize = 9;
/// 16 canales MIDI; sin contar el de drums, quedan 15 melódicos.
const MELODIC_CHANNELS: usize = 15;

/// Canal MIDI asignado a la pista `track_idx`, saltando el canal de drums.
fn channel_for_track(track_idx: usize) -> i32 {
    let i = track_idx % MELODIC_CHANNELS;
    if i >= DRUM_CHANNEL { (i + 1) as i32 } else { i as i32 }
}

/// Programa GM para la pista `track_idx` según un mapeo y un default.
/// Función pura para que los tests no necesiten un SoundFont real.
fn program_lookup(track_programs: &[u8], default: u8, track_idx: usize) -> u8 {
    track_programs.get(track_idx).copied().unwrap_or(default)
}

/// Renderer que asigna un preset GM por pista, todas en un solo `Synthesizer`.
#[derive(Clone)]
pub struct MultiProgramRenderer {
    sound_font: Arc<SoundFont>,
    pub sample_rate: u32,
    /// Programa GM por índice de pista. Las pistas sin entrada usan
    /// `default_program`.
    track_programs: Vec<u8>,
    /// Programa GM por default `0..=127`. `0` = Acoustic Grand Piano.
    pub default_program: u8,
    /// Segundos extra al final para los releases.
    pub tail_seconds: f32,
}

impl std::fmt::Debug for MultiProgramRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiProgramRenderer")
            .field("sample_rate", &self.sample_rate)
            .field("track_programs", &self.track_programs)
            .field("default_program", &self.default_program)
            .field("tail_seconds", &self.tail_seconds)
            .finish_non_exhaustive()
    }
}

impl MultiProgramRenderer {
    /// Carga un SoundFont y devuelve un renderer con todas las pistas en
    /// piano (program 0) a 44.1 kHz.
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, LoadError> {
        let mut f = File::open(path)?;
        let sf = SoundFont::new(&mut f)?;
        Ok(Self {
            sound_font: Arc::new(sf),
            sample_rate: 44_100,
            track_programs: Vec::new(),
            default_program: 0,
            tail_seconds: 2.0,
        })
    }

    /// Asigna un preset GM a la pista `track_idx`. Encadenable.
    pub fn with_track_program(mut self, track_idx: usize, program: u8) -> Self {
        if self.track_programs.len() <= track_idx {
            self.track_programs.resize(track_idx + 1, self.default_program);
        }
        self.track_programs[track_idx] = program;
        self
    }

    /// Cambia el programa de default. Encadenable.
    pub fn with_default_program(mut self, program: u8) -> Self {
        self.default_program = program;
        self
    }

    /// Cambia la tasa de muestreo. Encadenable.
    pub fn with_sample_rate(mut self, sample_rate: u32) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    fn program_for_track(&self, track_idx: usize) -> u8 {
        program_lookup(&self.track_programs, self.default_program, track_idx)
    }
}

#[derive(Debug, Clone, Copy)]
enum Event {
    On { channel: i32, key: i32, velocity: i32 },
    Off { channel: i32, key: i32 },
}

impl Renderer for MultiProgramRenderer {
    fn render(&self, score: &Score) -> AudioBuffer {
        let settings = SynthesizerSettings::new(self.sample_rate as i32);
        let mut synth = Synthesizer::new(&self.sound_font, &settings)
            .expect("settings.sample_rate ya está validado");

        const PROGRAM_CHANGE: i32 = 0xC0;

        // Program change por canal usado.
        for (track_idx, _track) in score.tracks().iter().enumerate() {
            let ch = channel_for_track(track_idx);
            let prog = self.program_for_track(track_idx) as i32;
            synth.process_midi_message(ch, PROGRAM_CHANGE, prog, 0);
        }

        let bpm = score.tempo_bpm.max(1e-6);
        let sec_per_beat = 60.0 / bpm;
        let sr = self.sample_rate as f32;

        let mut events: Vec<(usize, Event)> = Vec::new();
        for (track_idx, track) in score.tracks().iter().enumerate() {
            let channel = channel_for_track(track_idx);
            for note in track.notes() {
                let on = (note.start * sec_per_beat * sr) as usize;
                let off = (note.end() * sec_per_beat * sr) as usize;
                let key = note.pitch.midi() as i32;
                events.push((on, Event::On { channel, key, velocity: note.velocity as i32 }));
                events.push((off.max(on + 1), Event::Off { channel, key }));
            }
        }
        events.sort_by_key(|(s, _)| *s);

        let last_event = events.last().map(|(s, _)| *s).unwrap_or(0);
        let total = last_event + (self.tail_seconds * sr).ceil() as usize;

        let mut left = vec![0.0_f32; total];
        let mut right = vec![0.0_f32; total];

        let mut cursor = 0usize;
        for (sample_idx, ev) in events {
            let target = sample_idx.min(total);
            if target > cursor {
                synth.render(&mut left[cursor..target], &mut right[cursor..target]);
                cursor = target;
            }
            match ev {
                Event::On { channel, key, velocity } => synth.note_on(channel, key, velocity),
                Event::Off { channel, key } => synth.note_off(channel, key),
            }
        }
        if cursor < total {
            synth.render(&mut left[cursor..total], &mut right[cursor..total]);
        }

        let samples: Vec<f32> = left
            .iter()
            .zip(right.iter())
            .map(|(l, r)| (l + r) * 0.5)
            .collect();
        let mut buf = AudioBuffer { sample_rate: self.sample_rate, samples };
        buf.normalize_if_clipping();
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_skip_drum_channel() {
        // 0..9 → 0..9; 9 (que sería el de drums) → 10; …
        assert_eq!(channel_for_track(0), 0);
        assert_eq!(channel_for_track(8), 8);
        assert_eq!(channel_for_track(9), 10);
        assert_eq!(channel_for_track(14), 15);
        // Pista 15 envuelve al canal 0 (mismo timbre que la 0).
        assert_eq!(channel_for_track(15), 0);
    }

    #[test]
    fn unmapped_track_falls_back_to_default_program() {
        assert_eq!(program_lookup(&[5], 99, 0), 5);
        assert_eq!(program_lookup(&[5], 99, 7), 99);
        assert_eq!(program_lookup(&[], 42, 0), 42);
    }

    /// Test de integración: sólo corre si TAKIY_SF2 apunta a un .sf2.
    #[test]
    fn multi_program_renders_with_real_soundfont_if_provided() {
        let Ok(sf_path) = std::env::var("TAKIY_SF2") else {
            eprintln!("(skip) TAKIY_SF2 no está seteado");
            return;
        };
        use takiy_core::{Pitch, PitchClass, ScoreNote, Track};

        let renderer = MultiProgramRenderer::from_path(&sf_path)
            .expect("SF2 carga")
            .with_track_program(0, 0)  // piano
            .with_track_program(1, 32); // acoustic bass

        let mut score = Score::new(60.0);
        let mut melody = Track::new("melody");
        melody.add(ScoreNote::new(Pitch::A4, 0.0, 1.0, 100));
        score.add_track(melody);
        let mut bass = Track::new("bass");
        bass.add(ScoreNote::new(
            Pitch::from_class_octave(PitchClass::A, 2).unwrap(),
            0.0,
            1.0,
            100,
        ));
        score.add_track(bass);

        let buf = renderer.render(&score);
        assert!(buf.peak() > 0.01, "el render debe producir señal");
        // Verificá los programas seteados sin tocar el SF2.
        assert_eq!(renderer.program_for_track(0), 0);
        assert_eq!(renderer.program_for_track(1), 32);
        assert_eq!(renderer.program_for_track(99), 0); // default
    }
}
