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
        OpLocal::Curvas { puntos } => {
            let lut = lut_curva(puntos);
            mapear_rgb(src, |c| lut[c as usize])
        }
    };
    Ok(salida)
}

/// Construye la LUT de 256 entradas de una curva tonal. Re-exportada desde
/// `tullpu_core::pixel`, donde vive para que la comparta el compositor (capas
/// de ajuste de curvas en vivo) sin crear el ciclo `ops ↔ render`. El frontend
/// la usa para dibujar la curva; [`aplicar_op_local`] para mapear los canales.
pub use tullpu_core::pixel::lut_curva;

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
//  Rasterización de capas vectoriales
// =============================================================================

/// Rasteriza una capa vectorial a un buffer Rgba8 `(w*h*4)` de **alfa recta**
/// (no premultiplicada), del tamaño del lienzo, con relleno y/o trazo
/// anti-aliased (tiny-skia). El path va en coords-imagen (px, origen
/// arriba-izquierda). Es el análogo vectorial de la rasterización de texto: su
/// salida vive en `Capa::contenido` y el compositor la trata como píxeles.
///
/// Un path vacío o un lienzo de área cero devuelven un buffer transparente.
pub fn rasterizar_vector(params: &tullpu_core::ParamsVector, w: u32, h: u32) -> Vec<u8> {
    let n = (w as usize) * (h as usize);
    if n == 0 {
        return Vec::new();
    }
    let mut pixmap = match tiny_skia::Pixmap::new(w, h) {
        Some(p) => p,
        None => return vec![0u8; n * 4],
    };

    if let Some(path) = construir_path(&params.comandos) {
        let regla = match params.regla {
            tullpu_core::ReglaRelleno::ParImpar => tiny_skia::FillRule::EvenOdd,
            tullpu_core::ReglaRelleno::NoCero => tiny_skia::FillRule::Winding,
        };
        // Relleno: el gradiente (si lo hay) tiene prioridad sobre el sólido.
        if let Some(grad) = &params.gradiente {
            if let Some(shader) = shader_gradiente(grad) {
                let mut paint = tiny_skia::Paint::default();
                paint.shader = shader;
                paint.anti_alias = true;
                pixmap.fill_path(&path, &paint, regla, tiny_skia::Transform::identity(), None);
            }
        } else if let Some(c) = params.relleno {
            let mut paint = tiny_skia::Paint::default();
            paint.set_color(color_skia(c));
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, regla, tiny_skia::Transform::identity(), None);
        }
        // Trazo (contorno).
        if let (Some(c), true) = (params.trazo, params.ancho_trazo > 0.0) {
            let mut paint = tiny_skia::Paint::default();
            paint.set_color(color_skia(c));
            paint.anti_alias = true;
            let mut stroke = tiny_skia::Stroke::default();
            stroke.width = params.ancho_trazo;
            pixmap.stroke_path(&path, &paint, &stroke, tiny_skia::Transform::identity(), None);
        }
    }

    // tiny-skia guarda RGBA premultiplicado; tullpu trabaja en alfa recta.
    let mut out = Vec::with_capacity(n * 4);
    for px in pixmap.pixels() {
        let c = px.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out
}

fn color_skia(c: [u8; 4]) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Construye el shader tiny-skia de un [`tullpu_core::Gradiente`]. `None` si las
/// paradas son degeneradas (tiny-skia exige ≥ 2 y radio > 0).
fn shader_gradiente(g: &tullpu_core::Gradiente) -> Option<tiny_skia::Shader<'static>> {
    use tullpu_core::Gradiente;
    let stops = |paradas: &[(f32, [u8; 4])]| -> Vec<tiny_skia::GradientStop> {
        paradas
            .iter()
            .map(|(o, c)| tiny_skia::GradientStop::new(*o, color_skia(*c)))
            .collect()
    };
    match g {
        Gradiente::Lineal { x1, y1, x2, y2, paradas } => tiny_skia::LinearGradient::new(
            tiny_skia::Point::from_xy(*x1, *y1),
            tiny_skia::Point::from_xy(*x2, *y2),
            stops(paradas),
            tiny_skia::SpreadMode::Pad,
            tiny_skia::Transform::identity(),
        ),
        Gradiente::Radial { cx, cy, r, paradas } => tiny_skia::RadialGradient::new(
            tiny_skia::Point::from_xy(*cx, *cy),
            tiny_skia::Point::from_xy(*cx, *cy),
            *r,
            stops(paradas),
            tiny_skia::SpreadMode::Pad,
            tiny_skia::Transform::identity(),
        ),
    }
}

