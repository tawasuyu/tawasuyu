//! Síntesis e integración con el `Player`: carga del SF2, render offline
//! (osc o sampler) y construcción del `(AudioBuffer, PlayOpts)` con
//! metrónomo / count-in / loop region.

use takiy_app::{gm_program_for_track_name, EditorState};
use takiy_core::Score;
use takiy_playback::PlayOpts;
use takiy_synth::{
    apply_master_delay, apply_master_reverb, mix_clicks, prepend_count_in, AudioBuffer, Metronome,
    MultiProgramRenderer, OscRenderer, Renderer,
};

/// Sample-rate canónico para el export WAV offline. Coincide con el del
/// test de determinismo (F10), así que un render hecho desde la UI puede
/// hashearse byte-equal contra el WAV de referencia si el score es el
/// canónico. El device de audio puede correr a otro SR (48 kHz, 96 kHz),
/// pero el WAV exportado *siempre* se renderiza a 44100 para que dos
/// usuarios en máquinas distintas obtengan archivos iguales.
pub(crate) const WAV_EXPORT_SAMPLE_RATE: u32 = 44_100;

/// Si `TAKIY_SF2` apunta a un .sf2 válido, devuelve un
/// `MultiProgramRenderer` con un mapeo nombre→programa GM aplicado a
/// las pistas del score. Si no, devuelve `None` y la app cae a osc.
pub(crate) fn load_sf2(score: &Score, sample_rate: u32) -> (Option<MultiProgramRenderer>, String) {
    let Ok(path) = std::env::var("TAKIY_SF2") else {
        return (None, "engine osc".into());
    };
    let mut renderer = match MultiProgramRenderer::from_path(&path) {
        Ok(r) => r.with_sample_rate(sample_rate),
        Err(e) => {
            eprintln!("takiy · SF2 {path} no cargó ({e}) — cayendo a osc");
            return (None, format!("engine osc (SF2 error: {e})"));
        }
    };
    for (idx, track) in score.tracks().iter().enumerate() {
        let program = gm_program_for_track_name(&track.name);
        renderer = renderer.with_track_program(idx, program);
    }
    let label = std::path::Path::new(&path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&path)
        .to_owned();
    (Some(renderer), format!("engine sf2 {label}"))
}

/// Elige el renderer (SF2 si está disponible, osc en su defecto) y
/// renderiza el score al `sample_rate` del device.
pub(crate) fn render_score(
    score: &Score,
    sf2: Option<&MultiProgramRenderer>,
    sample_rate: u32,
) -> AudioBuffer {
    if let Some(sf2) = sf2 {
        if sf2.sample_rate == sample_rate {
            return sf2.render(score);
        }
        return sf2.clone().with_sample_rate(sample_rate).render(score);
    }
    let osc = OscRenderer { sample_rate, ..Default::default() };
    osc.render(score)
}

