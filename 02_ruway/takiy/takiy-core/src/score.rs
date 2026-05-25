//! El modelo de partitura — notas, pistas y un `Score` con tempo.
//!
//! El tiempo se mide en *pulsos* (beats), no en segundos: una partitura
//! es independiente del tempo hasta que se la reproduce. La conversión a
//! segundos vive en [`Score::duration_seconds`].

use serde::{Deserialize, Serialize};

use crate::pitch::Pitch;

/// Una nota dentro de una pista: altura, inicio y duración en pulsos,
/// y velocidad (intensidad MIDI).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScoreNote {
    pub pitch: Pitch,
    /// Pulso de inicio.
    pub start: f32,
    /// Duración en pulsos.
    pub duration: f32,
    /// Intensidad `0..=127`.
    pub velocity: u8,
}

impl ScoreNote {
    /// Crea una nota; la velocidad se acota a `127`.
    pub fn new(pitch: Pitch, start: f32, duration: f32, velocity: u8) -> Self {
        Self { pitch, start, duration, velocity: velocity.min(127) }
    }

    /// Pulso en que la nota termina.
    pub fn end(self) -> f32 {
        self.start + self.duration
    }

    /// `true` si la nota está sonando en el pulso `beat`.
    pub fn sounds_at(self, beat: f32) -> bool {
        beat >= self.start && beat < self.end()
    }
}

/// Una pista monofónica o polifónica: notas ordenadas por inicio.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub name: String,
    notes: Vec<ScoreNote>,
}

impl Track {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), notes: Vec::new() }
    }

    /// Inserta una nota manteniendo el orden por pulso de inicio.
    pub fn add(&mut self, note: ScoreNote) {
        let pos = self
            .notes
            .partition_point(|n| n.start <= note.start);
        self.notes.insert(pos, note);
    }

    /// Notas de la pista, ordenadas por inicio.
    pub fn notes(&self) -> &[ScoreNote] {
        &self.notes
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Pulso en que termina la última nota (0 si la pista está vacía).
    pub fn duration(&self) -> f32 {
        self.notes.iter().map(|n| n.end()).fold(0.0, f32::max)
    }

    /// Notas que suenan en el pulso `beat`.
    pub fn notes_at(&self, beat: f32) -> Vec<&ScoreNote> {
        self.notes.iter().filter(|n| n.sounds_at(beat)).collect()
    }

    /// Transpone la pista entera. Es atómico: si alguna nota se saldría
    /// del rango MIDI, no se cambia nada y devuelve `false`.
    pub fn transpose(&mut self, semitones: i32) -> bool {
        if self.notes.iter().any(|n| n.pitch.transpose(semitones).is_none()) {
            return false;
        }
        for n in &mut self.notes {
            n.pitch = n.pitch.transpose(semitones).expect("ya verificado");
        }
        true
    }
}

/// Una partitura: un tempo y varias pistas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Score {
    /// Pulsos por minuto.
    pub tempo_bpm: f32,
    tracks: Vec<Track>,
}

impl Score {
    /// Partitura vacía con el tempo dado.
    pub fn new(tempo_bpm: f32) -> Self {
        Self { tempo_bpm, tracks: Vec::new() }
    }

    /// Añade una pista y devuelve su índice.
    pub fn add_track(&mut self, track: Track) -> usize {
        self.tracks.push(track);
        self.tracks.len() - 1
    }

    pub fn track(&self, index: usize) -> Option<&Track> {
        self.tracks.get(index)
    }

    pub fn track_mut(&mut self, index: usize) -> Option<&mut Track> {
        self.tracks.get_mut(index)
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Duración en pulsos — la pista más larga.
    pub fn duration_beats(&self) -> f32 {
        self.tracks.iter().map(|t| t.duration()).fold(0.0, f32::max)
    }

    /// Duración en segundos según el tempo.
    pub fn duration_seconds(&self) -> f32 {
        if self.tempo_bpm <= 0.0 {
            return 0.0;
        }
        self.duration_beats() * 60.0 / self.tempo_bpm
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::{Pitch, PitchClass};

    fn note(class: PitchClass, start: f32) -> ScoreNote {
        ScoreNote::new(Pitch::from_class_octave(class, 4).unwrap(), start, 1.0, 100)
    }

    #[test]
    fn add_keeps_notes_sorted_by_start() {
        let mut t = Track::new("melodía");
        t.add(note(PitchClass::E, 2.0));
        t.add(note(PitchClass::C, 0.0));
        t.add(note(PitchClass::D, 1.0));
        let starts: Vec<f32> = t.notes().iter().map(|n| n.start).collect();
        assert_eq!(starts, vec![0.0, 1.0, 2.0]);
    }

    #[test]
    fn duration_is_end_of_last_note() {
        let mut t = Track::new("x");
        t.add(note(PitchClass::C, 0.0));
        t.add(note(PitchClass::G, 3.0)); // termina en 4.0
        assert_eq!(t.duration(), 4.0);
    }

    #[test]
    fn notes_at_finds_sounding_notes() {
        let mut t = Track::new("x");
        t.add(ScoreNote::new(Pitch::MIDDLE_C, 0.0, 2.0, 80));
        t.add(ScoreNote::new(Pitch::A4, 1.0, 2.0, 80));
        // En el pulso 1.5 ambas suenan; en 2.5 sólo la segunda.
        assert_eq!(t.notes_at(1.5).len(), 2);
        assert_eq!(t.notes_at(2.5).len(), 1);
        assert_eq!(t.notes_at(5.0).len(), 0);
    }

    #[test]
    fn transpose_is_atomic_on_overflow() {
        let mut t = Track::new("x");
        t.add(ScoreNote::new(Pitch::from_midi(120).unwrap(), 0.0, 1.0, 80));
        // +10 sacaría la nota del rango → no cambia nada.
        assert!(!t.transpose(10));
        assert_eq!(t.notes()[0].pitch.midi(), 120);
        // +5 sí cabe.
        assert!(t.transpose(5));
        assert_eq!(t.notes()[0].pitch.midi(), 125);
    }

    #[test]
    fn velocity_is_clamped() {
        let n = ScoreNote::new(Pitch::MIDDLE_C, 0.0, 1.0, 200);
        assert_eq!(n.velocity, 127);
    }

    #[test]
    fn score_duration_in_seconds_follows_tempo() {
        let mut s = Score::new(120.0); // 120 bpm → 2 pulsos por segundo
        let mut t = Track::new("x");
        t.add(ScoreNote::new(Pitch::MIDDLE_C, 0.0, 8.0, 100));
        s.add_track(t);
        assert_eq!(s.duration_beats(), 8.0);
        // 8 pulsos a 120 bpm = 4 segundos.
        assert!((s.duration_seconds() - 4.0).abs() < 1e-4);
    }

    #[test]
    fn score_duration_is_the_longest_track() {
        let mut s = Score::new(100.0);
        let mut a = Track::new("a");
        a.add(ScoreNote::new(Pitch::MIDDLE_C, 0.0, 2.0, 90));
        let mut b = Track::new("b");
        b.add(ScoreNote::new(Pitch::A4, 0.0, 6.0, 90));
        s.add_track(a);
        s.add_track(b);
        assert_eq!(s.duration_beats(), 6.0);
    }
}
