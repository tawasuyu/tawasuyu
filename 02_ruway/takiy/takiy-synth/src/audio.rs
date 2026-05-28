//! `AudioBuffer` — bloque PCM en `f32`, valores en `[-1.0, 1.0]`.
//!
//! Soporta mono y estéreo. En estéreo, `samples` es interleaved
//! `[L0, R0, L1, R1, …]` para que la transferencia al device (cpal,
//! WAV, etc.) sea directa sin reempaquetar.

/// Bloque de audio mono o estéreo. Los valores fuera de `[-1, 1]`
/// clipean al cuantizar (WAV) y al alimentar al device.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    /// Tasa de muestreo en Hz (44100, 48000, …).
    pub sample_rate: u32,
    /// Canales: `1` = mono, `2` = estéreo interleaved.
    pub channels: u16,
    /// Muestras. En mono: `[s0, s1, …]`. En estéreo: `[L0, R0, L1, R1, …]`.
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    /// Buffer mono silencioso de `n_samples` muestras (atajo histórico —
    /// equivale a `silence_with_channels(sr, n_samples, 1)`).
    pub fn silence(sample_rate: u32, n_samples: usize) -> Self {
        Self::silence_with_channels(sample_rate, n_samples, 1)
    }

    /// Buffer silencioso de `n_frames` cuadros × `channels` canales.
    /// `n_frames` es el tiempo en muestras-por-canal, no el total.
    pub fn silence_with_channels(sample_rate: u32, n_frames: usize, channels: u16) -> Self {
        let total = n_frames * channels.max(1) as usize;
        Self { sample_rate, channels: channels.max(1), samples: vec![0.0; total] }
    }

    /// Buffer mono a partir de un `Vec<f32>` existente.
    pub fn from_mono(sample_rate: u32, samples: Vec<f32>) -> Self {
        Self { sample_rate, channels: 1, samples }
    }

    /// Buffer estéreo interleaved a partir de un `Vec<f32>` `[L0, R0, …]`.
    /// `len()` debe ser par.
    pub fn from_stereo(sample_rate: u32, samples: Vec<f32>) -> Self {
        debug_assert!(samples.len() % 2 == 0, "estéreo interleaved necesita len par");
        Self { sample_rate, channels: 2, samples }
    }

    /// Cantidad de cuadros (muestras por canal). Para mono coincide con
    /// `samples.len()`; para estéreo es la mitad.
    pub fn frames(&self) -> usize {
        let c = self.channels.max(1) as usize;
        self.samples.len() / c
    }

    /// Duración en segundos según la tasa de muestreo y los canales.
    pub fn duration_seconds(&self) -> f32 {
        if self.sample_rate == 0 || self.channels == 0 {
            return 0.0;
        }
        self.frames() as f32 / self.sample_rate as f32
    }

    /// Pico absoluto en `[0, ∞)`. Considera ambos canales si es estéreo.
    pub fn peak(&self) -> f32 {
        self.samples.iter().copied().map(f32::abs).fold(0.0, f32::max)
    }

    /// Si el pico excede `1.0`, escala todo para que caiga justo en `1.0`.
    /// Si no, no toca nada.
    pub fn normalize_if_clipping(&mut self) {
        let peak = self.peak();
        if peak > 1.0 {
            let inv = 1.0 / peak;
            for s in &mut self.samples {
                *s *= inv;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_has_zero_peak() {
        let buf = AudioBuffer::silence(44_100, 1024);
        assert_eq!(buf.peak(), 0.0);
        assert!((buf.duration_seconds() - 1024.0 / 44_100.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_caps_at_one() {
        let mut buf = AudioBuffer::from_mono(44_100, vec![2.0, -3.0, 1.5]);
        buf.normalize_if_clipping();
        assert!((buf.peak() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_leaves_quiet_buffer_alone() {
        let mut buf = AudioBuffer::from_mono(44_100, vec![0.3, -0.2, 0.1]);
        let before = buf.samples.clone();
        buf.normalize_if_clipping();
        assert_eq!(buf.samples, before);
    }

    #[test]
    fn mono_constructor_yields_channels_one() {
        let buf = AudioBuffer::from_mono(44_100, vec![0.1, 0.2, 0.3]);
        assert_eq!(buf.channels, 1);
        assert_eq!(buf.frames(), 3);
    }

    #[test]
    fn stereo_silence_has_n_frames_per_channel() {
        let buf = AudioBuffer::silence_with_channels(48_000, 1024, 2);
        assert_eq!(buf.channels, 2);
        assert_eq!(buf.samples.len(), 2048);
        assert_eq!(buf.frames(), 1024);
        assert!((buf.duration_seconds() - 1024.0 / 48_000.0).abs() < 1e-6);
    }

    #[test]
    fn stereo_constructor_panics_on_odd_len_in_debug() {
        // En release `from_stereo` no panicea; en debug sí. Test corre
        // siempre porque `debug_assert!` está activo en `cfg(test)`.
        let result = std::panic::catch_unwind(|| {
            AudioBuffer::from_stereo(44_100, vec![0.1, 0.2, 0.3]);
        });
        assert!(result.is_err());
    }
}
