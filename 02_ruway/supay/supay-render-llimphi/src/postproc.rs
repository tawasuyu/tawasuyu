//! Post-proceso del framebuffer clásico de doomgeneric (Fase 4-post).
//!
//! El renderer software de Doom (el de `View::image` en modo Framebuffer)
//! es **geométricamente correcto** — sin warping ni huecos. Pero es plano:
//! sin glow en las luces, colores algo apagados, bordes duros. Este módulo
//! le aplica un realce *screen-space* barato sobre el RGBA 640×400 antes de
//! subirlo a la GPU, para que se vea más vivo **sin tocar la geometría**:
//!
//! - **Grading**: saturación + contraste suaves (colores más ricos).
//! - **Bloom**: bright-pass + blur + add aditivo (las luces/lámparas brillan).
//! - **Vignette**: oscurecimiento radial sutil (profundidad).
//!
//! Es CPU puro sobre 256k píxeles, corre holgado a 35 Hz. No es el agua 3D
//! real (eso necesita el renderer wgpu 2.5D con info por superficie) — es la
//! ganancia visual rápida y segura sobre la base que ya se ve bien.
//!
//! `Enhance::OFF` es identidad bit-exacta: con él, el framebuffer sale igual
//! que el original de doomgeneric.

/// Parámetros del realce. Todo en fracciones; `OFF` no toca nada.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Enhance {
    /// 1.0 = sin cambio. >1 satura, <1 desatura.
    pub saturation: f32,
    /// 1.0 = sin cambio. >1 agranda el contraste alrededor de 0.5.
    pub contrast: f32,
    /// Luminancia (0..1) por encima de la cual un píxel "sangra" bloom.
    pub bloom_threshold: f32,
    /// Cuánto bloom se suma de vuelta (0 = sin bloom).
    pub bloom_strength: f32,
    /// Oscurecimiento de las esquinas (0 = sin viñeta, 1 = fuerte).
    pub vignette: f32,
}

impl Enhance {
    /// Identidad: deja el framebuffer idéntico al original.
    pub const OFF: Enhance = Enhance {
        saturation: 1.0,
        contrast: 1.0,
        bloom_threshold: 1.0,
        bloom_strength: 0.0,
        vignette: 0.0,
    };

    /// Preset por defecto: realce contenido pensado para verse "mejor, no
    /// distinto". Glow notorio en luces, color un poco más rico, viñeta
    /// apenas perceptible. Umbral de bloom bajo para que las lámparas,
    /// barras de hazard y proyectiles brillen sin oscurecer la escena.
    pub const RICH: Enhance = Enhance {
        saturation: 1.22,
        contrast: 1.05,
        bloom_threshold: 0.58,
        bloom_strength: 0.85,
        vignette: 0.14,
    };

    /// `true` si este preset no produce ningún cambio (identidad).
    pub fn is_identity(&self) -> bool {
        *self == Enhance::OFF
    }
}

impl Default for Enhance {
    fn default() -> Self {
        Enhance::RICH
    }
}

