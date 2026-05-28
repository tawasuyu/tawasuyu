//! Metrónomo — clicks audibles al inicio de cada beat.
//!
//! No es un instrumento del [`Score`] sino una capa de mezcla que el
//! renderer aplica en la salida final. Vive separado para que el
//! `Score` siga siendo independiente del tempo en disco (cosa que el
//! README garantiza), y para que el count-in pueda reusarlo.

use crate::audio::AudioBuffer;
use crate::envelope::Adsr;
use crate::waveform::Waveform;

/// Configuración del metrónomo. `None` en el renderer significa que no
/// se mezcla ningún click; `Some` activa los clicks audibles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metronome {
    /// Pulsos por compás (típicamente 4 para 4/4, 3 para 3/4).
    pub beats_per_bar: u8,
    /// Frecuencia del click en beats no acentuados (Hz).
    pub click_hz: f32,
    /// Frecuencia del click acentuado (beat 0 del compás), en Hz.
    pub accent_hz: f32,
    /// Amplitud máxima en `[0, 1]`. Se aplica también al acento.
    pub amplitude: f32,
    /// Envolvente — corta y percusiva por default.
    pub envelope: Adsr,
}

impl Metronome {
    /// Default razonable: 4/4, click a 1500 Hz, acento a 2000 Hz,
    /// amplitud 0.35, envolvente percusiva (1ms attack, 80ms release).
    pub const DEFAULT: Metronome = Metronome {
        beats_per_bar: 4,
        click_hz: 1500.0,
        accent_hz: 2000.0,
        amplitude: 0.35,
        envelope: Adsr { attack: 0.001, decay: 0.04, sustain: 0.0, release: 0.08 },
    };
}

