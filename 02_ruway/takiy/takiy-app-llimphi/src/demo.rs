//! Score built-in y bootstrap de carga desde `TAKIY_SCORE_JSON`.

use takiy_core::{Pitch, PitchClass, Score, ScoreNote, Track};

/// Score demo built-in: una escala C mayor + dos notas de bajo. Pensado
/// para que `cargo run -p takiy-app-llimphi` muestre algo audible sin
/// configuración. Las pistas se llaman "melodía" y "bajo" para que el
/// mapeo heurístico de GM les asigne piano y bass respectivamente.
pub fn demo_score() -> Score {
    let mut score = Score::new(120.0);

    let mut melody = Track::new("melodía");
    let degrees = [
        PitchClass::C, PitchClass::D, PitchClass::E, PitchClass::F,
        PitchClass::G, PitchClass::A, PitchClass::B, PitchClass::C,
    ];
    for (i, pc) in degrees.iter().enumerate() {
        let octave = if i == 7 { 5 } else { 4 };
        let pitch = Pitch::from_class_octave(*pc, octave).unwrap();
        melody.add(ScoreNote::new(pitch, i as f32, 0.9, 100));
    }
    score.add_track(melody);

    let mut bass = Track::new("bajo");
    for (i, pc) in [PitchClass::C, PitchClass::G, PitchClass::C, PitchClass::G].iter().enumerate() {
        let pitch = Pitch::from_class_octave(*pc, 2).unwrap();
        bass.add(ScoreNote::new(pitch, (i * 2) as f32, 2.0, 110));
    }
    score.add_track(bass);

    score
}

/// Si `TAKIY_SCORE_JSON` apunta a un archivo válido, lo carga; si no,
/// devuelve el [`demo_score`] built-in. Devuelve también una etiqueta
/// para el header de la app (`"JSON path"` o `"demo built-in"`). Logea
/// errores al stderr en lugar de propagarlos — la UX es "siempre arranca".
pub fn load_score_or_demo() -> (Score, String) {
    if let Ok(path) = std::env::var("TAKIY_SCORE_JSON") {
        match std::fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<Score>(&s) {
                Ok(score) => return (score, format!("JSON {path}")),
                Err(e) => eprintln!("takiy · error parseando {path}: {e}"),
            },
            Err(e) => eprintln!("takiy · error leyendo {path}: {e}"),
        }
    }
    (demo_score(), "demo built-in".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_score_has_two_tracks_named_melody_and_bass() {
        let s = demo_score();
        assert_eq!(s.tracks().len(), 2);
        assert_eq!(s.track(0).unwrap().name, "melodía");
        assert_eq!(s.track(1).unwrap().name, "bajo");
    }

    #[test]
    fn demo_score_has_audible_content() {
        let s = demo_score();
        let total_notes: usize = s.tracks().iter().map(|t| t.notes().len()).sum();
        assert_eq!(total_notes, 12); // 8 + 4
        assert!(s.duration_beats() > 0.0);
    }
}
