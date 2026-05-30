//! `tullpu-ops` — el catálogo de operaciones locales del editor.
//!
//! Cada `OpLocal` declarada en `tullpu-core` se ejecuta aquí: una función
//! pura que toma un buffer Rgba8 `(W*H*4)` y devuelve uno nuevo del mismo
//! tamaño. La impureza (caché, marca *stale → fresca*, escritura al almacén)
//! la maneja el orquestador [`regenerar_stale`], que recorre el lienzo en
//! orden topológico y ejecuta cada capa derivada cuya madre está fresca.
//!
//! Las ops IA (`TransformacionPixel::Ia`) se delegan a un proveedor que
//! implementa `pixel_verbo_core::Proveedor` — el mock determinista en
//! tests/dev, el `ClienteBloqueante` del daemon en producción.
//! [`regenerar_stale_con_ia`] es la variante con proveedor; la versión
//! básica [`regenerar_stale`] mantiene el comportamiento de saltar las
//! Ia (útil cuando el daemon no está disponible).

#![forbid(unsafe_code)]

use image::{ImageBuffer, RgbaImage};
use pixel_verbo_core::{Imagen, OpPixel, Proveedor};
use tullpu_core::{
    Capa, Frescura, Hash, Lienzo, OpLocal, OrigenCapa, TransformacionPixel,
};
use tullpu_render::{AlmacenEnMemoria, FuenteBuffers};

// =============================================================================
//  Errores
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("buffer faltante: hash {0:02x?}")]
    BufferFaltante(Hash),
    #[error("tamaño de buffer Rgba8 inválido: esperaba {esperado}, encontré {encontrado}")]
    Tamanio { esperado: usize, encontrado: usize },
    #[error("operación IA '{modelo}' no la ejecuta este crate — la sirve pixel-verbo-daemon")]
    IaNoSoportada { modelo: String },
    #[error("op IA: el proveedor devolvió un error: {0}")]
    IaProveedor(String),
    #[error("params de op IA mal formados (postcard): {0}")]
    IaParams(String),
    #[error(
        "op IA devolvió dimensiones inesperadas: esperaba {ancho_esp}×{alto_esp}, vino {ancho}×{alto}"
    )]
    IaDimension {
        ancho_esp: u32,
        alto_esp: u32,
        ancho: u32,
        alto: u32,
    },
}

// =============================================================================
//  Ejecución de una op pura
// =============================================================================

/// Aplica una [`OpLocal`] sobre un buffer Rgba8 plano `(W*H*4)` y devuelve un
/// buffer nuevo del mismo tamaño. La operación es pura y determinista —
/// dadas las mismas entradas, mismo output bit-exacto.
pub fn aplicar_op_local(op: &OpLocal, src: &[u8], w: u32, h: u32) -> Result<Vec<u8>, Error> {
    let esperado = (w as usize) * (h as usize) * 4;
    if src.len() != esperado {
        return Err(Error::Tamanio {
            esperado,
            encontrado: src.len(),
        });
    }
    let salida = match op {
        OpLocal::Invertir => mapear_rgb(src, |c| 255 - c),
        OpLocal::Brillo { delta } => mapear_rgb_f(src, |c| c + *delta),
        OpLocal::Contraste { factor } => {
            mapear_rgb_f(src, |c| (c - 0.5) * *factor + 0.5)
        }
        OpLocal::Niveles {
            entrada_min,
            entrada_max,
            gamma,
        } => {
            let min = *entrada_min;
            let max = *entrada_max;
            let inv_g = if *gamma > f32::EPSILON {
                1.0 / *gamma
            } else {
                1.0
            };
            let rango = (max - min).max(1e-6);
            mapear_rgb_f(src, |c| ((c - min) / rango).clamp(0.0, 1.0).powf(inv_g))
        }
        OpLocal::Opacidad { factor } => mapear_alfa_f(src, |a| a * *factor),
        OpLocal::Saturacion { factor } => mapear_hsl(src, |h, s, l| (h, s * *factor, l)),
        OpLocal::Tonalidad { grados } => {
            let delta = grados / 360.0;
            mapear_hsl(src, |h, s, l| ((h + delta).rem_euclid(1.0), s, l))
        }
        OpLocal::Blur { radio } => {
            // image::imageops::blur usa sigma; sigma ≈ radio/2 da un blur
            // visualmente equivalente al "radio" tradicional de Photoshop.
            let sigma = (radio / 2.0).max(0.0);
            let buf: RgbaImage = ImageBuffer::from_raw(w, h, src.to_vec())
                .expect("dimensiones validadas arriba");
            let blurred = image::imageops::blur(&buf, sigma);
            blurred.into_raw()
        }
        OpLocal::EspejarHorizontal => espejar_horizontal(src, w, h),
        OpLocal::EspejarVertical => espejar_vertical(src, w, h),
    };
    Ok(salida)
}

