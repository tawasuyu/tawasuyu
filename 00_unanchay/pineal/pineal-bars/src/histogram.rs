//! Bineado de una muestra en conteos, para alimentar [`paint_bars`] como
//! histograma. Zero-boxing: la entrada es un `&[f32]` plano (P1 del SDD).

use crate::paint::Bar;
use pineal_render::Color;

/// Conteos por bin de una muestra sobre `[lo, hi]`.
#[derive(Debug, Clone)]
pub struct Histogram {
    /// Conteo por bin (longitud = nº de bins).
    pub counts: Vec<f64>,
    pub lo: f32,
    pub hi: f32,
    /// Ancho de cada bin en unidades de dato.
    pub bin_width: f32,
}

impl Histogram {
    /// Binea `values` en `n_bins` sobre su propio min..max. Valores
    /// fuera de rango (NaN) se ignoran. Con `n_bins == 0` o muestra
    /// vacía devuelve un histograma vacío.
    pub fn new(values: &[f32], n_bins: usize) -> Self {
        if n_bins == 0 || values.is_empty() {
            return Self { counts: Vec::new(), lo: 0.0, hi: 0.0, bin_width: 0.0 };
        }
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for &v in values {
            if v.is_nan() {
                continue;
            }
            lo = lo.min(v);
            hi = hi.max(v);
        }
        if !lo.is_finite() || !hi.is_finite() {
            return Self { counts: vec![0.0; n_bins], lo: 0.0, hi: 0.0, bin_width: 0.0 };
        }
        // Rango degenerado (todos iguales): ensancha para no dividir por 0.
        if (hi - lo).abs() < f32::EPSILON {
            hi = lo + 1.0;
        }
        let bin_width = (hi - lo) / n_bins as f32;
        let mut counts = vec![0.0_f64; n_bins];
        for &v in values {
            if v.is_nan() {
                continue;
            }
            let mut idx = ((v - lo) / bin_width) as usize;
            if idx >= n_bins {
                idx = n_bins - 1; // el máximo cae en el último bin
            }
            counts[idx] += 1.0;
        }
        Self { counts, lo, hi, bin_width }
    }

    /// Convierte los conteos en barras de un mismo color, listas para
    /// [`crate::paint_bars`].
    pub fn to_bars(&self, color: Color) -> Vec<Bar> {
        self.counts.iter().map(|&c| Bar::new(c, color)).collect()
    }

    /// Conteo del bin más poblado (útil para fijar el rango del eje).
    pub fn max_count(&self) -> f64 {
        self.counts.iter().copied().fold(0.0, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_sum_to_sample_size() {
        let v = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let h = Histogram::new(&v, 5);
        let total: f64 = h.counts.iter().sum();
        assert_eq!(total, 10.0);
        assert_eq!(h.counts.len(), 5);
    }

    #[test]
    fn max_value_lands_in_last_bin() {
        let v = [0.0, 10.0];
        let h = Histogram::new(&v, 4);
        assert_eq!(h.counts[3], 1.0, "el máximo cae en el último bin");
        assert_eq!(h.counts[0], 1.0);
    }

    #[test]
    fn empty_and_zero_bins_are_safe() {
        assert!(Histogram::new(&[], 5).counts.iter().all(|&c| c == 0.0) || Histogram::new(&[], 5).counts.is_empty());
        assert!(Histogram::new(&[1.0, 2.0], 0).counts.is_empty());
    }

    #[test]
    fn degenerate_range_does_not_panic() {
        let v = [3.0, 3.0, 3.0];
        let h = Histogram::new(&v, 4);
        assert_eq!(h.counts.iter().sum::<f64>(), 3.0);
    }
}
