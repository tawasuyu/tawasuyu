//! Acordes — una raíz y una cualidad armónica.

use serde::{Deserialize, Serialize};

use crate::pitch::{Pitch, PitchClass};

/// Cualidad de un acorde — define su patrón de intervalos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChordQuality {
    Major,
    Minor,
    Diminished,
    Augmented,
    Major7,
    Minor7,
    Dominant7,
}

impl ChordQuality {
    /// Intervalos en semitonos desde la raíz.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            ChordQuality::Major => &[0, 4, 7],
            ChordQuality::Minor => &[0, 3, 7],
            ChordQuality::Diminished => &[0, 3, 6],
            ChordQuality::Augmented => &[0, 4, 8],
            ChordQuality::Major7 => &[0, 4, 7, 11],
            ChordQuality::Minor7 => &[0, 3, 7, 10],
            ChordQuality::Dominant7 => &[0, 4, 7, 10],
        }
    }

    /// Sufijo legible — `""`, `"m"`, `"dim"`, `"maj7"`, …
    pub fn suffix(self) -> &'static str {
        match self {
            ChordQuality::Major => "",
            ChordQuality::Minor => "m",
            ChordQuality::Diminished => "dim",
            ChordQuality::Augmented => "aug",
            ChordQuality::Major7 => "maj7",
            ChordQuality::Minor7 => "m7",
            ChordQuality::Dominant7 => "7",
        }
    }
}

/// Un acorde: clase raíz + cualidad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chord {
    pub root: PitchClass,
    pub quality: ChordQuality,
}

impl Chord {
    pub fn new(root: PitchClass, quality: ChordQuality) -> Self {
        Self { root, quality }
    }

    /// Cantidad de notas del acorde.
    pub fn voice_count(self) -> usize {
        self.quality.intervals().len()
    }

    /// Las alturas concretas del acorde con la raíz en `root_octave`,
    /// en posición fundamental. Las voces que se salgan del rango MIDI
    /// se omiten.
    pub fn voicing(self, root_octave: i32) -> Vec<Pitch> {
        let Some(base) = Pitch::from_class_octave(self.root, root_octave) else {
            return Vec::new();
        };
        self.quality
            .intervals()
            .iter()
            .filter_map(|&iv| base.transpose(iv as i32))
            .collect()
    }

    /// `true` si la clase de `pitch` es una nota del acorde.
    pub fn contains(self, pitch: Pitch) -> bool {
        let rel = (pitch.class().semitone() + 12 - self.root.semitone()) % 12;
        self.quality.intervals().iter().any(|&iv| iv % 12 == rel)
    }

    /// Nombre legible — `"C"`, `"Am"`, `"G7"`, `"Dmaj7"`, …
    pub fn name(self) -> String {
        format!("{}{}", self.root.name(), self.quality.suffix())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_major_triad_is_c_e_g() {
        let voices = Chord::new(PitchClass::C, ChordQuality::Major).voicing(4);
        let classes: Vec<_> = voices.iter().map(|p| p.class()).collect();
        assert_eq!(classes, vec![PitchClass::C, PitchClass::E, PitchClass::G]);
    }

    #[test]
    fn a_minor_triad_is_a_c_e() {
        let voices = Chord::new(PitchClass::A, ChordQuality::Minor).voicing(3);
        let classes: Vec<_> = voices.iter().map(|p| p.class()).collect();
        assert_eq!(classes, vec![PitchClass::A, PitchClass::C, PitchClass::E]);
    }

    #[test]
    fn seventh_chords_have_four_voices() {
        assert_eq!(Chord::new(PitchClass::G, ChordQuality::Dominant7).voice_count(), 4);
    }

    #[test]
    fn contains_recognizes_chord_tones() {
        let g7 = Chord::new(PitchClass::G, ChordQuality::Dominant7);
        // G7 = G B D F
        assert!(g7.contains(Pitch::from_class_octave(PitchClass::F, 5).unwrap()));
        assert!(!g7.contains(Pitch::from_class_octave(PitchClass::E, 5).unwrap()));
    }

    #[test]
    fn names_render_correctly() {
        assert_eq!(Chord::new(PitchClass::C, ChordQuality::Major).name(), "C");
        assert_eq!(Chord::new(PitchClass::A, ChordQuality::Minor).name(), "Am");
        assert_eq!(Chord::new(PitchClass::D, ChordQuality::Major7).name(), "Dmaj7");
    }
}
