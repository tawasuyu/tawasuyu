//! Operaciones sobre capas y buffers de la app `tullpu`: agregar/combinar/
//! aplanar capas, recortes del lienzo, transformaciones del rect de
//! selección (limpiar/rellenar/copiar/cortar/pegar/duplicar), rotación
//! de buffers y lienzo, bounding box, ajuste de parámetros y etiquetas.
//!
//! Behavior-preserving split de `main.rs` — sin cambios funcionales.

use std::path::Path;

use tullpu_core::{
    Capa, Frescura, Lienzo, OpLocal, OrigenCapa, TransformacionPixel,
};
use tullpu_render::FuenteBuffers;
use uuid::Uuid;

use crate::carga::{ajustar_a_lienzo, cargar_png};
use crate::compose::aplicar_y_recomponer;
use crate::model::*;

pub(crate) fn op_etiqueta(op: &OpLocal) -> &'static str {
    match op {
        OpLocal::Invertir => "invertir",
        OpLocal::Brillo { .. } => "brillo",
        OpLocal::Contraste { .. } => "contraste",
        OpLocal::Niveles { .. } => "niveles",
        OpLocal::Blur { .. } => "blur",
        OpLocal::Opacidad { .. } => "opacidad",
        OpLocal::Saturacion { .. } => "saturación",
        OpLocal::Tonalidad { .. } => "tonalidad",
        OpLocal::EspejarHorizontal => "espejar ↔",
        OpLocal::EspejarVertical => "espejar ↕",
    }
}

/// Carga `path` como PNG/JPEG, lo ajusta al tamaño del lienzo y apila la
/// capa raster nueva. Se mete justo encima de la capa seleccionada (o al
/// tope si no hay selección). En éxito refresca compositor + thumbs y
/// devuelve `true` (para que el caller decida si snapshotear); en fallo deja
/// el lienzo intacto, escribe el error en el estado y devuelve `false`.
pub(crate) fn agregar_capa_desde_archivo(model: &mut Model, path: &Path) -> bool {
    let Some((w, h, bytes)) = cargar_png(path) else {
        model.estado = format!("error decodificando {}", path.display());
        return false;
    };
    let dst_w = model.lienzo.width;
    let dst_h = model.lienzo.height;
    let Some(buffer) = ajustar_a_lienzo(bytes, w, h, dst_w, dst_h) else {
        model.estado = format!("error ajustando {}×{} → {}×{}", w, h, dst_w, dst_h);
        return false;
    };
    let hash = model.almacen.insertar(buffer);
    let nombre = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("imagen")
        .to_string();
    let nueva = Capa::raster(nombre.clone(), hash);
    let nuevo_id = nueva.id;
    // Inserción justo encima de la seleccionada: el panel pinta top→fondo,
    // así que "encima" = índice mayor en `capas`. Si no hay selección o no
    // se encuentra, apilamos al tope.
    match model
        .seleccionada
        .and_then(|id| model.lienzo.capas.iter().position(|c| c.id == id))
    {
        Some(idx) => model.lienzo.capas.insert(idx + 1, nueva),
        None => model.lienzo.apilar(nueva),
    }
    model.seleccionada = Some(nuevo_id);
    let ajuste = if w == dst_w && h == dst_h {
        String::new()
    } else {
        format!(" (ajustada {}×{} → {}×{})", w, h, dst_w, dst_h)
    };
    aplicar_y_recomponer(model);
    model.estado = format!("agregada capa '{}'{}", nombre, ajuste);
    true
}


/// Construye un buffer Rgba8 de `w × h` lleno con `rgba`. Pura. Salvo
/// errores de overflow (improbables en tamaños sanos), el `w * h * 4`
/// nunca pasa de unos MB para los lienzos típicos de tullpu.
pub(crate) fn buffer_relleno(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut v = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for _ in 0..(w as usize * h as usize) {
        v.extend_from_slice(&rgba);
    }
    v
}

/// Apila una capa raster nueva del tamaño del lienzo llena con el
/// color leído por el cuentagotas (o `RELLENO_DEFAULT` si todavía no
/// hay color). Devuelve siempre `true` — no hay vía de error (el buffer
/// se construye en RAM, sin I/O). Inserción justo encima de la
/// seleccionada, mismo contrato que `agregar_capa_desde_archivo`.
pub(crate) fn agregar_capa_relleno(model: &mut Model) -> bool {
    let rgba = model.color_picked.unwrap_or(RELLENO_DEFAULT);
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let buffer = buffer_relleno(w, h, rgba);
    let hash = model.almacen.insertar(buffer);
    let nombre = format!(
        "relleno #{:02X}{:02X}{:02X}",
        rgba[0], rgba[1], rgba[2]
    );
    let nueva = Capa::raster(nombre.clone(), hash);
    let nuevo_id = nueva.id;
    match model
        .seleccionada
        .and_then(|id| model.lienzo.capas.iter().position(|c| c.id == id))
    {
        Some(idx) => model.lienzo.capas.insert(idx + 1, nueva),
        None => model.lienzo.apilar(nueva),
    }
    model.seleccionada = Some(nuevo_id);
    aplicar_y_recomponer(model);
    model.estado = format!("agregada '{}'", nombre);
    true
}

/// Combina la capa `id` con la que está directamente debajo (idx menor)
/// en una sola capa raster. La merge respeta blend + opacidad + visible
/// de ambas: arma un mini-`Lienzo` con sólo ese par (abajo primero,
/// arriba después — `componer` itera fondo→tope), compone, mete el
/// buffer al almacén content-addressed y reemplaza el par por una
/// `Capa::raster` nueva con defaults (Normal/1.0/visible). Las hijas
/// derivadas que apuntaban a cualquiera de las dos quedan huérfanas —
/// `regenerar_stale_con_ia` fallará con `BufferFaltante` (mismo
/// comportamiento que `Eliminar`). Devuelve `false` si la capa ya está
/// en el fondo (no hay nada debajo para combinar) o si no se encuentra
/// la `id`; el caller lo usa para decidir si snapshotear.
pub(crate) fn combinar_capa_abajo(model: &mut Model, id: Uuid) -> bool {
    let Some(idx) = model.lienzo.capas.iter().position(|c| c.id == id) else {
        return false;
    };
    if idx == 0 {
        model.estado = "no hay capa debajo para combinar".into();
        return false;
    }
    // Capas para el mini-Lienzo. Las clonamos: las originales se
    // borran del Lienzo más abajo. `apilar` consume por valor.
    let abajo = model.lienzo.capas[idx - 1].clone();
    let arriba = model.lienzo.capas[idx].clone();

    let mut mini = Lienzo::nuevo(model.lienzo.width, model.lienzo.height);
    mini.apilar(abajo.clone());
    mini.apilar(arriba.clone());

    let img = match tullpu_render::componer(&mini, &model.almacen) {
        Ok(im) => im,
        Err(e) => {
            // Errores típicos: BufferFaltante (alguna era derivada stale
            // que nunca se regeneró). Dejamos el lienzo intacto.
            model.estado = format!("merge falló: {e:?}");
            return false;
        }
    };
    let buffer = img.into_raw();
    let hash = model.almacen.insertar(buffer);
    let nombre = format!("{} ⊕ {}", abajo.nombre, arriba.nombre);
    let nueva = Capa::raster(nombre.clone(), hash);
    let nuevo_id = nueva.id;
    // Quitamos la de arriba primero (idx mayor) para no shiftear índices
    // antes de tocar la de abajo. Después reemplazamos la de abajo por
    // la merged.
    model.lienzo.capas.remove(idx);
    model.lienzo.capas[idx - 1] = nueva;
    model.seleccionada = Some(nuevo_id);
    aplicar_y_recomponer(model);
    model.estado = format!("combinada '{}'", nombre);
    true
}

