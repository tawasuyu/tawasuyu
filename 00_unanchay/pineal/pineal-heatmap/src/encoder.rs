//! Encoder: `HeatmapMatrix` → buffer ARGB para subir como textura.

use crate::matrix::HeatmapMatrix;
use crate::palette::Ramp;

/// Normaliza cada celda a `[0,1]` por min/max, la mapea por la rampa y
/// empaqueta el resultado como `u32` ARGB (0xAARRGGBB), fila por fila.
///
/// El backend GPUI sube este buffer como una textura y la rendea con un
/// solo `drawImageRect`, en vez de N draw calls.
pub fn encode_argb(matrix: &HeatmapMatrix, ramp: Ramp) -> Vec<u32> {
    let (min, max) = matrix.min_max();
    let span = max - min;
    let mut out = Vec::with_capacity(matrix.width() * matrix.height());
    for &v in matrix.data() {
        let t = if span > 0.0 { (v - min) / span } else { 0.0 };
        let c = ramp.sample(t);
        out.push(pack_argb(c.a, c.r, c.g, c.b));
    }
    out
}

/// Empaqueta 4 canales `f32` `[0,1]` en `0xAARRGGBB`.
fn pack_argb(a: f32, r: f32, g: f32, b: f32) -> u32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (q(a) << 24) | (q(r) << 16) | (q(g) << 8) | q(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_one_pixel_per_cell() {
        let m = HeatmapMatrix::from_data(vec![0.0, 1.0, 2.0, 3.0], 2, 2).unwrap();
        let buf = encode_argb(&m, Ramp::Grayscale);
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn normalizes_min_to_ramp_start() {
        // Grayscale: min → negro (0x000000), max → blanco (0xffffff).
        let m = HeatmapMatrix::from_data(vec![10.0, 20.0], 2, 1).unwrap();
        let buf = encode_argb(&m, Ramp::Grayscale);
        assert_eq!(buf[0] & 0x00ff_ffff, 0x0000_0000); // min
        assert_eq!(buf[1] & 0x00ff_ffff, 0x00ff_ffff); // max
        assert_eq!(buf[0] >> 24, 0xff); // alpha opaco
    }

    #[test]
    fn flat_matrix_does_not_divide_by_zero() {
        let m = HeatmapMatrix::from_data(vec![5.0; 4], 2, 2).unwrap();
        let buf = encode_argb(&m, Ramp::Viridis);
        assert_eq!(buf.len(), 4);
        assert!(buf.iter().all(|&p| p == buf[0]));
    }
}