/// Traduce los comandos de tullpu a un `tiny_skia::Path`. `None` si el path
/// queda vacío o degenerado (tiny-skia rechaza paths sin área de trazado).
fn construir_path(comandos: &[tullpu_core::ComandoPath]) -> Option<tiny_skia::Path> {
    use tullpu_core::ComandoPath as C;
    let mut pb = tiny_skia::PathBuilder::new();
    for cmd in comandos {
        match *cmd {
            C::MoverA { x, y } => pb.move_to(x, y),
            C::LineaA { x, y } => pb.line_to(x, y),
            C::CurvaA { c1x, c1y, c2x, c2y, x, y } => pb.cubic_to(c1x, c1y, c2x, c2y, x, y),
            C::Cerrar => pb.close(),
        }
    }
    pb.finish()
}

// =============================================================================
//  Operaciones booleanas entre dos buffers (a nivel alfa)
// =============================================================================

/// Operación booleana entre dos formas (buffers Rgba8 del mismo tamaño).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpBooleano {
    /// Unión: `a` sobre `b` (src-over). La silueta combinada de ambas.
    Union,
    /// Intersección: sólo donde ambas tienen cobertura; color de `a`.
    Interseccion,
    /// Resta `a − b`: `a` allí donde `b` **no** cubre.
    Resta,
}