#[inline]
fn luma(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Aplica el realce in-place sobre un buffer RGBA8 `w*h*4`. Si `cfg` es la
/// identidad, retorna sin tocar nada (camino bit-exacto).
///
/// `protect_bottom` deja intactas las últimas N filas — el framebuffer de
/// Doom incluye la status bar abajo, y no queremos que la viñeta/grading la
/// oscurezcan. El bloom tampoco se siembra desde esa zona.
pub fn enhance_framebuffer(rgba: &mut [u8], w: usize, h: usize, cfg: &Enhance, protect_bottom: usize) {
    if cfg.is_identity() {
        return;
    }
    debug_assert_eq!(rgba.len(), w * h * 4);
    if rgba.len() != w * h * 4 {
        return;
    }
    // Sólo procesamos las filas 0..view_h (el área de juego); la status bar
    // (las últimas `protect_bottom` filas) queda intacta.
    let view_h = h.saturating_sub(protect_bottom).max(1);

    // 1. Bright-pass a un buffer downsampleado (1/4 en cada eje) para el
    //    bloom. Trabajamos en floats 0..1.
    let bw = (w / 4).max(1);
    let bh = (view_h / 4).max(1);
    let mut bright = vec![0.0_f32; bw * bh * 3];
    if cfg.bloom_strength > 0.0 {
        for by in 0..bh {
            for bx in 0..bw {
                // Promedio del bloque 4×4 correspondiente.
                let (mut ar, mut ag, mut ab) = (0.0_f32, 0.0_f32, 0.0_f32);
                let mut n = 0.0_f32;
                for dy in 0..4 {
                    for dx in 0..4 {
                        let x = bx * 4 + dx;
                        let y = by * 4 + dy;
                        if x >= w || y >= view_h {
                            continue;
                        }
                        let o = (y * w + x) * 4;
                        ar += rgba[o] as f32 / 255.0;
                        ag += rgba[o + 1] as f32 / 255.0;
                        ab += rgba[o + 2] as f32 / 255.0;
                        n += 1.0;
                    }
                }
                if n > 0.0 {
                    ar /= n;
                    ag /= n;
                    ab /= n;
                }
                let l = luma(ar, ag, ab);
                // Bright-pass suave: lo que supera el umbral, normalizado.
                let t = cfg.bloom_threshold;
                let k = ((l - t) / (1.0 - t).max(1e-3)).clamp(0.0, 1.0);
                let bo = (by * bw + bx) * 3;
                bright[bo] = ar * k;
                bright[bo + 1] = ag * k;
                bright[bo + 2] = ab * k;
            }
        }
        // 2. Blur separable (box) de 2 pasadas sobre el bright buffer.
        box_blur_rgb(&mut bright, bw, bh, 2, 2);
    }

    // 3. Grading + bloom add + vignette, por píxel — sólo el área de juego
    //    (la viñeta se centra en el área de juego, no en la status bar).
    let cx = (w as f32 - 1.0) * 0.5;
    let cy = (view_h as f32 - 1.0) * 0.5;
    let max_r2 = cx * cx + cy * cy;
    for y in 0..view_h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            let mut r = rgba[o] as f32 / 255.0;
            let mut g = rgba[o + 1] as f32 / 255.0;
            let mut b = rgba[o + 2] as f32 / 255.0;

            // Saturación: lerp entre luma gris y color.
            if cfg.saturation != 1.0 {
                let l = luma(r, g, b);
                r = l + (r - l) * cfg.saturation;
                g = l + (g - l) * cfg.saturation;
                b = l + (b - l) * cfg.saturation;
            }
            // Contraste alrededor de 0.5.
            if cfg.contrast != 1.0 {
                r = (r - 0.5) * cfg.contrast + 0.5;
                g = (g - 0.5) * cfg.contrast + 0.5;
                b = (b - 0.5) * cfg.contrast + 0.5;
            }
            // Bloom aditivo (muestra bilineal simple del bright buffer).
            if cfg.bloom_strength > 0.0 {
                let (br, bg, bb) = sample_rgb_bilinear(&bright, bw, bh, x, y, w, view_h);
                r += br * cfg.bloom_strength;
                g += bg * cfg.bloom_strength;
                b += bb * cfg.bloom_strength;
            }
            // Vignette radial.
            if cfg.vignette > 0.0 {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let rr = (dx * dx + dy * dy) / max_r2;
                let v = 1.0 - cfg.vignette * rr * rr;
                r *= v;
                g *= v;
                b *= v;
            }

            rgba[o] = (r.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            rgba[o + 1] = (g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            rgba[o + 2] = (b.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            // alpha intacto.
        }
    }
}

/// Box-blur separable in-place sobre un buffer RGB float `w*h*3`.
fn box_blur_rgb(buf: &mut [f32], w: usize, h: usize, radius: usize, passes: usize) {
    if radius == 0 || w == 0 || h == 0 {
        return;
    }
    let mut tmp = vec![0.0_f32; buf.len()];
    for _ in 0..passes {
        // Horizontal.
        for y in 0..h {
            for x in 0..w {
                let (mut ar, mut ag, mut ab) = (0.0_f32, 0.0_f32, 0.0_f32);
                let mut n = 0.0_f32;
                let lo = x.saturating_sub(radius);
                let hi = (x + radius).min(w - 1);
                for xx in lo..=hi {
                    let o = (y * w + xx) * 3;
                    ar += buf[o];
                    ag += buf[o + 1];
                    ab += buf[o + 2];
                    n += 1.0;
                }
                let o = (y * w + x) * 3;
                tmp[o] = ar / n;
                tmp[o + 1] = ag / n;
                tmp[o + 2] = ab / n;
            }
        }
        // Vertical.
        for y in 0..h {
            for x in 0..w {
                let (mut ar, mut ag, mut ab) = (0.0_f32, 0.0_f32, 0.0_f32);
                let mut n = 0.0_f32;
                let lo = y.saturating_sub(radius);
                let hi = (y + radius).min(h - 1);
                for yy in lo..=hi {
                    let o = (yy * w + x) * 3;
                    ar += tmp[o];
                    ag += tmp[o + 1];
                    ab += tmp[o + 2];
                    n += 1.0;
                }
                let o = (y * w + x) * 3;
                buf[o] = ar / n;
                buf[o + 1] = ag / n;
                buf[o + 2] = ab / n;
            }
        }
    }
}

/// Muestra bilineal del bright buffer downsampleado en la coord full-res
/// `(x, y)`. Mapea full-res → bright-res y lerpea los 4 vecinos.
#[inline]
fn sample_rgb_bilinear(
    buf: &[f32],
    bw: usize,
    bh: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
) -> (f32, f32, f32) {
    let fx = (x as f32 + 0.5) * bw as f32 / w as f32 - 0.5;
    let fy = (y as f32 + 0.5) * bh as f32 / h as f32 - 0.5;
    let x0 = fx.floor().max(0.0) as usize;
    let y0 = fy.floor().max(0.0) as usize;
    let x1 = (x0 + 1).min(bw - 1);
    let y1 = (y0 + 1).min(bh - 1);
    let tx = (fx - x0 as f32).clamp(0.0, 1.0);
    let ty = (fy - y0 as f32).clamp(0.0, 1.0);
    let g = |xi: usize, yi: usize, c: usize| buf[(yi * bw + xi) * 3 + c];
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let mut out = [0.0_f32; 3];
    for c in 0..3 {
        let top = lerp(g(x0, y0, c), g(x1, y0, c), tx);
        let bot = lerp(g(x0, y1, c), g(x1, y1, c), tx);
        out[c] = lerp(top, bot, ty);
    }
    (out[0], out[1], out[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, rgb: [u8; 3]) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        for px in v.chunks_exact_mut(4) {
            px[0] = rgb[0];
            px[1] = rgb[1];
            px[2] = rgb[2];
            px[3] = 255;
        }
        v
    }

    #[test]
    fn off_es_identidad_bit_exacta() {
        let mut buf = solid(64, 40, [120, 80, 40]);
        let orig = buf.clone();
        enhance_framebuffer(&mut buf, 64, 40, &Enhance::OFF, 0);
        assert_eq!(buf, orig, "Enhance::OFF debe dejar el buffer idéntico");
    }

    #[test]
    fn vignette_oscurece_las_esquinas_no_el_centro() {
        let w = 80;
        let h = 60;
        let cfg = Enhance {
            vignette: 0.5,
            ..Enhance::OFF
        };
        let mut buf = solid(w, h, [200, 200, 200]);
        enhance_framebuffer(&mut buf, w, h, &cfg, 0);
        let center = buf[((h / 2) * w + w / 2) * 4];
        let corner = buf[0];
        assert!(
            corner < center,
            "la esquina ({corner}) debe quedar más oscura que el centro ({center})"
        );
        // El centro casi no se toca (vignette ∝ r⁴, ~0 en el medio).
        assert!(center >= 198, "el centro no debería oscurecerse: {center}");
    }

    #[test]
    fn bloom_agrega_brillo_alrededor_de_una_luz() {
        // Fondo oscuro con un cuadro brillante al centro. Tras el bloom, los
        // píxeles vecinos al brillo deben subir su luminancia.
        let w = 64;
        let h = 64;
        let mut buf = solid(w, h, [10, 10, 10]);
        for y in 28..36 {
            for x in 28..36 {
                let o = (y * w + x) * 4;
                buf[o] = 255;
                buf[o + 1] = 255;
                buf[o + 2] = 255;
            }
        }
        let before = buf.clone();
        let cfg = Enhance {
            bloom_threshold: 0.5,
            bloom_strength: 0.8,
            ..Enhance::OFF
        };
        enhance_framebuffer(&mut buf, w, h, &cfg, 0);
        // Un píxel a ~6 de la luz: antes era fondo (10), ahora debe brillar más.
        let probe = (42 * w + 42) * 4;
        assert!(
            buf[probe] > before[probe],
            "el bloom debe iluminar el vecindario: {} → {}",
            before[probe],
            buf[probe]
        );
    }

    #[test]
    fn saturacion_separa_los_canales() {
        // Un color con canales distintos se separa más al saturar.
        let w = 16;
        let h = 16;
        let cfg = Enhance {
            saturation: 1.5,
            ..Enhance::OFF
        };
        let mut buf = solid(w, h, [150, 100, 50]);
        enhance_framebuffer(&mut buf, w, h, &cfg, 0);
        let o = 0;
        let (r, b) = (buf[o] as i32, buf[o + 2] as i32);
        // El rango max-min debe crecer respecto al original (100).
        assert!((r - b) > 100, "saturar debe ampliar el spread: r={r} b={b}");
    }
}
