//! `OhlcBuffer` — buffer plano de bars con stride 6 `f32`.
//!
//! Memoria contigua: `[t0, o0, h0, l0, c0, v0, t1, o1, …]`.
//! Acceso O(1) por índice; un memcpy completo para hidratar desde
//! una fuente externa.

/// Una barra OHLC + volumen. Valor leído del buffer; no es la
/// representación de almacenamiento (que vive como `[f32; 6]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub t: f32,
    pub o: f32,
    pub h: f32,
    pub l: f32,
    pub c: f32,
    pub v: f32,
}

impl Bar {
    pub fn is_bull(self) -> bool {
        self.c > self.o
    }
    pub fn is_bear(self) -> bool {
        self.c < self.o
    }
}

pub const STRIDE: usize = 6;

#[derive(Debug, Clone, Default)]
pub struct OhlcBuffer {
    bars: Vec<f32>,
    revision: u64,
}

impl OhlcBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(n: usize) -> Self {
        Self {
            bars: Vec::with_capacity(n * STRIDE),
            revision: 0,
        }
    }

    pub fn from_raw(bars: Vec<f32>) -> Self {
        assert!(bars.len() % STRIDE == 0, "OhlcBuffer: stride 6 required");
        Self { bars, revision: 0 }
    }

    pub fn push_bar(&mut self, b: Bar) {
        self.bars.push(b.t);
        self.bars.push(b.o);
        self.bars.push(b.h);
        self.bars.push(b.l);
        self.bars.push(b.c);
        self.bars.push(b.v);
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn push_values(&mut self, t: f32, o: f32, h: f32, l: f32, c: f32, v: f32) {
        self.push_bar(Bar { t, o, h, l, c, v });
    }

    pub fn len(&self) -> usize {
        self.bars.len() / STRIDE
    }

    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }

    pub fn bar(&self, i: usize) -> Bar {
        let off = i * STRIDE;
        Bar {
            t: self.bars[off],
            o: self.bars[off + 1],
            h: self.bars[off + 2],
            l: self.bars[off + 3],
            c: self.bars[off + 4],
            v: self.bars[off + 5],
        }
    }

    /// Slice plano del buffer subyacente.
    pub fn bars(&self) -> &[f32] {
        &self.bars
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn clear(&mut self) {
        self.bars.clear();
        self.revision = self.revision.wrapping_add(1);
    }

    /// Min/max de `low` y `high` sobre todo el buffer.
    /// Útil para autoscale del Y axis.
    pub fn price_range(&self) -> Option<(f32, f32)> {
        if self.is_empty() {
            return None;
        }
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for i in 0..self.len() {
            let b = self.bar(i);
            if b.l < lo {
                lo = b.l;
            }
            if b.h > hi {
                hi = b.h;
            }
        }
        Some((lo, hi))
    }

    /// Rango temporal `[t_min, t_max]`. None si vacío.
    pub fn time_range(&self) -> Option<(f32, f32)> {
        if self.is_empty() {
            return None;
        }
        let first = self.bars[0];
        let last = self.bars[self.bars.len() - STRIDE];
        Some((first, last))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_y_lectura() {
        let mut b = OhlcBuffer::with_capacity(2);
        b.push_values(1.0, 10.0, 12.0, 9.0, 11.0, 100.0);
        b.push_values(2.0, 11.0, 13.0, 10.0, 10.5, 80.0);
        assert_eq!(b.len(), 2);
        assert_eq!(b.bar(0).c, 11.0);
        assert_eq!(b.bar(1).h, 13.0);
    }

    #[test]
    fn bull_y_bear() {
        let bull = Bar { t: 0.0, o: 10.0, h: 11.0, l: 9.0, c: 10.5, v: 0.0 };
        let bear = Bar { t: 0.0, o: 10.0, h: 11.0, l: 9.0, c: 9.5, v: 0.0 };
        assert!(bull.is_bull());
        assert!(!bull.is_bear());
        assert!(bear.is_bear());
        assert!(!bear.is_bull());
    }

    #[test]
    fn price_range_correcto() {
        let mut b = OhlcBuffer::new();
        b.push_values(0.0, 10.0, 15.0, 8.0, 12.0, 0.0);
        b.push_values(1.0, 12.0, 14.0, 7.0, 9.0, 0.0);
        b.push_values(2.0, 9.0, 11.0, 9.0, 10.0, 0.0);
        let (lo, hi) = b.price_range().unwrap();
        assert_eq!(lo, 7.0);
        assert_eq!(hi, 15.0);
    }

    #[test]
    fn time_range() {
        let mut b = OhlcBuffer::new();
        b.push_values(10.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        b.push_values(50.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        b.push_values(100.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(b.time_range(), Some((10.0, 100.0)));
    }

    #[test]
    fn revision_bumps_en_push_y_clear() {
        let mut b = OhlcBuffer::new();
        let r0 = b.revision();
        b.push_values(0.0, 1.0, 1.0, 1.0, 1.0, 1.0);
        assert_ne!(r0, b.revision());
        let r1 = b.revision();
        b.clear();
        assert_ne!(r1, b.revision());
    }
}
