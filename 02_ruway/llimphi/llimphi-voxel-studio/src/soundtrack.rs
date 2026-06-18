//! Banda sonora del export: compone un `Score` con `takiy-core` y lo sintetiza a
//! un WAV con `takiy-synth`, para muxearlo al video de la escena. Misma "partitura"
//! que el corto de `llimphi-voxel-app`: una progresión cálida I–V–vi–IV con pads,
//! bajo y una melodía que sube hacia el clímax, **más acentos** (campanas) que caen
//! sobre los *beats del guion* (cortes de cámara + gestos) para que la música
//! acompañe la acción.

use takiy_core::{Chord, ChordQuality, Pitch, PitchClass, ReverbParams, Score, ScoreNote, Track};
use takiy_synth::{write_wav, Adsr, OscRenderer, Renderer, Waveform};

/// Tempo de la banda (BPM). 86 → ~0.70 s/beat.
const BPM: f32 = 86.0;

/// Compone y sintetiza la banda sonora a `path` (WAV PCM 16-bit). Devuelve la
/// duración en segundos. `accents` = los beats del guion (segundos): en cada uno se
/// coloca una campana brillante para que la música caiga sobre la acción.
pub fn render_to(path: &str, accents: &[f32]) -> f32 {
    let mut score = Score::new(BPM);
    score.master_reverb = Some(ReverbParams { room_size: 0.6, damping: 0.5, mix: 0.22 });

    // Progresión I–V–vi–IV, dos beats por acorde.
    let prog = [
        (PitchClass::C, ChordQuality::Major),
        (PitchClass::G, ChordQuality::Major),
        (PitchClass::A, ChordQuality::Minor),
        (PitchClass::F, ChordQuality::Major),
    ];

    let mut pads = Track::new("pads");
    pads.volume = 0.5;
    let mut bass = Track::new("bass");
    bass.volume = 0.6;
    for (i, &(root, quality)) in prog.iter().enumerate() {
        let start = i as f32 * 2.0;
        for p in Chord::new(root, quality).voicing(3) {
            pads.add(ScoreNote::new(p, start, 2.0, 52));
        }
        if let Some(b) = Pitch::from_class_octave(root, 2) {
            bass.add(ScoreNote::new(b, start, 2.0, 64));
        }
    }

    // Melodía que se eleva hacia el clímax y resuelve.
    let mut lead = Track::new("lead");
    lead.volume = 0.85;
    let melody = [
        (0.0, 72, 1.0),
        (1.0, 76, 1.0),
        (2.0, 74, 1.0),
        (3.0, 79, 1.0),
        (4.0, 76, 1.0),
        (5.0, 81, 1.5),
        (6.5, 79, 0.5),
        (7.0, 77, 1.2),
    ];
    for (start, midi, dur) in melody {
        if let Some(p) = Pitch::from_midi(midi) {
            lead.add(ScoreNote::new(p, start, dur, 78));
        }
    }

    // Acentos: una campana aguda (fundamental + octava, corta) en cada beat del guion.
    let sec_per_beat = 60.0 / BPM;
    let mut hits = Track::new("acentos");
    hits.volume = 0.9;
    for &t in accents {
        let beat = t / sec_per_beat;
        for midi in [84, 96] {
            if let Some(p) = Pitch::from_midi(midi) {
                hits.add(ScoreNote::new(p, beat, 0.5, 90));
            }
        }
    }

    score.add_track(pads);
    score.add_track(bass);
    score.add_track(lead);
    if !accents.is_empty() {
        score.add_track(hits);
    }

    let renderer = OscRenderer {
        sample_rate: 44_100,
        waveform: Waveform::Triangle,
        envelope: Adsr { attack: 0.01, decay: 0.12, sustain: 0.7, release: 0.35 },
    };
    let buf = renderer.render(&score);
    let secs = buf.duration_seconds();
    let _ = write_wav(&buf, path);
    secs
}
