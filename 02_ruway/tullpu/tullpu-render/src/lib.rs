//! `tullpu-render` — compositor CPU top-down del editor de capas.
//!
//! Recorre las capas del [`tullpu_core::Lienzo`] en orden visual (índice 0
//! = fondo) y funde cada una sobre el buffer acumulado aplicando su
//! [`tullpu_core::ModoFusion`], opacidad y máscara opcional. La aritmética
//! corre en `f32` normalizado `[0,1]` y se redondea a `Rgba8` al final.
//!
//! El compositor **no decide** de dónde salen los buffers: el caller pasa una
//! implementación de [`FuenteBuffers`] que resuelve `Hash → bytes`. En el
//! mundo real esa fuente es el almacén content-addressed; aquí en `dev` y en
//! tests basta con [`AlmacenEnMemoria`].
//!
//! ## Forma de los buffers
//!
//! - Contenido de una capa: `W * H * 4` bytes Rgba8, fila por fila, no
//!   premultiplicado.
//! - Máscara: `W * H` bytes, un byte de alfa por píxel. `0` oculta, `255`
//!   muestra. Se multiplica al alfa del píxel **antes** de la fusión.
//!
//! ## Lo que NO hace
//!
//! - No regenera capas *stale*. Si una capa derivada está stale, el
//!   compositor usa su `contenido` cacheado tal cual y la pinta — pintar un
//!   buffer obsoleto es problema de la capa de ops/UI, que decide cuándo
//!   invocar el daemon o la op local. Ver `tullpu-ops`.
//! - No corre en GPU. Es CPU puro sobre `image::RgbaImage`; migrar a un
//!   compute shader queda detrás de este mismo API.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use tullpu_core::{pixel, Capa, ClaseCapa, Hash, Lienzo, ModoFusion};

// =============================================================================
//  Fuente de buffers
// =============================================================================

/// Resolución `Hash → bytes` que el compositor consume. La implementación
/// real es el almacén de wawa; aquí abstraemos para testear sin disco.
pub trait FuenteBuffers {
    fn obtener(&self, hash: Hash) -> Option<&[u8]>;
}

/// Almacén trivial en memoria. Útil para tests, demos y la app de escritorio
/// antes de cablear el almacén real.
#[derive(Default, Debug, Clone)]
pub struct AlmacenEnMemoria {
    pub buffers: HashMap<Hash, Vec<u8>>,
}

impl AlmacenEnMemoria {
    pub fn nuevo() -> Self {
        Self::default()
    }

    /// Inserta un buffer y devuelve el hash que lo identifica. Útil para
    /// "tengo este Vec<u8>, dame su hash y guárdalo".
    pub fn insertar(&mut self, bytes: Vec<u8>) -> Hash {
        let hash = tullpu_core::hash_bytes(&bytes);
        self.buffers.insert(hash, bytes);
        hash
    }
}

impl FuenteBuffers for AlmacenEnMemoria {
    fn obtener(&self, hash: Hash) -> Option<&[u8]> {
        self.buffers.get(&hash).map(|v| v.as_slice())
    }
}

