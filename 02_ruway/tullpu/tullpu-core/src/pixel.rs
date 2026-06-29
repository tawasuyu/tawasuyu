//! `tullpu_core::pixel` — math per-píxel pura, compartida entre el catálogo de
//! ops (`tullpu-ops`, que produce buffers cacheados) y el compositor
//! (`tullpu-render`, que aplica capas de ajuste **en vivo** al componer).
//!
//! Vive en `tullpu-core` —del que dependen ambos— para evitar el ciclo
//! `ops → render` ↔ `render → ops`. Son funciones puras sobre Rgba8 plano,
//! sin `image` ni dependencias gráficas: sólo los mapeos por-canal y HSL y la
//! interpolación de curvas. Las ops **espaciales** (blur, espejar) no viven
//! acá: necesitan vecindad/geometría y se quedan en `tullpu-ops`.

use crate::OpLocal;

#[inline]
fn clamp_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Aplica un ajuste **per-píxel** sobre los canales RGB de un buffer Rgba8
/// plano, *in situ*, dejando el alfa intacto. Devuelve `true` si la op es
/// per-píxel (y por lo tanto válida como capa de ajuste) y `false` si es
/// espacial o de alfa (`Blur`, `EspejarHorizontal`, `EspejarVertical`,
/// `Opacidad`) — esas no se modelan como capas de ajuste de composición y el
/// compositor las ignora.
///
/// Es la primitiva detrás de `ClaseCapa::Ajuste(op)`: el compositor copia el
/// compuesto-hasta-aquí, le aplica este ajuste y mezcla por opacidad/máscara.
pub fn ajustar_rgb_inplace(op: &OpLocal, buf: &mut [u8]) -> bool {
    match op {
        OpLocal::Invertir => {
            mapear_rgb(buf, |c| 255 - c);
            true
        }
        OpLocal::Brillo { delta } => {
            mapear_rgb_f(buf, |c| c + *delta);
            true
        }
        OpLocal::Contraste { factor } => {
            mapear_rgb_f(buf, |c| (c - 0.5) * *factor + 0.5);
            true
        }
        OpLocal::Niveles {
            entrada_min,
            entrada_max,
            gamma,
        } => {
            let min = *entrada_min;
            let max = *entrada_max;
            let inv_g = if *gamma > f32::EPSILON { 1.0 / *gamma } else { 1.0 };
            let rango = (max - min).max(1e-6);
            mapear_rgb_f(buf, |c| ((c - min) / rango).clamp(0.0, 1.0).powf(inv_g));
            true
        }
        OpLocal::Saturacion { factor } => {
            mapear_hsl(buf, |h, s, l| (h, s * *factor, l));
            true
        }
        OpLocal::Tonalidad { grados } => {
            let delta = grados / 360.0;
            mapear_hsl(buf, |h, s, l| ((h + delta).rem_euclid(1.0), s, l));
            true
        }
        OpLocal::Curvas { puntos } => {
            let lut = lut_curva(puntos);
            mapear_rgb(buf, |c| lut[c as usize]);
            true
        }
        // Espaciales / alfa: no son ajustes de composición.
        OpLocal::Blur { .. }
        | OpLocal::EspejarHorizontal
        | OpLocal::EspejarVertical
        | OpLocal::Opacidad { .. } => false,
    }
}

fn mapear_rgb<F: Fn(u8) -> u8>(buf: &mut [u8], f: F) {
    for px in buf.chunks_exact_mut(4) {
        px[0] = f(px[0]);
        px[1] = f(px[1]);
        px[2] = f(px[2]);
    }
}

fn mapear_rgb_f<F: Fn(f32) -> f32>(buf: &mut [u8], f: F) {
    for px in buf.chunks_exact_mut(4) {
        px[0] = clamp_u8(f(px[0] as f32 / 255.0));
        px[1] = clamp_u8(f(px[1] as f32 / 255.0));
        px[2] = clamp_u8(f(px[2] as f32 / 255.0));
    }
}

