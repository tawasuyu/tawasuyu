//! Efectos de bus master aplicados al `AudioBuffer` post-mezcla.
//!
//! Hoy hay uno solo: un delay simple por feedback. La idea es mantener
//! el procesamiento agnóstico al renderer (osc, SF2, …) y dejar que
//! cada uno llame al efecto antes de normalizar el output.

use takiy_core::DelayParams;

use crate::audio::AudioBuffer;

/// Tope superior del feedback para que la cola decaiga. Más de esto
/// acumula amplitud cada ciclo de la línea y termina divergiendo
/// (sobre todo si la mezcla wet también es alta).
const MAX_FEEDBACK: f32 = 0.95;

/// Aplica un delay master in-place al `buf`. `sec_per_beat` viene del
/// tempo del `Score` (`60 / bpm`) — necesario para convertir
/// `params.time_beats` a samples.
///
/// El algoritmo es un *comb feedback* clásico, por canal:
///
/// ```text
/// y[n] = (1-mix)·x[n] + mix·d[n]          # output al buffer
/// d[n+D] = x[n] + feedback·d[n]            # almacenado en la línea
/// ```
///
/// donde `D` son los samples del retardo y `d[]` el ring buffer. Tras
/// la `D` muestras iniciales el primer eco aparece; las repeticiones
/// caen geométricamente según `feedback`. Si `feedback >= 1`, la línea
/// diverge — por eso clampeamos.
///
/// Idempotente sobre delay-of-silence: si el buffer es todo cero,
/// queda todo cero. Si `params.mix == 0`, el buffer queda intacto.
/// Si `delay_frames == 0` (tempo*beats degenerado), no toca nada.
pub fn apply_master_delay(buf: &mut AudioBuffer, sec_per_beat: f32, params: &DelayParams) {
    let channels = buf.channels.max(1) as usize;
    let delay_frames =
        (params.time_beats * sec_per_beat * buf.sample_rate as f32).round() as usize;
    if delay_frames == 0 || params.mix <= 0.0 {
        return;
    }
    let feedback = params.feedback.clamp(0.0, MAX_FEEDBACK);
    let mix = params.mix.clamp(0.0, 1.0);
    let dry = 1.0 - mix;
    let n_frames = buf.frames();

    // Ring buffer por canal. Pre-alocado al delay exacto: el índice
    // avanza módulo `delay_frames`, así que cada canal sólo necesita
    // ese tamaño.
    let mut delay_lines: Vec<Vec<f32>> =
        (0..channels).map(|_| vec![0.0; delay_frames]).collect();
    let mut idx = 0usize;

    for f in 0..n_frames {
        for (c, line) in delay_lines.iter_mut().enumerate().take(channels) {
            let i = f * channels + c;
            let dry_in = buf.samples[i];
            let wet = line[idx];
            buf.samples[i] = dry * dry_in + mix * wet;
            line[idx] = dry_in + feedback * wet;
        }
        idx += 1;
        if idx == delay_frames {
            idx = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse_buf(channels: u16, sample_rate: u32, n_frames: usize) -> AudioBuffer {
        let mut buf = AudioBuffer::silence_with_channels(sample_rate, n_frames, channels);
        // Impulso unitario en el primer frame, todos los canales.
        for c in 0..channels as usize {
            buf.samples[c] = 1.0;
        }
        buf
    }

    #[test]
    fn delay_with_mix_zero_is_noop() {
        let mut buf = impulse_buf(2, 44_100, 1024);
        let before = buf.samples.clone();
        apply_master_delay(
            &mut buf,
            0.5,
            &DelayParams { time_beats: 0.25, feedback: 0.5, mix: 0.0 },
        );
        assert_eq!(buf.samples, before);
    }

    #[test]
    fn delay_inserts_echoes_at_expected_frames() {
        // 1 beat/sec @ 100 Hz → 100 samples por beat. time_beats = 0.25 → 25 frames.
        let sr = 100;
        let mut buf = impulse_buf(1, sr, 200);
        apply_master_delay(
            &mut buf,
            1.0, // sec_per_beat = 1
            &DelayParams { time_beats: 0.25, feedback: 0.5, mix: 1.0 },
        );
        // En t=0 hay un impulso de amplitud 1 (dry=0, mix=1 → out=0 + mix*wet,
        // pero wet de la línea arranca en 0 → out=0). Tras 25 frames aparece
        // el primer eco (= mix * 1.0 = 1.0). A los 50 frames otro eco = 0.5.
        assert!(buf.samples[0].abs() < 1e-6, "frame 0 sin dry");
        assert!((buf.samples[25] - 1.0).abs() < 1e-6, "eco 1 a 25 frames");
        assert!((buf.samples[50] - 0.5).abs() < 1e-6, "eco 2 a 50 frames (fb)");
        assert!((buf.samples[75] - 0.25).abs() < 1e-6, "eco 3 a 75 frames");
    }

    #[test]
    fn delay_decays_under_max_feedback() {
        // Con feedback = MAX la cola debe converger (no diverger). Tomamos
        // un buffer largo y vemos que el peak no crece sin cota.
        let mut buf = impulse_buf(1, 100, 10_000);
        apply_master_delay(
            &mut buf,
            1.0,
            &DelayParams { time_beats: 0.25, feedback: 1.5 /* clamp a 0.95 */, mix: 1.0 },
        );
        // Pico no debe exceder mucho más que 1.0 (algunos pocos % por
        // sumas). Si feedback no estuviera clampeado, divergiría.
        let peak = buf.samples.iter().fold(0.0_f32, |a, b| a.max(b.abs()));
        assert!(peak < 2.0, "el peak {peak} sugiere divergencia");
    }

    #[test]
    fn delay_skips_when_time_too_small() {
        // time_beats * sec_per_beat * sr ≈ 0 → delay_frames = 0 → no-op.
        let mut buf = impulse_buf(2, 44_100, 32);
        let before = buf.samples.clone();
        apply_master_delay(
            &mut buf,
            1.0,
            &DelayParams { time_beats: 1e-9, feedback: 0.5, mix: 0.5 },
        );
        assert_eq!(buf.samples, before);
    }
}
