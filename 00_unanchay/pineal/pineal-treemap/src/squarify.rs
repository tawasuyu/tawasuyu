//! Treemap squarified (Bruls, Huizing & van Wijk, 2000).
//!
//! Asigna a cada peso un rectángulo de área proporcional, minimizando
//! el peor aspect ratio (rects lo más cuadrados posible). Pre-escala los
//! pesos al área del rect destino para estabilidad numérica.

use pineal_render::Rect;

/// Calcula el layout: devuelve un `Rect` por peso, en el mismo orden de
/// entrada. Pesos `<= 0` o no finitos reciben un rect de área cero.
pub fn squarify(weights: &[f64], area: Rect) -> Vec<Rect> {
    let n = weights.len();
    let zero = Rect::new(area.x, area.y, 0.0, 0.0);
    let mut out = vec![zero; n];

    let area_px = area.w as f64 * area.h as f64;
    let total: f64 = weights.iter().filter(|w| w.is_finite() && **w > 0.0).sum();
    if n == 0 || total <= 0.0 || area_px <= 0.0 {
        return out;
    }
    let scale = area_px / total;

    // Sólo los pesos positivos participan; los demás quedan con rect cero.
    // `idx` se ordena por área descendente — mejora los aspect ratios.
    let areas: Vec<f64> = weights
        .iter()
        .map(|w| if w.is_finite() && *w > 0.0 { w * scale } else { 0.0 })
        .collect();
    let mut idx: Vec<usize> = (0..n).filter(|&i| areas[i] > 0.0).collect();
    idx.sort_by(|&a, &b| areas[b].partial_cmp(&areas[a]).unwrap_or(std::cmp::Ordering::Equal));

    let mut free = area;
    let mut row: Vec<usize> = Vec::new();
    let mut i = 0;

    while i < idx.len() {
        let side = free.w.min(free.h) as f64;
        let cur = worst_ratio(&row, &areas, side);
        row.push(idx[i]);
        let with_next = worst_ratio(&row, &areas, side);

        if cur > 0.0 && with_next > cur {
            // Agregar el item empeoró el ratio: revertir, cerrar la fila.
            row.pop();
            free = layout_row(&row, &areas, free, &mut out);
            row.clear();
        } else {
            i += 1;
        }
    }
    if !row.is_empty() {
        layout_row(&row, &areas, free, &mut out);
    }
    out
}

/// Peor aspect ratio de una fila tendida sobre un lado de longitud `side`.
/// Fórmula de Bruls et al.: `max(side²·max / sum², sum² / (side²·min))`.
fn worst_ratio(row: &[usize], areas: &[f64], side: f64) -> f64 {
    if row.is_empty() || side <= 0.0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut mx = f64::MIN;
    let mut mn = f64::MAX;
    for &i in row {
        let a = areas[i];
        sum += a;
        mx = mx.max(a);
        mn = mn.min(a);
    }
    if sum <= 0.0 || mn <= 0.0 {
        return f64::INFINITY;
    }
    let s2 = sum * sum;
    let w2 = side * side;
    (w2 * mx / s2).max(s2 / (w2 * mn))
}

/// Tiende una fila sobre el lado corto del rect libre y devuelve el rect
/// libre restante.
fn layout_row(row: &[usize], areas: &[f64], free: Rect, out: &mut [Rect]) -> Rect {
    let sum: f64 = row.iter().map(|&i| areas[i]).sum();
    if sum <= 0.0 {
        return free;
    }
    if free.w >= free.h {
        // Columna a la izquierda; items apilados verticalmente.
        let col_w = (sum / free.h as f64) as f32;
        let mut y = free.y;
        for &i in row {
            let h = (areas[i] / sum * free.h as f64) as f32;
            out[i] = Rect::new(free.x, y, col_w, h);
            y += h;
        }
        Rect::new(free.x + col_w, free.y, free.w - col_w, free.h)
    } else {
        // Fila arriba; items lado a lado horizontalmente.
        let row_h = (sum / free.w as f64) as f32;
        let mut x = free.x;
        for &i in row {
            let w = (areas[i] / sum * free.w as f64) as f32;
            out[i] = Rect::new(x, free.y, w, row_h);
            x += w;
        }
        Rect::new(free.x, free.y + row_h, free.w, free.h - row_h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area_of(r: &Rect) -> f64 {
        r.w as f64 * r.h as f64
    }

    #[test]
    fn empty_input() {
        assert!(squarify(&[], Rect::new(0.0, 0.0, 100.0, 100.0)).is_empty());
    }

    #[test]
    fn single_item_fills_rect() {
        let rects = squarify(&[1.0], Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!(rects.len(), 1);
        assert!((area_of(&rects[0]) - 5000.0).abs() < 1.0);
    }

    #[test]
    fn areas_proportional_to_weights() {
        let area = Rect::new(0.0, 0.0, 200.0, 100.0);
        let rects = squarify(&[1.0, 1.0, 2.0], area);
        let total: f64 = rects.iter().map(area_of).sum();
        assert!((total - 20_000.0).abs() < 5.0, "área total ≈ rect");
        // El tercer item pesa el doble que cada uno de los otros.
        assert!((area_of(&rects[2]) - 2.0 * area_of(&rects[0])).abs() < 50.0);
    }

    #[test]
    fn zero_and_negative_weights_get_empty_rects() {
        let rects = squarify(&[1.0, 0.0, -3.0], Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(area_of(&rects[1]) == 0.0);
        assert!(area_of(&rects[2]) == 0.0);
        assert!((area_of(&rects[0]) - 10_000.0).abs() < 1.0);
    }

    #[test]
    fn all_rects_within_bounds() {
        let area = Rect::new(10.0, 20.0, 300.0, 200.0);
        let rects = squarify(&[5.0, 3.0, 8.0, 1.0, 2.0, 6.0], area);
        for r in &rects {
            assert!(r.x >= area.x - 0.01 && r.right() <= area.right() + 0.01);
            assert!(r.y >= area.y - 0.01 && r.bottom() <= area.bottom() + 0.01);
        }
    }
}
