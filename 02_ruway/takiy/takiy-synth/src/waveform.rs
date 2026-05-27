//! Formas de onda básicas — todas evaluadas en fase normalizada `[0, 1)`.

/// Las cuatro ondas clásicas de un sintetizador analógico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    /// Suave, dulce, pobre en armónicos.
    Sine,
    /// Hueca, tipo clarinete; sólo armónicos impares.
    Square,
    /// Rasposa, brillante; todos los armónicos.
    Saw,
    /// Más suave que la cuadrada, sólo impares pero con caída rápida.
    Triangle,
}

impl Waveform {
    /// Muestra en `[-1, 1]` para una fase normalizada `phase ∈ [0, 1)`.
    /// Fases fuera de rango se envuelven con `fract`.
    pub fn sample(self, phase: f32) -> f32 {
        let p = phase.rem_euclid(1.0);
        match self {
            Waveform::Sine => (p * std::f32::consts::TAU).sin(),
            Waveform::Square => if p < 0.5 { 1.0 } else { -1.0 },
            Waveform::Saw => 2.0 * p - 1.0,
            Waveform::Triangle => {
                if p < 0.5 { 4.0 * p - 1.0 } else { 3.0 - 4.0 * p }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_starts_at_zero() {
        assert!(Waveform::Sine.sample(0.0).abs() < 1e-6);
    }

    #[test]
    fn sine_peaks_at_quarter() {
        assert!((Waveform::Sine.sample(0.25) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn square_is_pm_one() {
        assert_eq!(Waveform::Square.sample(0.1), 1.0);
        assert_eq!(Waveform::Square.sample(0.9), -1.0);
    }

    #[test]
    fn saw_ramps_minus_one_to_one() {
        assert!((Waveform::Saw.sample(0.0) - -1.0).abs() < 1e-6);
        assert!((Waveform::Saw.sample(0.999) - 1.0).abs() < 1e-2);
    }

    #[test]
    fn triangle_peaks_at_half() {
        assert!((Waveform::Triangle.sample(0.5) - 1.0).abs() < 1e-6);
        assert!((Waveform::Triangle.sample(0.0) - -1.0).abs() < 1e-6);
    }

    #[test]
    fn phase_wraps() {
        assert!((Waveform::Sine.sample(1.25) - Waveform::Sine.sample(0.25)).abs() < 1e-6);
    }
}