/// Render que honra las **ediciones de onda** por pista. Si ninguna
/// pista tiene ops, delega en [`render_score`] (camino rápido y byte-exact
/// — el test de determinismo no se entera). Si alguna las tiene, rinde
/// cada pista audible por separado, le aplica su envolvente de ganancia
/// muestra-a-muestra, suma, y aplica los efectos de master sobre la suma.
pub(crate) fn render_with_wave(
    score: &Score,
    sf2: Option<&MultiProgramRenderer>,
    sample_rate: u32,
) -> AudioBuffer {
    if !score.tracks().iter().any(|t| t.has_wave_edits()) {
        return render_score(score, sf2, sample_rate);
    }

    let sec_per_beat = 60.0 / score.tempo_bpm.max(1.0);
    let mut sum: Option<AudioBuffer> = None;
    for (idx, track) in score.tracks().iter().enumerate() {
        if !score.track_is_audible(idx) {
            continue;
        }
        // Pista aislada, sin efectos de master (se aplican una vez sobre
        // la suma). El timbre se preserva: con SF2 remapeamos el programa
        // de la pista al índice 0 de la sub-partitura.
        let mut one = Score::new(score.tempo_bpm);
        let mut t = track.clone();
        t.mute = false;
        t.solo = false;
        one.add_track(t);
        let mut buf = match sf2 {
            Some(sf2) => sf2
                .clone()
                .with_sample_rate(sample_rate)
                .with_track_program(0, sf2.program_for_track(idx))
                .render(&one),
            None => OscRenderer { sample_rate, ..Default::default() }.render(&one),
        };

        // Envolvente de ganancia de onda, evaluada por frame.
        if track.has_wave_edits() {
            let ch = buf.channels.max(1) as usize;
            let inv = 1.0 / (sample_rate as f32 * sec_per_beat);
            let frames = buf.frames();
            for f in 0..frames {
                let g = track.wave_gain_at(f as f32 * inv);
                if (g - 1.0).abs() > 1e-9 {
                    for c in 0..ch {
                        buf.samples[f * ch + c] *= g;
                    }
                }
            }
        }

        match sum.as_mut() {
            None => sum = Some(buf),
            Some(acc) => {
                if buf.samples.len() > acc.samples.len() {
                    acc.samples.resize(buf.samples.len(), 0.0);
                }
                for (a, b) in acc.samples.iter_mut().zip(buf.samples.iter()) {
                    *a += *b;
                }
            }
        }
    }

    let mut out = sum.unwrap_or_else(|| render_score(score, sf2, sample_rate));
    if let Some(delay) = score.master_delay.as_ref() {
        apply_master_delay(&mut out, sec_per_beat, delay);
    }
    if let Some(reverb) = score.master_reverb.as_ref() {
        apply_master_reverb(&mut out, reverb);
    }
    out.normalize_if_clipping();
    out
}

/// Construye el `(AudioBuffer, PlayOpts)` para una orden de reproducción
/// considerando metrónomo, loop region, count-in y la posición de
/// arranque pedida en beats. Si `start_beat` cae dentro de una región
/// de loop activa, arranca dentro de la región para que el primer ciclo
/// suene completo desde ahí.
pub(crate) fn build_play(
    editor: &EditorState,
    sf2: Option<&MultiProgramRenderer>,
    sample_rate: u32,
    start_beat: f32,
    count_in: bool,
) -> (AudioBuffer, PlayOpts) {
    let mut buf = render_with_wave(&editor.score, sf2, sample_rate);
    let bpm = editor.score.tempo_bpm.max(1.0);
    let sec_per_beat = 60.0 / bpm;

    // Mezcla del metrónomo: arranca en beat 0 absoluto del score y
    // sigue hasta que se acabe el buffer. La beats_per_bar viene del
    // estado del editor.
    if let Some(beats_per_bar) = editor.metronome_beats_per_bar {
        let metro = Metronome { beats_per_bar, ..Metronome::DEFAULT };
        mix_clicks(
            &mut buf.samples,
            sample_rate,
            buf.channels,
            sec_per_beat,
            &metro,
            0,
            None,
        );
    }

    // Count-in: prepende un compás (beats_per_bar o 4 si metrónomo off)
    // con clicks. La cuenta arranca en beat 0 del count-in y abarca esos
    // beats; el score arranca justo después.
    let pre_samples = if count_in {
        let bpb = editor.metronome_beats_per_bar.unwrap_or(4);
        let metro = Metronome { beats_per_bar: bpb, ..Metronome::DEFAULT };
        let pre_samples = takiy_synth::count_in_samples(sample_rate, sec_per_beat, bpb as u32);
        buf = prepend_count_in(buf, sec_per_beat, bpb as u32, &metro);
        pre_samples
    } else {
        0
    };

    // Loop region (en beats) → rango en frames ajustando por el count-in
    // (que vive en frames antes del score). PlayOpts/Player.position
    // cuentan frames (= samples por canal), no samples interleaved del
    // buffer mismo, así que el bound es buf.frames(), no buf.samples.len().
    let total_frames = buf.frames() as u64;
    let loop_range = editor.loop_region.and_then(|(from_b, to_b)| {
        let from_s = (from_b * sec_per_beat * sample_rate as f32) as u64
            + pre_samples as u64;
        let to_s = (to_b * sec_per_beat * sample_rate as f32) as u64
            + pre_samples as u64;
        if from_s < to_s && to_s <= total_frames {
            Some((from_s, to_s))
        } else {
            None
        }
    });

    // start_sample: si hay count-in arrancamos en 0 (durante el conteo);
    // si no, en el offset del beat pedido. La región de loop ya tiene
    // su pre_samples sumado, así que el cursor entra al score limpio.
    let beat_offset_samples = (start_beat * sec_per_beat * sample_rate as f32) as u64;
    let start_sample = if count_in { 0 } else { beat_offset_samples };

    (buf, PlayOpts { start_sample, loop_range })
}

