//! Composición y derivados visuales de la app `tullpu`: recomponer el
//! lienzo a un `peniko::Image`, regenerar las capas stale, sincronizar la
//! cache de thumbnails y calcular el histograma RGB del composite.
//!
//! Behavior-preserving split de `main.rs` — sin cambios funcionales.

use llimphi_ui::llimphi_raster::peniko::{
    Blob, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
};
use std::sync::OnceLock;

use tullpu_core::{Hash, Lienzo};
use tullpu_ops::regenerar_stale_con_ia;
use tullpu_render::{componer, FuenteBuffers};
use tullpu_render_gpu::Compositor;

use crate::model::*;

/// Compositor GPU del proceso, inicializado una sola vez. `None` si la máquina
/// no tiene adaptador GPU — en ese caso siempre se compone en CPU. La inicial-
/// ización (adaptador + dispositivo + shader) es cara; cada composición no.
fn compositor_gpu() -> Option<&'static Compositor> {
    static GPU: OnceLock<Option<Compositor>> = OnceLock::new();
    GPU.get_or_init(|| match Compositor::nuevo() {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("tullpu: compositor GPU no disponible ({e}); uso CPU");
            None
        }
    })
    .as_ref()
}

pub(crate) fn recomponer(l: &Lienzo, alm: &impl FuenteBuffers) -> Option<Image> {
    // Camino GPU con fallback transparente al compositor CPU: el GPU rechaza
    // (NoSoportado) los lienzos con capas de ajuste o modo Disolver, y cualquier
    // error de dispositivo cae también a CPU sin que el usuario lo note.
    let img = match compositor_gpu().and_then(|g| g.componer(l, alm).ok()) {
        Some(img) => img,
        None => componer(l, alm).ok()?,
    };
    let (w, h) = (img.width(), img.height());
    let blob = Blob::from(img.into_raw());
    Some(Image::new(ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: w, height: h }))
}

pub(crate) fn aplicar_y_recomponer(model: &mut Model) {
    match regenerar_stale_con_ia(
        &mut model.lienzo,
        &mut model.almacen,
        model.proveedor.as_ref(),
    ) {
        Ok(regen) => {
            model.estado = if regen.is_empty() {
                "listo".into()
            } else {
                format!("regeneradas {}", regen.len())
            };
        }
        Err(e) => {
            model.estado = format!("error ops: {e}");
        }
    }
    match recomponer(&model.lienzo, &model.almacen) {
        Some(img) => model.imagen = Some(img),
        None => model.estado = "error compositor".into(),
    }
    // Recompute histograma desde el nuevo composite. Es O(W*H) — para
    // un 4 MP el costo es despreciable comparado con `componer`.
    model.histograma = model
        .imagen
        .as_ref()
        .map(|img| histograma_rgb(img.image.data.data()));
    sincronizar_thumbs(model);
}

/// Asegura que cada capa del lienzo tenga su thumbnail en el cache, y
/// descarta entries cuyos hashes ya no están en uso. La regeneración por
/// op (vía `regenerar_stale_con_ia`) cambia `Capa.contenido` para las
/// derivadas; el hash nuevo entra al cache, el viejo se barre.
pub(crate) fn sincronizar_thumbs(model: &mut Model) {
    let lienzo_w = model.lienzo.width;
    let lienzo_h = model.lienzo.height;
    let vivos: std::collections::HashSet<Hash> =
        model.lienzo.capas.iter().map(|c| c.contenido).collect();
    model.thumbs.retain(|h, _| vivos.contains(h));
    for capa in &model.lienzo.capas {
        if model.thumbs.contains_key(&capa.contenido) {
            continue;
        }
        if let Some(img) = thumbnail_de_buffer(capa.contenido, lienzo_w, lienzo_h, &model.almacen)
        {
            model.thumbs.insert(capa.contenido, img);
        }
    }
    // Espejo para máscaras: thumb gris de cada buffer de 1 canal. Sólo
    // las capas con máscara aportan hashes; las demás no tocan el cache.
    let mascaras_vivas: std::collections::HashSet<Hash> =
        model.lienzo.capas.iter().filter_map(|c| c.mascara).collect();
    model.thumbs_mascara.retain(|h, _| mascaras_vivas.contains(h));
    for hash in mascaras_vivas {
        if model.thumbs_mascara.contains_key(&hash) {
            continue;
        }
        if let Some(img) = thumbnail_de_mascara(hash, lienzo_w, lienzo_h, &model.almacen) {
            model.thumbs_mascara.insert(hash, img);
        }
    }
}

/// Thumbnail de un buffer de máscara de 1 canal (`w*h` bytes): cada valor
/// `v` se expande a un píxel gris opaco `(v,v,v,255)` y luego se reescala
/// como cualquier thumb Rgba8. Devuelve `None` si el hash no está en el
/// almacén o el tamaño no cuadra con `w*h`.
pub(crate) fn thumbnail_de_mascara(
    hash: Hash,
    w: u32,
    h: u32,
    fuente: &impl FuenteBuffers,
) -> Option<Image> {
    let buf = fuente.obtener(hash)?;
    if buf.len() != (w as usize) * (h as usize) {
        return None;
    }
    let mut rgba = Vec::with_capacity(buf.len() * 4);
    for &v in buf {
        rgba.extend_from_slice(&[v, v, v, 255]);
    }
    let rgba = image::RgbaImage::from_raw(w, h, rgba)?;
    let thumb = image::imageops::thumbnail(&rgba, THUMB_LADO, THUMB_LADO);
    let (tw, th) = (thumb.width(), thumb.height());
    Some(Image::new(ImageData { data: Blob::from(thumb.into_raw()), format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: tw, height: th }))
}

/// Construye un thumbnail `peniko::Image` de lado máximo `THUMB_LADO`
/// preservando aspect ratio. `nearest` es suficiente para 22 px y mantiene
/// el costo cercano a cero — un PSD de 30 capas son ~30 reescalados de
/// imagen grande a 22 px, lineal en píxeles totales.
pub(crate) fn thumbnail_de_buffer(
    hash: Hash,
    w: u32,
    h: u32,
    fuente: &impl FuenteBuffers,
) -> Option<Image> {
    let buf = fuente.obtener(hash)?;
    let rgba = image::RgbaImage::from_raw(w, h, buf.to_vec())?;
    let thumb = image::imageops::thumbnail(&rgba, THUMB_LADO, THUMB_LADO);
    let (tw, th) = (thumb.width(), thumb.height());
    Some(Image::new(ImageData { data: Blob::from(thumb.into_raw()), format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: tw, height: th }))
}

/// Cuenta cuántos píxeles tiene cada valor 0..255 en cada canal RGB de
/// un buffer Rgba8. El alfa se ignora (no se incluye como canal y no se
/// usa para ponderar — un píxel con alfa=0 cuenta igual que uno
/// opaco). Devuelve `[[count_r, count_g, count_b]; 256]` pero
/// estructurado al revés para acceso eficiente: `out[c][v]` = cantidad
/// de píxeles donde el canal `c` (0=R, 1=G, 2=B) vale `v`. Pura.
pub(crate) fn histograma_rgb(data: &[u8]) -> [[u32; 256]; 3] {
    let mut out = [[0u32; 256]; 3];
    for px in data.chunks_exact(4) {
        out[0][px[0] as usize] += 1;
        out[1][px[1] as usize] += 1;
        out[2][px[2] as usize] += 1;
    }
    out
}
