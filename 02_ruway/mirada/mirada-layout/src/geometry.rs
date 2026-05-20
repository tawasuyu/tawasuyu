//! Geometría — el rectángulo en coordenadas de pantalla.

use serde::{Deserialize, Serialize};

/// Un rectángulo en píxeles de pantalla. El origen `(0,0)` es la
/// esquina superior-izquierda; `x` crece a la derecha, `y` hacia abajo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Área en píxeles cuadrados.
    pub fn area(&self) -> i64 {
        self.w.max(0) as i64 * self.h.max(0) as i64
    }

    /// `true` si el rectángulo tiene ancho y alto positivos.
    pub fn is_visible(&self) -> bool {
        self.w > 0 && self.h > 0
    }

    /// Encoge el rectángulo `g` píxeles por cada lado. Si el margen se
    /// come toda la dimensión, ésta queda en `0` (no negativa).
    pub fn inset(&self, g: i32) -> Rect {
        Rect {
            x: self.x + g,
            y: self.y + g,
            w: (self.w - 2 * g).max(0),
            h: (self.h - 2 * g).max(0),
        }
    }

    /// `true` si `(px, py)` cae dentro del rectángulo.
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Reparte `total` píxeles en `n` tramos contiguos sin perder ni un
/// píxel: las fronteras caen en `total · k / n`, así que la suma de los
/// tamaños es exactamente `total`. Devuelve `(offset, tamaño)` por tramo.
pub fn split(total: i32, n: usize) -> Vec<(i32, i32)> {
    if n == 0 {
        return Vec::new();
    }
    let total = total.max(0) as i64;
    let n64 = n as i64;
    (0..n)
        .map(|k| {
            let start = total * k as i64 / n64;
            let end = total * (k as i64 + 1) / n64;
            (start as i32, (end - start) as i32)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inset_shrinks_by_gap_on_every_side() {
        let r = Rect::new(0, 0, 100, 80).inset(5);
        assert_eq!(r, Rect::new(5, 5, 90, 70));
    }

    #[test]
    fn inset_clamps_to_zero() {
        let r = Rect::new(0, 0, 8, 8).inset(10);
        assert_eq!((r.w, r.h), (0, 0));
        assert!(!r.is_visible());
    }

    #[test]
    fn split_loses_no_pixels() {
        for n in 1..=13 {
            let parts = split(1000, n);
            assert_eq!(parts.len(), n);
            assert_eq!(parts.iter().map(|(_, s)| *s).sum::<i32>(), 1000);
            // Los tramos son contiguos.
            for w in parts.windows(2) {
                assert_eq!(w[0].0 + w[0].1, w[1].0);
            }
        }
    }

    #[test]
    fn contains_checks_bounds() {
        let r = Rect::new(10, 10, 20, 20);
        assert!(r.contains(10, 10));
        assert!(r.contains(29, 29));
        assert!(!r.contains(30, 30));
        assert!(!r.contains(9, 15));
    }
}
