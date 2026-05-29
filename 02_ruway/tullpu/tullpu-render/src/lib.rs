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

use image::RgbaImage;
use tullpu_core::{Capa, Hash, Lienzo, ModoFusion};

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
}

// =============================================================================
//  Composición
// =============================================================================

/// Compone un [`Lienzo`] sobre un buffer Rgba8 nuevo, transparente como base,
/// recorriendo las capas visibles en orden visual (fondo→tope) y fundiendo
/// con el modo de cada una. Devuelve una `RgbaImage` del tamaño del lienzo.
pub fn componer(l: &Lienzo, fuente: &impl FuenteBuffers) -> Result<RgbaImage, Error> {
    let w = l.width;
    let h = l.height;
    let n = (w as usize) * (h as usize);
    let mut acc = vec![0u8; n * 4];

    for capa in &l.capas {
        if !capa.visible {
            continue;
        }
        fundir_capa(&mut acc, w, h, capa, fuente)?;
    }

    Ok(RgbaImage::from_raw(w, h, acc).expect("dimensiones cuadran con el buffer"))
}

fn fundir_capa(
    acc: &mut [u8],
    w: u32,
    h: u32,
    capa: &Capa,
    fuente: &impl FuenteBuffers,
) -> Result<(), Error> {
    let n = (w as usize) * (h as usize);
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

    let mascara = match capa.mascara {
        Some(hm) => {
            let bytes = fuente.obtener(hm).ok_or(Error::BufferFaltante(hm))?;
            if bytes.len() != n {
                return Err(Error::TamanioMascara {
                    hash: hm,
                    esperado: n,
                    encontrado: bytes.len(),
                });
            }
            Some(bytes)
        }
        None => None,
    };

    let opacidad_global = capa.opacidad.clamp(0.0, 1.0);
    let modo = capa.blend;

    for i in 0..n {
        let s_idx = i * 4;
        let sr = src[s_idx] as f32 / 255.0;
        let sg = src[s_idx + 1] as f32 / 255.0;
        let sb = src[s_idx + 2] as f32 / 255.0;
        let sa = src[s_idx + 3] as f32 / 255.0;

        let m = mascara.map(|m| m[i] as f32 / 255.0).unwrap_or(1.0);
        let src_alpha = sa * opacidad_global * m;

        let dr = acc[s_idx] as f32 / 255.0;
        let dg = acc[s_idx + 1] as f32 / 255.0;
        let db = acc[s_idx + 2] as f32 / 255.0;
        let da = acc[s_idx + 3] as f32 / 255.0;

        let (br, bg, bb) = mezclar_canal(modo, (sr, sg, sb), (dr, dg, db));

        // Composite "over": el resultado del modo (br,bg,bb) actúa como
        // fuente con alfa `src_alpha` sobre el destino (dr,dg,db,da).
        let out_a = src_alpha + da * (1.0 - src_alpha);
        // Si out_a ~ 0, los canales no importan; evitamos NaN.
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

    Ok(())
}

#[inline]
fn clamp_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[inline]
fn mezclar_canal(modo: ModoFusion, s: (f32, f32, f32), d: (f32, f32, f32)) -> (f32, f32, f32) {
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
        }
    };
    (f(s.0, d.0), f(s.1, d.1), f(s.2, d.2))
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
