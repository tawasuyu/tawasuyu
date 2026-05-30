//! Composición y derivados visuales de la app `tullpu`: recomponer el
//! lienzo a un `peniko::Image`, regenerar las capas stale, sincronizar la
//! cache de thumbnails y calcular el histograma RGB del composite.
//!
//! Behavior-preserving split de `main.rs` — sin cambios funcionales.

use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
use tullpu_core::{Hash, Lienzo};
use tullpu_ops::regenerar_stale_con_ia;
use tullpu_render::{componer, FuenteBuffers};

use crate::model::*;

pub(crate) fn recomponer(l: &Lienzo, alm: &impl FuenteBuffers) -> Option<Image> {
    let img = componer(l, alm).ok()?;
    let (w, h) = (img.width(), img.height());
    let blob = Blob::from(img.into_raw());
    Some(Image::new(blob, ImageFormat::Rgba8, w, h))
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
        .map(|img| histograma_rgb(img.data.data()));
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
    Some(Image::new(
        Blob::from(thumb.into_raw()),
        ImageFormat::Rgba8,
        tw,
        th,
    ))
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
