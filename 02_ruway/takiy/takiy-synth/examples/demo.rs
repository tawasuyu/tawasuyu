//! Demo end-to-end: construye una partitura corta, la vuelca a JSON y
//! la renderiza a WAV. Ejecutable con:
//!
//! ```text
//! cargo run -p takiy-synth --example demo
//! ```
//!
//! Produce `target/takiy-demo.takiy.json` y `target/takiy-demo.wav` —
//! ábrelos con cualquier editor / reproductor.

use std::path::PathBuf;

use takiy_core::{Pitch, PitchClass, Score, ScoreNote, Track};
use takiy_synth::{write_wav, OscRenderer, Renderer, Waveform};

fn main() -> std::io::Result<()> {
    let mut score = Score::new(120.0);

    // Melodía: do-re-mi-fa-sol-la-si-do ascendente, una negra cada una.
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

    // Bajo: do2 y sol2 alternados, blancas.
    let mut bass = Track::new("bajo");
    bass.add(ScoreNote::new(Pitch::from_class_octave(PitchClass::C, 2).unwrap(), 0.0, 2.0, 110));
    bass.add(ScoreNote::new(Pitch::from_class_octave(PitchClass::G, 2).unwrap(), 2.0, 2.0, 110));
    bass.add(ScoreNote::new(Pitch::from_class_octave(PitchClass::C, 2).unwrap(), 4.0, 2.0, 110));
    bass.add(ScoreNote::new(Pitch::from_class_octave(PitchClass::G, 2).unwrap(), 6.0, 2.0, 110));
    score.add_track(bass);

    // Dump JSON
    let json_path = out_path("takiy-demo.takiy.json");
    let json = serde_json::to_string_pretty(&score)
        .expect("Score es Serialize por construcción");
    std::fs::write(&json_path, json)?;
    println!("score   → {}", json_path.display());

    // Render WAV (saw para que sea más audible que un seno en parlantes malos)
    let renderer = OscRenderer { waveform: Waveform::Saw, ..OscRenderer::default() };
    let audio = renderer.render(&score);
    let wav_path = out_path("takiy-demo.wav");
    write_wav(&audio, &wav_path)?;
    println!(
        "audio   → {} ({} muestras, {:.2}s @ {} Hz)",
        wav_path.display(),
        audio.samples.len(),
        audio.duration_seconds(),
        audio.sample_rate,
    );

    Ok(())
}

fn out_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // …/02_ruway/takiy/takiy-synth → …/02_ruway/takiy/takiy-synth/../../../target
    p.push("../../../target");
    p.push(name);
    p
}