/// Mutador in-place del parámetro de una capa derivada con `OpLocal`
/// parametrizable. Aplica `dv` (delta en unidades del parámetro,
/// emitido por el slider) al campo correspondiente con clamp al rango
/// visible. Devuelve `false` si la capa no se encuentra, no es una
/// derivada local, o el `param` no concuerda con la variante de op —
/// en esos casos el caller no recompone ni snapshotea. Marca la capa
/// `Stale` y propaga al cono descendiente (toda hija con esta como
/// madre transitiva se invalida).
pub(crate) fn ajustar_parametro_derivada(
    model: &mut Model,
    id: Uuid,
    param: ParametroSlider,
    dv: f32,
) -> bool {
    let Some(capa) = model.lienzo.capa_mut(id) else {
        return false;
    };
    let OrigenCapa::Derivada {
        op: TransformacionPixel::Local(op),
        estado,
        ..
    } = &mut capa.origen
    else {
        return false;
    };
    let cambio = match (param, op) {
        (ParametroSlider::BrilloDelta, OpLocal::Brillo { delta }) => {
            *delta = (*delta + dv).clamp(-1.0, 1.0);
            true
        }
        (ParametroSlider::ContrasteFactor, OpLocal::Contraste { factor }) => {
            *factor = (*factor + dv).clamp(0.0, 3.0);
            true
        }
        (ParametroSlider::SaturacionFactor, OpLocal::Saturacion { factor }) => {
            *factor = (*factor + dv).clamp(0.0, 3.0);
            true
        }
        (ParametroSlider::TonalidadGrados, OpLocal::Tonalidad { grados }) => {
            // Tonalidad es periódica; clamp visual a [-180, 180] para
            // que el slider tenga rango fijo, pero el módulo lo aplica
            // `aplicar_op_local` (rem_euclid).
            *grados = (*grados + dv).clamp(-180.0, 180.0);
            true
        }
        (ParametroSlider::BlurRadio, OpLocal::Blur { radio }) => {
            *radio = (*radio + dv).clamp(0.0, 20.0);
            true
        }
        (ParametroSlider::OpacidadFactor, OpLocal::Opacidad { factor }) => {
            *factor = (*factor + dv).clamp(0.0, 1.0);
            true
        }
        // Niveles tiene 3 campos; mutamos uno por evento. Permitimos que
        // entrada_min y entrada_max se crucen — `aplicar_op_local` protege
        // de división por cero con `(max - min).max(1e-6)`, y cruzarlos
        // es un truco válido (binarización por intervalo invertido).
        (ParametroSlider::NivelesEntradaMin, OpLocal::Niveles { entrada_min, .. }) => {
            *entrada_min = (*entrada_min + dv).clamp(0.0, 1.0);
            true
        }
        (ParametroSlider::NivelesEntradaMax, OpLocal::Niveles { entrada_max, .. }) => {
            *entrada_max = (*entrada_max + dv).clamp(0.0, 1.0);
            true
        }
        (ParametroSlider::NivelesGamma, OpLocal::Niveles { gamma, .. }) => {
            // Gamma > 0 es necesario; el rango usable cubre el clásico
            // [0.1, 4.0] de Photoshop (curva extrema arriba/abajo).
            *gamma = (*gamma + dv).clamp(0.1, 4.0);
            true
        }
        // Param solicitado no coincide con la op de la capa — no muta.
        _ => false,
    };
    if !cambio {
        return false;
    }
    *estado = Frescura::Stale;
    model.lienzo.propagar_stale(id);
    true
}

/// Calcula el bounding box (half-open `(x0, y0, x1, y1)`) de los píxeles
/// con alfa > 0 en un buffer Rgba8 `w × h`. Devuelve `None` si todos
/// los píxeles son transparentes (no hay nada para encerrar). Pura.
pub(crate) fn bbox_no_transparente(data: &[u8], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    if w == 0 || h == 0 || data.len() != (w as usize) * (h as usize) * 4 {
        return None;
    }
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            // Alfa estricto > 0; algunos pipelines premultiplican y dejan
            // valores 1..3 en bordes — eso sigue contando como "tinta".
            if data[i + 3] > 0 {
                found = true;
                if x < min_x {
                    min_x = x;
                }
                if y < min_y {
                    min_y = y;
                }
                if x > max_x {
                    max_x = x;
                }
                if y > max_y {
                    max_y = y;
                }
            }
        }
    }
    if !found {
        return None;
    }
    // Convención half-open: x1/y1 son exclusivos. Suma 1 al máximo
    // observado para que `x1 - x0` sea el ancho efectivo.
    Some((min_x, min_y, max_x + 1, max_y + 1))
}

