//! `pixel-verbo-mock` — proveedor de píxeles determinista, sin modelo
//! real, sin pesos descargables, sin GPU.
//!
//! Mismo input → mismo output, siempre. El propósito es desbloquear
//! consumidores (tullpu-ops, la app de escritorio, tests de integración
//! de `pixel-verbo-daemon`) antes de que existan modelos serios cargados.
//! Las cuatro ops del catálogo producen artefactos visualmente
//! reconocibles para que un humano vea que "el camino IA está cableado"
//! aunque ningún modelo de verdad esté corriendo.
//!
//! Ningún efecto pretende ser plausible — son marcadores. Específicamente:
//!
//! - `Segmentar` — máscara circular en el centro del lienzo, alfa 255
//!   dentro, 0 fuera; el RGB se preserva. Sirve para validar
//!   propagación stale → fresca y rendering de capas con alfa.
//! - `Inpaint` — donde el alfa de entrada es 0, escribe un color medio
//!   de la imagen; donde es 255, copia. Comprueba el path "máscara
//!   externa modula el output".
//! - `Restyle` — shift de matiz HSL por hash(prompt). Es el que mejor
//!   evidencia la diferencia "prompt distinto, output distinto".
//! - `Generar` — gradiente determinista del `(ancho, alto)` pedido,
//!   sembrado por hash(prompt). Ignora la entrada.

#![forbid(unsafe_code)]

use pixel_verbo_core::{Error, Imagen, ModelId, OpPixel, Proveedor};

/// Proveedor mock — instanciable sin parámetros.
#[derive(Debug, Clone)]
pub struct ProveedorMock {
    model: ModelId,
}

impl ProveedorMock {
    pub fn nuevo() -> Self {
        Self {
            model: ModelId::new("pixel-verbo-mock-v0"),
        }
    }
}

impl Default for ProveedorMock {
    fn default() -> Self {
        Self::nuevo()
    }
}

impl Proveedor for ProveedorMock {
    fn model_id(&self) -> &ModelId {
        &self.model
    }

    fn aplicar(&self, op: &OpPixel, entrada: Option<Imagen>) -> Result<Imagen, Error> {
        match op {
            OpPixel::Segmentar { prompt: _ } => {
                let img = entrada.ok_or(Error::EntradaFaltante)?;
                Ok(segmentar_circular(img))
            }
            OpPixel::Inpaint { prompt: _ } => {
                let img = entrada.ok_or(Error::EntradaFaltante)?;
                Ok(inpaint_promedio(img))
            }
            OpPixel::Restyle { prompt } => {
                let img = entrada.ok_or(Error::EntradaFaltante)?;
                Ok(restyle_hue_shift(img, prompt))
            }
            OpPixel::Generar { prompt, ancho, alto } => {
                Ok(generar_gradiente(*ancho, *alto, prompt))
            }
        }
    }
}

// =============================================================================
//  Implementaciones de cada op
// =============================================================================

fn segmentar_circular(mut img: Imagen) -> Imagen {
    let w = img.ancho as i32;
    let h = img.alto as i32;
    let cx = w / 2;
    let cy = h / 2;
    let r = w.min(h) / 3;
    let r2 = (r * r) as i64;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let dx = x - cx;
            let dy = y - cy;
            let d2 = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
            if d2 > r2 {
                img.bytes[i + 3] = 0;
            } else {
                img.bytes[i + 3] = 255;
            }
        }
    }
    img
}

fn inpaint_promedio(mut img: Imagen) -> Imagen {
    let mut suma = [0u64; 3];
    let mut n: u64 = 0;
    for px in img.bytes.chunks_exact(4) {
        if px[3] > 0 {
            suma[0] += px[0] as u64;
            suma[1] += px[1] as u64;
            suma[2] += px[2] as u64;
            n += 1;
        }
    }
    if n == 0 {
        return img;
    }
    let medio = [
        (suma[0] / n) as u8,
        (suma[1] / n) as u8,
        (suma[2] / n) as u8,
    ];
    for px in img.bytes.chunks_exact_mut(4) {
        if px[3] == 0 {
            px[0] = medio[0];
            px[1] = medio[1];
            px[2] = medio[2];
            px[3] = 255;
        }
    }
    img
}

fn restyle_hue_shift(mut img: Imagen, prompt: &str) -> Imagen {
    let h = fnv1a(prompt) as f32;
    let shift = (h / u64::MAX as f32) % 1.0;
    for px in img.bytes.chunks_exact_mut(4) {
        let (hh, s, ll) = rgb_a_hsl(px[0], px[1], px[2]);
        let hh = (hh + shift).rem_euclid(1.0);
        let (r, g, b) = hsl_a_rgb(hh, s, ll);
        px[0] = r;
        px[1] = g;
        px[2] = b;
    }
    img
}

