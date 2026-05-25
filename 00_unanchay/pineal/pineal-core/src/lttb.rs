//! LTTB (Largest-Triangle-Three-Buckets) — downsampling preservador
//! de silueta para series cartesianas.
//!
//! Algoritmo: dividir `n` puntos en `k-2` buckets (los extremos se
//! mantienen siempre). Por cada bucket, elegir el punto que forma
//! el triángulo de área máxima con el último punto elegido y el
//! centroide del bucket siguiente. Costo total O(n). Output ≤ k.
//!
//! Knob práctico: `target ≈ width_px × 3`. Tres vértices por pixel,
//! el anti-aliasing rellena el resto.

/// Reduce `coords` (interleaved `[x,y,x,y,…]`) a a lo sumo `target`
/// puntos, escribiendo los **índices originales** seleccionados en
/// `out` (sin clearearlo: el caller decide).
///
/// Si `n <= target` o `target < 3`, devuelve todos los índices
/// `[0..n)`.
pub fn lttb_indices(coords: &[f32], target: usize, out: &mut Vec<usize>) {
    let n = coords.len() / 2;
    if n == 0 {
        return;
    }
    if n <= target || target < 3 {
        out.extend(0..n);
        return;
    }
    lttb_in_range_indices(coords, 0, n, target, out);
}

/// Variante que opera sobre el rango `[start, end)` de un buffer
/// más grande. Los índices devueltos son **absolutos** (relativos
/// al `coords` original), no al sub-rango — esto le ahorra al caller
/// la corrección de offset después de un `SpatialIndex::range`.
pub fn lttb_in_range_indices(
    coords: &[f32],
    start: usize,
    end: usize,
    target: usize,
    out: &mut Vec<usize>,
) {
    debug_assert!(coords.len() % 2 == 0);
    debug_assert!(start <= end && end <= coords.len() / 2);

    let len = end - start;
    if len == 0 {
        return;
    }
    if len <= target || target < 3 {
        out.extend(start..end);
        return;
    }

    // Primero el extremo izquierdo.
    out.push(start);

    let bucket_size = (len - 2) as f64 / (target - 2) as f64;
    let mut a = start; // último punto elegido

    for i in 0..target - 2 {
        // Bucket actual y siguiente, en índices absolutos.
        let cur_lo = start + 1 + (i as f64 * bucket_size).floor() as usize;
        let cur_hi = start + 1 + ((i + 1) as f64 * bucket_size).floor() as usize;
        let next_lo = cur_hi.min(end);
        let next_hi = (start + 1 + ((i + 2) as f64 * bucket_size).floor() as usize).min(end);

        // Centroide del bucket siguiente. Si está vacío, fallback
        // al último punto.
        let (avg_x, avg_y) = if next_hi > next_lo {
            let span = (next_hi - next_lo) as f32;
            let mut sx = 0.0f32;
            let mut sy = 0.0f32;
            for j in next_lo..next_hi {
                sx += coords[j * 2];
                sy += coords[j * 2 + 1];
            }
            (sx / span, sy / span)
        } else {
            (coords[(end - 1) * 2], coords[(end - 1) * 2 + 1])
        };

        let ax = coords[a * 2];
        let ay = coords[a * 2 + 1];

        let mut max_area = -1.0f32;
        let mut max_idx = cur_lo;
        for j in cur_lo..cur_hi.min(end) {
            let bx = coords[j * 2];
            let by = coords[j * 2 + 1];
            // Área del triángulo (sin /2 porque comparamos relativos).
            let area = ((ax - avg_x) * (by - ay) - (ax - bx) * (avg_y - ay)).abs();
            if area > max_area {
                max_area = area;
                max_idx = j;
            }
        }
        out.push(max_idx);
        a = max_idx;
    }

    // Extremo derecho.
    out.push(end - 1);
}

/// Variante que materializa coords decimadas directamente — útil
/// cuando el painter sólo quiere un slice listo para `drawRawPoints`
/// y no necesita los índices.
pub fn lttb_coords(coords: &[f32], target: usize, out: &mut Vec<f32>) {
    let mut idx_buf: Vec<usize> = Vec::with_capacity(target);
    lttb_indices(coords, target, &mut idx_buf);
    for i in idx_buf {
        out.push(coords[i * 2]);
        out.push(coords[i * 2 + 1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_decimate_si_n_menor_que_target() {
        let coords: Vec<f32> = (0..5).flat_map(|i| [i as f32, (i * i) as f32]).collect();
        let mut out = Vec::new();
        lttb_indices(&coords, 10, &mut out);
        assert_eq!(out, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn extremos_preservados() {
        let n = 100;
        let coords: Vec<f32> = (0..n).flat_map(|i| [i as f32, (i as f32).sin()]).collect();
        let mut out = Vec::new();
        lttb_indices(&coords, 10, &mut out);
        assert_eq!(out.first(), Some(&0));
        assert_eq!(out.last(), Some(&(n - 1)));
        assert!(out.len() <= 10);
    }

    #[test]
    fn indices_sorted_y_unicos() {
        let coords: Vec<f32> = (0..1000)
            .flat_map(|i| [i as f32, (i as f32 * 0.01).sin()])
            .collect();
        let mut out = Vec::new();
        lttb_indices(&coords, 50, &mut out);
        for w in out.windows(2) {
            assert!(w[0] < w[1], "indices deben ser estrictamente crecientes");
        }
    }

    #[test]
    fn in_range_indices_son_absolutos() {
        let n = 100;
        let coords: Vec<f32> = (0..n).flat_map(|i| [i as f32, i as f32]).collect();
        let mut out = Vec::new();
        lttb_in_range_indices(&coords, 20, 80, 10, &mut out);
        assert_eq!(out.first(), Some(&20));
        assert_eq!(out.last(), Some(&79));
        // ningún índice fuera del rango pedido
        for &i in &out {
            assert!(i >= 20 && i < 80);
        }
    }

    #[test]
    fn preserva_picos_extremos() {
        // Señal plana con un pico al medio: LTTB debe agarrar el pico.
        let mut coords: Vec<f32> = Vec::new();
        for i in 0..200 {
            coords.push(i as f32);
            coords.push(if i == 100 { 10.0 } else { 0.0 });
        }
        let mut out = Vec::new();
        lttb_indices(&coords, 20, &mut out);
        assert!(out.contains(&100), "pico debe sobrevivir el downsample");
    }
}
