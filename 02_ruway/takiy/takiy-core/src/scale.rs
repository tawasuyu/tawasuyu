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

    /// Pitch dentro de la escala más cercano a `pitch`. Si `pitch` ya
    /// está en la escala devuelve el mismo. Empata hacia arriba — al
    /// estar exactamente entre dos grados (distancia idéntica), gana
    /// el pitch más agudo. Si no hubiese ningún pitch en escala dentro
    /// del rango MIDI (caso imposible para escalas no vacías), devuelve
    /// el `pitch` original como fallback inocuo.
    pub fn nearest_in_scale(&self, pitch: Pitch) -> Pitch {
        if self.contains(pitch) {
            return pitch;
        }
        let midi = pitch.midi() as i32;
        for d in 1..=11_i32 {
            // Arriba primero — la convención de "empate hacia arriba"
            // se cae natural del orden de prueba.
            if let Some(p) = u8::try_from(midi + d).ok().and_then(Pitch::from_midi) {
                if self.contains(p) {
                    return p;
                }
            }
            if let Some(p) = u8::try_from(midi - d).ok().and_then(Pitch::from_midi) {
                if self.contains(p) {
                    return p;
                }
            }
        }
        pitch
    }

    /// Salta `steps` grados dentro de la escala (positivo arriba,
    /// negativo abajo). Si `pitch` no está en escala, primero lo lleva
    /// al [`nearest_in_scale`](Self::nearest_in_scale) y desde ahí cuenta
    /// — así un Alt+↑ desde una nota fuera de escala primero la corrige
    /// y después sube. `None` si la cadena se sale del rango MIDI.
    pub fn step_in_scale(&self, pitch: Pitch, steps: i32) -> Option<Pitch> {
        let mut current = self.nearest_in_scale(pitch);
        if steps == 0 {
            return Some(current);
        }
        let dir = if steps > 0 { 1_i32 } else { -1 };
        let total = steps.unsigned_abs();
        for _ in 0..total {
            let mut next = None;
            for d in 1..=11_i32 {
                let m = current.midi() as i32 + dir * d;
                if let Some(p) = u8::try_from(m).ok().and_then(Pitch::from_midi) {
                    if self.contains(p) {
                        next = Some(p);
                        break;
                    }
                }
            }
            current = next?;
        }
        Some(current)
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

    #[test]
    fn nearest_in_scale_keeps_in_scale_pitches() {
        let s = Scale::major(PitchClass::C);
        let c4 = Pitch::from_midi(60).unwrap();
        assert_eq!(s.nearest_in_scale(c4), c4);
        // E4 — también está en C mayor.
        let e4 = Pitch::from_midi(64).unwrap();
        assert_eq!(s.nearest_in_scale(e4), e4);
    }

    #[test]
    fn nearest_in_scale_corrects_chromatic() {
        let s = Scale::major(PitchClass::C);
        // C#4 (61) está a 1 de C4 (60) y a 1 de D4 (62). Empate → arriba.
        let cs4 = Pitch::from_midi(61).unwrap();
        assert_eq!(s.nearest_in_scale(cs4).midi(), 62);
        // F#4 (66): F=65 (a 1), G=67 (a 1). Empate → arriba (G4).
        let fs4 = Pitch::from_midi(66).unwrap();
        assert_eq!(s.nearest_in_scale(fs4).midi(), 67);
    }

    #[test]
    fn nearest_in_scale_pentatonic_jumps_more() {
        // En pentatónica C, no hay F ni B. C#4 (61) cae a D4 (62, +1).
        // F4 (65) cae a E4 (64, -1) — F a E = 1, F a G = 2.
        let s = Scale::pentatonic_major(PitchClass::C);
        let cs4 = Pitch::from_midi(61).unwrap();
        assert_eq!(s.nearest_in_scale(cs4).midi(), 62);
        let f4 = Pitch::from_midi(65).unwrap();
        assert_eq!(s.nearest_in_scale(f4).midi(), 64);
    }

    #[test]
    fn step_in_scale_climbs_by_degrees() {
        let s = Scale::major(PitchClass::C);
        let c4 = Pitch::from_midi(60).unwrap();
        // +1 grado: D4 (62), +2: E4 (64), +3: F4 (65).
        assert_eq!(s.step_in_scale(c4, 1).unwrap().midi(), 62);
        assert_eq!(s.step_in_scale(c4, 2).unwrap().midi(), 64);
        assert_eq!(s.step_in_scale(c4, 7).unwrap().midi(), 72);
    }

    #[test]
    fn step_in_scale_descends_by_degrees() {
        let s = Scale::major(PitchClass::C);
        let c4 = Pitch::from_midi(60).unwrap();
        assert_eq!(s.step_in_scale(c4, -1).unwrap().midi(), 59); // B3
        assert_eq!(s.step_in_scale(c4, -7).unwrap().midi(), 48); // C3
    }

    #[test]
    fn step_in_scale_from_off_scale_first_quantizes() {
        let s = Scale::major(PitchClass::C);
        // C#4 (61) — nearest = D4 (62). +1 = E4 (64).
        let cs4 = Pitch::from_midi(61).unwrap();
        assert_eq!(s.step_in_scale(cs4, 1).unwrap().midi(), 64);
    }

    #[test]
    fn step_in_scale_returns_none_at_midi_edges() {
        let s = Scale::major(PitchClass::C);
        let top = Pitch::from_midi(125).unwrap(); // F8 (en C major)
        // C major terminal cerca de 127: 125(F)→127(G)→? siguiente sería A(129) → fuera.
        assert!(s.step_in_scale(top, 2).is_none());
        let bottom = Pitch::from_midi(0).unwrap();
        assert!(s.step_in_scale(bottom, -1).is_none());
    }
}