fn generar_gradiente(ancho: u32, alto: u32, prompt: &str) -> Imagen {
    let seed = fnv1a(prompt);
    let r_base = ((seed >> 0) & 0xFF) as u8;
    let g_base = ((seed >> 8) & 0xFF) as u8;
    let b_base = ((seed >> 16) & 0xFF) as u8;
    let mut bytes = Vec::with_capacity((ancho * alto * 4) as usize);
    for y in 0..alto {
        for x in 0..ancho {
            let fx = if ancho > 1 { x as f32 / (ancho - 1) as f32 } else { 0.0 };
            let fy = if alto > 1 { y as f32 / (alto - 1) as f32 } else { 0.0 };
            let r = ((r_base as f32) * (1.0 - fx) + 255.0 * fx).clamp(0.0, 255.0) as u8;
            let g = ((g_base as f32) * (1.0 - fy) + 255.0 * fy).clamp(0.0, 255.0) as u8;
            let mix = ((fx + fy) * 0.5).clamp(0.0, 1.0);
            let b = ((b_base as f32) * (1.0 - mix) + 255.0 * mix).clamp(0.0, 255.0) as u8;
            bytes.extend_from_slice(&[r, g, b, 255]);
        }
    }
    Imagen {
        ancho,
        alto,
        bytes,
    }
}

// =============================================================================
//  Helpers
// =============================================================================

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
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
        let v = (l.clamp(0.0, 1.0) * 255.0).round() as u8;
        return (v, v, v);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let r = hue_a_rgb(p, q, h + 1.0 / 3.0);
    let g = hue_a_rgb(p, q, h);
    let b = hue_a_rgb(p, q, h - 1.0 / 3.0);
    (cl(r), cl(g), cl(b))
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

fn cl(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

// =============================================================================
//  Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn solido(w: u32, h: u32, color: [u8; 4]) -> Imagen {
        let mut bytes = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            bytes.extend_from_slice(&color);
        }
        Imagen::nueva(w, h, bytes).unwrap()
    }

    #[test]
    fn model_id_estable() {
        let p = ProveedorMock::nuevo();
        assert_eq!(p.model_id().name, "pixel-verbo-mock-v0");
    }

    #[test]
    fn segmentar_pone_alfa_circular() {
        let p = ProveedorMock::nuevo();
        let img = solido(12, 12, [100, 100, 100, 255]);
        let out = p
            .aplicar(&OpPixel::Segmentar { prompt: None }, Some(img))
            .unwrap();
        // El centro queda visible.
        let centro = &out.bytes[((6 * 12 + 6) * 4) as usize..((6 * 12 + 6) * 4 + 4) as usize];
        assert_eq!(centro[3], 255);
        // Una esquina queda transparente.
        let esquina = &out.bytes[0..4];
        assert_eq!(esquina[3], 0);
    }

    #[test]
    fn inpaint_rellena_con_promedio() {
        let p = ProveedorMock::nuevo();
        // 2 píxeles visibles (rojo) + 2 transparentes.
        let bytes = vec![
            255, 0, 0, 255, // rojo visible
            255, 0, 0, 255, // rojo visible
            0, 0, 0, 0, // hueco
            0, 0, 0, 0, // hueco
        ];
        let img = Imagen::nueva(4, 1, bytes).unwrap();
        let out = p.aplicar(&OpPixel::Inpaint { prompt: None }, Some(img)).unwrap();
        // Los huecos quedan pintados con el promedio (rojo).
        let p3 = &out.bytes[12..16];
        assert_eq!(p3, &[255, 0, 0, 255]);
    }

    #[test]
    fn restyle_es_determinista_por_prompt() {
        let p = ProveedorMock::nuevo();
        let img = solido(2, 2, [120, 80, 200, 255]);
        let a = p
            .aplicar(
                &OpPixel::Restyle {
                    prompt: "tropical".into(),
                },
                Some(img.clone()),
            )
            .unwrap();
        let b = p
            .aplicar(
                &OpPixel::Restyle {
                    prompt: "tropical".into(),
                },
                Some(img.clone()),
            )
            .unwrap();
        assert_eq!(a.bytes, b.bytes);
        let c = p
            .aplicar(
                &OpPixel::Restyle {
                    prompt: "frío".into(),
                },
                Some(img.clone()),
            )
            .unwrap();
        assert_ne!(a.bytes, c.bytes);
    }

    #[test]
    fn generar_ignora_entrada_y_arma_imagen() {
        let p = ProveedorMock::nuevo();
        let out = p
            .aplicar(
                &OpPixel::Generar {
                    prompt: "atardecer".into(),
                    ancho: 16,
                    alto: 9,
                },
                None,
            )
            .unwrap();
        assert_eq!(out.ancho, 16);
        assert_eq!(out.alto, 9);
        assert_eq!(out.bytes.len(), 16 * 9 * 4);
    }

    #[test]
    fn entrada_faltante_es_error_explicito() {
        let p = ProveedorMock::nuevo();
        let err = p
            .aplicar(&OpPixel::Segmentar { prompt: None }, None)
            .unwrap_err();
        assert!(matches!(err, Error::EntradaFaltante));
    }
}
