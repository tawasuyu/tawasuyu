//! `AudioBuffer` — bloque PCM mono en `f32`, valores en `[-1.0, 1.0]`.

/// Bloque de audio mono. Los valores fuera de `[-1, 1]` clipean al escribir.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    /// Tasa de muestreo en Hz (44100, 48000, …).
    pub sample_rate: u32,
    /// Muestras mono, intercaladas en el tiempo.
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    /// Buffer silencioso de `n_samples` muestras.
    pub fn silence(sample_rate: u32, n_samples: usize) -> Self {
        Self { sample_rate, samples: vec![0.0; n_samples] }
    }

    /// Duración en segundos según la tasa de muestreo.
    pub fn duration_seconds(&self) -> f32 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.samples.len() as f32 / self.sample_rate as f32
    }

    /// Pico absoluto en `[0, ∞)`.
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
        let mut buf = AudioBuffer { sample_rate: 44_100, samples: vec![2.0, -3.0, 1.5] };
        buf.normalize_if_clipping();
        assert!((buf.peak() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_leaves_quiet_buffer_alone() {
        let mut buf = AudioBuffer { sample_rate: 44_100, samples: vec![0.3, -0.2, 0.1] };
        let before = buf.samples.clone();
        buf.normalize_if_clipping();
        assert_eq!(buf.samples, before);
    }
}