/// Combina dos buffers Rgba8 `(w*h*4)` por una [`OpBooleano`] a nivel de alfa,
/// devolviendo un buffer nuevo (alfa recta). Es el modo "destructivo" de
/// combinar formas: el resultado es raster. `a` es la forma de arriba.
pub fn booleano(a: &[u8], b: &[u8], op: OpBooleano) -> Vec<u8> {
    let n = a.len().min(b.len()) / 4;
    let mut out = vec![0u8; n * 4];
    for i in 0..n {
        let j = i * 4;
        let (ar, ag, ab, aa) = (a[j] as f32, a[j + 1] as f32, a[j + 2] as f32, a[j + 3] as f32 / 255.0);
        let (br, bg, bb, ba) = (b[j] as f32, b[j + 1] as f32, b[j + 2] as f32, b[j + 3] as f32 / 255.0);
        let (r, g, bl, al) = match op {
            OpBooleano::Union => {
                // a-over-b: alfa y color compuestos src-over.
                let oa = aa + ba * (1.0 - aa);
                if oa <= f32::EPSILON {
                    (0.0, 0.0, 0.0, 0.0)
                } else {
                    (
                        (ar * aa + br * ba * (1.0 - aa)) / oa,
                        (ag * aa + bg * ba * (1.0 - aa)) / oa,
                        (ab * aa + bb * ba * (1.0 - aa)) / oa,
                        oa,
                    )
                }
            }
            OpBooleano::Interseccion => (ar, ag, ab, aa * ba),
            OpBooleano::Resta => (ar, ag, ab, aa * (1.0 - ba)),
        };
        out[j] = r.round().clamp(0.0, 255.0) as u8;
        out[j + 1] = g.round().clamp(0.0, 255.0) as u8;
        out[j + 2] = bl.round().clamp(0.0, 255.0) as u8;
        out[j + 3] = (al * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
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

    // ----- rasterización vectorial ------------------------------------------

    use tullpu_core::ParamsVector;

    fn en(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        px(buf, (y * w + x) as usize)
    }

    #[test]
    fn rectangulo_rellena_adentro_y_deja_afuera_transparente() {
        let (w, h) = (40, 40);
        let p = ParamsVector::rectangulo(10.0, 10.0, 20.0, 20.0, [200, 30, 40, 255]);
        let buf = rasterizar_vector(&p, w, h);
        assert_eq!(buf.len(), (w * h * 4) as usize);
        // Centro (20,20) dentro del rect → color de relleno opaco.
        assert_eq!(en(&buf, w, 20, 20), [200, 30, 40, 255]);
        // Esquina (2,2) fuera → transparente.
        assert_eq!(en(&buf, w, 2, 2), [0, 0, 0, 0]);
    }

    #[test]
    fn elipse_rellena_el_centro_y_no_las_esquinas() {
        let (w, h) = (40, 40);
        let p = ParamsVector::elipse(20.0, 20.0, 15.0, 15.0, [20, 180, 90, 255]);
        let buf = rasterizar_vector(&p, w, h);
        assert_eq!(en(&buf, w, 20, 20), [20, 180, 90, 255]); // centro
        assert_eq!(en(&buf, w, 1, 1)[3], 0); // esquina fuera del círculo
    }

    #[test]
    fn salida_es_alfa_recta_no_premultiplicada() {
        // Relleno semitransparente: el RGB debe quedar pleno (alfa recta), no
        // multiplicado por el alfa (que lo oscurecería).
        let (w, h) = (10, 10);
        let p = ParamsVector::rectangulo(0.0, 0.0, 10.0, 10.0, [255, 255, 255, 128]);
        let buf = rasterizar_vector(&p, w, h);
        let c = en(&buf, w, 5, 5);
        assert!(c[0] >= 254 && c[1] >= 254 && c[2] >= 254, "RGB debería ser ~255, fue {c:?}");
        assert!((c[3] as i32 - 128).abs() <= 1, "alfa ~128, fue {}", c[3]);
    }

    #[test]
    fn gradiente_lineal_varia_de_un_color_al_otro() {
        use tullpu_core::Gradiente;
        let (w, h) = (32, 8);
        let mut p = ParamsVector::rectangulo(0.0, 0.0, w as f32, h as f32, [0, 0, 0, 255]);
        // Rojo en x=0 → azul en x=w.
        p.gradiente = Some(Gradiente::lineal(0.0, 0.0, w as f32, 0.0, [255, 0, 0, 255], [0, 0, 255, 255]));
        let buf = rasterizar_vector(&p, w, h);
        let izq = en(&buf, w, 1, 4);
        let der = en(&buf, w, w - 2, 4);
        assert!(izq[0] > 200 && izq[2] < 60, "izquierda roja, fue {izq:?}");
        assert!(der[2] > 200 && der[0] < 60, "derecha azul, fue {der:?}");
    }

    #[test]
    fn booleano_union_interseccion_resta() {
        // Dos píxeles: A opaco rojo, B opaco verde (mismo lugar).
        let a = [255, 0, 0, 255];
        let b = [0, 255, 0, 255];
        // Unión: A sobre B → rojo opaco.
        let u = booleano(&a, &b, OpBooleano::Union);
        assert_eq!(px(&u, 0), [255, 0, 0, 255]);
        // Intersección: ambos opacos → color A, alfa pleno.
        let i = booleano(&a, &b, OpBooleano::Interseccion);
        assert_eq!(px(&i, 0), [255, 0, 0, 255]);
        // Resta A−B: B opaco tapa todo → alfa 0.
        let r = booleano(&a, &b, OpBooleano::Resta);
        assert_eq!(px(&r, 0)[3], 0);

        // B transparente: intersección vacía, resta = A.
        let bt = [0, 255, 0, 0];
        assert_eq!(px(&booleano(&a, &bt, OpBooleano::Interseccion), 0)[3], 0);
        assert_eq!(px(&booleano(&a, &bt, OpBooleano::Resta), 0), [255, 0, 0, 255]);
    }

    #[test]
    fn vector_se_compone_sobre_el_fondo() {
        // Rasteriza un vector, lo mete al almacén y compone sobre un fondo.
        let (w, h) = (24, 24);
        let mut alm = AlmacenEnMemoria::nuevo();
        let fondo = alm.insertar(buffer_solido(w, h, [0, 0, 0, 255]));
        let p = ParamsVector::rectangulo(6.0, 6.0, 12.0, 12.0, [255, 0, 0, 255]);
        let raster = alm.insertar(rasterizar_vector(&p, w, h));

        let mut l = Lienzo::nuevo(w, h);
        l.apilar(Capa::raster("fondo", fondo));
        l.apilar(Capa::vector("forma", raster, p));

        let img = tullpu_render::componer(&l, &alm).unwrap();
        // Dentro del rect: rojo. Fuera: negro del fondo.
        assert_eq!(en(img.as_raw(), w, 12, 12), [255, 0, 0, 255]);
        assert_eq!(en(img.as_raw(), w, 1, 1), [0, 0, 0, 255]);
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
    fn curva_identidad_es_identidad() {
        // La diagonal (0,0)→(1,1) debe dejar la LUT en lut[i]=i, así que
        // mapear cualquier color lo devuelve intacto.
        let lut = lut_curva(&[(0.0, 0.0), (1.0, 1.0)]);
        for i in 0..256 {
            assert_eq!(lut[i], i as u8, "lut[{i}] no es identidad");
        }
        let src = buffer_solido(1, 1, [10, 128, 240, 77]);
        let out = aplicar_op_local(
            &OpLocal::Curvas {
                puntos: vec![(0.0, 0.0), (1.0, 1.0)],
            },
            &src,
            1,
            1,
        )
        .unwrap();
        assert_eq!(px(&out, 0), [10, 128, 240, 77]); // alfa intacto también.
    }

    #[test]
    fn curva_menos_de_dos_puntos_cae_a_identidad() {
        let lut = lut_curva(&[(0.5, 0.9)]);
        for i in 0..256 {
            assert_eq!(lut[i], i as u8);
        }
        // Vacío también.
        let lut0 = lut_curva(&[]);
        assert_eq!(lut0[200], 200);
    }

    #[test]
    fn curva_respeta_extremos_y_punto_medio_levantado() {
        // Curva que sube el medio: (0,0)-(0.5,0.75)-(1,1). Extremos exactos,
        // el centro queda por encima de la diagonal.
        let lut = lut_curva(&[(0.0, 0.0), (0.5, 0.75), (1.0, 1.0)]);
        assert_eq!(lut[0], 0);
        assert_eq!(lut[255], 255);
        // En x=0.5 (idx 128) la salida ~0.75*255 ≈ 191; tolerancia por el
        // muestreo de la LUT.
        assert!(
            (lut[128] as i32 - 191).abs() <= 4,
            "centro={} esperado ~191",
            lut[128]
        );
        assert!(lut[128] > 128, "el medio debe quedar levantado");
    }

    #[test]
    fn curva_es_monotona_no_decreciente_y_dentro_de_rango() {
        // Una curva en S de contraste no debe rebotar fuera de [0,1] ni
        // invertir su pendiente — eso valida la corrección Fritsch–Carlson.
        let lut = lut_curva(&[(0.0, 0.0), (0.25, 0.15), (0.75, 0.85), (1.0, 1.0)]);
        for i in 1..256 {
            assert!(
                lut[i] >= lut[i - 1],
                "no monótona en {i}: {} < {}",
                lut[i],
                lut[i - 1]
            );
        }
        // Sombras bajadas, luces subidas respecto a la diagonal.
        assert!(lut[64] < 64, "sombras={} deberían bajar", lut[64]);
        assert!(lut[192] > 192, "luces={} deberían subir", lut[192]);
    }

    #[test]
    fn curva_clampa_fuera_de_dominio() {
        // Puntos que no cubren los extremos del eje: dominio [0.3, 0.7].
        // Antes de 0.3 la salida se aplana a y(0.3)=0.2; después de 0.7 a
        // y(0.7)=0.9.
        let lut = lut_curva(&[(0.3, 0.2), (0.7, 0.9)]);
        let y_lo = (0.2_f32 * 255.0).round() as u8;
        let y_hi = (0.9_f32 * 255.0).round() as u8;
        assert_eq!(lut[0], y_lo);
        assert_eq!(lut[10], y_lo);
        assert_eq!(lut[255], y_hi);
        assert_eq!(lut[250], y_hi);
    }

    #[test]
    fn curva_puntos_desordenados_se_ordenan() {
        // Los mismos puntos en cualquier orden producen la misma LUT.
        let a = lut_curva(&[(1.0, 1.0), (0.0, 0.0), (0.5, 0.3)]);
        let b = lut_curva(&[(0.5, 0.3), (1.0, 1.0), (0.0, 0.0)]);
        assert_eq!(a, b);
    }

    #[test]
    fn curva_invertida_baja_todo() {
        // (0,1)→(1,0) es una inversión lineal: lut[i] ≈ 255-i.
        let lut = lut_curva(&[(0.0, 1.0), (1.0, 0.0)]);
        assert_eq!(lut[0], 255);
        assert_eq!(lut[255], 0);
        assert!((lut[128] as i32 - 127).abs() <= 2);
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
