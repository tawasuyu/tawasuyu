//! Determinismo del render: el WAV de un score fijo + renderer fijo +
//! sample-rate fijo es byte-equal entre invocaciones.
//!
//! El README de takiy promete "mismo seed → mismo WAV". Hoy no hay seed:
//! todo el render es determinístico por construcción (sin `rand`,
//! `Instant::now`, hash en hot path, etc.). Este test es el contrato
//! viviente que detecta regresiones — si alguna pasa cambia el
//! oscilador o la mezcla y el hash cambia, el commit explícitamente
//! actualiza la constante con una justificación en el mensaje.
//!
//! El score sintético — no el `demo_score` del binario — está hardcoded
//! aquí para que el test no dependa de `takiy-app-llimphi` y sobreviva
//! a renombres futuros del demo. Cubre los casos relevantes:
//! - 2 pistas (mixer por-pista + audibilidad).
//! - Notas en varias octavas + velocities distintas (envelope+mixing).
//! - Volume != 1.0 y pan != 0.0 en una pista (estéreo activo).

use takiy_core::{Pitch, PitchClass, Score, ScoreNote, Track};
use takiy_synth::{write_wav_to, OscRenderer, Renderer};

const SAMPLE_RATE: u32 = 44_100;

/// Score canónico para el hash determinista. **No tocar sin actualizar
/// `EXPECTED_BLAKE3`** en el mismo commit con justificación.
fn canonical_score() -> Score {
    let mut s = Score::new(120.0);

    let mut melody = Track::new("melodía");
    for (i, pc) in [
        PitchClass::C, PitchClass::E, PitchClass::G, PitchClass::C,
    ].iter().enumerate() {
        let p = Pitch::from_class_octave(*pc, 4).unwrap();
        melody.add(ScoreNote::new(p, i as f32, 0.5, 100));
    }
    s.add_track(melody);

    let mut bass = Track::new("bajo");
    bass.volume = 0.8;
    bass.pan = -0.3;
    for (i, pc) in [PitchClass::C, PitchClass::G].iter().enumerate() {
        let p = Pitch::from_class_octave(*pc, 2).unwrap();
        bass.add(ScoreNote::new(p, (i * 2) as f32, 2.0, 110));
    }
    s.add_track(bass);

    s
}

/// BLAKE3 del WAV serializado del [`canonical_score`] renderizado con
/// `OscRenderer::default()` a 44100 Hz. Si este hash cambia, ALGO cambió
/// en el pipeline (oscilador, envelope, mixer, WAV header, ...).
///
/// Si el cambio fue intencional, actualizar este hash con el nuevo y
/// documentar la causa en el mensaje del commit.
const EXPECTED_BLAKE3: &str =
    "a0c4c7adae59e3101788e96026d55c3f9e73999d63b035f8b207dff04f703444";

#[test]
fn canonical_render_is_deterministic_byte_for_byte() {
    // Render dos veces y verifica byte-equality entre corridas — esto
    // garantiza determinismo *en esta máquina, en este momento*. La
    // constante hash garantiza también compat entre commits.
    let renderer = OscRenderer { sample_rate: SAMPLE_RATE, ..Default::default() };
    let s = canonical_score();
    let a = renderer.render(&s);
    let b = renderer.render(&s);
    assert_eq!(a.samples, b.samples, "render no es determinista en esta máquina");

    let mut wav_a = Vec::new();
    write_wav_to(&a, &mut wav_a).unwrap();
    let mut wav_b = Vec::new();
    write_wav_to(&b, &mut wav_b).unwrap();
    assert_eq!(wav_a, wav_b, "WAV serializado no es determinista");

    let hash = blake3::hash(&wav_a).to_hex().to_string();
    if EXPECTED_BLAKE3 == "REPLACE_WITH_ACTUAL_HASH_ON_FIRST_RUN" {
        // Primer arranque: imprime el hash real para que el dev lo
        // copie a la constante en un commit aparte.
        eprintln!("takiy · WAV hash actual: {hash}");
        eprintln!("       Pegar en EXPECTED_BLAKE3 (wav_determinism.rs) y volver a correr.");
        return;
    }
    assert_eq!(hash, EXPECTED_BLAKE3,
        "el hash del WAV cambió.\n\
         Si el cambio es intencional, actualizar EXPECTED_BLAKE3 con: {hash}\n\
         y explicar la causa en el commit (cambio de oscilador, mezcla, etc.).");
}

#[test]
fn buffer_is_stereo_and_audible() {
    // Sanity check — si el oscilador o el mixer dejan de emitir señal,
    // el test de hash sigue pasando porque también es determinista en
    // silencio. Aquí verificamos que sí hay audio.
    let renderer = OscRenderer { sample_rate: SAMPLE_RATE, ..Default::default() };
    let buf = renderer.render(&canonical_score());
    assert_eq!(buf.channels, 2, "render debe ser estéreo (F3.b)");
    assert!(buf.peak() > 0.1, "buffer debe ser audible (peak {})", buf.peak());
}