fn mapear_hsl<F: Fn(f32, f32, f32) -> (f32, f32, f32)>(buf: &mut [u8], f: F) {
    for px in buf.chunks_exact_mut(4) {
        let (h, s, l) = rgb_a_hsl(px[0], px[1], px[2]);
        let (h, s, l) = f(h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
        let (r, g, b) = hsl_a_rgb(h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
        px[0] = r;
        px[1] = g;
        px[2] = b;
    }
}

fn rgb_a_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-6 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l < 0.5 {
        d / (max + min)
    } else {
        d / (2.0 - max - min)
    };
    let h = if (max - r).abs() < 1e-6 {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < 1e-6 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_a_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    if s.abs() < 1e-6 {
        let v = clamp_u8(l);
        return (v, v, v);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let r = hue_a_rgb(p, q, h + 1.0 / 3.0);
    let g = hue_a_rgb(p, q, h);
    let b = hue_a_rgb(p, q, h - 1.0 / 3.0);
    (clamp_u8(r), clamp_u8(g), clamp_u8(b))
}

fn hue_a_rgb(p: f32, q: f32, t: f32) -> f32 {
    let t = if t < 0.0 {
        t + 1.0
    } else if t > 1.0 {
        t - 1.0
    } else {
        t
    };
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

/// Construye la LUT de 256 entradas de una curva tonal a partir de sus puntos
/// de control `(x_entrada, y_salida)` en `[0,1]²`. Ordena por `x`, clampa a
/// `[0,1]`, deduplica `x` colapsados, y con < 2 puntos cae a identidad. Con
/// ≥ 2 puntos interpola por Hermite cúbica con tangentes monótonas de
/// **Fritsch–Carlson** (sin overshoot). Fuera del dominio cubierto, aplana al
/// extremo más cercano (clamp, como el panel Curves de Photoshop).
///
/// Vive acá (antes en `tullpu-ops`) para que la comparta el compositor cuando
/// una capa de ajuste de curvas se aplica en vivo. `tullpu-ops` la re-exporta.
pub fn lut_curva(puntos: &[(f32, f32)]) -> [u8; 256] {
    let mut pts: Vec<(f32, f32)> = puntos
        .iter()
        .map(|&(x, y)| (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
        .collect();
    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);

    let mut lut = [0u8; 256];
    if pts.len() < 2 {
        for (i, slot) in lut.iter_mut().enumerate() {
            *slot = i as u8;
        }
        return lut;
    }

    let n = pts.len();
    let xs: Vec<f32> = pts.iter().map(|p| p.0).collect();
    let ys: Vec<f32> = pts.iter().map(|p| p.1).collect();

    let mut d = vec![0.0f32; n - 1];
    for k in 0..n - 1 {
        let h = (xs[k + 1] - xs[k]).max(1e-6);
        d[k] = (ys[k + 1] - ys[k]) / h;
    }
    let mut m = vec![0.0f32; n];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for k in 1..n - 1 {
        m[k] = (d[k - 1] + d[k]) / 2.0;
    }
    for k in 0..n - 1 {
        if d[k].abs() < 1e-12 {
            m[k] = 0.0;
            m[k + 1] = 0.0;
        } else {
            let alpha = m[k] / d[k];
            let beta = m[k + 1] / d[k];
            let s = alpha * alpha + beta * beta;
            if s > 9.0 {
                let tau = 3.0 / s.sqrt();
                m[k] = tau * alpha * d[k];
                m[k + 1] = tau * beta * d[k];
            }
        }
    }

    for (i, slot) in lut.iter_mut().enumerate() {
        let x = i as f32 / 255.0;
        let y = if x <= xs[0] {
            ys[0]
        } else if x >= xs[n - 1] {
            ys[n - 1]
        } else {
            let mut k = 0;
            while k < n - 1 && x > xs[k + 1] {
                k += 1;
            }
            let h = (xs[k + 1] - xs[k]).max(1e-6);
            let t = ((x - xs[k]) / h).clamp(0.0, 1.0);
            let t2 = t * t;
            let t3 = t2 * t;
            let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
            let h10 = t3 - 2.0 * t2 + t;
            let h01 = -2.0 * t3 + 3.0 * t2;
            let h11 = t3 - t2;
            h00 * ys[k] + h10 * h * m[k] + h01 * ys[k + 1] + h11 * h * m[k + 1]
        };
        *slot = clamp_u8(y);
    }
    lut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lut_identidad_con_un_punto() {
        let lut = lut_curva(&[(0.5, 0.5)]);
        for i in 0..256 {
            assert_eq!(lut[i], i as u8);
        }
    }

    #[test]
    fn lut_diagonal_es_identidad() {
        let lut = lut_curva(&[(0.0, 0.0), (1.0, 1.0)]);
        for i in 0..256 {
            assert!((lut[i] as i32 - i as i32).abs() <= 1);
        }
    }

    #[test]
    fn invertir_inplace_da_complemento() {
        let mut buf = vec![10, 20, 30, 255, 0, 0, 0, 128];
        assert!(ajustar_rgb_inplace(&OpLocal::Invertir, &mut buf));
        assert_eq!(buf, vec![245, 235, 225, 255, 255, 255, 255, 128]);
    }

    #[test]
    fn blur_no_es_ajuste() {
        let mut buf = vec![0u8; 16];
        assert!(!ajustar_rgb_inplace(&OpLocal::Blur { radio: 2.0 }, &mut buf));
    }

    #[test]
    fn saturacion_cero_da_gris() {
        let mut buf = vec![200, 50, 50, 255];
        assert!(ajustar_rgb_inplace(&OpLocal::Saturacion { factor: 0.0 }, &mut buf));
        // Los tres canales deben quedar casi iguales (desaturado).
        let d = (buf[0] as i32 - buf[1] as i32).abs().max((buf[1] as i32 - buf[2] as i32).abs());
        assert!(d <= 2, "esperaba gris, obtuve {:?}", &buf[0..3]);
        assert_eq!(buf[3], 255, "alfa intacto");
    }
}
