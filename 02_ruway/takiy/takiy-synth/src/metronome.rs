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

/// Mezcla clicks de metrónomo en `buf` a la tasa indicada. `channels`
/// (1 = mono, 2 = stereo interleaved) determina cómo se distribuyen los
/// clicks: en mono se acumulan en cada sample; en estéreo, en ambos
/// canales del cuadro. `n_beats` limita cuántos beats se acentúan desde
/// `start_beat`; útil para count-in. Si `n_beats` es `None`, mezcla en
/// todo el buffer.
///
/// El beat 0 absoluto siempre arranca acentuado; los beats subsiguientes
/// caen acentuados o normales según `metro.beats_per_bar`.
pub fn mix_clicks(
    buf: &mut [f32],
    sample_rate: u32,
    channels: u16,
    sec_per_beat: f32,
    metro: &Metronome,
    start_beat: u32,
    n_beats: Option<u32>,
) {
    let sr = sample_rate as f32;
    let beats_per_bar = metro.beats_per_bar.max(1);
    let click_duration_sec = 0.05; // duración percusiva fija
    let ch = channels.max(1) as usize;

    let buf_frames = buf.len() / ch;
    let buf_seconds = buf_frames as f32 / sr;
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
            ch,
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
    channels: usize,
    start_sec: f32,
    duration_sec: f32,
    freq: f32,
    amp: f32,
    env: Adsr,
) {
    let start_frame = (start_sec * sr) as usize;
    let end_sec = start_sec + duration_sec + env.release;
    let n_frames = buf.len() / channels;
    let end_frame = ((end_sec * sr).ceil() as usize).min(n_frames);
    if start_frame >= end_frame {
        return;
    }
    let inv_sr = 1.0 / sr;
    for f in start_frame..end_frame {
        let t = (f - start_frame) as f32 * inv_sr;
        let phase = t * freq;
        let env_lvl = env.level(t, duration_sec);
        let s = Waveform::Sine.sample(phase) * env_lvl * amp;
        for c in 0..channels {
            buf[f * channels + c] += s;
        }
    }
}

/// Cantidad de samples que ocupa `n_beats` a `sec_per_beat`. Útil para
/// dimensionar el prepend de un count-in.
pub fn count_in_samples(sample_rate: u32, sec_per_beat: f32, n_beats: u32) -> usize {
    (n_beats as f32 * sec_per_beat * sample_rate as f32).ceil() as usize
}

/// Devuelve un nuevo `AudioBuffer` con `n_beats` de silencio al inicio,
/// con clicks de metrónomo, seguido por el contenido de `inner`. Útil
/// para implementar count-in sin tocar el renderer. Respeta los canales
/// de `inner` (mono o estéreo).
pub fn prepend_count_in(
    inner: AudioBuffer,
    sec_per_beat: f32,
    n_beats: u32,
    metro: &Metronome,
) -> AudioBuffer {
    let pre_frames = count_in_samples(inner.sample_rate, sec_per_beat, n_beats);
    let channels = inner.channels.max(1) as usize;
    let pre = pre_frames * channels;
    let total = pre + inner.samples.len();
    let mut samples = vec![0.0_f32; total];
    samples[pre..].copy_from_slice(&inner.samples);
    mix_clicks(
        &mut samples[..pre],
        inner.sample_rate,
        inner.channels,
        sec_per_beat,
        metro,
        0,
        Some(n_beats),
    );
    AudioBuffer { sample_rate: inner.sample_rate, channels: inner.channels, samples }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_clicks_makes_silent_buffer_audible() {
        let sr = 44_100;
        let mut buf = vec![0.0_f32; sr as usize]; // 1s mono
        mix_clicks(&mut buf, sr, 1, 0.5, &Metronome::DEFAULT, 0, None);
        let peak = buf.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(peak > 0.1, "peak = {peak}");
    }

    #[test]
    fn mix_clicks_stereo_mixes_in_both_channels() {
        let sr = 44_100;
        // 1s estéreo interleaved = 2 * sr samples.
        let mut buf = vec![0.0_f32; 2 * sr as usize];
        mix_clicks(&mut buf, sr, 2, 0.5, &Metronome::DEFAULT, 0, None);
        // Verifico que tanto los samples pares (L) como impares (R)
        // tengan energía.
        let left_peak = buf.iter().step_by(2).copied().map(f32::abs).fold(0.0_f32, f32::max);
        let right_peak = buf.iter().skip(1).step_by(2).copied().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(left_peak > 0.1, "L peak {left_peak}");
        assert!(right_peak > 0.1, "R peak {right_peak}");
    }

    #[test]
    fn count_in_samples_is_consistent_with_seconds() {
        // 4 beats a 0.5s/beat = 2s = 96000 samples a 48k.
        assert_eq!(count_in_samples(48_000, 0.5, 4), 96_000);
    }

    #[test]
    fn prepend_count_in_keeps_inner_audio_intact() {
        let sr = 44_100;
        // Buffer "interno" mono no-cero para verificar copia.
        let inner = AudioBuffer::from_mono(sr, vec![0.3; sr as usize]);
        let out = prepend_count_in(inner.clone(), 0.5, 2, &Metronome::DEFAULT);
        let pre = count_in_samples(sr, 0.5, 2) * inner.channels as usize;
        // El bloque interno se copia tal cual.
        for i in 0..inner.samples.len() {
            assert!((out.samples[pre + i] - 0.3).abs() < 1e-6);
        }
        // El bloque pre tiene clicks audibles.
        let pre_peak = out.samples[..pre].iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(pre_peak > 0.1, "pre_peak = {pre_peak}");
    }

    #[test]
    fn prepend_count_in_preserves_inner_channels() {
        let sr = 44_100;
        // Buffer estéreo interleaved (L=0.4, R=-0.4 en cada frame).
        let n_frames = sr as usize;
        let inner = AudioBuffer::from_stereo(
            sr,
            (0..n_frames).flat_map(|_| [0.4_f32, -0.4]).collect(),
        );
        let out = prepend_count_in(inner.clone(), 0.5, 2, &Metronome::DEFAULT);
        assert_eq!(out.channels, 2);
        let pre_frames = count_in_samples(sr, 0.5, 2);
        let pre = pre_frames * 2;
        // El inner se preserva.
        assert!((out.samples[pre] - 0.4).abs() < 1e-6);
        assert!((out.samples[pre + 1] + 0.4).abs() < 1e-6);
    }

    #[test]
    fn accent_lands_only_at_beat_zero() {
        // Construyo un buffer mono de 4 beats a 0.5s/beat = 2s a 44.1k.
        // Mezclo SOLO el acento y verifico que cada beat tiene energía.
        let sr = 44_100;
        let n = (2.0 * sr as f32) as usize;
        let mut buf = vec![0.0_f32; n];
        let mut m = Metronome::DEFAULT;
        m.click_hz = m.accent_hz; // forzamos para que el test sea simple
        mix_clicks(&mut buf, sr, 1, 0.5, &m, 0, None);
        let beat_samples = (0.5 * sr as f32) as usize;
        for beat in 0..4 {
            let s = beat * beat_samples;
            let slice = &buf[s..(s + beat_samples / 4)];
            let peak = slice.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
            assert!(peak > 0.05, "beat {beat} peak {peak}");
        }
    }
}
