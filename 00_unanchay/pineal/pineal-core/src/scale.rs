//! Escalas value→pixel para series cartesianas.
//!
//! La proyección no se aplica sobre los datos (eso rompería el
//! P2 zero-alloc — habría que reescribir todo el buffer por frame).
//! Las escalas devuelven el `(scale_x, scale_y, translate_x,
//! translate_y)` que el painter mete en un transform GPU. Los
//! datos quedan intactos.

/// Trait común a Linear / Log / Time. Cada implementación traduce
/// un valor de dominio a posición normalizada `[0, 1]` (que luego
/// el painter mapea al pixel range del plot).
pub trait Scale {
    fn to_norm(&self, value: f64) -> f64;
    fn from_norm(&self, norm: f64) -> f64;
    fn domain(&self) -> (f64, f64);
}

#[derive(Debug, Clone, Copy)]
pub struct LinearScale {
    min: f64,
    max: f64,
}

impl LinearScale {
    pub fn new(min: f64, max: f64) -> Self {
        debug_assert!(max > min, "LinearScale: max debe ser > min");
        Self { min, max }
    }
}

impl Scale for LinearScale {
    fn to_norm(&self, v: f64) -> f64 {
        (v - self.min) / (self.max - self.min)
    }
    fn from_norm(&self, n: f64) -> f64 {
        self.min + n * (self.max - self.min)
    }
    fn domain(&self) -> (f64, f64) {
        (self.min, self.max)
    }
}

/// Escala logarítmica base e. `min` y `max` deben ser positivos.
#[derive(Debug, Clone, Copy)]
pub struct LogScale {
    log_min: f64,
    log_max: f64,
    min: f64,
    max: f64,
}

impl LogScale {
    pub fn new(min: f64, max: f64) -> Self {
        debug_assert!(min > 0.0 && max > min, "LogScale: 0 < min < max");
        Self {
            log_min: min.ln(),
            log_max: max.ln(),
            min,
            max,
        }
    }
}

impl Scale for LogScale {
    fn to_norm(&self, v: f64) -> f64 {
        (v.ln() - self.log_min) / (self.log_max - self.log_min)
    }
    fn from_norm(&self, n: f64) -> f64 {
        (self.log_min + n * (self.log_max - self.log_min)).exp()
    }
    fn domain(&self) -> (f64, f64) {
        (self.min, self.max)
    }
}

/// Escala temporal sobre epoch ms. Internamente lineal.
#[derive(Debug, Clone, Copy)]
pub struct TimeScale {
    inner: LinearScale,
}

impl TimeScale {
    pub fn new(min_epoch_ms: f64, max_epoch_ms: f64) -> Self {
        Self {
            inner: LinearScale::new(min_epoch_ms, max_epoch_ms),
        }
    }
}

impl Scale for TimeScale {
    fn to_norm(&self, v: f64) -> f64 {
        self.inner.to_norm(v)
    }
    fn from_norm(&self, n: f64) -> f64 {
        self.inner.from_norm(n)
    }
    fn domain(&self) -> (f64, f64) {
        self.inner.domain()
    }
}

/// Wilkinson "nice numbers" — devuelve el step ideal en `{1, 2, 5} × 10^k`
/// para que un rango `[min, max]` tenga ~`target_ticks` divisiones.
pub fn nice_step(min: f64, max: f64, target_ticks: usize) -> f64 {
    debug_assert!(max > min && target_ticks > 0);
    let raw = (max - min) / target_ticks as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let nice = if norm < 1.5 {
        1.0
    } else if norm < 3.0 {
        2.0
    } else if norm < 7.0 {
        5.0
    } else {
        10.0
    };
    nice * mag
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_roundtrip() {
        let s = LinearScale::new(10.0, 20.0);
        assert!((s.to_norm(15.0) - 0.5).abs() < 1e-9);
        assert!((s.from_norm(0.5) - 15.0).abs() < 1e-9);
    }

    #[test]
    fn log_roundtrip() {
        let s = LogScale::new(1.0, 1000.0);
        // 10 está a 1/3 del camino en log10. ln(10)/ln(1000) = 1/3.
        assert!((s.to_norm(10.0) - 1.0 / 3.0).abs() < 1e-9);
        assert!((s.from_norm(2.0 / 3.0) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn nice_step_es_potencia() {
        // 100/5 = 20 — exact match para el branch nice=2.0 · mag=10.
        assert!((nice_step(0.0, 100.0, 5) - 20.0).abs() < 1e-9);
        // 1.0/10 = 0.1 — branch nice=1.0 · mag=0.1.
        assert!((nice_step(0.0, 1.0, 10) - 0.1).abs() < 1e-9);
        // 14/5 = 2.8 — branch nice=2.0 (1.5 ≤ norm < 3) · mag=1.
        assert!((nice_step(0.0, 14.0, 5) - 2.0).abs() < 1e-9);
        // 7/5 = 1.4 — cae bajo 1.5 → snap a 1.0 · mag=1 = 1.0.
        assert!((nice_step(0.0, 7.0, 5) - 1.0).abs() < 1e-9);
        // 50/5 = 10 — branch nice=10 · mag=1 = 10. (Equivalente a 1·10.)
        assert!((nice_step(0.0, 50.0, 5) - 10.0).abs() < 1e-9);
    }
}