/// Suffix del status: agrega "· loop X..Y" y "· 🎼 M" cuando aplican.
pub(crate) fn play_status_extras(editor: &EditorState) -> String {
    let mut s = String::new();
    if let Some((from, to)) = editor.loop_region {
        s.push_str(&format!(" · loop [{from:.0}, {to:.0})"));
    }
    if let Some(bpb) = editor.metronome_beats_per_bar {
        s.push_str(&format!(" · click {bpb}/4"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use takiy_core::{Pitch, ScoreNote, Track, WaveLayer, WaveOp};

    /// Peak absoluto del buffer estéreo en el rango de beats `[from, to)`.
    fn peak_in_beats(buf: &AudioBuffer, bpm: f32, sr: u32, from: f32, to: f32) -> f32 {
        let spb = 60.0 / bpm;
        let f0 = (from * spb * sr as f32) as usize;
        let f1 = ((to * spb * sr as f32) as usize).min(buf.frames());
        let ch = buf.channels.max(1) as usize;
        buf.samples[f0 * ch..f1 * ch]
            .iter()
            .fold(0.0_f32, |a, &b| a.max(b.abs()))
    }

    #[test]
    fn wave_silence_zeroes_that_region_in_the_render() {
        let sr = 8_000;
        let bpm = 120.0;
        let mut score = Score::new(bpm);
        let mut t = Track::new("nota larga");
        // Una nota sostenida 0..8 beats → señal continua en todo el tramo.
        t.add(ScoreNote::new(Pitch::A4, 0.0, 8.0, 120));
        score.add_track(t);

        // Sin ops: hay señal en [4,6).
        let dry = render_with_wave(&score, None, sr);
        assert!(peak_in_beats(&dry, bpm, sr, 4.5, 5.5) > 0.05, "el tramo suena sin editar");

        // Con Silence[4,6): ese tramo queda ~0 y el resto sigue sonando.
        score.track_mut(0).unwrap().wave =
            Some(WaveLayer { ops: vec![WaveOp::Silence { from: 4.0, to: 6.0 }] });
        let wet = render_with_wave(&score, None, sr);
        let silenced = peak_in_beats(&wet, bpm, sr, 4.5, 5.5);
        let audible = peak_in_beats(&wet, bpm, sr, 1.0, 2.0);
        assert!(silenced < 1e-3, "tramo silenciado debe ser ~0, fue {silenced}");
        assert!(audible > 0.05, "fuera de la selección sigue sonando, fue {audible}");
    }

    #[test]
    fn no_wave_ops_takes_the_byte_exact_path() {
        // Sin ops de onda, render_with_wave == render_score (mismo buffer).
        let mut score = Score::new(100.0);
        let mut t = Track::new("a");
        t.add(ScoreNote::new(Pitch::MIDDLE_C, 0.0, 2.0, 100));
        score.add_track(t);
        let a = render_with_wave(&score, None, 16_000);
        let b = render_score(&score, None, 16_000);
        assert_eq!(a.samples, b.samples);
    }
}