/// Recorta un buffer Rgba8 `w × h` al rect half-open
/// `(x0, y0, x1, y1)` y devuelve un buffer del nuevo tamaño
/// `(x1 - x0) × (y1 - y0)`. Asume el rect dentro de los bounds
/// (validación aguas arriba). Pura.
pub(crate) fn recortar_buffer(src: &[u8], w: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<u8> {
    let w = w as usize;
    let new_w = (x1 - x0) as usize;
    let new_h = (y1 - y0) as usize;
    let mut out = Vec::with_capacity(new_w * new_h * 4);
    for y in y0..y1 {
        let row_start = (y as usize * w + x0 as usize) * 4;
        let row_end = row_start + new_w * 4;
        out.extend_from_slice(&src[row_start..row_end]);
    }
    out
}


/// Recorta el lienzo entero al rect half-open `(x0, y0, x1, y1)`. La
/// estrategia espeja `rotar_lienzo`: (1) recorta el buffer de cada
/// capa al rect, inserta al almacén content-addressed; (2) actualiza
/// dims del lienzo; (3) marca todas las derivadas Stale (Blur/Niveles
/// no conmutan exacto con crop por los bordes — se regen desde la
/// madre recortada). Pre: el rect debe estar dentro de los bounds del
/// lienzo y tener área positiva (validación aguas arriba).
pub(crate) fn recortar_lienzo_a(model: &mut Model, x0: u32, y0: u32, x1: u32, y1: u32) {
    let w = model.lienzo.width;
    let new_w = x1 - x0;
    let new_h = y1 - y0;
    for capa in model.lienzo.capas.iter_mut() {
        let Some(src) = model.almacen.obtener(capa.contenido) else {
            // Derivada nunca regenerada — la regen post-recorte la
            // armará desde la madre recortada.
            continue;
        };
        let src = src.to_vec();
        let cropped = recortar_buffer(&src, w, x0, y0, x1, y1);
        let new_hash = model.almacen.insertar(cropped);
        capa.contenido = new_hash;
    }
    model.lienzo.width = new_w;
    model.lienzo.height = new_h;
    for capa in model.lienzo.capas.iter_mut() {
        if let OrigenCapa::Derivada { estado, .. } = &mut capa.origen {
            *estado = Frescura::Stale;
        }
    }
    aplicar_y_recomponer(model);
}

/// Recorta el lienzo al bbox no-transparente del compuesto. Es el
/// "Trim Transparent Pixels" de Photoshop. No-op si el lienzo está
/// vacío (todo transparente) o si ya estaba justo (bbox = lienzo
/// entero).
pub(crate) fn recortar_lienzo_a_visible(model: &mut Model) -> bool {
    let Some(img) = model.imagen.as_ref() else {
        model.estado = "no hay composite que medir".into();
        return false;
    };
    let w = img.width;
    let h = img.height;
    let bytes = img.data.data();
    let Some((x0, y0, x1, y1)) = bbox_no_transparente(bytes, w, h) else {
        model.estado = "lienzo vacío, nada que recortar".into();
        return false;
    };
    if x0 == 0 && y0 == 0 && x1 == w && y1 == h {
        model.estado = "ya está justo, nada que recortar".into();
        return false;
    }
    let new_w = x1 - x0;
    let new_h = y1 - y0;
    recortar_lienzo_a(model, x0, y0, x1, y1);
    model.estado = format!(
        "recortado a {}×{} (offset {},{})",
        new_w, new_h, x0, y0
    );
    true
}

/// Recorta el lienzo al rect de `model.seleccion`. Re-clampea contra
/// el lienzo vigente (un rotar/recortar posterior puede haber dejado
/// la selección parcial o fuera). No-op si no hay selección, si la
/// intersección con el lienzo es vacía, o si el rect cubre el lienzo
/// entero. Tras el crop limpia la selección — sus coords pertenecen
/// al coord-space anterior.
pub(crate) fn recortar_lienzo_a_seleccion(model: &mut Model) -> bool {
    let Some(rect) = model.seleccion else {
        model.estado = "no hay selección — `r` y arrastrar".into();
        return false;
    };
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let x0 = rect.x0.min(w);
    let y0 = rect.y0.min(h);
    let x1 = rect.x1.min(w);
    let y1 = rect.y1.min(h);
    if x1 <= x0 || y1 <= y0 {
        model.estado = "selección fuera del lienzo".into();
        return false;
    }
    if x0 == 0 && y0 == 0 && x1 == w && y1 == h {
        model.estado = "selección cubre todo, nada que recortar".into();
        return false;
    }
    let new_w = x1 - x0;
    let new_h = y1 - y0;
    recortar_lienzo_a(model, x0, y0, x1, y1);
    model.seleccion = None;
    model.estado = format!(
        "recortado a selección {}×{} (offset {},{})",
        new_w, new_h, x0, y0
    );
    true
}

/// Pone `[0, 0, 0, 0]` (transparente full) en cada píxel del rect
/// half-open `(x0, y0, x1, y1)` de un buffer Rgba8 `w × h`. Devuelve
/// un buffer nuevo del mismo tamaño con el resto intacto. Pura.
/// Pre: rect dentro de bounds (validación aguas arriba).
pub(crate) fn limpiar_rect_en_buffer(
    src: &[u8],
    w: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> Vec<u8> {
    let mut out = src.to_vec();
    let w = w as usize;
    for y in y0..y1 {
        let row = y as usize * w;
        for x in x0..x1 {
            let i = (row + x as usize) * 4;
            out[i] = 0;
            out[i + 1] = 0;
            out[i + 2] = 0;
            out[i + 3] = 0;
        }
    }
    out
}

/// Pone `rgba` en cada píxel del rect half-open `(x0, y0, x1, y1)` de
/// un buffer Rgba8 `w × h`. Devuelve un buffer nuevo del mismo tamaño
/// con el resto intacto. Pura. Pre: rect dentro de bounds.
pub(crate) fn rellenar_rect_en_buffer(
    src: &[u8],
    w: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    rgba: [u8; 4],
) -> Vec<u8> {
    let mut out = src.to_vec();
    let w = w as usize;
    for y in y0..y1 {
        let row = y as usize * w;
        for x in x0..x1 {
            let i = (row + x as usize) * 4;
            out[i..i + 4].copy_from_slice(&rgba);
        }
    }
    out
}

/// Construye un buffer Rgba8 `w × h` todo transparente excepto el rect
/// half-open `(x0, y0, x1, y1)`, donde copia los píxeles de `src`. Es
/// el complemento de [`limpiar_rect_en_buffer`]: aquél conserva el
/// afuera y borra el rect; éste borra el afuera y conserva el rect.
/// Devuelve también si quedó algún píxel con alfa > 0 dentro del rect
/// (`false` ⇒ nada visible que copiar). Pura. Pre: rect dentro de
/// bounds (validación aguas arriba).
pub(crate) fn extraer_rect_a_buffer(
    src: &[u8],
    w: u32,
    h: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> (Vec<u8>, bool) {
    let w = w as usize;
    let mut out = vec![0u8; w * h as usize * 4];
    let mut hubo_contenido = false;
    for y in y0..y1 {
        let row = y as usize * w;
        for x in x0..x1 {
            let i = (row + x as usize) * 4;
            out[i..i + 4].copy_from_slice(&src[i..i + 4]);
            if src[i + 3] != 0 {
                hubo_contenido = true;
            }
        }
    }
    (out, hubo_contenido)
}

/// Copia los píxeles del rect de `model.seleccion` de la capa
/// seleccionada a una capa raster nueva del tamaño del lienzo,
/// transparente fuera del rect, e inserta esa capa encima de la madre
/// (Photoshop Ctrl+J). Re-clampea contra el lienzo vigente. No es
/// destructivo: lee `capa.contenido` (raster o derivada — el buffer
/// composite cacheado sirve igual) y no modifica la madre. No-op si:
/// no hay selección, no hay capa seleccionada, el rect queda con área
/// cero, o el rect era todo transparente (nada visible que copiar). La
/// selección se mantiene.
pub(crate) fn duplicar_seleccion_a_capa(model: &mut Model) -> bool {
    let Some(rect) = model.seleccion else {
        model.estado = "no hay selección — `r` y arrastrar".into();
        return false;
    };
    let Some(id) = model.seleccionada else {
        model.estado = "no hay capa seleccionada".into();
        return false;
    };
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let x0 = rect.x0.min(w);
    let y0 = rect.y0.min(h);
    let x1 = rect.x1.min(w);
    let y1 = rect.y1.min(h);
    if x1 <= x0 || y1 <= y0 {
        model.estado = "selección fuera del lienzo".into();
        return false;
    }
    let Some(capa) = model.lienzo.capas.iter().find(|c| c.id == id) else {
        return false;
    };
    let Some(src) = model.almacen.obtener(capa.contenido) else {
        return false;
    };
    let src = src.to_vec();
    let (extraido, hubo_contenido) =
        extraer_rect_a_buffer(&src, w, h, x0, y0, x1, y1);
    if !hubo_contenido {
        model.estado = "selección transparente, nada que copiar".into();
        return false;
    }
    let hash = model.almacen.insertar(extraido);
    let nombre = format!("copia ({}×{})", x1 - x0, y1 - y0);
    let nueva = Capa::raster(nombre.clone(), hash);
    let nuevo_id = nueva.id;
    match model.lienzo.capas.iter().position(|c| c.id == id) {
        Some(idx) => model.lienzo.capas.insert(idx + 1, nueva),
        None => model.lienzo.apilar(nueva),
    }
    model.seleccionada = Some(nuevo_id);
    aplicar_y_recomponer(model);
    model.estado = format!("duplicada selección a '{}'", nombre);
    true
}

/// Recorta el rect half-open `(x0, y0, x1, y1)` de un buffer Rgba8
/// `w × *` a un buffer **tight** de `(x1-x0) × (y1-y0)` (NO del tamaño
/// del origen). Devuelve también si quedó algún píxel con alfa > 0
/// (`false` ⇒ nada visible). Pura. Pre: rect dentro de bounds.
pub(crate) fn recortar_subbuffer(
    src: &[u8],
    w: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> (Vec<u8>, bool) {
    let sw = w as usize;
    let rw = (x1 - x0) as usize;
    let rh = (y1 - y0) as usize;
    let mut out = Vec::with_capacity(rw * rh * 4);
    let mut hubo = false;
    for y in y0..y1 {
        let row = y as usize * sw;
        for x in x0..x1 {
            let i = (row + x as usize) * 4;
            out.extend_from_slice(&src[i..i + 4]);
            if src[i + 3] != 0 {
                hubo = true;
            }
        }
    }
    (out, hubo)
}

/// Compone un `clip` tight de `clip_w × clip_h` sobre un lienzo fresco
/// transparente de `canvas_w × canvas_h`, con la esquina superior
/// izquierda en `(dx, dy)`. Los píxeles del clip que caigan fuera del
/// lienzo se descartan (blit con recorte por-píxel). Reemplazo directo,
/// no alfa-compositing — el clip pisa lo que haya debajo (el lienzo
/// destino arranca transparente, así que da igual). Pura.
pub(crate) fn componer_clip_en_canvas(
    clip: &[u8],
    clip_w: u32,
    clip_h: u32,
    canvas_w: u32,
    canvas_h: u32,
    dx: u32,
    dy: u32,
) -> Vec<u8> {
    let cw = canvas_w as usize;
    let mut out = vec![0u8; cw * canvas_h as usize * 4];
    let clip_w = clip_w as usize;
    for cy in 0..clip_h as usize {
        let ty = dy as usize + cy;
        if ty >= canvas_h as usize {
            break;
        }
        for cx in 0..clip_w {
            let tx = dx as usize + cx;
            if tx >= cw {
                continue;
            }
            let si = (cy * clip_w + cx) * 4;
            let di = (ty * cw + tx) * 4;
            out[di..di + 4].copy_from_slice(&clip[si..si + 4]);
        }
    }
    out
}

/// Copia los píxeles del rect de `model.seleccion` de la capa
/// seleccionada al portapapeles interno, recortados al rect. No
/// destructivo (lee `capa.contenido` de cualquier capa). No-op si: no
/// hay selección/capa, área cero tras clampear, o el rect era todo
/// transparente. No snapshotea — el portapapeles vive fuera del DAG.
pub(crate) fn copiar_seleccion(model: &mut Model) -> bool {
    let Some(rect) = model.seleccion else {
        model.estado = "no hay selección — `r` y arrastrar".into();
        return false;
    };
    let Some(id) = model.seleccionada else {
        model.estado = "no hay capa seleccionada".into();
        return false;
    };
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let x0 = rect.x0.min(w);
    let y0 = rect.y0.min(h);
    let x1 = rect.x1.min(w);
    let y1 = rect.y1.min(h);
    if x1 <= x0 || y1 <= y0 {
        model.estado = "selección fuera del lienzo".into();
        return false;
    }
    let Some(capa) = model.lienzo.capas.iter().find(|c| c.id == id) else {
        return false;
    };
    let Some(src) = model.almacen.obtener(capa.contenido) else {
        return false;
    };
    let src = src.to_vec();
    let (sub, hubo) = recortar_subbuffer(&src, w, x0, y0, x1, y1);
    if !hubo {
        model.estado = "selección transparente, nada que copiar".into();
        return false;
    }
    let datos = model.almacen.insertar(sub);
    model.portapapeles = Some(PortaPixeles {
        w: x1 - x0,
        h: y1 - y0,
        datos,
        ox: x0,
        oy: y0,
    });
    model.estado =
        format!("copiada selección {}×{} al portapapeles", x1 - x0, y1 - y0);
    true
}

/// Copia la selección al portapapeles y limpia el rect en la capa
/// raster seleccionada (cut). Devuelve `true` (⇒ snapshot) sólo si
/// efectivamente borró píxeles: si la capa es derivada o el rect ya
/// era transparente, copia pero no borra y devuelve `false`.
pub(crate) fn cortar_seleccion(model: &mut Model) -> bool {
    if !copiar_seleccion(model) {
        return false; // estado ya seteado por copiar
    }
    let borro = limpiar_seleccion_en_capa(model);
    if borro {
        model.estado = "cortada selección al portapapeles".into();
    } else {
        // Copió pero no pudo borrar (derivada / ya transparente). El
        // estado de `limpiar_seleccion_en_capa` explica por qué.
        model.estado =
            format!("copiada (no se borró: {})", model.estado);
    }
    borro
}

/// Compone el clip de `model.portapapeles` sobre una capa raster nueva
/// del tamaño del lienzo vigente, ubicada en su origen `(ox, oy)`
/// clampeado para que el clip entre entero si cabe (tras un crop el
/// origen puede haber quedado fuera). Inserta encima de la seleccionada
/// y la selecciona. No-op si el portapapeles está vacío. La selección
/// se mantiene.
pub(crate) fn pegar_portapapeles(model: &mut Model) -> bool {
    let Some(clip) = model.portapapeles else {
        model.estado = "portapapeles vacío — Ctrl+C primero".into();
        return false;
    };
    let cw = model.lienzo.width;
    let ch = model.lienzo.height;
    let Some(datos) = model.almacen.obtener(clip.datos) else {
        return false;
    };
    let datos = datos.to_vec();
    // Clampea el origen: si el clip cabe en el eje, lo empuja para que
    // entre entero; si es más grande que el lienzo, lo ancla en 0.
    let dx = clip.ox.min(cw.saturating_sub(clip.w));
    let dy = clip.oy.min(ch.saturating_sub(clip.h));
    let buffer =
        componer_clip_en_canvas(&datos, clip.w, clip.h, cw, ch, dx, dy);
    let hash = model.almacen.insertar(buffer);
    let nombre = format!("pegado ({}×{})", clip.w, clip.h);
    let nueva = Capa::raster(nombre.clone(), hash);
    let nuevo_id = nueva.id;
    match model
        .seleccionada
        .and_then(|id| model.lienzo.capas.iter().position(|c| c.id == id))
    {
        Some(idx) => model.lienzo.capas.insert(idx + 1, nueva),
        None => model.lienzo.apilar(nueva),
    }
    model.seleccionada = Some(nuevo_id);
    aplicar_y_recomponer(model);
    model.estado = format!("pegado '{}' en ({}, {})", nombre, dx, dy);
    true
}

/// Expande (`delta > 0`) o contrae (`delta < 0`) un rect half-open
/// `delta` px por cada lado, clampeando al lienzo `w × h`. Devuelve
/// `None` si el resultado colapsa (área cero — típico al contraer un
/// rect chico). Pura. La selección no vive en el DAG, así que esto no
/// toca el almacén ni el historial.
pub(crate) fn expandir_rect(
    rect: RectImagen,
    delta: i32,
    w: u32,
    h: u32,
) -> Option<RectImagen> {
    let x0 = (rect.x0 as i32 - delta).clamp(0, w as i32);
    let y0 = (rect.y0 as i32 - delta).clamp(0, h as i32);
    let x1 = (rect.x1 as i32 + delta).clamp(0, w as i32);
    let y1 = (rect.y1 as i32 + delta).clamp(0, h as i32);
    if x1 > x0 && y1 > y0 {
        Some(RectImagen {
            x0: x0 as u32,
            y0: y0 as u32,
            x1: x1 as u32,
            y1: y1 as u32,
        })
    } else {
        None
    }
}

/// Flood fill (balde) sobre un buffer Rgba8 `w × h`. Desde la semilla
/// `(sx, sy)` expande en 4-conexión a todos los píxeles cuyo color esté
/// dentro de `tol` (suma de |Δ| RGBA) respecto al color semilla, y los
/// pinta de `nuevo`. Si `bounds` es `Some((x0,y0,x1,y1))` el relleno
/// queda confinado a ese rect half-open (los bordes actúan como muro) y
/// una semilla fuera de él no rellena nada. Devuelve `Some(buffer)` si
/// cambió algún píxel, `None` si no (semilla fuera, o región ya del
/// color destino). Pura. La tolerancia se chequea SIEMPRE contra el
/// color original del píxel (el pintado ocurre sólo al visitarlo, así
/// que un vecino nunca se evalúa con un color ya modificado).
pub(crate) fn flood_fill(
    src: &[u8],
    w: u32,
    h: u32,
    sx: u32,
    sy: u32,
    nuevo: [u8; 4],
    tol: u32,
    bounds: Option<(u32, u32, u32, u32)>,
) -> Option<Vec<u8>> {
    let w_us = w as usize;
    let (bx0, by0, bx1, by1) = bounds.unwrap_or((0, 0, w, h));
    // Recortar bounds al canvas por si vinieran sobredimensionados.
    let bx1 = bx1.min(w);
    let by1 = by1.min(h);
    if sx < bx0 || sx >= bx1 || sy < by0 || sy >= by1 {
        return None;
    }
    let idx = |x: u32, y: u32| ((y as usize) * w_us + x as usize) * 4;
    let si = idx(sx, sy);
    let seed = [src[si], src[si + 1], src[si + 2], src[si + 3]];
    let dentro_tol = |c: &[u8]| -> bool {
        let d = (c[0] as i32 - seed[0] as i32).unsigned_abs()
            + (c[1] as i32 - seed[1] as i32).unsigned_abs()
            + (c[2] as i32 - seed[2] as i32).unsigned_abs()
            + (c[3] as i32 - seed[3] as i32).unsigned_abs();
        d <= tol
    };
    let mut out = src.to_vec();
    let mut visto = vec![false; w_us * h as usize];
    let mut pila = vec![(sx, sy)];
    let mut cambio = false;
    while let Some((x, y)) = pila.pop() {
        let vi = y as usize * w_us + x as usize;
        if visto[vi] {
            continue;
        }
        visto[vi] = true;
        let i = vi * 4;
        if !dentro_tol(&out[i..i + 4]) {
            continue;
        }
        let actual = [out[i], out[i + 1], out[i + 2], out[i + 3]];
        if actual != nuevo {
            out[i..i + 4].copy_from_slice(&nuevo);
            cambio = true;
        }
        if x + 1 < bx1 {
            pila.push((x + 1, y));
        }
        if x > bx0 {
            pila.push((x - 1, y));
        }
        if y + 1 < by1 {
            pila.push((x, y + 1));
        }
        if y > by0 {
            pila.push((x, y - 1));
        }
    }
    if cambio {
        Some(out)
    } else {
        None
    }
}

/// Flood fill desde la coord-imagen `(sx, sy)` con el color activo sobre
/// la capa raster seleccionada, acotado a `model.seleccion` si la hay.
/// Re-clampea contra el lienzo. No-op si: no hay capa, la semilla cae
/// fuera del lienzo, la capa es derivada, o el relleno no cambia nada.
pub(crate) fn rellenar_flood_en_capa(
    model: &mut Model,
    sx: u32,
    sy: u32,
) -> bool {
    let Some(id) = model.seleccionada else {
        model.estado = "no hay capa seleccionada".into();
        return false;
    };
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    if sx >= w || sy >= h {
        model.estado = "balde fuera del lienzo".into();
        return false;
    }
    let Some(capa) = model.lienzo.capas.iter().find(|c| c.id == id) else {
        return false;
    };
    if !matches!(capa.origen, OrigenCapa::Raster) {
        model.estado =
            "la capa seleccionada es derivada — usá la raster madre".into();
        return false;
    }
    let hash_actual = capa.contenido;
    let color = model.color_picked.unwrap_or(RELLENO_DEFAULT);
    let bounds = model.seleccion.map(|r| (r.x0, r.y0, r.x1, r.y1));
    let Some(src) = model.almacen.obtener(hash_actual) else {
        return false;
    };
    let src = src.to_vec();
    let Some(nuevo) =
        flood_fill(&src, w, h, sx, sy, color, TOL_BALDE, bounds)
    else {
        model.estado = "balde: nada que rellenar".into();
        return false;
    };
    let new_hash = model.almacen.insertar(nuevo);
    if let Some(capa_mut) = model.lienzo.capa_mut(id) {
        capa_mut.contenido = new_hash;
    }
    model.lienzo.propagar_stale(id);
    aplicar_y_recomponer(model);
    model.estado =
        format!("balde @ ({}, {}) {}", sx, sy, etiqueta_color_activo(Some(color)));
    true
}

/// Factor de cobertura del pincel en `[0,1]` para un píxel a distancia
/// `d` del centro, con radio `r` y `dureza` en `[0,1]`. Dentro del núcleo
/// `dureza·r` es 1.0; entre ahí y `r` cae linealmente a 0; fuera de `r`
/// es 0. `r == 0` (1 px) o `dureza == 1` (borde duro) → 1.0 dentro del
/// disco. Pura.
pub(crate) fn cobertura_pincel(d: f32, r: f32, dureza: f32) -> f32 {
    if d > r {
        return 0.0;
    }
    if r <= 0.0 || dureza >= 1.0 {
        return 1.0;
    }
    let inner = dureza * r;
    if d <= inner {
        1.0
    } else {
        ((r - d) / (r - inner)).clamp(0.0, 1.0)
    }
}

/// Estampa un disco de radio `radio` centrado en `(cx, cy)` sobre un
/// buffer Rgba8 `w × h`, con `dureza` controlando el degradé del borde
/// (1.0 = duro; <1.0 = el alfa cae hacia el borde, ver [`cobertura_pincel`]).
/// Si `borrar`, reduce el alfa destino por la cobertura (goma suave); si
/// no, compone `color` (con su alfa escalado por la cobertura) src-over
/// ([`mezclar_src_over`]). Recorta al canvas y, si `bounds` es `Some`, al
/// rect half-open. Pura (muta `buf`); `cx, cy` pueden caer fuera.
#[allow(clippy::too_many_arguments)]
pub(crate) fn estampar_disco(
    buf: &mut [u8],
    w: u32,
    h: u32,
    cx: i32,
    cy: i32,
    radio: i32,
    color: [u8; 4],
    borrar: bool,
    dureza: f32,
    bounds: Option<(u32, u32, u32, u32)>,
) {
    let (bx0, by0, bx1, by1) = bounds.unwrap_or((0, 0, w, h));
    let bx1 = bx1.min(w) as i32;
    let by1 = by1.min(h) as i32;
    let bx0 = bx0 as i32;
    let by0 = by0 as i32;
    let r2 = radio * radio;
    let rf = radio as f32;
    for dy in -radio..=radio {
        let y = cy + dy;
        if y < by0 || y >= by1 {
            continue;
        }
        for dx in -radio..=radio {
            let x = cx + dx;
            if x < bx0 || x >= bx1 {
                continue;
            }
            if dx * dx + dy * dy <= r2 {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                let cob = cobertura_pincel(d, rf, dureza);
                if cob <= 0.0 {
                    continue;
                }
                let i = ((y as usize) * w as usize + x as usize) * 4;
                if borrar {
                    // Goma: baja el alfa destino por la cobertura.
                    let a = buf[i + 3] as f32;
                    buf[i + 3] = (a * (1.0 - cob)) as u8;
                } else {
                    let a = (color[3] as f32 * cob).round() as u8;
                    mezclar_src_over(
                        &mut buf[i..i + 4],
                        [color[0], color[1], color[2], a],
                    );
                }
            }
        }
    }
}

/// Estampa discos a lo largo del segmento `(x0, y0) → (x1, y1)`, uno por
/// cada paso entero del eje más largo, de modo que el trazo quede
/// continuo (sin huecos para `radio ≥ 1`). Pura (muta `buf`). Ver
/// [`estampar_disco`] para `borrar`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn trazar_linea_pincel(
    buf: &mut [u8],
    w: u32,
    h: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    radio: i32,
    color: [u8; 4],
    borrar: bool,
    dureza: f32,
    bounds: Option<(u32, u32, u32, u32)>,
) {
    let n = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    for k in 0..=n {
        let t = k as f32 / n as f32;
        let x = x0 + ((x1 - x0) as f32 * t).round() as i32;
        let y = y0 + ((y1 - y0) as f32 * t).round() as i32;
        estampar_disco(buf, w, h, x, y, radio, color, borrar, dureza, bounds);
    }
}

/// Cableado común de las ops del pincel: valida capa raster seleccionada,
/// resuelve color activo + bounds de selección, clona el buffer, aplica
/// `dibujar`, y si cambió el hash repunta la capa + propaga stale +
/// recompone. Devuelve `true` si hubo cambio efectivo.
fn pincel_aplicar(
    model: &mut Model,
    dibujar: impl FnOnce(&mut Vec<u8>, u32, u32, [u8; 4], Option<(u32, u32, u32, u32)>),
) -> bool {
    let Some(id) = model.seleccionada else {
        return false;
    };
    let Some(capa) = model.lienzo.capas.iter().find(|c| c.id == id) else {
        return false;
    };
    if !matches!(capa.origen, OrigenCapa::Raster) {
        model.estado =
            "la capa seleccionada es derivada — usá la raster madre".into();
        return false;
    }
    let hash_actual = capa.contenido;
    let color = model.color_picked.unwrap_or(RELLENO_DEFAULT);
    let bounds = model.seleccion.map(|r| (r.x0, r.y0, r.x1, r.y1));
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let Some(src) = model.almacen.obtener(hash_actual) else {
        return false;
    };
    let mut buf = src.to_vec();
    dibujar(&mut buf, w, h, color, bounds);
    let new_hash = model.almacen.insertar(buf);
    if new_hash == hash_actual {
        return false;
    }
    if let Some(capa_mut) = model.lienzo.capa_mut(id) {
        capa_mut.contenido = new_hash;
    }
    model.lienzo.propagar_stale(id);
    aplicar_y_recomponer(model);
    true
}

/// Estampa un disco del pincel en `(cx, cy)` sobre la capa raster
/// seleccionada (inicio de trazo). `borrar` → goma. Ver [`pincel_aplicar`].
/// Ejes de espejo activos para una simetría: lista de `(flip_x, flip_y)`.
/// Siempre incluye `(false, false)` (la estampa original). Pura.
pub(crate) fn ejes_simetria(sim: Simetria) -> Vec<(bool, bool)> {
    match sim {
        Simetria::Ninguna => vec![(false, false)],
        Simetria::Vertical => vec![(false, false), (true, false)],
        Simetria::Horizontal => vec![(false, false), (false, true)],
        Simetria::Ambas => {
            vec![(false, false), (true, false), (false, true), (true, true)]
        }
    }
}

/// Refleja `(x, y)` en un lienzo `w × h` según `(flip_x, flip_y)`
/// (espejo sobre el eje central de cada dimensión). Pura.
pub(crate) fn aplicar_eje(
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    flip: (bool, bool),
) -> (i32, i32) {
    let nx = if flip.0 { w as i32 - 1 - x } else { x };
    let ny = if flip.1 { h as i32 - 1 - y } else { y };
    (nx, ny)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn pincel_punto_en_capa(
    model: &mut Model,
    cx: i32,
    cy: i32,
    radio: i32,
    borrar: bool,
    dureza: f32,
    sim: Simetria,
) -> bool {
    pincel_aplicar(model, |buf, w, h, color, bounds| {
        for eje in ejes_simetria(sim) {
            let (x, y) = aplicar_eje(cx, cy, w, h, eje);
            estampar_disco(buf, w, h, x, y, radio, color, borrar, dureza, bounds);
        }
    })
}

/// Pinta el segmento `(x0,y0) → (x1,y1)` del pincel sobre la capa raster
/// seleccionada (continuación de trazo). `borrar` → goma. Ver
/// [`pincel_aplicar`].
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn pincel_segmento_en_capa(
    model: &mut Model,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    radio: i32,
    borrar: bool,
    dureza: f32,
    sim: Simetria,
) -> bool {
    pincel_aplicar(model, |buf, w, h, color, bounds| {
        for eje in ejes_simetria(sim) {
            let (ax, ay) = aplicar_eje(x0, y0, w, h, eje);
            let (bx, by) = aplicar_eje(x1, y1, w, h, eje);
            trazar_linea_pincel(
                buf, w, h, ax, ay, bx, by, radio, color, borrar, dureza, bounds,
            );
        }
    })
}

/// Compone (alpha src-over, Rgba8 NO premultiplicado) un `clip` de
/// `clip_w × clip_h` sobre `dst` (`dst_w × dst_h`) con la esquina
/// superior izquierda en el offset CON SIGNO `(dx, dy)`. Los píxeles del
/// clip que caen fuera de `dst` se descartan. A diferencia de
/// [`componer_clip_en_canvas`] (que parte de un lienzo fresco y pisa),
/// éste preserva y compone sobre el contenido previo de `dst` — sirve
/// para "dejar caer" píxeles movidos encima de lo que ya hay. Pura.
pub(crate) fn blit_alpha_sobre(
    dst: &[u8],
    dst_w: u32,
    dst_h: u32,
    clip: &[u8],
    clip_w: u32,
    clip_h: u32,
    dx: i32,
    dy: i32,
) -> Vec<u8> {
    let mut out = dst.to_vec();
    let dw = dst_w as i32;
    let dh = dst_h as i32;
    let cw = clip_w as usize;
    for cy in 0..clip_h as i32 {
        let ty = dy + cy;
        if ty < 0 || ty >= dh {
            continue;
        }
        for cx in 0..clip_w as i32 {
            let tx = dx + cx;
            if tx < 0 || tx >= dw {
                continue;
            }
            let si = ((cy as usize) * cw + cx as usize) * 4;
            let di = ((ty as usize) * dst_w as usize + tx as usize) * 4;
            let src = [clip[si], clip[si + 1], clip[si + 2], clip[si + 3]];
            mezclar_src_over(&mut out[di..di + 4], src);
        }
    }
    out
}

/// Compone `src` (Rgba8 NO premultiplicado) sobre el píxel destino
/// `dst` (slice de 4 bytes) con la fórmula src-over, redondeo entero
/// `/255`. Fast-path: alfa 0 no hace nada, alfa 255 pisa. Pura sobre el
/// slice. Es el núcleo compartido por `blit_alpha_sobre` (Fase 41) y el
/// pincel con alpha (Fase 46).
pub(crate) fn mezclar_src_over(dst: &mut [u8], src: [u8; 4]) {
    let sa = src[3] as u32;
    if sa == 0 {
        return;
    }
    if sa == 255 {
        dst.copy_from_slice(&src);
        return;
    }
    let da = dst[3] as u32;
    let da_eff = da * (255 - sa) / 255;
    let oa = sa + da_eff;
    for k in 0..3 {
        let num = src[k] as u32 * sa + dst[k] as u32 * da_eff;
        dst[k] = if oa == 0 { 0 } else { (num / oa) as u8 };
    }
    dst[3] = oa as u8;
}

/// Mueve los píxeles del rect de `model.seleccion` por el offset con
/// signo `(dx, dy)` dentro de la capa raster seleccionada: extrae el
/// contenido del rect, lo borra de su posición original y lo recompone
/// (alpha src-over) en el destino, recortando lo que salga del lienzo.
/// La selección sigue al contenido (trasladada y clampeada). No-op si:
/// no hay selección/capa, la capa es derivada, área cero tras clampear,
/// o el movimiento no cambia el buffer (mismo hash — delta cero o todo
/// fuera del lienzo).
pub(crate) fn mover_pixeles_seleccion(
    model: &mut Model,
    dx: i32,
    dy: i32,
) -> bool {
    let Some(rect) = model.seleccion else {
        model.estado = "no hay selección — `r` y arrastrar".into();
        return false;
    };
    let Some(id) = model.seleccionada else {
        model.estado = "no hay capa seleccionada".into();
        return false;
    };
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let x0 = rect.x0.min(w);
    let y0 = rect.y0.min(h);
    let x1 = rect.x1.min(w);
    let y1 = rect.y1.min(h);
    if x1 <= x0 || y1 <= y0 {
        model.estado = "selección fuera del lienzo".into();
        return false;
    }
    let Some(capa) = model.lienzo.capas.iter().find(|c| c.id == id) else {
        return false;
    };
    if !matches!(capa.origen, OrigenCapa::Raster) {
        model.estado =
            "la capa seleccionada es derivada — usá la raster madre".into();
        return false;
    }
    let hash_actual = capa.contenido;
    let Some(src) = model.almacen.obtener(hash_actual) else {
        return false;
    };
    let src = src.to_vec();
    // Levantar el contenido del rect, borrarlo de su lugar, recomponerlo
    // en el destino.
    let (sub, _) = recortar_subbuffer(&src, w, x0, y0, x1, y1);
    let limpio = limpiar_rect_en_buffer(&src, w, x0, y0, x1, y1);
    let nuevo = blit_alpha_sobre(
        &limpio,
        w,
        h,
        &sub,
        x1 - x0,
        y1 - y0,
        x0 as i32 + dx,
        y0 as i32 + dy,
    );
    let new_hash = model.almacen.insertar(nuevo);
    if new_hash == hash_actual {
        model.estado = "movimiento sin efecto".into();
        return false;
    }
    if let Some(capa_mut) = model.lienzo.capa_mut(id) {
        capa_mut.contenido = new_hash;
    }
    model.lienzo.propagar_stale(id);
    aplicar_y_recomponer(model);
    // La selección sigue al contenido: trasladar el rect y clampear al
    // lienzo (half-open). Si quedó fuera por completo, se limpia.
    let nx0 = (x0 as i32 + dx).clamp(0, w as i32) as u32;
    let ny0 = (y0 as i32 + dy).clamp(0, h as i32) as u32;
    let nx1 = (x1 as i32 + dx).clamp(0, w as i32) as u32;
    let ny1 = (y1 as i32 + dy).clamp(0, h as i32) as u32;
    model.seleccion = if nx1 > nx0 && ny1 > ny0 {
        Some(RectImagen { x0: nx0, y0: ny0, x1: nx1, y1: ny1 })
    } else {
        None
    };
    model.estado = format!("movida selección ({:+}, {:+})", dx, dy);
    true
}

/// Aplica una transformación de buffer al rect de `model.seleccion`
/// dentro de la capa raster seleccionada, compartiendo toda la
/// validación y el cableado entre limpiar (Fase 37) y rellenar
/// (Fase 38). `transformar(src, w, x0, y0, x1, y1)` produce el buffer
/// nuevo. Re-clampea el rect contra el lienzo vigente. No-op si: no
/// hay selección, no hay capa seleccionada, la capa es derivada (su
/// buffer es cache regenerable), el rect queda con área cero tras
/// clampear, o el buffer resultante es idéntico al original (mismo
/// hash content-addressed). Tras la mutación propaga stale al cono
/// descendiente y recompone; la selección se mantiene. `verbo`
/// describe el éxito, `sin_cambio` el caso de hash igual.
pub(crate) fn aplicar_a_seleccion_en_capa(
    model: &mut Model,
    transformar: impl Fn(&[u8], u32, u32, u32, u32, u32) -> Vec<u8>,
    verbo: &str,
    sin_cambio: &str,
) -> bool {
    let Some(rect) = model.seleccion else {
        model.estado = "no hay selección — `r` y arrastrar".into();
        return false;
    };
    let Some(id) = model.seleccionada else {
        model.estado = "no hay capa seleccionada".into();
        return false;
    };
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let x0 = rect.x0.min(w);
    let y0 = rect.y0.min(h);
    let x1 = rect.x1.min(w);
    let y1 = rect.y1.min(h);
    if x1 <= x0 || y1 <= y0 {
        model.estado = "selección fuera del lienzo".into();
        return false;
    }
    let Some(capa) = model.lienzo.capas.iter().find(|c| c.id == id) else {
        return false;
    };
    if !matches!(capa.origen, OrigenCapa::Raster) {
        model.estado =
            "la capa seleccionada es derivada — usá la raster madre".into();
        return false;
    }
    let hash_actual = capa.contenido;
    let Some(src) = model.almacen.obtener(hash_actual) else {
        return false;
    };
    let src = src.to_vec();
    let nuevo = transformar(&src, w, x0, y0, x1, y1);
    let new_hash = model.almacen.insertar(nuevo);
    if new_hash == hash_actual {
        model.estado = sin_cambio.into();
        return false;
    }
    if let Some(capa_mut) = model.lienzo.capa_mut(id) {
        capa_mut.contenido = new_hash;
    }
    model.lienzo.propagar_stale(id);
    aplicar_y_recomponer(model);
    model.estado = format!("{} {}×{} (capa raster)", verbo, x1 - x0, y1 - y0);
    true
}

/// Pone alfa=0 en los píxeles del rect de `model.seleccion` dentro de
/// la capa raster seleccionada (ver [`aplicar_a_seleccion_en_capa`]).
/// La selección se mantiene — encaja con flujos tipo "marquee + Delete
/// + re-pintar"; un Esc la limpia explícitamente.
pub(crate) fn limpiar_seleccion_en_capa(model: &mut Model) -> bool {
    aplicar_a_seleccion_en_capa(
        model,
        limpiar_rect_en_buffer,
        "limpiada selección",
        "selección ya transparente, nada que limpiar",
    )
}

/// Rellena los píxeles del rect de `model.seleccion` con el color
/// activo (`color_picked`, o `RELLENO_DEFAULT` si no se leyó ninguno)
/// dentro de la capa raster seleccionada (ver
/// [`aplicar_a_seleccion_en_capa`]). No-op extra si el rect ya tenía
/// ese color exacto (hash sin cambio).
pub(crate) fn rellenar_seleccion_en_capa(model: &mut Model) -> bool {
    let rgba = model.color_picked.unwrap_or(RELLENO_DEFAULT);
    aplicar_a_seleccion_en_capa(
        model,
        |src, w, x0, y0, x1, y1| {
            rellenar_rect_en_buffer(src, w, x0, y0, x1, y1, rgba)
        },
        "rellenada selección",
        "selección ya tenía ese color, sin cambio",
    )
}

/// Etiqueta corta del color activo: hex `#RRGGBB` si el cuentagotas
/// leyó alguno, o `"gris"` (el `RELLENO_DEFAULT`) si todavía no.
/// Compartida por el botón "+ relleno" y "rellenar selección".
pub(crate) fn etiqueta_color_activo(picked: Option<[u8; 4]>) -> String {
    match picked {
        Some(c) => format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]),
        None => "gris".to_string(),
    }
}

/// Rota 90° en sentido horario un buffer Rgba8 `w × h`. El buffer
/// resultante tiene el mismo conteo de bytes pero su layout corresponde
/// a dimensiones `h × w` (el ancho del destino = el alto del origen).
/// Pura. Pre: `src.len() == w*h*4` (la validación va aguas arriba).
///
/// Mapeo: src `(x, y)` → dst `(h-1-y, x)` con `w_new = h`.
pub(crate) fn rotar_buffer_90_cw(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let w = w as usize;
    let h = h as usize;
    let mut out = vec![0u8; src.len()];
    let w_new = h;
    for y in 0..h {
        for x in 0..w {
            let i_src = (y * w + x) * 4;
            let i_dst = (x * w_new + (h - 1 - y)) * 4;
            out[i_dst..i_dst + 4].copy_from_slice(&src[i_src..i_src + 4]);
        }
    }
    out
}

/// Rota 90° en sentido antihorario. Mapeo: src `(x, y)` → dst
/// `(y, w-1-x)` con `w_new = h`. Inversa exacta de `rotar_buffer_90_cw`.
pub(crate) fn rotar_buffer_90_ccw(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let w = w as usize;
    let h = h as usize;
    let mut out = vec![0u8; src.len()];
    let w_new = h;
    for y in 0..h {
        for x in 0..w {
            let i_src = (y * w + x) * 4;
            let i_dst = ((w - 1 - x) * w_new + y) * 4;
            out[i_dst..i_dst + 4].copy_from_slice(&src[i_src..i_src + 4]);
        }
    }
    out
}

/// Rota el lienzo entero 90° (CW si `cw=true`, CCW si no). Estrategia:
/// 1. Rotar el buffer Rgba8 de cada capa (raster o cache de derivada),
///    insertando el resultado al almacén content-addressed → nuevo hash.
/// 2. Swap `lienzo.width ↔ lienzo.height`.
/// 3. Marcar TODAS las derivadas Stale. Las ops `Espejar↔/↕` no
///    conmutan con rotación, así que la cache rotada quedaría
///    incorrecta para esos casos; el regen las recalcula desde la madre
///    ya rotada en `orden_regeneracion` topológico.
/// Devuelve `false` si las dims son cero o si el lienzo no tiene capas.
pub(crate) fn rotar_lienzo(model: &mut Model, cw: bool) -> bool {
    let w_old = model.lienzo.width;
    let h_old = model.lienzo.height;
    if w_old == 0 || h_old == 0 || model.lienzo.capas.is_empty() {
        model.estado = "nada que rotar".into();
        return false;
    }
    // Paso 1: rotar cada buffer. Iteramos las capas en orden de aparición;
    // no hay dependencias entre rotaciones (cada una es local al buffer).
    for capa in model.lienzo.capas.iter_mut() {
        let Some(src) = model.almacen.obtener(capa.contenido) else {
            // Derivada que nunca regeneró — el regen post-rotación la
            // armará desde la madre rotada. Saltamos.
            continue;
        };
        // `obtener` devuelve `&[u8]` (préstamo del almacén); lo copiamos
        // antes de liberar el préstamo para poder llamar `insertar`.
        let src = src.to_vec();
        let rotated = if cw {
            rotar_buffer_90_cw(&src, w_old, h_old)
        } else {
            rotar_buffer_90_ccw(&src, w_old, h_old)
        };
        let new_hash = model.almacen.insertar(rotated);
        capa.contenido = new_hash;
    }
    // Paso 2: swap de dimensiones.
    model.lienzo.width = h_old;
    model.lienzo.height = w_old;
    // Paso 3: marcar TODAS las derivadas Stale (las ops espejar no
    // conmutan con rotación). El regen reconstruye en orden topológico.
    for capa in model.lienzo.capas.iter_mut() {
        if let OrigenCapa::Derivada { estado, .. } = &mut capa.origen {
            *estado = Frescura::Stale;
        }
    }
    aplicar_y_recomponer(model);
    let signo = if cw { "+90" } else { "-90" };
    model.estado = format!(
        "lienzo rotado {signo}° → {}×{}",
        model.lienzo.width, model.lienzo.height
    );
    true
}

/// Aplana todas las capas visibles a una sola `Capa::raster` con el
/// composite del lienzo entero. Las hidden se preservan tal cual en su
/// posición relativa; el resultado se inserta donde estaba la *más
/// alta* visible (Photoshop "Merge Visible"). Esto exige un cálculo
/// topológico de la nueva posición:
///
/// ```text
/// original  visibles  hidden        nueva_pos
/// [bg v]    [0]       []            0  (todo se aplanó al primer slot)
/// [bg v, hidA h, fg v, hidB h]      [0, 2]   [1, 3]   2  (preservo hidA debajo, hidB encima)
/// ```
///
/// El criterio: cuántos hidden hay por debajo del top de los visibles.
/// Devuelve `false` si hay 0 o 1 visibles (nada que aplanar) o si el
/// `componer` falla (típicamente derivada stale → `BufferFaltante`).
pub(crate) fn aplanar_capas_visibles(model: &mut Model) -> bool {
    let visibles: Vec<usize> = model
        .lienzo
        .capas
        .iter()
        .enumerate()
        .filter(|(_, c)| c.visible)
        .map(|(i, _)| i)
        .collect();
    if visibles.len() < 2 {
        model.estado = if visibles.is_empty() {
            "nada visible que aplanar".into()
        } else {
            "ya hay una sola capa visible".into()
        };
        return false;
    }
    // `componer` ya itera sobre el lienzo entero saltando `!visible`, así
    // que el composite del Lienzo actual ES exactamente "merge visible".
    let img = match tullpu_render::componer(&model.lienzo, &model.almacen) {
        Ok(im) => im,
        Err(e) => {
            model.estado = format!("aplanar falló: {e:?}");
            return false;
        }
    };
    let buffer = img.into_raw();
    let hash = model.almacen.insertar(buffer);
    let n_aplanadas = visibles.len();
    let nombre = format!("aplanado de {} capas", n_aplanadas);
    let nueva = Capa::raster(nombre.clone(), hash);
    let nuevo_id = nueva.id;
    // Posición topológica: cuántos hidden hay por debajo del más alto
    // visible. Esos son los que quedan "debajo" de la merged en el nuevo
    // lienzo. Después de quitar los visibles (que viven en `0..=max_v`),
    // los hidden de ese rango se quedan al principio del Vec restante.
    let max_v = *visibles.last().unwrap();
    let insert_idx = (0..=max_v)
        .filter(|i| !model.lienzo.capas[*i].visible)
        .count();
    // Quitar los visibles en orden inverso para no descolocar los índices
    // que todavía no procesamos.
    for &i in visibles.iter().rev() {
        model.lienzo.capas.remove(i);
    }
    model.lienzo.capas.insert(insert_idx, nueva);
    model.seleccionada = Some(nuevo_id);
    aplicar_y_recomponer(model);
    model.estado = format!("aplanadas {} → '{}'", n_aplanadas, nombre);
    true
}