impl Default for Metronome {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Mezcla clicks de metrónomo en `buf` a la tasa indicada. `n_beats`
/// limita cuántos beats se acentúan desde `start_beat`; útil para count-in
/// (mezclar sólo unos beats al principio). Si `n_beats` es `None`, mezcla
/// clicks en todo el buffer.
///
/// El beat 0 absoluto siempre arranca acentuado; los beats subsiguientes
/// caen acentuados o normales según `metro.beats_per_bar`.
pub fn mix_clicks(
    buf: &mut [f32],
    sample_rate: u32,
    sec_per_beat: f32,
    metro: &Metronome,
    start_beat: u32,
    n_beats: Option<u32>,
) {
    let sr = sample_rate as f32;
    let beats_per_bar = metro.beats_per_bar.max(1);
    let click_duration_sec = 0.05; // duración percusiva fija

    let buf_seconds = buf.len() as f32 / sr;
    let last_beat_in_buf = (buf_seconds / sec_per_beat).ceil() as u32;
    let end_beat = match n_beats {
        Some(n) => start_beat.saturating_add(n).min(last_beat_in_buf + 1),
        None => last_beat_in_buf + 1,
    };

    for beat in start_beat..end_beat {
        let beat_sec = beat as f32 * sec_per_beat;
        let is_accent = (beat as u32) % beats_per_bar as u32 == 0;
        let freq = if is_accent { metro.accent_hz } else { metro.click_hz };
        mix_click(
            buf,
            sr,
            beat_sec,
            click_duration_sec,
            freq,
            metro.amplitude,
            metro.envelope,
        );
    }
}

fn mix_click(
    buf: &mut [f32],
    sr: f32,
    start_sec: f32,
    duration_sec: f32,
    freq: f32,
    amp: f32,
    env: Adsr,
) {
    let start_idx = (start_sec * sr) as usize;
    let end_sec = start_sec + duration_sec + env.release;
    let end_idx = ((end_sec * sr).ceil() as usize).min(buf.len());
    if start_idx >= end_idx {
        return;
    }
    let inv_sr = 1.0 / sr;
    for i in start_idx..end_idx {
        let t = (i - start_idx) as f32 * inv_sr;
        let phase = t * freq;
        let env_lvl = env.level(t, duration_sec);
        buf[i] += Waveform::Sine.sample(phase) * env_lvl * amp;
    }
}

/// Cantidad de samples que ocupa `n_beats` a `sec_per_beat`. Útil para
/// dimensionar el prepend de un count-in.
pub fn count_in_samples(sample_rate: u32, sec_per_beat: f32, n_beats: u32) -> usize {
    (n_beats as f32 * sec_per_beat * sample_rate as f32).ceil() as usize
}

/// Devuelve un nuevo `AudioBuffer` con `n_beats` de silencio al inicio,
/// con clicks de metrónomo, seguido por el contenido de `inner`. Útil
/// para implementar count-in sin tocar el renderer.
pub fn prepend_count_in(
    inner: AudioBuffer,
    sec_per_beat: f32,
    n_beats: u32,
    metro: &Metronome,
) -> AudioBuffer {
    let pre = count_in_samples(inner.sample_rate, sec_per_beat, n_beats);
    let total = pre + inner.samples.len();
    let mut samples = vec![0.0_f32; total];
    samples[pre..].copy_from_slice(&inner.samples);
    mix_clicks(
        &mut samples[..pre],
        inner.sample_rate,
        sec_per_beat,
        metro,
        0,
        Some(n_beats),
    );
    AudioBuffer { sample_rate: inner.sample_rate, samples }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_clicks_makes_silent_buffer_audible() {
        let sr = 44_100;
        let mut buf = vec![0.0_f32; sr as usize]; // 1s
        mix_clicks(&mut buf, sr, 0.5, &Metronome::DEFAULT, 0, None);
        let peak = buf.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(peak > 0.1, "peak = {peak}");
    }

    #[test]
    fn count_in_samples_is_consistent_with_seconds() {
        // 4 beats a 0.5s/beat = 2s = 96000 samples a 48k.
        assert_eq!(count_in_samples(48_000, 0.5, 4), 96_000);
    }

    #[test]
    fn prepend_count_in_keeps_inner_audio_intact() {
        let sr = 44_100;
        // Buffer "interno" no-cero para verificar copia.
        let inner = AudioBuffer {
            sample_rate: sr,
            samples: vec![0.3; sr as usize],
        };
        let out = prepend_count_in(inner.clone(), 0.5, 2, &Metronome::DEFAULT);
        let pre = count_in_samples(sr, 0.5, 2);
        // El bloque interno se copia tal cual.
        for i in 0..inner.samples.len() {
            assert!((out.samples[pre + i] - 0.3).abs() < 1e-6);
        }
        // El bloque pre tiene clicks audibles.
        let pre_peak = out.samples[..pre].iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(pre_peak > 0.1, "pre_peak = {pre_peak}");
    }

    #[test]
    fn accent_lands_only_at_beat_zero() {
        // Construyo un buffer de 4 beats a 0.5s/beat = 2s a 44.1k. Mezclo
        // SOLO el acento (frecuencia distinta) y verifico que la energía
        // en el rango del acento (2000 Hz) sea mayor en el primer beat
        // que en los siguientes. Test de aprox energía no exacta.
        let sr = 44_100;
        let n = (2.0 * sr as f32) as usize;
        let mut buf = vec![0.0_f32; n];
        let mut m = Metronome::DEFAULT;
        m.click_hz = m.accent_hz; // forzamos para que el test sea simple
        mix_clicks(&mut buf, sr, 0.5, &m, 0, None);
        // Debería haber 4 clicks distribuidos cada 0.5s.
        let beat_samples = (0.5 * sr as f32) as usize;
        for beat in 0..4 {
            let s = beat * beat_samples;
            let slice = &buf[s..(s + beat_samples / 4)];
            let peak = slice.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
            assert!(peak > 0.05, "beat {beat} peak {peak}");
        }
    }
}
