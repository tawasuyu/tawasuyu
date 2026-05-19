//! `SpatialIndex` — hit-testing sobre coords interleaved sorted-by-X.
//!
//! Cuando los puntos vienen ordenados por X (caso típico de series
//! temporales) un binary search basta y es O(log n) sin estructuras
//! auxiliares. Para nodos que se mueven cada frame (mesh graph)
//! corresponde un spatial hash uniforme — ese va en `pineal-mesh`,
//! no acá.

/// View sobre un buffer interleaved `[x0,y0,x1,y1,…]` sorted-asc por X.
///
/// El binary search asume invariante de ordenamiento. Si tu pipeline
/// puede generar coords desordenadas, sortealas antes de construir
/// el índice (no hay debug-assert porque sería O(n) en hot path).
#[derive(Debug, Clone, Copy)]
pub struct SpatialIndex<'a> {
    coords: &'a [f32],
}

impl<'a> SpatialIndex<'a> {
    pub fn new(coords: &'a [f32]) -> Self {
        debug_assert!(coords.len() % 2 == 0);
        Self { coords }
    }

    pub fn len(&self) -> usize {
        self.coords.len() / 2
    }

    pub fn is_empty(&self) -> bool {
        self.coords.is_empty()
    }

    /// Índice del punto cuya X está más cerca de `target_x`.
    /// `None` si el buffer está vacío.
    pub fn nearest(&self, target_x: f32) -> Option<usize> {
        let n = self.len();
        if n == 0 {
            return None;
        }
        // Binary search sobre la columna X.
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.coords[mid * 2] < target_x {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        // `lo` es la primera X >= target_x. El más cercano es lo o lo-1.
        if lo == 0 {
            Some(0)
        } else if lo >= n {
            Some(n - 1)
        } else {
            let prev = lo - 1;
            let dx_prev = target_x - self.coords[prev * 2];
            let dx_next = self.coords[lo * 2] - target_x;
            if dx_prev <= dx_next {
                Some(prev)
            } else {
                Some(lo)
            }
        }
    }

    /// Rango `[start, end)` de puntos con X en `[x_min, x_max]`.
    /// Útil para clip-to-viewport antes de LTTB.
    pub fn range(&self, x_min: f32, x_max: f32) -> (usize, usize) {
        let n = self.len();
        if n == 0 {
            return (0, 0);
        }
        // lower bound: primer i con coords[i*2] >= x_min
        let start = {
            let mut lo = 0usize;
            let mut hi = n;
            while lo < hi {
                let mid = (lo + hi) / 2;
                if self.coords[mid * 2] < x_min {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        // upper bound: primer i con coords[i*2] > x_max
        let end = {
            let mut lo = start;
            let mut hi = n;
            while lo < hi {
                let mid = (lo + hi) / 2;
                if self.coords[mid * 2] <= x_max {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        (start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<f32> {
        // x: 0, 1, 3, 5, 8 — y irrelevante.
        vec![0.0, 0.0, 1.0, 0.0, 3.0, 0.0, 5.0, 0.0, 8.0, 0.0]
    }

    #[test]
    fn nearest_dentro() {
        let c = fixture();
        let s = SpatialIndex::new(&c);
        assert_eq!(s.nearest(0.0), Some(0));
        assert_eq!(s.nearest(2.0), Some(1)); // 1 está más cerca que 3
        assert_eq!(s.nearest(2.5), Some(2)); // 3 está más cerca que 1
        assert_eq!(s.nearest(8.0), Some(4));
    }

    #[test]
    fn nearest_fuera_clamp() {
        let c = fixture();
        let s = SpatialIndex::new(&c);
        assert_eq!(s.nearest(-10.0), Some(0));
        assert_eq!(s.nearest(99.0), Some(4));
    }

    #[test]
    fn nearest_empty() {
        let empty: [f32; 0] = [];
        assert_eq!(SpatialIndex::new(&empty).nearest(0.0), None);
    }

    #[test]
    fn range_clip() {
        let c = fixture();
        let s = SpatialIndex::new(&c);
        assert_eq!(s.range(1.0, 5.0), (1, 4)); // incluye x=1,3,5
        assert_eq!(s.range(2.0, 4.0), (2, 3)); // sólo x=3
        assert_eq!(s.range(-1.0, 100.0), (0, 5));
        assert_eq!(s.range(10.0, 20.0), (5, 5));
    }
}
