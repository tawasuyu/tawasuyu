//! Alturas — clases de altura y notas MIDI.
//!
//! La altura interna es un número MIDI (`0..=127`); MIDI 69 es A4 a
//! 440 Hz, MIDI 60 es el do central. Todo lo demás —clase, octava,
//! frecuencia— se deriva de ahí.

use serde::{Deserialize, Serialize};

/// Las doce clases de altura del temperamento igual.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PitchClass {
    C,
    Cs,
    D,
    Ds,
    E,
    F,
    Fs,
    G,
    Gs,
    A,
    As,
    B,
}

impl PitchClass {
    /// Semitono dentro de la octava (`C = 0 … B = 11`).
    pub fn semitone(self) -> u8 {
        self as u8
    }

    /// Clase de altura desde un semitono — toma `semitone % 12`.
    pub fn from_semitone(semitone: u8) -> PitchClass {
        use PitchClass::*;
        const ALL: [PitchClass; 12] =
            [C, Cs, D, Ds, E, F, Fs, G, Gs, A, As, B];
        ALL[(semitone % 12) as usize]
    }

    /// Nombre con sostenidos — `"C"`, `"C#"`, `"D"`, …
    pub fn name(self) -> &'static str {
        ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]
            [self.semitone() as usize]
    }
}

/// Una altura concreta: número de nota MIDI, `0..=127`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Pitch(u8);

impl Pitch {
    /// Do central (MIDI 60).
    pub const MIDDLE_C: Pitch = Pitch(60);
    /// La de afinación, A4 (MIDI 69, 440 Hz).
    pub const A4: Pitch = Pitch(69);

    /// Construye desde un número MIDI; `None` si excede `127`.
    pub fn from_midi(midi: u8) -> Option<Pitch> {
        (midi <= 127).then_some(Pitch(midi))
    }

    /// Construye desde clase + octava (convención científica: la octava
    /// del do central es 4). `None` si cae fuera del rango MIDI.
    pub fn from_class_octave(class: PitchClass, octave: i32) -> Option<Pitch> {
        let midi = (octave + 1) * 12 + class.semitone() as i32;
        (0..=127).contains(&midi).then_some(Pitch(midi as u8))
    }

    /// Número de nota MIDI.
    pub fn midi(self) -> u8 {
        self.0
    }

    /// Clase de altura.
    pub fn class(self) -> PitchClass {
        PitchClass::from_semitone(self.0 % 12)
    }

    /// Octava en convención científica (do central → 4).
    pub fn octave(self) -> i32 {
        self.0 as i32 / 12 - 1
    }

    /// Transpone por `semitones`; `None` si sale del rango MIDI.
    pub fn transpose(self, semitones: i32) -> Option<Pitch> {
        let midi = self.0 as i32 + semitones;
        (0..=127).contains(&midi).then_some(Pitch(midi as u8))
    }

    /// Frecuencia en Hz bajo temperamento igual con A4 = 440 Hz.
    pub fn frequency(self) -> f32 {
        440.0 * 2.0f32.powf((self.0 as f32 - 69.0) / 12.0)
    }

    /// Nombre legible — `"C4"`, `"F#5"`, …
    pub fn name(self) -> String {
        format!("{}{}", self.class().name(), self.octave())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_c_is_c4() {
        assert_eq!(Pitch::MIDDLE_C.class(), PitchClass::C);
        assert_eq!(Pitch::MIDDLE_C.octave(), 4);
        assert_eq!(Pitch::MIDDLE_C.name(), "C4");
    }

    #[test]
    fn a4_is_440_hz() {
        assert!((Pitch::A4.frequency() - 440.0).abs() < 1e-3);
    }

    #[test]
    fn octave_up_doubles_frequency() {
        let a5 = Pitch::A4.transpose(12).unwrap();
        assert!((a5.frequency() - 880.0).abs() < 1e-2);
    }

    #[test]
    fn class_octave_roundtrips() {
        let p = Pitch::from_class_octave(PitchClass::Fs, 5).unwrap();
        assert_eq!(p.class(), PitchClass::Fs);
        assert_eq!(p.octave(), 5);
        assert_eq!(p.name(), "F#5");
    }

    #[test]
    fn transpose_past_range_fails() {
        assert!(Pitch::from_midi(125).unwrap().transpose(10).is_none());
        assert!(Pitch::from_midi(2).unwrap().transpose(-10).is_none());
    }

    #[test]
    fn from_midi_rejects_out_of_range() {
        assert!(Pitch::from_midi(128).is_none());
        assert!(Pitch::from_midi(127).is_some());
    }

    #[test]
    fn semitone_wraps() {
        assert_eq!(PitchClass::from_semitone(13), PitchClass::Cs);
    }
}