/// Aplica una [`TransformacionPixel`] entera. Las Ia devuelven
/// [`Error::IaNoSoportada`] — quedan para `pixel-verbo-daemon`.
pub fn aplicar_transformacion(
    t: &TransformacionPixel,
    src: &[u8],
    w: u32,
    h: u32,
) -> Result<Vec<u8>, Error> {
    match t {
        TransformacionPixel::Local(op) => aplicar_op_local(op, src, w, h),
        TransformacionPixel::Ia { modelo, .. } => Err(Error::IaNoSoportada {
            modelo: modelo.clone(),
        }),
    }
}

// =============================================================================
//  Helpers de mapeo
// =============================================================================

/// Espeja horizontalmente un buffer Rgba8 `w×h`: cada fila se invierte
/// por columna. Pura. Pre: `src.len() == w*h*4` (validado en el caller).
fn espejar_horizontal(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let w = w as usize;
    let h = h as usize;
    let mut out = vec![0u8; src.len()];
    for y in 0..h {
        for x in 0..w {
            let i_src = (y * w + x) * 4;
            let i_dst = (y * w + (w - 1 - x)) * 4;
            out[i_dst..i_dst + 4].copy_from_slice(&src[i_src..i_src + 4]);
        }
    }
    out
}

/// Espeja verticalmente un buffer Rgba8 `w×h`: cada fila viaja al
/// índice complementario. Más barato que el horizontal — un swap por
/// fila, no por píxel.
fn espejar_vertical(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let w = w as usize;
    let h = h as usize;
    let stride = w * 4;
    let mut out = vec![0u8; src.len()];
    for y in 0..h {
        let i_src = y * stride;
        let i_dst = (h - 1 - y) * stride;
        out[i_dst..i_dst + stride].copy_from_slice(&src[i_src..i_src + stride]);
    }
    out
}

fn mapear_rgb<F: Fn(u8) -> u8>(src: &[u8], f: F) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    for px in src.chunks_exact(4) {
        out.push(f(px[0]));
        out.push(f(px[1]));
        out.push(f(px[2]));
        out.push(px[3]);
    }
    out
}

fn mapear_rgb_f<F: Fn(f32) -> f32>(src: &[u8], f: F) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    for px in src.chunks_exact(4) {
        out.push(clamp_u8(f(px[0] as f32 / 255.0)));
        out.push(clamp_u8(f(px[1] as f32 / 255.0)));
        out.push(clamp_u8(f(px[2] as f32 / 255.0)));
        out.push(px[3]);
    }
    out
}

fn mapear_alfa_f<F: Fn(f32) -> f32>(src: &[u8], f: F) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    for px in src.chunks_exact(4) {
        out.push(px[0]);
        out.push(px[1]);
        out.push(px[2]);
        out.push(clamp_u8(f(px[3] as f32 / 255.0)));
    }
    out
}

