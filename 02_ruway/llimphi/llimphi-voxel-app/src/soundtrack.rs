//! Banda sonora de la película: compone un `Score` con `takiy-core` y lo
//! sintetiza a un WAV con `takiy-synth`, para muxearlo al video (`--film`).
//! Cierra el último ingrediente del "director": la peli **con sonido**.
//!
//! Es contenido (la "partitura"), igual que `screenplay()` es la dirección: una
//! progresión cálida I–V–vi–IV (do–sol–lam–fa) con pads, bajo y una melodía que
//! sube hacia el momento del gesto. El sintetizador es el de osciladores (feo a
//! propósito), suavizado con un poco de reverb.

use takiy_core::{Chord, ChordQuality, Pitch, PitchClass, ReverbParams, Score, ScoreNote, Track};
use takiy_synth::{write_wav, Adsr, OscRenderer, Renderer, Waveform};

/// Compone y sintetiza la banda sonora a `path` (WAV PCM 16-bit). Devuelve la
/// duración en segundos del audio generado (para informar). El tempo se elige
/// para que las 8 negras de la progresión duren ~la película.
pub fn render_to(path: &str) -> f32 {
    let mut score = Score::new(86.0); // ~0.70 s/beat → 8 beats ≈ 5.6 s
    // Reverb suave: da aire sin tapar la mezcla.
    score.master_reverb = Some(ReverbParams { room_size: 0.6, damping: 0.5, mix: 0.22 });

    // Progresión I–V–vi–IV, dos beats por acorde.
    let prog = [
        (PitchClass::C, ChordQuality::Major),
        (PitchClass::G, ChordQuality::Major),
        (PitchClass::A, ChordQuality::Minor),
        (PitchClass::F, ChordQuality::Major),
    ];

    // Pads: el acorde completo en octava 3, sostenido 2 beats.
    let mut pads = Track::new("pads");
    pads.volume = 0.5;
    // Bajo: la fundamental en octava 2.
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

    // Melodía: una línea que se eleva hacia el beat 5 (≈ el gesto a cámara) y
    // resuelve. Notas `(start_beat, midi, duración_beats)`.
    let mut lead = Track::new("lead");
    lead.volume = 0.85;
    let melody = [
        (0.0, 72, 1.0), // C5
        (1.0, 76, 1.0), // E5
        (2.0, 74, 1.0), // D5
        (3.0, 79, 1.0), // G5
        (4.0, 76, 1.0), // E5
        (5.0, 81, 1.5), // A5 — el clímax
        (6.5, 79, 0.5), // G5
        (7.0, 77, 1.2), // F5 — resuelve
    ];
    for (start, midi, dur) in melody {
        if let Some(p) = Pitch::from_midi(midi) {
            lead.add(ScoreNote::new(p, start, dur, 78));
        }
    }

    score.add_track(pads);
    score.add_track(bass);
    score.add_track(lead);

    // Síntesis: triángulo (más cuerpo que el seno, menos áspero que la sierra)
    // con una envolvente algo más dulce que la default.
    let renderer = OscRenderer {
        sample_rate: 44_100,
        waveform: Waveform::Triangle,
        envelope: Adsr { attack: 0.01, decay: 0.12, sustain: 0.7, release: 0.35 },
    };
    let buf = renderer.render(&score);
    let secs = buf.duration_seconds();
    write_wav(&buf, path).expect("escribir WAV de la banda sonora");
    secs
}