// =============================================================================
//  Errores
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("buffer faltante: hash {0:02x?}")]
    BufferFaltante(Hash),
    #[error("tamaño de buffer Rgba8 inválido para {hash:02x?}: esperaba {esperado}, encontré {encontrado}")]
    TamanioRgba {
        hash: Hash,
        esperado: usize,
        encontrado: usize,
    },
    #[error("tamaño de máscara inválido para {hash:02x?}: esperaba {esperado}, encontré {encontrado}")]
    TamanioMascara {
        hash: Hash,
        esperado: usize,
        encontrado: usize,
    },
    #[error("guardar imagen falló: {0}")]
    Imagen(#[from] image::ImageError),
    #[error("io export: {0}")]
    Io(#[from] std::io::Error),
}

// =============================================================================
//  Composición
// =============================================================================

/// Compone un [`Lienzo`] sobre un buffer Rgba8 nuevo, transparente como base.
/// Recorre la jerarquía de capas (carpetas/grupos anidados) en orden visual
/// (fondo→tope), funde cada capa con su modo/opacidad/máscara, aplica las
/// **capas de ajuste** en vivo al compuesto inferior y respeta las **clipping
/// masks**. Devuelve una `RgbaImage` del tamaño del lienzo.
///
/// Para un lienzo plano (sólo capas `Pixeles` en la raíz, sin grupos, ajustes
/// ni clipping) el resultado es idéntico bit-a-bit al compositor anterior.
pub fn componer(l: &Lienzo, fuente: &impl FuenteBuffers) -> Result<RgbaImage, Error> {
    let w = l.width;
    let h = l.height;
    let acc = componer_lista(l, None, fuente)?;
    Ok(RgbaImage::from_raw(w, h, acc).expect("dimensiones cuadran con el buffer"))
}

/// Compone las capas hijas directas de `grupo` (`None` = raíz) sobre un buffer
/// transparente y lo devuelve como Rgba8 plano (alfa recta). Recursa en cada
/// capa-grupo. Es la unidad de "aislamiento" de un grupo de Photoshop: los
/// hijos se funden entre sí en su propio lienzo antes de que el padre aplique
/// el blend/opacidad/máscara del grupo.
fn componer_lista(
    l: &Lienzo,
    grupo: Option<tullpu_core::Uuid>,
    fuente: &impl FuenteBuffers,
) -> Result<Vec<u8>, Error> {
    let w = l.width;
    let h = l.height;
    let n = (w as usize) * (h as usize);
    let mut acc = vec![0u8; n * 4];

    // Cobertura (alfa efectiva) de la última capa de base no-clipping. Las
    // capas con `clipping` se recortan a ella; no la actualizan.
    let mut base_alpha: Option<Vec<f32>> = None;

    for i in l.hijos_directos(grupo) {
        let capa = &l.capas[i];
        if !capa.visible {
            continue;
        }
        let mascara = cargar_mascara(capa, n, fuente)?;
        let clip = if capa.clipping {
            base_alpha.as_deref()
        } else {
            None
        };

        match &capa.clase {
            ClaseCapa::Ajuste(op) => {
                aplicar_ajuste(&mut acc, n, op, capa.opacidad, mascara.as_deref(), clip);
                // Un ajuste no aporta base de clipping.
            }
            ClaseCapa::Grupo => {
                let sub = componer_lista(l, Some(capa.id), fuente)?;
                let cobertura =
                    fundir_buffer(&mut acc, n, &sub, capa, mascara.as_deref(), clip);
                if !capa.clipping {
                    base_alpha = Some(cobertura);
                }
            }
            // Texto y vector se componen igual que píxeles: su `contenido` ya
            // es el buffer rasterizado.
            ClaseCapa::Pixeles | ClaseCapa::Texto(_) | ClaseCapa::Vector(_) => {
                let esperado_rgba = n * 4;
                let src = fuente
                    .obtener(capa.contenido)
                    .ok_or(Error::BufferFaltante(capa.contenido))?;
                if src.len() != esperado_rgba {
                    return Err(Error::TamanioRgba {
                        hash: capa.contenido,
                        esperado: esperado_rgba,
                        encontrado: src.len(),
                    });
                }
                // `fundir_buffer` toma `&[u8]`; clonamos para soltar el borrow
                // inmutable de `fuente` (la máscara ya se resolvió arriba).
                let src = src.to_vec();
                let cobertura =
                    fundir_buffer(&mut acc, n, &src, capa, mascara.as_deref(), clip);
                if !capa.clipping {
                    base_alpha = Some(cobertura);
                }
            }
        }
    }

    Ok(acc)
}

/// Resuelve y valida la máscara de una capa, si tiene. `W*H` bytes de alfa.
fn cargar_mascara(
    capa: &Capa,
    n: usize,
    fuente: &impl FuenteBuffers,
) -> Result<Option<Vec<u8>>, Error> {
    match capa.mascara {
        Some(hm) => {
            let bytes = fuente.obtener(hm).ok_or(Error::BufferFaltante(hm))?;
            if bytes.len() != n {
                return Err(Error::TamanioMascara {
                    hash: hm,
                    esperado: n,
                    encontrado: bytes.len(),
                });
            }
            Ok(Some(bytes.to_vec()))
        }
        None => Ok(None),
    }
}

/// Funde un buffer Rgba8 `src` (ya validado a `n*4` bytes) sobre `acc`
/// aplicando el modo/opacidad de `capa`, la `mascara` opcional y el recorte
/// `clip` opcional (alfa de la capa base para clipping masks). Devuelve la
/// **cobertura** por píxel (alfa efectiva aplicada), que sirve de base de
/// clipping para las capas siguientes.
fn fundir_buffer(
    acc: &mut [u8],
    n: usize,
    src: &[u8],
    capa: &Capa,
    mascara: Option<&[u8]>,
    clip: Option<&[f32]>,
) -> Vec<f32> {
    let opacidad_global = capa.opacidad.clamp(0.0, 1.0);
    let modo = capa.blend;

    if matches!(modo, ModoFusion::Disolver) {
        return fundir_disolver(acc, n, src, mascara, opacidad_global, clip, capa);
    }

    let mut cobertura = vec![0.0f32; n];
    for i in 0..n {
        let s_idx = i * 4;
        let sr = src[s_idx] as f32 / 255.0;
        let sg = src[s_idx + 1] as f32 / 255.0;
        let sb = src[s_idx + 2] as f32 / 255.0;
        let sa = src[s_idx + 3] as f32 / 255.0;

        let m = mascara.map(|m| m[i] as f32 / 255.0).unwrap_or(1.0);
        let c = clip.map(|c| c[i]).unwrap_or(1.0);
        let src_alpha = sa * opacidad_global * m * c;
        cobertura[i] = src_alpha;

        let dr = acc[s_idx] as f32 / 255.0;
        let dg = acc[s_idx + 1] as f32 / 255.0;
        let db = acc[s_idx + 2] as f32 / 255.0;
        let da = acc[s_idx + 3] as f32 / 255.0;

        let (br, bg, bb) = mezclar_canal(modo, (sr, sg, sb), (dr, dg, db));

        let out_a = src_alpha + da * (1.0 - src_alpha);
        let (or_, og, ob) = if out_a > f32::EPSILON {
            (
                (br * src_alpha + dr * da * (1.0 - src_alpha)) / out_a,
                (bg * src_alpha + dg * da * (1.0 - src_alpha)) / out_a,
                (bb * src_alpha + db * da * (1.0 - src_alpha)) / out_a,
            )
        } else {
            (0.0, 0.0, 0.0)
        };

        acc[s_idx] = clamp_u8(or_);
        acc[s_idx + 1] = clamp_u8(og);
        acc[s_idx + 2] = clamp_u8(ob);
        acc[s_idx + 3] = clamp_u8(out_a);
    }
    cobertura
}

/// Aplica una capa de ajuste sobre `acc` en vivo: copia el compuesto, le aplica
/// la op per-píxel (RGB; alfa intacto) y mezcla el resultado de vuelta por
/// `opacidad * máscara * clip` por píxel. Las ops espaciales/alfa no son
/// ajustes — `ajustar_rgb_inplace` devuelve `false` y no se toca nada.
fn aplicar_ajuste(
    acc: &mut [u8],
    n: usize,
    op: &tullpu_core::OpLocal,
    opacidad: f32,
    mascara: Option<&[u8]>,
    clip: Option<&[f32]>,
) {
    let mut adj = acc.to_vec();
    if !pixel::ajustar_rgb_inplace(op, &mut adj) {
        return;
    }
    let opac = opacidad.clamp(0.0, 1.0);
    for i in 0..n {
        let s_idx = i * 4;
        let m = mascara.map(|m| m[i] as f32 / 255.0).unwrap_or(1.0);
        let c = clip.map(|c| c[i]).unwrap_or(1.0);
        let f = opac * m * c;
        if f <= 0.0 {
            continue;
        }
        for ch in 0..3 {
            let base = acc[s_idx + ch] as f32;
            let nuevo = adj[s_idx + ch] as f32;
            acc[s_idx + ch] = (base * (1.0 - f) + nuevo * f).round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[inline]
fn clamp_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

// =============================================================================
//  Dissolve — umbralizador estocástico estable
// -----------------------------------------------------------------------------
//  Semilla por capa: primeros 8 bytes del Uuid. El Uuid es estable a través
//  de regeneraciones (lo garantiza tullpu-core), así que el patrón de ruido
//  acompaña a la capa aunque cambie su contenido. Splitmix64 por píxel: a
//  partir de `(seed XOR (i * φ))` con φ = 0x9E3779B97F4A7C15 (Golden ratio
//  scaled). Es lo mismo que usa `rand::SmallRng` internamente — barato y de
//  buena distribución para 1 sample/píxel.
// =============================================================================

#[inline]
fn semilla_dissolve(capa: &Capa) -> u64 {
    let b = capa.id.as_bytes();
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

#[inline]
fn umbral_dissolve(seed: u64, i: usize) -> f32 {
    let mut x = seed.wrapping_add((i as u64).wrapping_mul(0x9E3779B97F4A7C15));
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^= x >> 31;
    // Mantissa de 24 bits para evitar artefactos de redondeo en el borde 1.0.
    ((x >> 40) as f32) / ((1u64 << 24) as f32)
}

fn fundir_disolver(
    acc: &mut [u8],
    n: usize,
    src: &[u8],
    mascara: Option<&[u8]>,
    opacidad_global: f32,
    clip: Option<&[f32]>,
    capa: &Capa,
) -> Vec<f32> {
    let seed = semilla_dissolve(capa);
    let mut cobertura = vec![0.0f32; n];
    for i in 0..n {
        let s_idx = i * 4;
        let sa = src[s_idx + 3] as f32 / 255.0;
        let m = mascara.map(|m| m[i] as f32 / 255.0).unwrap_or(1.0);
        let c = clip.map(|c| c[i]).unwrap_or(1.0);
        let alfa_efectivo = sa * opacidad_global * m * c;
        let umbral = umbral_dissolve(seed, i);

        if alfa_efectivo > umbral {
            // Píxel gana: src completo, opaco. Reemplaza dst sin mezclar.
            acc[s_idx] = src[s_idx];
            acc[s_idx + 1] = src[s_idx + 1];
            acc[s_idx + 2] = src[s_idx + 2];
            acc[s_idx + 3] = 255;
            cobertura[i] = 1.0;
        }
        // si no, dst se queda tal cual — no tocamos `acc`.
    }
    cobertura
}

#[inline]
fn mezclar_canal(modo: ModoFusion, s: (f32, f32, f32), d: (f32, f32, f32)) -> (f32, f32, f32) {
    // Los blends HSL operan sobre el triple — no factorizan por canal.
    // Lo mismo los comparativos por luminosidad (ColorMasOscuro/Claro): la
    // decisión es por píxel completo, no canal a canal. Cortocircuito antes
    // del despacho per-channel.
    match modo {
        ModoFusion::HslTono => return blend_hsl_tono(s, d),
        ModoFusion::HslSaturacion => return blend_hsl_saturacion(s, d),
        ModoFusion::HslColor => return blend_hsl_color(s, d),
        ModoFusion::HslLuminosidad => return blend_hsl_luminosidad(s, d),
        ModoFusion::ColorMasOscuro => return if lum(s) < lum(d) { s } else { d },
        ModoFusion::ColorMasClaro => return if lum(s) > lum(d) { s } else { d },
        _ => {}
    }
    let f = |s: f32, d: f32| -> f32 {
        match modo {
            ModoFusion::Normal => s,
            ModoFusion::Multiplicar => s * d,
            ModoFusion::Pantalla => 1.0 - (1.0 - s) * (1.0 - d),
            ModoFusion::Superponer => {
                if d < 0.5 {
                    2.0 * s * d
                } else {
                    1.0 - 2.0 * (1.0 - s) * (1.0 - d)
                }
            }
            ModoFusion::Aclarar => s.max(d),
            ModoFusion::Oscurecer => s.min(d),
            ModoFusion::Diferencia => (s - d).abs(),
            ModoFusion::Aditivo => (s + d).clamp(0.0, 1.0),
            ModoFusion::SubExpQuemado => {
                if s <= f32::EPSILON {
                    0.0
                } else {
                    (1.0 - (1.0 - d) / s).clamp(0.0, 1.0)
                }
            }
            ModoFusion::SubLinealQuemado => (s + d - 1.0).clamp(0.0, 1.0),
            ModoFusion::SobreExpAclarado => {
                if s >= 1.0 - f32::EPSILON {
                    1.0
                } else {
                    (d / (1.0 - s)).clamp(0.0, 1.0)
                }
            }
            // Hard Light = Superponer con (s, d) intercambiados.
            ModoFusion::LuzFuerte => {
                if s < 0.5 {
                    2.0 * s * d
                } else {
                    1.0 - 2.0 * (1.0 - s) * (1.0 - d)
                }
            }
            // Soft Light — fórmula Photoshop / W3C `soft-light`.
            ModoFusion::LuzSuave => {
                let g_d = if d <= 0.25 {
                    ((16.0 * d - 12.0) * d + 4.0) * d
                } else {
                    d.sqrt()
                };
                if s <= 0.5 {
                    (d - (1.0 - 2.0 * s) * d * (1.0 - d)).clamp(0.0, 1.0)
                } else {
                    (d + (2.0 * s - 1.0) * (g_d - d)).clamp(0.0, 1.0)
                }
            }
            // Vivid Light = Color Burn(2s) si s<0.5, Color Dodge(2s-1) si no.
            ModoFusion::LuzViva => {
                if s < 0.5 {
                    let s2 = 2.0 * s;
                    if s2 <= f32::EPSILON {
                        0.0
                    } else {
                        (1.0 - (1.0 - d) / s2).clamp(0.0, 1.0)
                    }
                } else {
                    let s2 = 2.0 * s - 1.0;
                    if s2 >= 1.0 - f32::EPSILON {
                        1.0
                    } else {
                        (d / (1.0 - s2)).clamp(0.0, 1.0)
                    }
                }
            }
            ModoFusion::LuzLineal => (d + 2.0 * s - 1.0).clamp(0.0, 1.0),
            ModoFusion::LuzPunto => {
                if s < 0.5 {
                    d.min(2.0 * s)
                } else {
                    d.max(2.0 * s - 1.0)
                }
            }
            ModoFusion::MezclaDura => {
                if s + d >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            ModoFusion::Exclusion => (s + d - 2.0 * s * d).clamp(0.0, 1.0),
            ModoFusion::Resta => (d - s).clamp(0.0, 1.0),
            ModoFusion::Division => {
                if s <= f32::EPSILON {
                    1.0
                } else {
                    (d / s).clamp(0.0, 1.0)
                }
            }
            // Inalcanzables: HSL y comparativos por-luminosidad se manejan
            // arriba del match. Quedan acá sólo para que el match siga
            // exhaustivo y el compilador nos avise si en el futuro alguien
            // agrega una variante nueva sin cablearla.
            ModoFusion::HslTono
            | ModoFusion::HslSaturacion
            | ModoFusion::HslColor
            | ModoFusion::HslLuminosidad => unreachable!("HSL atendido arriba"),
            ModoFusion::ColorMasOscuro | ModoFusion::ColorMasClaro => {
                unreachable!("comparativos atendidos arriba")
            }
            ModoFusion::Disolver => unreachable!("dissolve atendido en rama propia de fundir_capa"),
        }
    };
    (f(s.0, d.0), f(s.1, d.1), f(s.2, d.2))
}

// =============================================================================
//  Blends HSL — W3C Compositing & Blending Level 1 §10.3
// -----------------------------------------------------------------------------
//  Los cuatro blends no separables (Hue, Saturation, Color, Luminosity) operan
//  sobre el triple RGB completo via las primitivas Lum/SetLum/Sat/SetSat.
//  Los pesos de luminosidad (0.3, 0.59, 0.11) son los del spec — no
//  Rec.601/709, sino los originales de Photoshop / sRGB.
// =============================================================================

#[inline]
fn lum(c: (f32, f32, f32)) -> f32 {
    0.3 * c.0 + 0.59 * c.1 + 0.11 * c.2
}

#[inline]
fn sat(c: (f32, f32, f32)) -> f32 {
    let max = c.0.max(c.1).max(c.2);
    let min = c.0.min(c.1).min(c.2);
    max - min
}

/// Reescala `c` hacia luminosidad `l` y clampa al cubo `[0,1]³` preservando
/// el matiz/saturación tanto como sea posible (`ClipColor` del spec).
#[inline]
fn set_lum(c: (f32, f32, f32), l: f32) -> (f32, f32, f32) {
    let d = l - lum(c);
    let cc = (c.0 + d, c.1 + d, c.2 + d);
    clip_color(cc)
}

#[inline]
fn clip_color(mut c: (f32, f32, f32)) -> (f32, f32, f32) {
    let l = lum(c);
    let n = c.0.min(c.1).min(c.2);
    let x = c.0.max(c.1).max(c.2);
    if n < 0.0 {
        let k = l / (l - n);
        c = (
            l + (c.0 - l) * k,
            l + (c.1 - l) * k,
            l + (c.2 - l) * k,
        );
    }
    if x > 1.0 {
        let k = (1.0 - l) / (x - l);
        c = (
            l + (c.0 - l) * k,
            l + (c.1 - l) * k,
            l + (c.2 - l) * k,
        );
    }
    c
}

/// Reescala `c` para que su saturación (max-min) sea `s`, preservando el
/// orden relativo de los canales. Implementa `SetSat` del spec.
fn set_sat(c: (f32, f32, f32), s: f32) -> (f32, f32, f32) {
    let mut arr = [(c.0, 0usize), (c.1, 1), (c.2, 2)];
    arr.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let (cmin_v, cmin_i) = arr[0];
    let (cmid_v, cmid_i) = arr[1];
    let (cmax_v, cmax_i) = arr[2];
    let (new_cmid, new_cmax) = if cmax_v > cmin_v {
        (((cmid_v - cmin_v) * s) / (cmax_v - cmin_v), s)
    } else {
        (0.0, 0.0)
    };
    let mut out = [0.0f32; 3];
    out[cmin_i] = 0.0;
    out[cmid_i] = new_cmid;
    out[cmax_i] = new_cmax;
    (out[0], out[1], out[2])
}

#[inline]
fn blend_hsl_tono(s: (f32, f32, f32), d: (f32, f32, f32)) -> (f32, f32, f32) {
    // SetLum(SetSat(src, Sat(dst)), Lum(dst))
    set_lum(set_sat(s, sat(d)), lum(d))
}

#[inline]
fn blend_hsl_saturacion(s: (f32, f32, f32), d: (f32, f32, f32)) -> (f32, f32, f32) {
    // SetLum(SetSat(dst, Sat(src)), Lum(dst))
    set_lum(set_sat(d, sat(s)), lum(d))
}

#[inline]
fn blend_hsl_color(s: (f32, f32, f32), d: (f32, f32, f32)) -> (f32, f32, f32) {
    // SetLum(src, Lum(dst))
    set_lum(s, lum(d))
}

#[inline]
fn blend_hsl_luminosidad(s: (f32, f32, f32), d: (f32, f32, f32)) -> (f32, f32, f32) {
    // SetLum(dst, Lum(src))
    set_lum(d, lum(s))
}

// =============================================================================
//  Helpers de construcción para tests/demos
// =============================================================================

/// Construye un buffer Rgba8 sólido del tamaño `w*h` con el color dado.
/// Útil para tests y para sembrar el almacén con capas planas.
pub fn buffer_solido(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
    let n = (w as usize) * (h as usize);
    let mut v = Vec::with_capacity(n * 4);
    for _ in 0..n {
        v.extend_from_slice(&color);
    }
    v
}

/// Buffer máscara plano del valor dado.
pub fn buffer_mascara(w: u32, h: u32, valor: u8) -> Vec<u8> {
    vec![valor; (w as usize) * (h as usize)]
}

// =============================================================================
//  Export
// =============================================================================

/// Formato de salida solicitado al exportar. Atado al codec, no a la
/// extensión: el caller elige formato y path por separado.
///
/// - `Png` lossless, conserva alfa.
/// - `Jpeg { calidad }` con `calidad ∈ 1..=100` (80–90 para foto estándar);
///   descarta alfa porque JPEG no lo soporta — el alfa del compuesto se pierde.
/// - `Webp` lossless (el encoder puro-Rust de `image` no expone lossy).
///   Conserva alfa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatoExport {
    Png,
    Jpeg { calidad: u8 },
    Webp,
}

/// Compone el lienzo y lo escribe en `ruta` con el codec correspondiente al
/// `formato`. Devuelve los píxeles compuestos por si el caller los necesita
/// además del archivo en disco. Si `componer` falla, no se toca el disco
/// (no se llega a abrir el archivo).
pub fn exportar(
    l: &Lienzo,
    fuente: &impl FuenteBuffers,
    ruta: impl AsRef<std::path::Path>,
    formato: FormatoExport,
) -> Result<RgbaImage, Error> {
    let img = componer(l, fuente)?;
    let archivo = std::fs::File::create(ruta.as_ref())?;
    let writer = std::io::BufWriter::new(archivo);
    let (w, h) = (img.width(), img.height());
    match formato {
        FormatoExport::Png => {
            let enc = image::codecs::png::PngEncoder::new(writer);
            enc.write_image(img.as_raw(), w, h, ExtendedColorType::Rgba8)?;
        }
        FormatoExport::Jpeg { calidad } => {
            // JpegEncoder::encode sólo acepta L8/Rgb8 — descartamos alfa
            // antes de codificar (calco de lo que hace `save()` internamente).
            let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(
                writer,
                calidad.clamp(1, 100),
            );
            enc.write_image(rgb.as_raw(), w, h, ExtendedColorType::Rgb8)?;
        }
        FormatoExport::Webp => {
            let enc = image::codecs::webp::WebPEncoder::new_lossless(writer);
            enc.write_image(img.as_raw(), w, h, ExtendedColorType::Rgba8)?;
        }
    }
    Ok(img)
}

/// Atajo histórico: equivale a `exportar(.., FormatoExport::Png)`.
pub fn exportar_png(
    l: &Lienzo,
    fuente: &impl FuenteBuffers,
    ruta: impl AsRef<std::path::Path>,
) -> Result<RgbaImage, Error> {
    exportar(l, fuente, ruta, FormatoExport::Png)
}

// =============================================================================
//  Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tullpu_core::Capa;

    fn pixel(img: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
        let p = img.get_pixel(x, y);
        [p.0[0], p.0[1], p.0[2], p.0[3]]
    }

    #[test]
    fn lienzo_vacio_es_transparente() {
        let l = Lienzo::nuevo(4, 4);
        let alm = AlmacenEnMemoria::nuevo();
        let img = componer(&l, &alm).unwrap();
        assert_eq!(pixel(&img, 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn una_capa_opaca_se_ve_tal_cual() {
        let mut alm = AlmacenEnMemoria::nuevo();
        let h = alm.insertar(buffer_solido(2, 2, [200, 100, 50, 255]));
        let mut l = Lienzo::nuevo(2, 2);
        l.apilar(Capa::raster("a", h));
        let img = componer(&l, &alm).unwrap();
        assert_eq!(pixel(&img, 0, 0), [200, 100, 50, 255]);
        assert_eq!(pixel(&img, 1, 1), [200, 100, 50, 255]);
    }

    #[test]
    fn normal_top_gana_sobre_fondo() {
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [255, 0, 0, 255]));
        let top = alm.insertar(buffer_solido(1, 1, [0, 0, 255, 255]));
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("fondo", fondo));
        l.apilar(Capa::raster("top", top));
        let img = componer(&l, &alm).unwrap();
        assert_eq!(pixel(&img, 0, 0), [0, 0, 255, 255]);
    }

    #[test]
    fn opacidad_05_promedia() {
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [0, 0, 0, 255]));
        let top = alm.insertar(buffer_solido(1, 1, [255, 255, 255, 255]));
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.opacidad = 0.5;
        l.apilar(c);
        let img = componer(&l, &alm).unwrap();
        let p = pixel(&img, 0, 0);
        // 0.5 blanco sobre 1.0 negro: ~128 por canal RGB.
        assert!((p[0] as i32 - 128).abs() <= 1, "got {:?}", p);
        assert!((p[1] as i32 - 128).abs() <= 1);
        assert!((p[2] as i32 - 128).abs() <= 1);
        assert_eq!(p[3], 255);
    }

    #[test]
    fn multiplicar_rojo_por_blanco_es_rojo() {
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [255, 0, 0, 255]));
        let top = alm.insertar(buffer_solido(1, 1, [255, 255, 255, 255]));
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.blend = ModoFusion::Multiplicar;
        l.apilar(c);
        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        assert_eq!(p, [255, 0, 0, 255]);
    }

    #[test]
    fn multiplicar_por_negro_es_negro() {
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [255, 200, 100, 255]));
        let top = alm.insertar(buffer_solido(1, 1, [0, 0, 0, 255]));
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.blend = ModoFusion::Multiplicar;
        l.apilar(c);
        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        assert_eq!(&p[0..3], &[0, 0, 0]);
    }

    #[test]
    fn pantalla_con_blanco_es_blanco() {
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [50, 50, 50, 255]));
        let top = alm.insertar(buffer_solido(1, 1, [255, 255, 255, 255]));
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.blend = ModoFusion::Pantalla;
        l.apilar(c);
        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        assert_eq!(&p[0..3], &[255, 255, 255]);
    }

    #[test]
    fn diferencia_iguales_es_negro() {
        let mut alm = AlmacenEnMemoria::nuevo();
        let a = alm.insertar(buffer_solido(1, 1, [200, 100, 50, 255]));
        let b = alm.insertar(buffer_solido(1, 1, [200, 100, 50, 255]));
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("a", a));
        let mut c = Capa::raster("b", b);
        c.blend = ModoFusion::Diferencia;
        l.apilar(c);
        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        assert_eq!(&p[0..3], &[0, 0, 0]);
    }

    #[test]
    fn capa_invisible_se_salta() {
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [10, 20, 30, 255]));
        let top = alm.insertar(buffer_solido(1, 1, [200, 200, 200, 255]));
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.visible = false;
        l.apilar(c);
        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        assert_eq!(p, [10, 20, 30, 255]);
    }

    #[test]
    fn mascara_oculta_lo_oculto() {
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(2, 1, [10, 10, 10, 255]));
        let top = alm.insertar(buffer_solido(2, 1, [255, 255, 255, 255]));
        // Máscara: primer píxel 0 (oculto), segundo 255 (visible).
        let mask = alm.insertar(vec![0u8, 255u8]);
        let mut l = Lienzo::nuevo(2, 1);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.mascara = Some(mask);
        l.apilar(c);
        let img = componer(&l, &alm).unwrap();
        assert_eq!(pixel(&img, 0, 0), [10, 10, 10, 255]);
        assert_eq!(pixel(&img, 1, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn buffer_faltante_es_error_explicito() {
        let l = {
            let mut l = Lienzo::nuevo(1, 1);
            l.apilar(Capa::raster("perdida", [9u8; 32]));
            l
        };
        let alm = AlmacenEnMemoria::nuevo();
        let err = componer(&l, &alm).unwrap_err();
        assert!(matches!(err, Error::BufferFaltante(_)));
    }

    #[test]
    fn tamanio_invalido_se_detecta() {
        let mut alm = AlmacenEnMemoria::nuevo();
        // Insertamos un buffer de 4 bytes y le hacemos creer al lienzo que
        // es 2x2.
        let h = alm.insertar(vec![0, 0, 0, 255]);
        let mut l = Lienzo::nuevo(2, 2);
        l.apilar(Capa::raster("chica", h));
        let err = componer(&l, &alm).unwrap_err();
        assert!(matches!(err, Error::TamanioRgba { .. }));
    }

    /// Helper: compone dos capas 1×1 opacas con el blend dado y devuelve los
    /// canales RGB del resultado. Espacio compacto para barrer la nueva
    /// familia de blends sin repetir el setup.
    fn blend_1x1(modo: ModoFusion, fondo_rgb: [u8; 3], top_rgb: [u8; 3]) -> [u8; 3] {
        let mut alm = AlmacenEnMemoria::nuevo();
        let f = alm.insertar(buffer_solido(
            1,
            1,
            [fondo_rgb[0], fondo_rgb[1], fondo_rgb[2], 255],
        ));
        let t = alm.insertar(buffer_solido(
            1,
            1,
            [top_rgb[0], top_rgb[1], top_rgb[2], 255],
        ));
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("f", f));
        let mut c = Capa::raster("t", t);
        c.blend = modo;
        l.apilar(c);
        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        [p[0], p[1], p[2]]
    }

    #[test]
    fn sub_exp_quemado_negro_sobre_cualquier_cosa_es_negro() {
        // Color Burn: src=0 fuerza out=0.
        let r = blend_1x1(ModoFusion::SubExpQuemado, [200, 100, 50], [0, 0, 0]);
        assert_eq!(r, [0, 0, 0]);
    }

    #[test]
    fn sub_exp_quemado_blanco_es_identidad() {
        // src=1 ⇒ 1 - (1-d)/1 = d.
        let r = blend_1x1(ModoFusion::SubExpQuemado, [80, 120, 200], [255, 255, 255]);
        assert_eq!(r, [80, 120, 200]);
    }

    #[test]
    fn sub_lineal_quemado_negro_y_blanco() {
        // src=0 ⇒ d-1 < 0 ⇒ 0.
        assert_eq!(
            blend_1x1(ModoFusion::SubLinealQuemado, [100, 100, 100], [0, 0, 0]),
            [0, 0, 0]
        );
        // src=1 ⇒ d.
        assert_eq!(
            blend_1x1(ModoFusion::SubLinealQuemado, [120, 80, 200], [255, 255, 255]),
            [120, 80, 200]
        );
    }

    #[test]
    fn sobre_exp_aclarado_blanco_es_blanco() {
        // src=1 forza out=1.
        let r = blend_1x1(ModoFusion::SobreExpAclarado, [50, 50, 50], [255, 255, 255]);
        assert_eq!(r, [255, 255, 255]);
    }

    #[test]
    fn sobre_exp_aclarado_negro_es_identidad() {
        // src=0 ⇒ d/1 = d.
        let r = blend_1x1(ModoFusion::SobreExpAclarado, [80, 120, 200], [0, 0, 0]);
        assert_eq!(r, [80, 120, 200]);
    }

    #[test]
    fn luz_fuerte_invierte_roles_vs_superponer() {
        // Hard Light(s,d) == Superponer(d,s). Cambiando top ↔ fondo deberían dar igual.
        let a = blend_1x1(ModoFusion::LuzFuerte, [50, 100, 200], [180, 60, 20]);
        let b = blend_1x1(ModoFusion::Superponer, [180, 60, 20], [50, 100, 200]);
        for c in 0..3 {
            assert!((a[c] as i32 - b[c] as i32).abs() <= 1, "canal {c}: {a:?} vs {b:?}");
        }
    }

    #[test]
    fn luz_suave_blanco_es_pantalla_aprox() {
        // s=1: out = d + (g(d) - d) = g(d). Para d=0.5: g(0.5)=sqrt(0.5)≈0.707 ⇒ 180.
        let r = blend_1x1(ModoFusion::LuzSuave, [128, 128, 128], [255, 255, 255]);
        for c in 0..3 {
            assert!(
                (r[c] as i32 - 180).abs() <= 2,
                "esperaba ~180, encontré {:?}",
                r
            );
        }
    }

    #[test]
    fn luz_suave_gris_medio_es_identidad() {
        // s=0.5: out = d - 0*d*(1-d) = d.
        let r = blend_1x1(ModoFusion::LuzSuave, [40, 100, 200], [128, 128, 128]);
        // Tolerancia 1 LSB por redondeo de s≈0.502.
        for (c, esperado) in [40, 100, 200].iter().enumerate() {
            assert!(
                (r[c] as i32 - *esperado).abs() <= 2,
                "canal {c}: {r:?} vs {esperado}"
            );
        }
    }

    #[test]
    fn luz_lineal_clamping() {
        // d=0.5, s=0.5 ⇒ out = 0.5 + 0 = 0.5 ≈ 128.
        let r = blend_1x1(ModoFusion::LuzLineal, [128, 128, 128], [128, 128, 128]);
        for c in 0..3 {
            assert!((r[c] as i32 - 128).abs() <= 1);
        }
        // d=1, s=1 ⇒ out clamped a 1.
        let r2 = blend_1x1(ModoFusion::LuzLineal, [255, 255, 255], [255, 255, 255]);
        assert_eq!(r2, [255, 255, 255]);
        // d=0, s=0 ⇒ out clamped a 0.
        let r3 = blend_1x1(ModoFusion::LuzLineal, [0, 0, 0], [0, 0, 0]);
        assert_eq!(r3, [0, 0, 0]);
    }

    #[test]
    fn luz_punto_es_combinacion_min_max() {
        // s < 0.5 ⇒ min(d, 2s). s=0 ⇒ min(d, 0) = 0.
        assert_eq!(
            blend_1x1(ModoFusion::LuzPunto, [200, 200, 200], [0, 0, 0]),
            [0, 0, 0]
        );
        // s > 0.5 ⇒ max(d, 2s-1). s=1 ⇒ max(d, 1) = 1.
        assert_eq!(
            blend_1x1(ModoFusion::LuzPunto, [50, 50, 50], [255, 255, 255]),
            [255, 255, 255]
        );
    }

    #[test]
    fn mezcla_dura_es_binaria() {
        // s+d < 1 ⇒ 0; s+d ≥ 1 ⇒ 1.
        let r = blend_1x1(ModoFusion::MezclaDura, [100, 200, 50], [100, 100, 100]);
        // Canal 0: 100+100=200 < 255 ⇒ 0. Canal 1: 200+100=300 ≥ 255 ⇒ 1. Canal 2: 50+100=150 < 255 ⇒ 0.
        // En normalizado [0,1]: 100/255+100/255 ≈ 0.78 < 1 ⇒ 0. 200/255+100/255 ≈ 1.18 ≥ 1 ⇒ 1.
        assert_eq!(r, [0, 255, 0]);
    }

    #[test]
    fn exclusion_simetrica() {
        // f(s,d) = f(d,s) — barrer una pareja.
        let a = blend_1x1(ModoFusion::Exclusion, [200, 50, 100], [80, 180, 60]);
        let b = blend_1x1(ModoFusion::Exclusion, [80, 180, 60], [200, 50, 100]);
        for c in 0..3 {
            assert!((a[c] as i32 - b[c] as i32).abs() <= 1);
        }
        // s=d=0.5 ⇒ 0.5 + 0.5 - 2*0.25 = 0.5 ⇒ 128.
        let r = blend_1x1(ModoFusion::Exclusion, [128, 128, 128], [128, 128, 128]);
        for c in 0..3 {
            assert!((r[c] as i32 - 128).abs() <= 1);
        }
    }

    #[test]
    fn resta_clamp_a_cero() {
        // d=100, s=200 ⇒ negativo ⇒ 0.
        assert_eq!(
            blend_1x1(ModoFusion::Resta, [100, 100, 100], [200, 200, 200]),
            [0, 0, 0]
        );
        // d=200, s=50 ⇒ 150.
        let r = blend_1x1(ModoFusion::Resta, [200, 200, 200], [50, 50, 50]);
        for c in 0..3 {
            assert!((r[c] as i32 - 150).abs() <= 1);
        }
    }

    #[test]
    fn division_negro_es_blanco() {
        // s=0 ⇒ out=1 (definición acordada para evitar NaN).
        let r = blend_1x1(ModoFusion::Division, [50, 100, 200], [0, 0, 0]);
        assert_eq!(r, [255, 255, 255]);
    }

    #[test]
    fn division_uno_es_identidad() {
        // s=1 ⇒ d/1 = d.
        let r = blend_1x1(ModoFusion::Division, [80, 120, 200], [255, 255, 255]);
        assert_eq!(r, [80, 120, 200]);
    }

    #[test]
    fn luz_viva_extremos() {
        // s=0 ⇒ Color Burn(0) = 0.
        assert_eq!(
            blend_1x1(ModoFusion::LuzViva, [200, 100, 50], [0, 0, 0]),
            [0, 0, 0]
        );
        // s=1 ⇒ Color Dodge(1) = 1.
        assert_eq!(
            blend_1x1(ModoFusion::LuzViva, [50, 50, 50], [255, 255, 255]),
            [255, 255, 255]
        );
    }

    #[test]
    fn hsl_color_sobre_grayscale_coloriza() {
        // Source rojo puro sobre fondo gris medio: el matiz/saturación de src
        // gana, la luminosidad de dst se preserva. Cálculo W3C:
        // Lum(0.5)=0.5; SetLum((1,0,0)→Lum 0.3, target 0.5):
        // d=0.2 ⇒ (1.2,0.2,0.2), ClipColor (x=1.2 > 1):
        // k=0.5/0.7 ⇒ (1.0, 0.286, 0.286) ⇒ [255, 73, 73].
        let r = blend_1x1(ModoFusion::HslColor, [128, 128, 128], [255, 0, 0]);
        // Matiz preservado: R domina, G≈B (rojo puro tiene G=B=0 en src).
        assert!(r[0] > r[1], "esperaba R > G: {:?}", r);
        assert!(r[0] > r[2], "esperaba R > B: {:?}", r);
        assert!((r[1] as i32 - r[2] as i32).abs() <= 1, "G≈B: {:?}", r);
        // G/B no son cero — la luminosidad de dst lifted el suelo.
        assert!(r[1] > 50, "fondo gris elevó suelo: {:?}", r);
        // Y la luminosidad ponderada del resultado coincide con dst (≈128).
        let lum = 0.3 * (r[0] as f32) + 0.59 * (r[1] as f32) + 0.11 * (r[2] as f32);
        assert!((lum - 128.0).abs() < 3.0, "lum~128, obtuve {lum}: {:?}", r);
    }

    #[test]
    fn hsl_luminosidad_pasa_brillo_de_src() {
        // Source blanco sobre fondo rojo: aplica Lum(blanco)=1 al dst rojo
        // ⇒ blanco (clip al cubo 1³).
        let r = blend_1x1(ModoFusion::HslLuminosidad, [255, 0, 0], [255, 255, 255]);
        assert_eq!(r, [255, 255, 255]);
        // Source negro sobre fondo rojo: Lum(negro)=0 ⇒ negro.
        let r = blend_1x1(ModoFusion::HslLuminosidad, [255, 0, 0], [0, 0, 0]);
        assert_eq!(r, [0, 0, 0]);
    }

    #[test]
    fn hsl_saturacion_grayscale_anula_dst() {
        // Source grayscale (Sat=0) sobre fondo colorido ⇒ dst se desatura:
        // SetSat(dst, 0) = (0,0,0), SetLum(esto, Lum(dst)) ⇒ gris con la
        // luminosidad de dst.
        let r = blend_1x1(ModoFusion::HslSaturacion, [200, 100, 50], [128, 128, 128]);
        // Los 3 canales deben quedar aproximadamente iguales (gris).
        let dif_max = ((r[0] as i32 - r[1] as i32).abs())
            .max((r[1] as i32 - r[2] as i32).abs())
            .max((r[0] as i32 - r[2] as i32).abs());
        assert!(dif_max <= 2, "esperaba gris uniforme, encontré {:?}", r);
    }

    #[test]
    fn hsl_tono_preserva_lum_de_dst() {
        // Tomamos un dst con Lum específica (verde puro: Lum = 0.59).
        // Source rojo puro: hue cambia, Lum(dst)≈0.59 se mantiene.
        let r = blend_1x1(ModoFusion::HslTono, [0, 255, 0], [255, 0, 0]);
        let lum = 0.3 * (r[0] as f32) + 0.59 * (r[1] as f32) + 0.11 * (r[2] as f32);
        let lum_dst = 0.59 * 255.0;
        // Tolerancia generosa por redondeo a u8 + clip.
        assert!(
            (lum - lum_dst).abs() < 3.0,
            "esperaba lum~{lum_dst}, obtuve {lum}: {r:?}"
        );
    }

    #[test]
    fn color_mas_oscuro_elige_triple_completo() {
        // Lum ponderada: rojo puro (1,0,0)=0.30 vs verde puro (0,1,0)=0.59.
        // El rojo es más oscuro → gana sobre el verde para ColorMasOscuro.
        let r = blend_1x1(ModoFusion::ColorMasOscuro, [0, 255, 0], [255, 0, 0]);
        assert_eq!(r, [255, 0, 0]);
        // Si en cambio src tiene MÁS luminosidad que dst, gana dst.
        let r = blend_1x1(ModoFusion::ColorMasOscuro, [255, 0, 0], [0, 255, 0]);
        assert_eq!(r, [255, 0, 0]);
    }

    #[test]
    fn color_mas_oscuro_es_per_pixel_no_per_canal() {
        // Distinción clave con `Oscurecer` (min por canal): aquí ningún canal
        // se interpola; o sale el triple src, o sale el triple dst. Con
        // fondo (200,50,50) Lum≈75 y top (50,50,200) Lum≈37, gana el top
        // entero — incluyendo el azul 200 aunque el fondo sea 50 ahí.
        let r = blend_1x1(ModoFusion::ColorMasOscuro, [200, 50, 50], [50, 50, 200]);
        assert_eq!(r, [50, 50, 200]);
    }

    #[test]
    fn color_mas_claro_elige_triple_completo() {
        // Verde puro Lum=0.59 > rojo puro Lum=0.30: con src verde sobre dst
        // rojo, gana src (verde).
        let r = blend_1x1(ModoFusion::ColorMasClaro, [255, 0, 0], [0, 255, 0]);
        assert_eq!(r, [0, 255, 0]);
        // Y al revés: src rojo, dst verde — gana dst (más claro).
        let r = blend_1x1(ModoFusion::ColorMasClaro, [0, 255, 0], [255, 0, 0]);
        assert_eq!(r, [0, 255, 0]);
    }

    #[test]
    fn comparativos_empate_eligen_dst() {
        // Cuando Lum(src) == Lum(dst) el orden estricto < / > deja al dst.
        // No es un requisito hard del spec, pero documentamos la convención.
        let r = blend_1x1(ModoFusion::ColorMasOscuro, [128, 128, 128], [128, 128, 128]);
        assert_eq!(r, [128, 128, 128]);
        let r = blend_1x1(ModoFusion::ColorMasClaro, [128, 128, 128], [128, 128, 128]);
        assert_eq!(r, [128, 128, 128]);
    }

    #[test]
    fn disolver_alfa_uno_pinta_todo_src() {
        // src opaco con opacidad 1.0: todos los píxeles deben terminar en src,
        // porque el umbral PRNG ∈ [0,1) siempre es < 1.0.
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(32, 32, [0, 0, 0, 255]));
        let top = alm.insertar(buffer_solido(32, 32, [200, 100, 50, 255]));
        let mut l = Lienzo::nuevo(32, 32);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.blend = ModoFusion::Disolver;
        l.apilar(c);
        let img = componer(&l, &alm).unwrap();
        for y in 0..32 {
            for x in 0..32 {
                assert_eq!(pixel(&img, x, y), [200, 100, 50, 255]);
            }
        }
    }

    #[test]
    fn disolver_alfa_cero_no_pinta_nada() {
        // src con opacidad 0: el umbral [0,1) nunca es < 0, así que ningún
        // píxel se reemplaza — el fondo gana entero.
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(16, 16, [10, 20, 30, 255]));
        let top = alm.insertar(buffer_solido(16, 16, [255, 255, 255, 255]));
        let mut l = Lienzo::nuevo(16, 16);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.blend = ModoFusion::Disolver;
        c.opacidad = 0.0;
        l.apilar(c);
        let img = componer(&l, &alm).unwrap();
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(pixel(&img, x, y), [10, 20, 30, 255]);
            }
        }
    }

    #[test]
    fn disolver_alfa_medio_da_ruido_50_50() {
        // Opacidad 0.5: ~50% píxeles deben quedar src y ~50% dst. Toleramos
        // ±10% sobre 64×64 = 4096 píxeles para que el test no sea flaky.
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(64, 64, [0, 0, 0, 255]));
        let top = alm.insertar(buffer_solido(64, 64, [255, 255, 255, 255]));
        let mut l = Lienzo::nuevo(64, 64);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.blend = ModoFusion::Disolver;
        c.opacidad = 0.5;
        l.apilar(c);
        let img = componer(&l, &alm).unwrap();
        let mut blancos = 0usize;
        for y in 0..64 {
            for x in 0..64 {
                if pixel(&img, x, y)[0] == 255 {
                    blancos += 1;
                }
            }
        }
        let total = 64 * 64;
        let mitad = total / 2;
        let tolerancia = (total / 10) as i32;
        let diff = (blancos as i32 - mitad as i32).abs();
        assert!(
            diff <= tolerancia,
            "esperaba ~{mitad} blancos, obtuve {blancos} (tol ±{tolerancia})",
        );
    }

    #[test]
    fn disolver_es_determinista_entre_renders() {
        // Mismo lienzo + misma capa ⇒ mismo output bit a bit. El patrón
        // depende sólo del `Capa.id` y la posición del píxel.
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(32, 32, [0, 0, 0, 255]));
        let top = alm.insertar(buffer_solido(32, 32, [255, 255, 255, 255]));
        let mut l = Lienzo::nuevo(32, 32);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.blend = ModoFusion::Disolver;
        c.opacidad = 0.5;
        l.apilar(c);
        let a = componer(&l, &alm).unwrap();
        let b = componer(&l, &alm).unwrap();
        assert_eq!(a.as_raw(), b.as_raw());
    }

    #[test]
    fn disolver_patron_cambia_con_capa_id() {
        // Dos capas distintas (Uuid distinto) con el mismo contenido y
        // opacidad: los patrones de píxeles ganadores difieren con muy alta
        // probabilidad. Validamos que NO sean idénticos bit a bit.
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(32, 32, [0, 0, 0, 255]));
        let top = alm.insertar(buffer_solido(32, 32, [255, 255, 255, 255]));

        let render = |opacidad: f32| {
            let mut l = Lienzo::nuevo(32, 32);
            l.apilar(Capa::raster("fondo", fondo));
            let mut c = Capa::raster("top", top);
            c.blend = ModoFusion::Disolver;
            c.opacidad = opacidad;
            l.apilar(c);
            componer(&l, &alm).unwrap().into_raw()
        };

        let a = render(0.5);
        let b = render(0.5);
        // Mismas opacidades, distintos Uuid (Capa::raster genera uno nuevo
        // cada vez). El patrón debe diferir.
        assert_ne!(a, b, "patrón Dissolve no debería repetirse entre Uuid distintos");
    }

    #[test]
    fn exportar_png_guarda_archivo_valido() {
        // Componer un lienzo 4×3 con dos capas y guardarlo a un PNG real;
        // releer el PNG con `image` y verificar que los píxeles coinciden.
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(4, 3, [10, 20, 30, 255]));
        let top = alm.insertar(buffer_solido(4, 3, [200, 100, 50, 255]));
        let mut l = Lienzo::nuevo(4, 3);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.opacidad = 0.5;
        l.apilar(c);

        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("salida.png");
        let img_compuesto = super::exportar_png(&l, &alm, &ruta).unwrap();

        assert!(ruta.exists(), "el archivo PNG debe existir");
        let leido = image::open(&ruta).unwrap().to_rgba8();
        assert_eq!(leido.width(), 4);
        assert_eq!(leido.height(), 3);
        assert_eq!(leido.as_raw(), img_compuesto.as_raw());
    }

    #[test]
    fn exportar_webp_es_lossless() {
        // WebPEncoder::new_lossless conserva alfa y píxeles bit a bit.
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(8, 5, [10, 20, 30, 255]));
        let top = alm.insertar(buffer_solido(8, 5, [200, 100, 50, 128]));
        let mut l = Lienzo::nuevo(8, 5);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c = Capa::raster("top", top);
        c.opacidad = 0.7;
        l.apilar(c);

        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("salida.webp");
        let img_compuesto =
            super::exportar(&l, &alm, &ruta, super::FormatoExport::Webp).unwrap();

        assert!(ruta.exists());
        let leido = image::open(&ruta).unwrap().to_rgba8();
        assert_eq!(leido.dimensions(), img_compuesto.dimensions());
        assert_eq!(leido.as_raw(), img_compuesto.as_raw());
    }

    #[test]
    fn exportar_jpeg_descarta_alfa_y_se_relee() {
        // JPEG no soporta alfa: lo descartamos antes de codificar y al releer
        // el RGB debe parecerse al RGB compuesto dentro de la tolerancia de
        // calidad 90 (color sólido → cuantización mínima).
        let mut alm = AlmacenEnMemoria::nuevo();
        let h = alm.insertar(buffer_solido(8, 8, [200, 100, 50, 255]));
        let mut l = Lienzo::nuevo(8, 8);
        l.apilar(Capa::raster("solido", h));

        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("salida.jpg");
        let img = super::exportar(
            &l,
            &alm,
            &ruta,
            super::FormatoExport::Jpeg { calidad: 90 },
        )
        .unwrap();

        assert!(ruta.exists());
        let leido = image::open(&ruta).unwrap().to_rgb8();
        assert_eq!(leido.dimensions(), img.dimensions());
        // Tolerancia ±6 por canal — JPEG a q90 sobre un color uniforme no
        // debería moverse mucho más que eso en ningún píxel.
        let p = leido.get_pixel(4, 4);
        assert!((p.0[0] as i16 - 200).abs() <= 6, "R: {:?}", p);
        assert!((p.0[1] as i16 - 100).abs() <= 6, "G: {:?}", p);
        assert!((p.0[2] as i16 - 50).abs() <= 6, "B: {:?}", p);
    }

    #[test]
    fn exportar_jpeg_clamp_calidad_fuera_de_rango() {
        // calidad=0 es inválido para el encoder; nosotros clampamos a 1.
        let mut alm = AlmacenEnMemoria::nuevo();
        let h = alm.insertar(buffer_solido(4, 4, [128, 128, 128, 255]));
        let mut l = Lienzo::nuevo(4, 4);
        l.apilar(Capa::raster("g", h));

        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("salida.jpg");
        super::exportar(&l, &alm, &ruta, super::FormatoExport::Jpeg { calidad: 0 }).unwrap();
        assert!(ruta.exists());
        // El archivo abre como JPEG válido — no se rechazó.
        let leido = image::open(&ruta).unwrap().to_rgb8();
        assert_eq!(leido.dimensions(), (4, 4));
    }

    #[test]
    fn exportar_png_propaga_error_de_compose() {
        // Si el lienzo apunta a un buffer que no está en el almacén, el
        // error de composición se propaga sin tocar el disco.
        let l = {
            let mut l = Lienzo::nuevo(2, 2);
            l.apilar(Capa::raster("perdida", [42u8; 32]));
            l
        };
        let alm = AlmacenEnMemoria::nuevo();
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("no-debe-existir.png");
        let err = super::exportar_png(&l, &alm, &ruta).unwrap_err();
        assert!(matches!(err, Error::BufferFaltante(_)));
        assert!(!ruta.exists(), "no se debe crear el archivo si compose falla");
    }

    // =========================================================================
    //  Grupos · clipping · capas de ajuste (Fase A)
    // =========================================================================

    use tullpu_core::OpLocal;

    #[test]
    fn grupo_compone_hijos_en_aislamiento() {
        // Un grupo con un solo hijo opaco rojo debe verse igual que el hijo
        // suelto sobre el fondo.
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [0, 0, 0, 255]));
        let rojo = alm.insertar(buffer_solido(1, 1, [255, 0, 0, 255]));

        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("fondo", fondo));
        let g = Capa::grupo("carpeta");
        let gid = g.id;
        l.apilar(g);
        let mut hijo = Capa::raster("rojo", rojo);
        hijo.grupo = Some(gid);
        l.apilar(hijo);

        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        assert_eq!(p, [255, 0, 0, 255]);
    }

    #[test]
    fn opacidad_de_grupo_modula_todo_el_contenido() {
        // Grupo al 50% sobre fondo negro: su hijo blanco opaco debe salir ~gris
        // medio — la opacidad se aplica al compuesto del grupo.
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [0, 0, 0, 255]));
        let blanco = alm.insertar(buffer_solido(1, 1, [255, 255, 255, 255]));

        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("fondo", fondo));
        let mut g = Capa::grupo("carpeta");
        g.opacidad = 0.5;
        let gid = g.id;
        l.apilar(g);
        let mut hijo = Capa::raster("blanco", blanco);
        hijo.grupo = Some(gid);
        l.apilar(hijo);

        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        for c in 0..3 {
            assert!((p[c] as i32 - 128).abs() <= 1, "esperaba ~128, {:?}", p);
        }
    }

    #[test]
    fn grupo_invisible_no_pinta() {
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [10, 20, 30, 255]));
        let blanco = alm.insertar(buffer_solido(1, 1, [255, 255, 255, 255]));
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("fondo", fondo));
        let mut g = Capa::grupo("oculta");
        g.visible = false;
        let gid = g.id;
        l.apilar(g);
        let mut hijo = Capa::raster("blanco", blanco);
        hijo.grupo = Some(gid);
        l.apilar(hijo);
        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        assert_eq!(p, [10, 20, 30, 255]);
    }

    #[test]
    fn grupos_anidados_componen_recursivo() {
        // raíz → grupo A → grupo B → hijo verde. Debe verse el verde.
        let mut alm = AlmacenEnMemoria::nuevo();
        let verde = alm.insertar(buffer_solido(1, 1, [0, 200, 0, 255]));
        let mut l = Lienzo::nuevo(1, 1);
        let a = Capa::grupo("A");
        let aid = a.id;
        l.apilar(a);
        let mut b = Capa::grupo("B");
        b.grupo = Some(aid);
        let bid = b.id;
        l.apilar(b);
        let mut hijo = Capa::raster("verde", verde);
        hijo.grupo = Some(bid);
        l.apilar(hijo);
        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        assert_eq!(p, [0, 200, 0, 255]);
    }

    #[test]
    fn clipping_recorta_a_la_alfa_de_la_base() {
        // Base 2×1: px0 opaco, px1 transparente. Capa clip blanca opaca encima:
        // sólo se ve donde la base tiene alfa (px0); px1 queda como el fondo.
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(2, 1, [10, 10, 10, 255]));
        let base = alm.insertar(vec![255, 0, 0, 255, 0, 0, 0, 0]);
        let blanco = alm.insertar(buffer_solido(2, 1, [255, 255, 255, 255]));

        let mut l = Lienzo::nuevo(2, 1);
        l.apilar(Capa::raster("fondo", fondo));
        l.apilar(Capa::raster("base", base));
        let mut clip = Capa::raster("clip", blanco);
        clip.clipping = true;
        l.apilar(clip);

        let img = componer(&l, &alm).unwrap();
        assert_eq!(pixel(&img, 0, 0), [255, 255, 255, 255]);
        assert_eq!(pixel(&img, 1, 0), [10, 10, 10, 255]);
    }

    #[test]
    fn ajuste_invertir_afecta_todo_lo_de_abajo() {
        // Fondo rojo + capa de ajuste Invertir encima ⇒ cian (255-rojo).
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [200, 50, 10, 255]));
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("fondo", fondo));
        l.apilar(Capa::ajuste("invertir", OpLocal::Invertir));
        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        assert_eq!(&p[0..3], &[55, 205, 245]);
        assert_eq!(p[3], 255);
    }

    #[test]
    fn ajuste_opacidad_media_mezcla_a_mitad_de_camino() {
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [200, 200, 200, 255]));
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("fondo", fondo));
        let mut aj = Capa::ajuste("inv", OpLocal::Invertir);
        aj.opacidad = 0.5;
        l.apilar(aj);
        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        // base 200, invertido 55, mezcla 0.5 ⇒ 127.5 ≈ 128.
        for c in 0..3 {
            assert!((p[c] as i32 - 128).abs() <= 1, "{:?}", p);
        }
    }

    #[test]
    fn ajuste_dentro_de_grupo_no_escapa_del_grupo() {
        // Ajuste Invertir dentro de un grupo afecta sólo a los hijos del grupo.
        // Fondo rojo raíz; grupo con hijo verde + ajuste invertir. El grupo
        // muestra el verde invertido (magenta) opaco sobre el rojo.
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [255, 0, 0, 255]));
        let verde = alm.insertar(buffer_solido(1, 1, [0, 255, 0, 255]));
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("fondo", fondo));
        let g = Capa::grupo("grupo");
        let gid = g.id;
        l.apilar(g);
        let mut hijo = Capa::raster("verde", verde);
        hijo.grupo = Some(gid);
        l.apilar(hijo);
        let mut aj = Capa::ajuste("inv", OpLocal::Invertir);
        aj.grupo = Some(gid);
        l.apilar(aj);

        let p = pixel(&componer(&l, &alm).unwrap(), 0, 0);
        assert_eq!(&p[0..3], &[255, 0, 255]);
    }

    #[test]
    fn agrupar_mete_capas_en_carpeta_y_compone_igual() {
        // Agrupar dos capas con `Lienzo::agrupar` (grupo Normal, opacidad 1)
        // no debe cambiar el render respecto a tenerlas sueltas.
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(1, 1, [0, 0, 0, 255]));
        let rojo = alm.insertar(buffer_solido(1, 1, [255, 0, 0, 128]));
        let azul = alm.insertar(buffer_solido(1, 1, [0, 0, 255, 128]));

        let mut plano = Lienzo::nuevo(1, 1);
        plano.apilar(Capa::raster("fondo", fondo));
        plano.apilar(Capa::raster("rojo", rojo));
        plano.apilar(Capa::raster("azul", azul));
        let suelto = componer(&plano, &alm).unwrap();

        let mut agr = Lienzo::nuevo(1, 1);
        agr.apilar(Capa::raster("fondo", fondo));
        let r = Capa::raster("rojo", rojo);
        let a = Capa::raster("azul", azul);
        let (rid, aid) = (r.id, a.id);
        agr.apilar(r);
        agr.apilar(a);
        agr.agrupar(&[rid, aid], "carpeta").unwrap();
        let agrupado = componer(&agr, &alm).unwrap();

        assert_eq!(suelto.as_raw(), agrupado.as_raw());
    }

    #[test]
    fn tres_capas_componen_a_color_predecible() {
        // Hito del SDD: cargar 3 capas y componer.
        let mut alm = AlmacenEnMemoria::nuevo();
        // Fondo gris medio.
        let fondo = alm.insertar(buffer_solido(2, 2, [128, 128, 128, 255]));
        // Tinte rojo con opacidad 0.5 → debería empujar canal rojo arriba.
        let tinte = alm.insertar(buffer_solido(2, 2, [255, 0, 0, 255]));
        // Capa blanca semitransparente con Pantalla → aclara global.
        let glow = alm.insertar(buffer_solido(2, 2, [255, 255, 255, 255]));

        let mut l = Lienzo::nuevo(2, 2);
        l.apilar(Capa::raster("fondo", fondo));
        let mut c1 = Capa::raster("tinte", tinte);
        c1.opacidad = 0.5;
        l.apilar(c1);
        let mut c2 = Capa::raster("glow", glow);
        c2.blend = ModoFusion::Pantalla;
        c2.opacidad = 0.3;
        l.apilar(c2);

        let img = componer(&l, &alm).unwrap();
        let p = pixel(&img, 0, 0);
        // Sanity: cada píxel terminó con alfa máxima.
        assert_eq!(p[3], 255);
        // Pantalla con blanco a 0.3 sobre cualquier color empuja todos los
        // canales hacia 255; ninguno debería ser menor que el fondo.
        assert!(p[0] >= 191, "rojo dominante y aclarado: {:?}", p);
        assert!(p[1] >= 64);
        assert!(p[2] >= 64);
    }
}
