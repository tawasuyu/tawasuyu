//! `tullpu-ops` — el catálogo de operaciones locales del editor.
//!
//! Cada `OpLocal` declarada en `tullpu-core` se ejecuta aquí: una función
//! pura que toma un buffer Rgba8 `(W*H*4)` y devuelve uno nuevo del mismo
//! tamaño. La impureza (caché, marca *stale → fresca*, escritura al almacén)
//! la maneja el orquestador [`regenerar_stale`], que recorre el lienzo en
//! orden topológico y ejecuta cada capa derivada cuya madre está fresca.
//!
//! El catálogo arranca con operaciones deterministas en proceso. Las ops IA
//! (`TransformacionPixel::Ia`) se delegarán a `pixel-verbo-daemon` por
//! socket Unix —fase 5 del SDD—; este crate las reconoce pero no las
//! ejecuta.

#![forbid(unsafe_code)]

use image::{ImageBuffer, RgbaImage};
use tullpu_core::{
    Frescura, Hash, Lienzo, OpLocal, OrigenCapa, TransformacionPixel,
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
}
