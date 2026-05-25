//! Escalas — una raíz y un patrón de semitonos.

use serde::{Deserialize, Serialize};

use crate::pitch::{Pitch, PitchClass};

/// Una escala: clase raíz + offsets en semitonos desde la raíz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scale {
    root: PitchClass,
    /// Semitonos desde la raíz, ascendentes, dentro de una octava.
    intervals: Vec<u8>,
}

impl Scale {
    /// Escala arbitraria desde su patrón de intervalos.
    pub fn new(root: PitchClass, intervals: Vec<u8>) -> Self {
        Self { root, intervals }
    }

    /// Escala mayor (jónica): `T-T-S-T-T-T-S`.
    pub fn major(root: PitchClass) -> Self {
        Self::new(root, vec![0, 2, 4, 5, 7, 9, 11])
    }

    /// Escala menor natural (eólica).
    pub fn natural_minor(root: PitchClass) -> Self {
        Self::new(root, vec![0, 2, 3, 5, 7, 8, 10])
    }

    /// Escala pentatónica mayor.
    pub fn pentatonic_major(root: PitchClass) -> Self {
        Self::new(root, vec![0, 2, 4, 7, 9])
    }

    /// Clase raíz.
    pub fn root(&self) -> PitchClass {
        self.root
    }

    /// Cantidad de grados de la escala.
    pub fn degree_count(&self) -> usize {
        self.intervals.len()
    }

    /// Clase de altura del grado `degree` (0-indexado, módulo el largo).
    pub fn degree(&self, degree: usize) -> PitchClass {
        let iv = self.intervals[degree % self.intervals.len()];
        PitchClass::from_semitone(self.root.semitone() + iv)
    }

    /// `true` si la clase de `pitch` pertenece a la escala.
    pub fn contains(&self, pitch: Pitch) -> bool {
        let rel = (pitch.class().semitone() + 12 - self.root.semitone()) % 12;
        self.intervals.contains(&rel)
    }

    /// Las alturas de la escala en la `octave` dada, un grado por
    /// elemento. Las que se salgan del rango MIDI se omiten.
    pub fn pitches_in_octave(&self, octave: i32) -> Vec<Pitch> {
        self.intervals
            .iter()
            .filter_map(|&iv| {
                Pitch::from_class_octave(self.root, octave)
                    .and_then(|p| p.transpose(iv as i32))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_major_has_no_accidentals() {
        let s = Scale::major(PitchClass::C);
        for pc in [
            PitchClass::C,
            PitchClass::D,
            PitchClass::E,
            PitchClass::F,
            PitchClass::G,
            PitchClass::A,
            PitchClass::B,
        ] {
            assert!(s.contains(Pitch::from_class_octave(pc, 4).unwrap()));
        }
        // F# no está en do mayor.
        assert!(!s.contains(Pitch::from_class_octave(PitchClass::Fs, 4).unwrap()));
    }

    #[test]
    fn degrees_of_a_minor() {
        let s = Scale::natural_minor(PitchClass::A);
        assert_eq!(s.degree(0), PitchClass::A);
        assert_eq!(s.degree(2), PitchClass::C);
        // El grado envuelve.
        assert_eq!(s.degree(7), PitchClass::A);
    }

    #[test]
    fn pitches_in_octave_count_matches_degrees() {
        let s = Scale::major(PitchClass::G);
        assert_eq!(s.pitches_in_octave(4).len(), 7);
    }

    #[test]
    fn pentatonic_has_five_degrees() {
        assert_eq!(Scale::pentatonic_major(PitchClass::D).degree_count(), 5);
    }

    #[test]
    fn contains_is_octave_agnostic() {
        let s = Scale::major(PitchClass::C);
        assert!(s.contains(Pitch::from_class_octave(PitchClass::E, 2).unwrap()));
        assert!(s.contains(Pitch::from_class_octave(PitchClass::E, 7).unwrap()));
    }
}