/// Aplica una transformación en HSL: RGB → HSL → modificar → RGB.
fn mapear_hsl<F: Fn(f32, f32, f32) -> (f32, f32, f32)>(src: &[u8], f: F) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    for px in src.chunks_exact(4) {
        let (h, s, l) = rgb_a_hsl(px[0], px[1], px[2]);
        let (h, s, l) = f(h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
        let (r, g, b) = hsl_a_rgb(h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
        out.push(r);
        out.push(g);
        out.push(b);
        out.push(px[3]);
    }
    out
}

#[inline]
fn clamp_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
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

// =============================================================================
//  Orquestador — regenerar el cono stale
// =============================================================================

/// Recorre el lienzo en orden topológico y, para cada capa derivada que
/// está *stale* (y cuya op es Local — Ia se salta), ejecuta su
/// transformación sobre el buffer de su madre, escribe el resultado en el
/// almacén y marca la capa *fresca* con el nuevo hash.
///
/// Devuelve la lista de capas regeneradas en orden de ejecución. Es
/// idempotente: una segunda llamada sobre un lienzo sin stale devuelve
/// `vec![]`.
pub fn regenerar_stale(
    l: &mut Lienzo,
    alm: &mut AlmacenEnMemoria,
) -> Result<Vec<tullpu_core::Capa>, Error> {
    let orden = l.orden_regeneracion();
    let mut regeneradas = Vec::new();
    let w = l.width;
    let h = l.height;

    for id in orden {
        let (madre_id, op) = {
            let capa = match l.capa(id) {
                Some(c) => c,
                None => continue,
            };
            match &capa.origen {
                OrigenCapa::Derivada {
                    madre,
                    op,
                    estado: Frescura::Stale,
                } => (*madre, op.clone()),
                _ => continue,
            }
        };

        // Solo ejecutamos ops Local; las Ia las salta este crate.
        let op = match op {
            TransformacionPixel::Local(o) => o,
            TransformacionPixel::Ia { .. } => continue,
        };

        // Buffer de la madre — vía el almacén, ya con su contenido vigente.
        let madre_hash = l
            .capa(madre_id)
            .ok_or(Error::BufferFaltante([0u8; 32]))?
            .contenido;
        let src_bytes = alm
            .obtener(madre_hash)
            .ok_or(Error::BufferFaltante(madre_hash))?
            .to_vec();

        let salida = aplicar_op_local(&op, &src_bytes, w, h)?;
        let nuevo_hash = alm.insertar(salida);

        l.marcar_fresca(id, nuevo_hash);
        if let Some(c) = l.capa(id) {
            regeneradas.push(c.clone());
        }
    }

    Ok(regeneradas)
}

// =============================================================================
//  Puente con pixel-verbo: codificar/decodificar OpPixel en params
// =============================================================================

/// Nombre canónico de modelo que tullpu usa al crear capas Ia "del mock".
/// Es el mismo string que devuelve `ProveedorMock::nuevo().model_id()`,
/// repetido aquí para que tullpu-ops no dependa de pixel-verbo-mock.
pub const MODELO_MOCK: &str = "pixel-verbo-mock-v0";

/// Construye una [`TransformacionPixel::Ia`] desde una [`OpPixel`],
/// codificando la op en `params` con postcard. `modelo` es el nombre del
/// proveedor que la app quiere usar para esta capa — en runtime el
/// orquestador no lo verifica (un proveedor incompatible se manifestará
/// como error del proveedor), pero queda guardado para el render del
/// grafo en UI y para una validación futura.
pub fn transformacion_ia(modelo: impl Into<String>, op: &OpPixel) -> TransformacionPixel {
    let prompt = prompt_de_op(op);
    let params = postcard::to_allocvec(op)
        .expect("OpPixel siempre serializa: enum cerrado sin floats no-finitos");
    TransformacionPixel::Ia {
        modelo: modelo.into(),
        prompt,
        params,
    }
}

/// Decodifica la [`OpPixel`] que viaja en los `params` de una
/// `TransformacionPixel::Ia`.
pub fn op_pixel_desde_params(params: &[u8]) -> Result<OpPixel, Error> {
    postcard::from_bytes(params).map_err(|e| Error::IaParams(e.to_string()))
}

fn prompt_de_op(op: &OpPixel) -> Option<String> {
    match op {
        OpPixel::Segmentar { prompt } | OpPixel::Inpaint { prompt } => prompt.clone(),
        OpPixel::Restyle { prompt } | OpPixel::Generar { prompt, .. } => Some(prompt.clone()),
    }
}

// =============================================================================
//  Orquestador con proveedor IA
// =============================================================================

/// Como [`regenerar_stale`] pero acepta un proveedor de píxel para
/// ejecutar las capas `TransformacionPixel::Ia`. Locales y IA se
/// despachan en el mismo orden topológico — una capa IA puede depender
/// de la salida de una local, y viceversa.
///
/// Validaciones de dimensión: se exige que el output del proveedor mida
/// igual que el lienzo (mismo invariante que las locales). El caller que
/// quiera ops con cambio de resolución (upscale) debe redimensionar el
/// lienzo y propagar stale aguas abajo, no relajar este invariante.
pub fn regenerar_stale_con_ia(
    l: &mut Lienzo,
    alm: &mut AlmacenEnMemoria,
    proveedor: &dyn Proveedor,
) -> Result<Vec<Capa>, Error> {
    let orden = l.orden_regeneracion();
    let mut regeneradas = Vec::new();
    let w = l.width;
    let h = l.height;

    for id in orden {
        let (madre_id, op) = {
            let capa = match l.capa(id) {
                Some(c) => c,
                None => continue,
            };
            match &capa.origen {
                OrigenCapa::Derivada {
                    madre,
                    op,
                    estado: Frescura::Stale,
                } => (*madre, op.clone()),
                _ => continue,
            }
        };

        let salida = match op {
            TransformacionPixel::Local(o) => {
                let madre_hash = l
                    .capa(madre_id)
                    .ok_or(Error::BufferFaltante([0u8; 32]))?
                    .contenido;
                let src = alm
                    .obtener(madre_hash)
                    .ok_or(Error::BufferFaltante(madre_hash))?
                    .to_vec();
                aplicar_op_local(&o, &src, w, h)?
            }
            TransformacionPixel::Ia {
                modelo: _, params, ..
            } => {
                let op_pixel = op_pixel_desde_params(&params)?;
                let entrada = if op_pixel.requiere_entrada() {
                    let madre_hash = l
                        .capa(madre_id)
                        .ok_or(Error::BufferFaltante([0u8; 32]))?
                        .contenido;
                    let bytes = alm
                        .obtener(madre_hash)
                        .ok_or(Error::BufferFaltante(madre_hash))?
                        .to_vec();
                    Some(Imagen { ancho: w, alto: h, bytes })
                } else {
                    None
                };
                let img = proveedor
                    .aplicar(&op_pixel, entrada)
                    .map_err(|e| Error::IaProveedor(e.to_string()))?;
                if img.ancho != w || img.alto != h {
                    return Err(Error::IaDimension {
                        ancho_esp: w,
                        alto_esp: h,
                        ancho: img.ancho,
                        alto: img.alto,
                    });
                }
                img.bytes
            }
        };

        let nuevo_hash = alm.insertar(salida);
        l.marcar_fresca(id, nuevo_hash);
        if let Some(c) = l.capa(id) {
            regeneradas.push(c.clone());
        }
    }

    Ok(regeneradas)
}

// =============================================================================
//  Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tullpu_core::Capa;
    use tullpu_render::buffer_solido;

    fn px(buf: &[u8], i: usize) -> [u8; 4] {
        [buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]]
    }

    #[test]
    fn invertir_complementa_rgb_y_respeta_alfa() {
        let src = buffer_solido(1, 1, [10, 200, 50, 128]);
        let out = aplicar_op_local(&OpLocal::Invertir, &src, 1, 1).unwrap();
        assert_eq!(px(&out, 0), [245, 55, 205, 128]);
    }

    #[test]
    fn brillo_suma_al_canal() {
        let src = buffer_solido(1, 1, [100, 100, 100, 255]);
        let out = aplicar_op_local(&OpLocal::Brillo { delta: 0.2 }, &src, 1, 1).unwrap();
        // 100/255 + 0.2 ≈ 0.592 → 151.
        let p = px(&out, 0);
        assert!((p[0] as i32 - 151).abs() <= 1, "got {:?}", p);
        assert_eq!(p[3], 255);
    }

    #[test]
    fn brillo_satura_hacia_blanco_y_negro() {
        let src = buffer_solido(1, 1, [200, 200, 200, 255]);
        let out = aplicar_op_local(&OpLocal::Brillo { delta: 1.0 }, &src, 1, 1).unwrap();
        assert_eq!(px(&out, 0)[0], 255);
        let out = aplicar_op_local(&OpLocal::Brillo { delta: -1.0 }, &src, 1, 1).unwrap();
        assert_eq!(px(&out, 0)[0], 0);
    }

    #[test]
    fn contraste_centra_en_05() {
        // Píxel 0.5 ↔ 128 no se mueve con cualquier factor.
        let src = buffer_solido(1, 1, [128, 128, 128, 255]);
        let out = aplicar_op_local(&OpLocal::Contraste { factor: 2.0 }, &src, 1, 1).unwrap();
        let p = px(&out, 0);
        // Tolerancia de redondeo.
        assert!((p[0] as i32 - 128).abs() <= 1);
    }

    #[test]
    fn opacidad_escala_alfa() {
        let src = buffer_solido(1, 1, [200, 200, 200, 200]);
        let out = aplicar_op_local(&OpLocal::Opacidad { factor: 0.5 }, &src, 1, 1).unwrap();
        let p = px(&out, 0);
        assert_eq!(&p[0..3], &[200, 200, 200]);
        assert!((p[3] as i32 - 100).abs() <= 1);
    }

    #[test]
    fn saturacion_cero_da_gris() {
        let src = buffer_solido(1, 1, [200, 50, 30, 255]);
        let out =
            aplicar_op_local(&OpLocal::Saturacion { factor: 0.0 }, &src, 1, 1).unwrap();
        let p = px(&out, 0);
        // Gris ↔ R==G==B.
        assert_eq!(p[0], p[1]);
        assert_eq!(p[1], p[2]);
        assert_eq!(p[3], 255);
    }

    #[test]
    fn tonalidad_360_es_identidad() {
        let src = buffer_solido(1, 1, [120, 60, 200, 255]);
        let out =
            aplicar_op_local(&OpLocal::Tonalidad { grados: 360.0 }, &src, 1, 1).unwrap();
        let p = px(&out, 0);
        // Mínima deriva por ida-y-vuelta RGB↔HSL.
        assert!((p[0] as i32 - 120).abs() <= 2, "got {:?}", p);
        assert!((p[1] as i32 - 60).abs() <= 2);
        assert!((p[2] as i32 - 200).abs() <= 2);
    }

    #[test]
    fn espejar_horizontal_invierte_columnas() {
        // 3×2 con un patrón distinguible por columna: x=0 rojo, x=1 verde,
        // x=2 azul. Tras espejar horizontal, las columnas quedan azul,
        // verde, rojo (las filas mantienen su orden).
        let mut src = Vec::with_capacity(3 * 2 * 4);
        for _y in 0..2 {
            src.extend_from_slice(&[255, 0, 0, 255]);
            src.extend_from_slice(&[0, 255, 0, 255]);
            src.extend_from_slice(&[0, 0, 255, 255]);
        }
        let out = aplicar_op_local(&OpLocal::EspejarHorizontal, &src, 3, 2).unwrap();
        // Píxel (0, 0) — esquina superior izquierda — ahora es azul.
        assert_eq!(px(&out, 0), [0, 0, 255, 255]);
        // Píxel (1, 0) — columna media — verde (sin cambio).
        assert_eq!(px(&out, 1), [0, 255, 0, 255]);
        // Píxel (2, 0) — esquina superior derecha — ahora es rojo.
        assert_eq!(px(&out, 2), [255, 0, 0, 255]);
    }

    #[test]
    fn espejar_horizontal_dos_veces_es_identidad() {
        // Propiedad básica de una involución: aplicar dos veces vuelve al
        // origen. Verifica que el cálculo no introduce drift.
        let mut src = Vec::with_capacity(4 * 4 * 4);
        for i in 0..(4 * 4) {
            src.extend_from_slice(&[i as u8 * 16, 100, 200, 255]);
        }
        let una = aplicar_op_local(&OpLocal::EspejarHorizontal, &src, 4, 4).unwrap();
        let dos = aplicar_op_local(&OpLocal::EspejarHorizontal, &una, 4, 4).unwrap();
        assert_eq!(dos, src);
    }

    #[test]
    fn espejar_vertical_invierte_filas() {
        // 2×3 con un patrón distinguible por fila: y=0 rojo, y=1 verde,
        // y=2 azul. Tras espejar vertical, las filas quedan azul, verde,
        // rojo (las columnas mantienen su orden).
        let mut src = Vec::new();
        for &color in &[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]] {
            for _x in 0..2 {
                src.extend_from_slice(&color);
            }
        }
        let out = aplicar_op_local(&OpLocal::EspejarVertical, &src, 2, 3).unwrap();
        // Píxel (0, 0) — fila 0 — ahora es azul.
        assert_eq!(px(&out, 0), [0, 0, 255, 255]);
        // Píxel (0, 2) — fila inferior — ahora es rojo.
        assert_eq!(px(&out, 2 * 2), [255, 0, 0, 255]);
    }

    #[test]
    fn espejar_vertical_dos_veces_es_identidad() {
        let mut src = Vec::with_capacity(4 * 4 * 4);
        for i in 0..(4 * 4) {
            src.extend_from_slice(&[i as u8 * 16, 100, 200, 255]);
        }
        let una = aplicar_op_local(&OpLocal::EspejarVertical, &src, 4, 4).unwrap();
        let dos = aplicar_op_local(&OpLocal::EspejarVertical, &una, 4, 4).unwrap();
        assert_eq!(dos, src);
    }

    #[test]
    fn espejar_h_y_v_no_conmutan_con_orden_arbitrario_pero_componen_a_rotacion_180() {
        // Composición h∘v = v∘h = rotación 180°. Verifica que ambas órdenes
        // dan el mismo resultado y que ese resultado es exactamente el
        // píxel opuesto por el centro.
        let mut src = Vec::with_capacity(3 * 3 * 4);
        for i in 0..9u8 {
            src.extend_from_slice(&[i * 20, 0, 0, 255]);
        }
        let h_then_v = {
            let h = aplicar_op_local(&OpLocal::EspejarHorizontal, &src, 3, 3).unwrap();
            aplicar_op_local(&OpLocal::EspejarVertical, &h, 3, 3).unwrap()
        };
        let v_then_h = {
            let v = aplicar_op_local(&OpLocal::EspejarVertical, &src, 3, 3).unwrap();
            aplicar_op_local(&OpLocal::EspejarHorizontal, &v, 3, 3).unwrap()
        };
        assert_eq!(h_then_v, v_then_h);
        // Píxel central (idx 4) queda en sí mismo bajo 180°.
        assert_eq!(px(&h_then_v, 4), px(&src, 4));
        // Píxel (0, 0) intercambia con (2, 2) — esquinas opuestas.
        assert_eq!(px(&h_then_v, 0), px(&src, 8));
    }

    #[test]
    fn blur_radio_cero_es_identidad() {
        let src = buffer_solido(4, 4, [100, 150, 200, 255]);
        let out = aplicar_op_local(&OpLocal::Blur { radio: 0.0 }, &src, 4, 4).unwrap();
        assert_eq!(out, src);
    }

    #[test]
    fn niveles_es_monotonico_y_extremos_correctos() {
        // Niveles [0..1] con gamma=1 es identidad para el rango.
        let src = buffer_solido(1, 1, [255, 128, 0, 255]);
        let out = aplicar_op_local(
            &OpLocal::Niveles {
                entrada_min: 0.0,
                entrada_max: 1.0,
                gamma: 1.0,
            },
            &src,
            1,
            1,
        )
        .unwrap();
        let p = px(&out, 0);
        assert_eq!(p[0], 255);
        assert!((p[1] as i32 - 128).abs() <= 1);
        assert_eq!(p[2], 0);
    }

    #[test]
    fn tamanio_invalido_es_error() {
        let src = vec![0u8; 4]; // 1 píxel, no 4.
        let err = aplicar_op_local(&OpLocal::Invertir, &src, 2, 2).unwrap_err();
        assert!(matches!(err, Error::Tamanio { .. }));
    }

    #[test]
    fn ia_no_soportada_es_error_explicito() {
        let src = buffer_solido(1, 1, [0, 0, 0, 255]);
        let t = TransformacionPixel::Ia {
            modelo: "sam".into(),
            prompt: None,
            params: vec![],
        };
        let err = aplicar_transformacion(&t, &src, 1, 1).unwrap_err();
        assert!(matches!(err, Error::IaNoSoportada { .. }));
    }

    #[test]
    fn regenerar_stale_recorre_cono_completo() {
        // A (raster) → B (invertir A) → C (brillo +0.2 sobre B).
        let mut alm = AlmacenEnMemoria::nuevo();
        let h_a = alm.insertar(buffer_solido(1, 1, [200, 100, 50, 255]));

        let mut l = Lienzo::nuevo(1, 1);
        let a = Capa::raster("a", h_a);
        let id_a = a.id;
        l.apilar(a);

        // Sembramos B y C *stale* con un hash placeholder.
        let b = Capa::derivada(
            "b",
            id_a,
            TransformacionPixel::Local(OpLocal::Invertir),
            [0u8; 32],
        );
        let id_b = b.id;
        l.apilar(b);
        let c = Capa::derivada(
            "c",
            id_b,
            TransformacionPixel::Local(OpLocal::Brillo { delta: 0.2 }),
            [0u8; 32],
        );
        let id_c = c.id;
        l.apilar(c);

        let regen = regenerar_stale(&mut l, &mut alm).unwrap();
        assert_eq!(regen.len(), 2, "B y C debieron regenerarse");
        assert!(!l.capa(id_b).unwrap().esta_stale());
        assert!(!l.capa(id_c).unwrap().esta_stale());

        // El buffer de B debería ser el invertido de A.
        let h_b = l.capa(id_b).unwrap().contenido;
        let b_bytes = alm.obtener(h_b).unwrap();
        assert_eq!(px(b_bytes, 0), [55, 155, 205, 255]);

        // Y el de C debería ser brillo +0.2 sobre B.
        let h_c = l.capa(id_c).unwrap().contenido;
        let c_bytes = alm.obtener(h_c).unwrap();
        let p = px(c_bytes, 0);
        // 55/255 + 0.2 ≈ 0.416 → 106; 155/255 + 0.2 ≈ 0.808 → 206;
        // 205/255 + 0.2 ≈ 1.004 → 255.
        assert!((p[0] as i32 - 106).abs() <= 1, "got {:?}", p);
        assert!((p[1] as i32 - 206).abs() <= 1);
        assert_eq!(p[2], 255);

        // Segunda llamada idempotente.
        let regen2 = regenerar_stale(&mut l, &mut alm).unwrap();
        assert!(regen2.is_empty());
    }

    #[test]
    fn regenerar_stale_propaga_tras_invalidar_madre() {
        // A → B; regeneramos; cambiamos A; propagar_stale; B vuelve stale.
        let mut alm = AlmacenEnMemoria::nuevo();
        let h_a = alm.insertar(buffer_solido(1, 1, [255, 0, 0, 255]));

        let mut l = Lienzo::nuevo(1, 1);
        let a = Capa::raster("a", h_a);
        let id_a = a.id;
        l.apilar(a);
        let b = Capa::derivada(
            "b",
            id_a,
            TransformacionPixel::Local(OpLocal::Invertir),
            [0u8; 32],
        );
        let id_b = b.id;
        l.apilar(b);

        regenerar_stale(&mut l, &mut alm).unwrap();
        assert!(!l.capa(id_b).unwrap().esta_stale());

        // Cambiamos el contenido de A a otro color y propagamos.
        let nuevo_a = alm.insertar(buffer_solido(1, 1, [10, 20, 30, 255]));
        l.capa_mut(id_a).unwrap().contenido = nuevo_a;
        l.propagar_stale(id_a);
        assert!(l.capa(id_b).unwrap().esta_stale());

        regenerar_stale(&mut l, &mut alm).unwrap();
        let h_b = l.capa(id_b).unwrap().contenido;
        let b_bytes = alm.obtener(h_b).unwrap();
        // Invertido del nuevo A.
        assert_eq!(px(b_bytes, 0), [245, 235, 225, 255]);
    }

    #[test]
    fn regenerar_stale_salta_ia() {
        let mut alm = AlmacenEnMemoria::nuevo();
        let h_a = alm.insertar(buffer_solido(1, 1, [0, 0, 0, 255]));

        let mut l = Lienzo::nuevo(1, 1);
        let a = Capa::raster("a", h_a);
        let id_a = a.id;
        l.apilar(a);
        let b = Capa::derivada(
            "b",
            id_a,
            TransformacionPixel::Ia {
                modelo: "sam".into(),
                prompt: None,
                params: vec![],
            },
            [0u8; 32],
        );
        let id_b = b.id;
        l.apilar(b);

        let regen = regenerar_stale(&mut l, &mut alm).unwrap();
        assert!(regen.is_empty(), "Ia no la ejecuta este crate");
        assert!(l.capa(id_b).unwrap().esta_stale());
    }

    #[test]
    fn regenerar_stale_con_ia_despacha_al_proveedor() {
        use pixel_verbo_mock::ProveedorMock;

        // A (raster) → B (Ia::Restyle prompt="tropical")
        let mut alm = AlmacenEnMemoria::nuevo();
        // El input debe tener saturación > 0 — un gris puro queda
        // invariante ante un shift de matiz HSL.
        let h_a = alm.insertar(buffer_solido(2, 2, [200, 80, 40, 255]));

        let mut l = Lienzo::nuevo(2, 2);
        let a = Capa::raster("a", h_a);
        let id_a = a.id;
        l.apilar(a);

        let op = OpPixel::Restyle {
            prompt: "tropical".into(),
        };
        let trans = transformacion_ia(MODELO_MOCK, &op);
        let b = Capa::derivada("ia-restyle", id_a, trans, [0u8; 32]);
        let id_b = b.id;
        l.apilar(b);

        let prov = ProveedorMock::nuevo();
        let regen = regenerar_stale_con_ia(&mut l, &mut alm, &prov).unwrap();
        assert_eq!(regen.len(), 1);
        assert!(!l.capa(id_b).unwrap().esta_stale());

        // El hash de B debe diferir del de A — Restyle "tropical" no es
        // identidad sobre un gris saturado bajo (un gris seguirá siendo
        // gris por shift de matiz, pero el helper de hash usa el output
        // entero; usamos un color con saturación para evidenciar el
        // cambio).
        let h_a2 = l.capa(id_a).unwrap().contenido;
        let h_b = l.capa(id_b).unwrap().contenido;
        assert_ne!(h_a2, h_b, "Restyle debe modificar al menos un canal");
    }

    #[test]
    fn regenerar_stale_con_ia_generar_no_requiere_entrada() {
        use pixel_verbo_mock::ProveedorMock;

        let mut alm = AlmacenEnMemoria::nuevo();
        let h_a = alm.insertar(buffer_solido(4, 4, [0, 0, 0, 255]));

        let mut l = Lienzo::nuevo(4, 4);
        let a = Capa::raster("a", h_a);
        let id_a = a.id;
        l.apilar(a);

        // Generar ignora la entrada — pero seguimos exigiendo una madre
        // por la topología del DAG. El proveedor mock genera un
        // gradiente del tamaño del lienzo.
        let op = OpPixel::Generar {
            prompt: "atardecer".into(),
            ancho: 4,
            alto: 4,
        };
        let trans = transformacion_ia(MODELO_MOCK, &op);
        let b = Capa::derivada("ia-generar", id_a, trans, [0u8; 32]);
        let id_b = b.id;
        l.apilar(b);

        let prov = ProveedorMock::nuevo();
        regenerar_stale_con_ia(&mut l, &mut alm, &prov).unwrap();
        assert!(!l.capa(id_b).unwrap().esta_stale());

        let h_b = l.capa(id_b).unwrap().contenido;
        let bytes = alm.obtener(h_b).unwrap();
        assert_eq!(bytes.len(), 4 * 4 * 4);
    }

    #[test]
    fn regenerar_stale_con_ia_dimension_invalida_es_error() {
        // Si el proveedor devuelve un tamaño distinto al lienzo, fallamos.
        // Construimos un Generar con dims mentidas vs lienzo.
        use pixel_verbo_mock::ProveedorMock;

        let mut alm = AlmacenEnMemoria::nuevo();
        let h_a = alm.insertar(buffer_solido(2, 2, [0, 0, 0, 255]));

        let mut l = Lienzo::nuevo(2, 2);
        let a = Capa::raster("a", h_a);
        let id_a = a.id;
        l.apilar(a);

        let op = OpPixel::Generar {
            prompt: "x".into(),
            ancho: 8,
            alto: 8, // ≠ lienzo
        };
        let trans = transformacion_ia(MODELO_MOCK, &op);
        let b = Capa::derivada("ia-malformada", id_a, trans, [0u8; 32]);
        l.apilar(b);

        let prov = ProveedorMock::nuevo();
        let err = regenerar_stale_con_ia(&mut l, &mut alm, &prov).unwrap_err();
        assert!(matches!(err, Error::IaDimension { .. }));
    }

    #[test]
    fn params_postcard_roundtrip() {
        let op = OpPixel::Restyle {
            prompt: "frío".into(),
        };
        let trans = transformacion_ia(MODELO_MOCK, &op);
        let params = match &trans {
            TransformacionPixel::Ia { params, .. } => params.clone(),
            _ => panic!(),
        };
        let dec = op_pixel_desde_params(&params).unwrap();
        assert_eq!(dec, op);
    }
}
