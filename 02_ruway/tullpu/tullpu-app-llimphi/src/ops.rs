//! Operaciones sobre capas y buffers de la app `tullpu`: agregar/combinar/
//! aplanar capas, recortes del lienzo, transformaciones del rect de
//! selección (limpiar/rellenar/copiar/cortar/pegar/duplicar), rotación
//! de buffers y lienzo, bounding box, ajuste de parámetros y etiquetas.
//!
//! Behavior-preserving split de `main.rs` — sin cambios funcionales.

use std::path::Path;

use tullpu_core::{
    Capa, ClaseCapa, Frescura, Lienzo, OpLocal, OrigenCapa, TransformacionPixel,
};
use tullpu_render::FuenteBuffers;
use uuid::Uuid;

use crate::carga::{ajustar_a_lienzo, cargar_png};
use crate::compose::aplicar_y_recomponer;
use crate::historial::pushear_snapshot;
use crate::model::*;
use crate::viewport::transform_lienzo;

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
        OpLocal::Curvas { .. } => "curvas",
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


// === kernel de pintura buffer-puro: extraído a `tullpu-paint` (regla #2). ===
// Re-exportado con visibilidad de crate para que callers y tests (`use
// crate::ops::*`) sigan resolviendo estos nombres sin cambios.
pub(crate) use tullpu_paint::{
    aplicar_eje,
    bbox_no_transparente,
    blit_alpha_sobre,
    buffer_relleno,
    componer_clip_en_canvas,
    cobertura_pincel,
    estampar_disco,
    estampar_disco_mascara,
    extraer_rect_a_buffer,
    flood_fill,
    flood_fill_mascara,
    flood_mascara,
    limpiar_rect_en_buffer,
    poligono_a_mascara,
    recortar_buffer,
    recortar_buffer_bpp,
    recortar_subbuffer,
    rellenar_gradiente,
    rellenar_gradiente_mascara,
    rellenar_rect_en_buffer,
    rotar_buffer_90_ccw,
    rotar_buffer_90_ccw_bpp,
    rotar_buffer_90_cw,
    rotar_buffer_90_cw_bpp,
    trazar_linea_mascara,
    trazar_linea_pincel,
    transformar_afin,
};

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
    // El op editable sale de una capa derivada (cacheada) o de una capa de
    // ajuste (recalculada en vivo). Tomamos `&mut OpLocal` de cualquiera.
    let es_ajuste = matches!(capa.clase, ClaseCapa::Ajuste(_));
    let op: &mut OpLocal = match &mut capa.clase {
        ClaseCapa::Ajuste(op) => op,
        _ => match &mut capa.origen {
            OrigenCapa::Derivada {
                op: TransformacionPixel::Local(op),
                ..
            } => op,
            _ => return false,
        },
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
    // Sólo las derivadas cachean su buffer ⇒ hay que marcarlas stale y
    // propagar al cono. Los ajustes se recomponen en vivo: sin stale.
    if !es_ajuste {
        if let Some(capa) = model.lienzo.capa_mut(id) {
            if let OrigenCapa::Derivada { estado, .. } = &mut capa.origen {
                *estado = Frescura::Stale;
            }
        }
        model.lienzo.propagar_stale(id);
    }
    true
}

/// Acceso mutable a los puntos de control de una capa derivada `Curvas`.
/// `None` si la capa no existe, no es derivada local, o su op no es
/// `Curvas`.
fn puntos_curva_mut(model: &mut Model, id: Uuid) -> Option<&mut Vec<(f32, f32)>> {
    let capa = model.lienzo.capa_mut(id)?;
    match &mut capa.clase {
        ClaseCapa::Ajuste(OpLocal::Curvas { puntos }) => Some(puntos),
        _ => match &mut capa.origen {
            OrigenCapa::Derivada {
                op: TransformacionPixel::Local(OpLocal::Curvas { puntos }),
                ..
            } => Some(puntos),
            _ => None,
        },
    }
}

/// Marca la capa `id` como stale (si es derivada), propaga al cono descendiente
/// y recompone. Helper común de las tres mutaciones del editor de curvas. Las
/// capas de ajuste no cachean ⇒ sólo recomponen, sin stale.
fn marcar_stale_curva_y_recomponer(model: &mut Model, id: Uuid) {
    let es_ajuste = model
        .lienzo
        .capa(id)
        .map(|c| matches!(c.clase, ClaseCapa::Ajuste(_)))
        .unwrap_or(false);
    if !es_ajuste {
        if let Some(capa) = model.lienzo.capa_mut(id) {
            if let OrigenCapa::Derivada { estado, .. } = &mut capa.origen {
                *estado = Frescura::Stale;
            }
        }
        model.lienzo.propagar_stale(id);
    }
    aplicar_y_recomponer(model);
}

/// Press sobre el canvas del editor de curvas: convierte `(lx, ly)` a
/// coords-curva `[0,1]` (invierte `y` — arriba = salida 1.0), engancha el
/// punto de control más cercano dentro de un umbral, o inserta uno nuevo si
/// el click cae lejos de todos. Arma el `CurvaDrag` y recompone. Devuelve
/// `false` (sin tocar nada) si `id` no es una capa derivada `Curvas`.
pub(crate) fn curva_press(
    model: &mut Model,
    id: Uuid,
    lx: f32,
    ly: f32,
    rw: f32,
    rh: f32,
) -> bool {
    if rw <= 0.0 || rh <= 0.0 {
        return false;
    }
    let x = (lx / rw).clamp(0.0, 1.0);
    let y = (1.0 - ly / rh).clamp(0.0, 1.0);
    let Some(puntos) = puntos_curva_mut(model, id) else {
        return false;
    };
    // Umbral de enganche en coords-curva (radio del "imán" sobre un punto
    // existente). ~5% del lado del canvas.
    const UMBRAL: f32 = 0.06;
    let mut mejor: Option<(usize, f32)> = None;
    for (i, (px, py)) in puntos.iter().enumerate() {
        let d = ((px - x).powi(2) + (py - y).powi(2)).sqrt();
        if d < UMBRAL && mejor.map_or(true, |(_, md)| d < md) {
            mejor = Some((i, d));
        }
    }
    let idx = if let Some((i, _)) = mejor {
        i
    } else {
        // Inserta manteniendo el orden por x. El nuevo punto toma la
        // posición exacta del click; el drag posterior lo refina.
        let pos = puntos
            .iter()
            .position(|(px, _)| *px > x)
            .unwrap_or(puntos.len());
        puntos.insert(pos, (x, y));
        pos
    };
    model.curva_arrastrando = Some(CurvaDrag { idx, rw, rh });
    marcar_stale_curva_y_recomponer(model, id);
    true
}

/// Move durante el drag de un punto de la curva: normaliza los deltas-px
/// con las dims guardadas en `curva_arrastrando` y reubica el punto activo.
/// Los extremos (idx 0 y último) sólo se mueven en `y` (x fijo en 0/1); los
/// interiores se acotan en `x` entre sus vecinos para no cruzarlos. No-op si
/// no hay drag activo o la capa cambió.
pub(crate) fn curva_arrastrar(model: &mut Model, id: Uuid, dx: f32, dy: f32) -> bool {
    let Some(drag) = model.curva_arrastrando else {
        return false;
    };
    let dxn = dx / drag.rw;
    let dyn_curva = -dy / drag.rh; // pantalla: y crece hacia abajo; curva: al revés.
    let Some(puntos) = puntos_curva_mut(model, id) else {
        return false;
    };
    let n = puntos.len();
    if drag.idx >= n {
        return false;
    }
    let (mut nx, mut ny) = puntos[drag.idx];
    ny = (ny + dyn_curva).clamp(0.0, 1.0);
    if drag.idx == 0 {
        nx = 0.0;
    } else if drag.idx == n - 1 {
        nx = 1.0;
    } else {
        let lo = puntos[drag.idx - 1].0 + 1e-3;
        let hi = puntos[drag.idx + 1].0 - 1e-3;
        nx = (nx + dxn).clamp(lo, hi);
    }
    puntos[drag.idx] = (nx, ny);
    marcar_stale_curva_y_recomponer(model, id);
    true
}

/// Resetea la curva de `id` a la diagonal identidad `(0,0)→(1,1)`.
/// Devuelve `false` si `id` no es una capa derivada `Curvas`.
pub(crate) fn curva_reset(model: &mut Model, id: Uuid) -> bool {
    let Some(puntos) = puntos_curva_mut(model, id) else {
        return false;
    };
    *puntos = vec![(0.0, 0.0), (1.0, 1.0)];
    marcar_stale_curva_y_recomponer(model, id);
    true
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
        // La máscara (1 byte/píxel) acompaña al contenido: si no la
        // recortáramos, el render fallaría con `MascaraInvalida` por
        // tamaño tras cambiar las dims del lienzo.
        if let Some(mh) = capa.mascara {
            if let Some(ms) = model.almacen.obtener(mh) {
                let ms = ms.to_vec();
                let mc = recortar_buffer_bpp(&ms, w, x0, y0, x1, y1, 1);
                capa.mascara = Some(model.almacen.insertar(mc));
            }
        }
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
    let w = img.image.width;
    let h = img.image.height;
    let bytes = img.image.data.data();
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
    model.seleccion_mascara = None;
    model.seleccion_overlay = None;
    model.estado = format!(
        "recortado a selección {}×{} (offset {},{})",
        new_w, new_h, x0, y0
    );
    true
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
    // En modo máscara, el balde rellena la región contigua de máscara a
    // 255 (revelar). Reusa `mascara_aplicar` (recompone sin propagar stale).
    if pintando_en_mascara(model) {
        let valor = model.valor_mascara;
        let ok = mascara_aplicar(model, |buf, w, h, bounds| {
            if let Some(nuevo) =
                flood_fill_mascara(buf, w, h, sx, sy, valor, TOL_BALDE, bounds)
            {
                *buf = nuevo;
            }
        });
        if ok {
            model.estado = format!("balde máscara @ ({}, {}) → {valor}", sx, sy);
        } else {
            model.estado = "balde máscara: nada que rellenar".into();
        }
        return ok;
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




/// Estampa un disco de **clonado** en `(cx, cy)`: cada píxel del disco copia el
/// píxel de `src` desplazado por `(offx, offy)` (el origen del clon), compuesto
/// src-over sobre `dst` con la cobertura suave del pincel. `src` es una copia
/// del buffer previa a la estampa (para no realimentarse dentro del disco).
/// Respeta `bounds` (selección): no escribe fuera del rect.
#[allow(clippy::too_many_arguments)]
fn estampar_clon(
    dst: &mut [u8],
    src: &[u8],
    w: u32,
    h: u32,
    cx: i32,
    cy: i32,
    offx: i32,
    offy: i32,
    radio: i32,
    dureza: f32,
    bounds: Option<(u32, u32, u32, u32)>,
) {
    let w_i = w as i32;
    let h_i = h as i32;
    let (bx0, by0, bx1, by1) = bounds.unwrap_or((0, 0, w, h));
    let r = radio.max(0);
    for dy in -r..=r {
        for dx in -r..=r {
            let px = cx + dx;
            let py = cy + dy;
            if px < 0 || px >= w_i || py < 0 || py >= h_i {
                continue;
            }
            if (px as u32) < bx0 || (px as u32) >= bx1 || (py as u32) < by0 || (py as u32) >= by1 {
                continue;
            }
            let dist = ((dx * dx + dy * dy) as f32).sqrt();
            let cov = cobertura_pincel(dist, r as f32, dureza);
            if cov <= 0.0 {
                continue;
            }
            // Píxel fuente (origen del clon).
            let sx = px + offx;
            let sy = py + offy;
            if sx < 0 || sx >= w_i || sy < 0 || sy >= h_i {
                continue;
            }
            let si = ((sy * w_i + sx) as usize) * 4;
            let di = ((py * w_i + px) as usize) * 4;
            let sa = (src[si + 3] as f32 / 255.0) * cov;
            if sa <= 0.0 {
                continue;
            }
            let da = dst[di + 3] as f32 / 255.0;
            let inv = 1.0 - sa;
            let out_a = sa + da * inv;
            if out_a > 0.0 {
                for k in 0..3 {
                    let sc = src[si + k] as f32;
                    let dc = dst[di + k] as f32;
                    dst[di + k] = ((sc * sa + dc * da * inv) / out_a).round().clamp(0.0, 255.0) as u8;
                }
                dst[di + 3] = (out_a * 255.0).round() as u8;
            }
        }
    }
}

/// Estampa un disco de **sanado** (healing brush) en `(cx, cy)`. Como el
/// clonado, copia píxeles del origen desplazado por `(offx, offy)`, pero les
/// suma el delta `(media_destino − media_origen)` calculado sobre el disco:
/// así transfiere la **textura** (las desviaciones respecto a su media) del
/// origen sobre el **color base** del destino, fundiendo el parche en vez de
/// pegar un recorte visible. El delta se computa sobre los píxeles opacos
/// cubiertos; si el destino o el origen están vacíos cae a clon puro
/// (delta cero). Respeta `bounds`.
#[allow(clippy::too_many_arguments)]
fn estampar_sanar(
    dst: &mut [u8],
    src: &[u8],
    w: u32,
    h: u32,
    cx: i32,
    cy: i32,
    offx: i32,
    offy: i32,
    radio: i32,
    dureza: f32,
    bounds: Option<(u32, u32, u32, u32)>,
) {
    let w_i = w as i32;
    let h_i = h as i32;
    let (bx0, by0, bx1, by1) = bounds.unwrap_or((0, 0, w, h));
    let r = radio.max(0);
    // Píxeles del disco (destino) con su píxel-origen, sólo donde la cobertura
    // es > 0 y ambos caen en el lienzo + bounds. Se reúsan en las dos pasadas.
    let mut celdas: Vec<(usize, usize)> = Vec::new(); // (idx_destino, idx_origen)
    let mut sum_d = [0f64; 3];
    let mut sum_s = [0f64; 3];
    let mut n_d = 0u32;
    let mut n_s = 0u32;
    for dy in -r..=r {
        for dx in -r..=r {
            let px = cx + dx;
            let py = cy + dy;
            if px < 0 || px >= w_i || py < 0 || py >= h_i {
                continue;
            }
            if (px as u32) < bx0 || (px as u32) >= bx1 || (py as u32) < by0 || (py as u32) >= by1 {
                continue;
            }
            let dist = ((dx * dx + dy * dy) as f32).sqrt();
            if cobertura_pincel(dist, r as f32, dureza) <= 0.0 {
                continue;
            }
            let sx = px + offx;
            let sy = py + offy;
            if sx < 0 || sx >= w_i || sy < 0 || sy >= h_i {
                continue;
            }
            let di = ((py * w_i + px) as usize) * 4;
            let si = ((sy * w_i + sx) as usize) * 4;
            celdas.push((di, si));
            if dst[di + 3] > 0 {
                for k in 0..3 {
                    sum_d[k] += dst[di + k] as f64;
                }
                n_d += 1;
            }
            if src[si + 3] > 0 {
                for k in 0..3 {
                    sum_s[k] += src[si + k] as f64;
                }
                n_s += 1;
            }
        }
    }
    // Delta de color base: media del destino menos media del origen. Sin
    // píxeles opacos en alguno de los dos, el delta es cero (clon puro).
    let mut delta = [0f64; 3];
    if n_d > 0 && n_s > 0 {
        for k in 0..3 {
            delta[k] = sum_d[k] / n_d as f64 - sum_s[k] / n_s as f64;
        }
    }
    // Segunda pasada: compone src+delta sobre dst con la cobertura del pincel.
    for &(di, si) in &celdas {
        let py = (di / 4) as i32 / w_i;
        let px = (di / 4) as i32 % w_i;
        let dx = px - cx;
        let dy = py - cy;
        let dist = ((dx * dx + dy * dy) as f32).sqrt();
        let cov = cobertura_pincel(dist, r as f32, dureza);
        let sa = (src[si + 3] as f32 / 255.0) * cov;
        if sa <= 0.0 {
            continue;
        }
        let da = dst[di + 3] as f32 / 255.0;
        let inv = 1.0 - sa;
        let out_a = sa + da * inv;
        if out_a > 0.0 {
            for k in 0..3 {
                let sc = (src[si + k] as f64 + delta[k]).clamp(0.0, 255.0) as f32;
                let dc = dst[di + k] as f32;
                dst[di + k] = ((sc * sa + dc * da * inv) / out_a).round().clamp(0.0, 255.0) as u8;
            }
            dst[di + 3] = (out_a * 255.0).round() as u8;
        }
    }
}

/// Sana un punto: estampa el disco de sanado en `(cx, cy)` desde el origen
/// desplazado por `(offx, offy)`, sobre la capa raster seleccionada.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sanar_punto_en_capa(
    model: &mut Model,
    cx: i32,
    cy: i32,
    offx: i32,
    offy: i32,
    radio: i32,
    dureza: f32,
) -> bool {
    pincel_aplicar(model, |buf, w, h, _color, bounds| {
        let src = buf.clone();
        estampar_sanar(buf, &src, w, h, cx, cy, offx, offy, radio, dureza, bounds);
    })
}

/// Sana un segmento `(x0,y0)→(x1,y1)`: estampa discos de sanado a lo largo de
/// la línea (paso ~1 px). Cada disco recalcula su delta de color, así el parche
/// sigue el color base del destino a lo largo del trazo.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sanar_segmento_en_capa(
    model: &mut Model,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    offx: i32,
    offy: i32,
    radio: i32,
    dureza: f32,
) -> bool {
    pincel_aplicar(model, |buf, w, h, _color, bounds| {
        let src = buf.clone();
        let dx = x1 - x0;
        let dy = y1 - y0;
        let pasos = dx.abs().max(dy.abs()).max(1);
        for i in 0..=pasos {
            let t = i as f32 / pasos as f32;
            let cx = (x0 as f32 + t * dx as f32).round() as i32;
            let cy = (y0 as f32 + t * dy as f32).round() as i32;
            estampar_sanar(buf, &src, w, h, cx, cy, offx, offy, radio, dureza, bounds);
        }
    })
}

/// Clona un punto: estampa el disco de clonado en `(cx, cy)` muestreando desde
/// el origen desplazado por `(offx, offy)`, sobre la capa raster seleccionada.
#[allow(clippy::too_many_arguments)]
pub(crate) fn clonar_punto_en_capa(
    model: &mut Model,
    cx: i32,
    cy: i32,
    offx: i32,
    offy: i32,
    radio: i32,
    dureza: f32,
) -> bool {
    pincel_aplicar(model, |buf, w, h, _color, bounds| {
        let src = buf.clone();
        estampar_clon(buf, &src, w, h, cx, cy, offx, offy, radio, dureza, bounds);
    })
}

/// Clona un segmento `(x0,y0)→(x1,y1)`: estampa discos de clonado a lo largo
/// de la línea (paso ~1 px) para no dejar huecos en un drag rápido.
#[allow(clippy::too_many_arguments)]
pub(crate) fn clonar_segmento_en_capa(
    model: &mut Model,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    offx: i32,
    offy: i32,
    radio: i32,
    dureza: f32,
) -> bool {
    pincel_aplicar(model, |buf, w, h, _color, bounds| {
        let src = buf.clone();
        let dx = x1 - x0;
        let dy = y1 - y0;
        let pasos = dx.abs().max(dy.abs()).max(1);
        for i in 0..=pasos {
            let t = i as f32 / pasos as f32;
            let cx = (x0 as f32 + t * dx as f32).round() as i32;
            let cy = (y0 as f32 + t * dy as f32).round() as i32;
            estampar_clon(buf, &src, w, h, cx, cy, offx, offy, radio, dureza, bounds);
        }
    })
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
    if pintando_en_mascara(model) {
        let valor = if borrar { 0u8 } else { model.valor_mascara };
        return mascara_aplicar(model, |buf, w, h, bounds| {
            for eje in ejes_simetria(sim) {
                let (x, y) = aplicar_eje(cx, cy, w, h, eje);
                estampar_disco_mascara(buf, w, h, x, y, radio, valor, dureza, bounds);
            }
        });
    }
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
    if pintando_en_mascara(model) {
        let valor = if borrar { 0u8 } else { model.valor_mascara };
        return mascara_aplicar(model, |buf, w, h, bounds| {
            for eje in ejes_simetria(sim) {
                let (ax, ay) = aplicar_eje(x0, y0, w, h, eje);
                let (bx, by) = aplicar_eje(x1, y1, w, h, eje);
                trazar_linea_mascara(
                    buf, w, h, ax, ay, bx, by, radio, valor, dureza, bounds,
                );
            }
        });
    }
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


/// Rellena un degradé del color activo (en el ancla) a transparente (en
/// el extremo) sobre la capa raster seleccionada, acotado a la selección.
/// Reusa [`pincel_aplicar`] (validación raster + color + bounds + snapshot
/// implícito por el caller). No-op si la capa es derivada o nada cambia.
pub(crate) fn rellenar_gradiente_en_capa(
    model: &mut Model,
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
) -> bool {
    if pintando_en_mascara(model) {
        // Degradé sobre máscara: revela (valor_mascara) en el ancla, se
        // desvanece hacia el extremo. Para un degradé que oculta, invertí
        // la máscara.
        let valor = model.valor_mascara;
        return mascara_aplicar(model, |buf, w, h, bounds| {
            *buf = rellenar_gradiente_mascara(buf, w, h, ax, ay, bx, by, valor, bounds);
        });
    }
    pincel_aplicar(model, |buf, w, h, color, bounds| {
        *buf = rellenar_gradiente(buf, w, h, ax, ay, bx, by, color, bounds);
    })
}

// =============================================================================
//  Pintar sobre la máscara (fase 53) — buffers de un canal
// =============================================================================
//
// Cuando `Model.editando_mascara` está activo y la capa tiene máscara, las
// herramientas de trazo escriben el buffer de máscara (1 byte/píxel) en
// lugar del contenido Rgba8. La semántica es value-lerp por cobertura:
// pincel apunta a 255 (revelar), borrador a 0 (ocultar). No hay color ni
// src-over — es más simple que el estampado Rgba8.

/// `true` si el trazo debe ir a la máscara: el modo está activo Y la capa
/// seleccionada tiene una máscara adjunta. Si no, el trazo cae al contenido.
pub(crate) fn pintando_en_mascara(model: &Model) -> bool {
    model.editando_mascara
        && model
            .seleccionada
            .and_then(|id| model.lienzo.capas.iter().find(|c| c.id == id))
            .map(|c| c.mascara.is_some())
            .unwrap_or(false)
}





/// Cableado común de las ops de trazo sobre la MÁSCARA de la capa
/// seleccionada (espejo de [`pincel_aplicar`] para 1 canal). Resuelve el
/// buffer de máscara, aplica `dibujar`, y si cambió el hash repunta
/// `capa.mascara` + recompone. NO propaga stale: la máscara no entra en el
/// cómputo de las derivadas, sólo en el composite. Devuelve `true` si hubo
/// cambio. Pre: el caller garantizó que la capa tiene máscara
/// ([`pintando_en_mascara`]).
fn mascara_aplicar(
    model: &mut Model,
    dibujar: impl FnOnce(&mut Vec<u8>, u32, u32, Option<(u32, u32, u32, u32)>),
) -> bool {
    let Some(id) = model.seleccionada else {
        return false;
    };
    let mh = match model.lienzo.capas.iter().find(|c| c.id == id) {
        Some(capa) => match capa.mascara {
            Some(m) => m,
            None => return false,
        },
        None => return false,
    };
    let bounds = model.seleccion.map(|r| (r.x0, r.y0, r.x1, r.y1));
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let Some(src) = model.almacen.obtener(mh) else {
        return false;
    };
    let mut buf = src.to_vec();
    dibujar(&mut buf, w, h, bounds);
    let new_hash = model.almacen.insertar(buf);
    if new_hash == mh {
        return false;
    }
    if let Some(capa_mut) = model.lienzo.capa_mut(id) {
        capa_mut.mascara = Some(new_hash);
    }
    aplicar_y_recomponer(model);
    true
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
    // Mover píxeles degrada a selección rectangular (la máscara de la varita
    // no acompaña el desplazamiento por ahora).
    model.seleccion_mascara = None;
    model.seleccion_overlay = None;
    model.estado = format!("movida selección ({:+}, {:+})", dx, dy);
    true
}

/// Crea una capa de texto en `(x, y)` (coords-imagen) con un string inicial,
/// la rasteriza y la apila encima de la seleccionada. Devuelve su `Uuid`.
/// El color sale de `color_picked` (o negro). Hereda el grupo de la selección.
pub(crate) fn agregar_capa_texto(model: &mut Model, x: u32, y: u32) -> uuid::Uuid {
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let color = model.color_picked.unwrap_or([20, 20, 20, 255]);
    let params = tullpu_core::ParamsTexto {
        texto: "Texto".into(),
        tamano: 48.0,
        color,
        x: x.min(w.saturating_sub(1)),
        y: y.min(h.saturating_sub(1)),
    };
    let buffer = crate::texto::rasterizar_texto(&params, w, h);
    let hash = model.almacen.insertar(buffer);
    let mut capa = tullpu_core::Capa::texto("texto", hash, params);
    capa.grupo = model
        .seleccionada
        .and_then(|id| model.lienzo.capa(id))
        .and_then(|c| c.grupo);
    let id = capa.id;
    model.lienzo.apilar(capa);
    model.seleccionada = Some(id);
    aplicar_y_recomponer(model);
    id
}

/// Inserta una capa vectorial nueva a partir de su path, rasterizándola al
/// tamaño del lienzo. La cuelga del mismo grupo que la capa seleccionada (si la
/// hay). Devuelve el id de la capa creada y recompone.
pub(crate) fn agregar_capa_vector(
    model: &mut Model,
    params: tullpu_core::ParamsVector,
    nombre: &str,
) -> uuid::Uuid {
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let buffer = tullpu_ops::rasterizar_vector(&params, w, h);
    let hash = model.almacen.insertar(buffer);
    let mut capa = tullpu_core::Capa::vector(nombre, hash, params);
    capa.grupo = model
        .seleccionada
        .and_then(|id| model.lienzo.capa(id))
        .and_then(|c| c.grupo);
    let id = capa.id;
    model.lienzo.apilar(capa);
    model.seleccionada = Some(id);
    aplicar_y_recomponer(model);
    id
}

/// Re-rasteriza la capa vectorial `id` desde sus params vigentes y actualiza su
/// `contenido`. No-op si la capa no es vectorial. Recompone.
pub(crate) fn rerasterizar_vector(model: &mut Model, id: uuid::Uuid) {
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let Some(params) = model.lienzo.capa(id).and_then(|c| c.params_vector()).cloned() else {
        return;
    };
    let buffer = tullpu_ops::rasterizar_vector(&params, w, h);
    let hash = model.almacen.insertar(buffer);
    if let Some(c) = model.lienzo.capa_mut(id) {
        c.contenido = hash;
    }
    aplicar_y_recomponer(model);
}

/// Muta los params vectoriales de la capa `id` por el closure `f` y
/// re-rasteriza. No-op si la capa no es vectorial.
pub(crate) fn editar_params_vector(
    model: &mut Model,
    id: uuid::Uuid,
    f: impl FnOnce(&mut tullpu_core::ParamsVector),
) {
    let cambio = if let Some(c) = model.lienzo.capa_mut(id) {
        if let tullpu_core::ClaseCapa::Vector(p) = &mut c.clase {
            f(p);
            true
        } else {
            false
        }
    } else {
        false
    };
    if cambio {
        rerasterizar_vector(model, id);
    }
}

/// Aplica `f` a los params vectoriales de la capa **seleccionada** y
/// re-rasteriza. No-op si no hay selección o no es vectorial.
pub(crate) fn editar_vector_seleccionado(
    model: &mut Model,
    f: impl FnOnce(&mut tullpu_core::ParamsVector),
) {
    if let Some(id) = model.seleccionada {
        editar_params_vector(model, id, f);
    }
}

/// Control Bézier de la capa `id` más cercano a `(ix, iy)` dentro de `umbral`;
/// `(índice de comando, es_c1)` o `None`.
fn control_cercano(model: &Model, id: uuid::Uuid, ix: f32, iy: f32, umbral: f32) -> Option<(usize, bool)> {
    let p = model.lienzo.capa(id)?.params_vector()?;
    let mut mejor = None;
    let mut mejor_d = umbral * umbral;
    for (idx, es_c1, [ax, ay]) in p.puntos_control() {
        let d = (ax - ix).powi(2) + (ay - iy).powi(2);
        if d <= mejor_d {
            mejor_d = d;
            mejor = Some((idx, es_c1));
        }
    }
    mejor
}

/// Ancla de la capa vectorial `id` más cercana a `(ix, iy)` dentro de `umbral`
/// px-imagen; `None` si ninguna. Devuelve el índice de comando del ancla.
fn ancla_cercana(model: &Model, id: uuid::Uuid, ix: f32, iy: f32, umbral: f32) -> Option<usize> {
    let p = model.lienzo.capa(id)?.params_vector()?;
    let mut mejor = None;
    let mut mejor_d = umbral * umbral;
    for (idx, [ax, ay]) in p.puntos_ancla() {
        let d = (ax - ix).powi(2) + (ay - iy).powi(2);
        if d <= mejor_d {
            mejor_d = d;
            mejor = Some(idx);
        }
    }
    mejor
}

/// Pluma: press sobre el lienzo. Si cae cerca de un ancla de la capa en
/// edición, la agarra para arrastrar; si no, agrega un vértice (creando la capa
/// vectorial en el primer click, rellena con el color activo).
pub(crate) fn pluma_press(model: &mut Model, lx: f32, ly: f32, rw: f32, rh: f32) -> bool {
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    model.pluma_rect = Some((rw, rh));
    let Some((ixf, iyf)) = crate::viewport::local_a_imagen(
        lx, ly, rw, rh, w, h, model.factor_zoom, model.pan_x, model.pan_y,
    ) else {
        return false;
    };
    let (ix, iy) = (ixf as f32, iyf as f32);

    // Umbral de hit-test: ~9 px de pantalla, convertidos a px-imagen.
    let s = crate::viewport::transform_lienzo(w, h, rw, rh, model.factor_zoom, model.pan_x, model.pan_y)
        .map(|(s, _, _)| s)
        .unwrap_or(1.0);
    let umbral = (9.0 / s.max(1e-6)) as f32;

    // Capa objetivo de la edición: la que está en curso, o la seleccionada si
    // es vectorial (la adoptamos para editarla con la pluma).
    let objetivo = model.pluma_capa.or_else(|| {
        model.seleccionada.filter(|&id| {
            model.lienzo.capa(id).map(|c| c.params_vector().is_some()).unwrap_or(false)
        })
    });

    // 1) Punto de control Bézier (prioridad sobre las anclas): lo agarra.
    if let Some(id) = objetivo {
        if let Some((idx, es_c1)) = control_cercano(model, id, ix, iy, umbral) {
            model.pluma_capa = Some(id);
            model.pluma_control = Some((idx, es_c1, ix, iy));
            return false;
        }
    }

    // 2) Ancla: Alt+click la borra; Shift+click convierte su segmento recto en
    //    curva (para poder curvarlo); si no, la agarra para arrastrar.
    if let Some(id) = objetivo {
        if let Some(idx) = ancla_cercana(model, id, ix, iy, umbral) {
            model.pluma_capa = Some(id);
            if model.alt_held {
                editar_params_vector(model, id, |p| p.eliminar_vertice(idx));
                model.pluma_ancla = None;
                return true;
            }
            if model.shift_held {
                editar_params_vector(model, id, |p| p.convertir_a_curva(idx));
                model.pluma_ancla = None;
                return true;
            }
            model.pluma_ancla = Some((idx, ix, iy));
            return false;
        }
    }

    // 3) Adoptar la capa seleccionada (sin ancla bajo el cursor) para seguir
    //    agregándole vértices.
    if model.pluma_capa.is_none() {
        if let Some(id) = objetivo {
            model.pluma_capa = Some(id);
        }
    }

    // Agregar un vértice: crea la capa vectorial si no hay ninguna en edición.
    let id = match model.pluma_capa {
        Some(id) => id,
        None => {
            let color = model.color_picked.unwrap_or([60, 120, 220, 255]);
            let params = tullpu_core::ParamsVector {
                comandos: Vec::new(),
                relleno: Some(color),
                regla: tullpu_core::ReglaRelleno::NoCero,
                trazo: None,
                ancho_trazo: 0.0,
            };
            let id = agregar_capa_vector(model, params, "path");
            model.pluma_capa = Some(id);
            id
        }
    };
    editar_params_vector(model, id, |p| p.agregar_vertice(ix, iy));
    model.pluma_ancla = None;
    true
}

/// Pluma: arrastre — mueve el ancla agarrada por `(dx, dy)` (deltas de pantalla,
/// convertidos a coords-imagen con la escala vigente).
pub(crate) fn pluma_arrastrar(model: &mut Model, dx: f32, dy: f32) {
    let Some(id) = model.pluma_capa else { return };
    let (rw, rh) = model.pluma_rect.unwrap_or((0.0, 0.0));
    let s = crate::viewport::transform_lienzo(
        model.lienzo.width, model.lienzo.height, rw, rh,
        model.factor_zoom, model.pan_x, model.pan_y,
    )
    .map(|(s, _, _)| s as f32)
    .unwrap_or(1.0)
    .max(1e-6);

    // Prioridad al control Bézier; si no hay, mueve el ancla.
    if let Some((idx, es_c1, px, py)) = model.pluma_control {
        let (nx, ny) = (px + dx / s, py + dy / s);
        model.pluma_control = Some((idx, es_c1, nx, ny));
        editar_params_vector(model, id, |p| p.mover_control(idx, es_c1, nx, ny));
        return;
    }
    if let Some((idx, px, py)) = model.pluma_ancla {
        let (nx, ny) = (px + dx / s, py + dy / s);
        model.pluma_ancla = Some((idx, nx, ny));
        editar_params_vector(model, id, |p| p.mover_ancla(idx, nx, ny));
    }
}

/// Pluma: cierra el path en edición y termina la edición (Enter).
pub(crate) fn pluma_cerrar(model: &mut Model) {
    if let Some(id) = model.pluma_capa.take() {
        editar_params_vector(model, id, |p| p.cerrar_path());
    }
    model.pluma_ancla = None;
}

/// Re-rasteriza la capa de texto `id` desde sus params vigentes y actualiza su
/// `contenido`. No-op si la capa no es de texto. Recompone.
pub(crate) fn rerasterizar_texto(model: &mut Model, id: uuid::Uuid) {
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let Some(params) = model.lienzo.capa(id).and_then(|c| c.params_texto()).cloned() else {
        return;
    };
    let buffer = crate::texto::rasterizar_texto(&params, w, h);
    let hash = model.almacen.insertar(buffer);
    if let Some(c) = model.lienzo.capa_mut(id) {
        c.contenido = hash;
    }
    aplicar_y_recomponer(model);
}

/// Muta los params de la capa de texto `id` por el closure `f` (p. ej. cambiar
/// el string o el tamaño) y re-rasteriza. No-op si no es capa de texto.
pub(crate) fn editar_params_texto(
    model: &mut Model,
    id: uuid::Uuid,
    f: impl FnOnce(&mut tullpu_core::ParamsTexto),
) {
    let cambio = if let Some(c) = model.lienzo.capa_mut(id) {
        if let tullpu_core::ClaseCapa::Texto(p) = &mut c.clase {
            f(p);
            true
        } else {
            false
        }
    } else {
        false
    };
    if cambio {
        rerasterizar_texto(model, id);
    }
}

/// Voltea (espeja) el buffer de la capa raster seleccionada in situ:
/// `horizontal=true` ↔ eje vertical (izq↔der), `false` ↕ eje horizontal
/// (arriba↔abajo). Edición raster directa (no genera una capa derivada). Las
/// dimensiones no cambian, así que encaja en la capa canvas-sized. No-op si no
/// hay capa, no es raster, o el buffer no cambia. Propaga stale y recompone.
pub(crate) fn voltear_capa_activa(model: &mut Model, horizontal: bool) -> bool {
    let Some(id) = model.seleccionada else {
        model.estado = "no hay capa seleccionada".into();
        return false;
    };
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let Some(capa) = model.lienzo.capas.iter().find(|c| c.id == id) else {
        return false;
    };
    if !matches!(capa.origen, OrigenCapa::Raster) {
        model.estado = "la capa es derivada — usá la raster madre".into();
        return false;
    }
    let hash_actual = capa.contenido;
    let Some(src) = model.almacen.obtener(hash_actual) else {
        return false;
    };
    let src = src.to_vec();
    let w_us = w as usize;
    let h_us = h as usize;
    let mut out = vec![0u8; src.len()];
    for y in 0..h_us {
        for x in 0..w_us {
            let (sx, sy) = if horizontal {
                (w_us - 1 - x, y)
            } else {
                (x, h_us - 1 - y)
            };
            let di = (y * w_us + x) * 4;
            let si = (sy * w_us + sx) * 4;
            out[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    let new_hash = model.almacen.insertar(out);
    if new_hash == hash_actual {
        model.estado = "capa simétrica · sin cambio".into();
        return false;
    }
    if let Some(c) = model.lienzo.capa_mut(id) {
        c.contenido = new_hash;
    }
    model.lienzo.propagar_stale(id);
    aplicar_y_recomponer(model);
    model.estado = if horizontal { "capa volteada ↔" } else { "capa volteada ↕" }.into();
    true
}

/// Geometría de los handles de transformación libre en coords-imagen. Las 4
/// esquinas del cuadro (TL, TR, BR, BL), los 8 handles (4 esquinas + 4 lados)
/// con su semi-extensión local `(hx, hy)`, y el handle de rotación con su
/// ancla en el borde superior. `inv_s` (px-imagen por px-pantalla) sólo afecta
/// la separación del handle de rotación, que se mantiene a distancia de
/// pantalla constante del borde.
pub(crate) struct PuntosTransform {
    pub(crate) esquinas: [(f64, f64); 4],
    /// `(world_x, world_y, hx_local, hy_local)` por handle.
    pub(crate) handles: [(f64, f64, f64, f64); 8],
    pub(crate) rot_world: (f64, f64),
    pub(crate) rot_ancla: (f64, f64),
}

/// Mapea los puntos del cuadro de transformación al espacio-imagen aplicando
/// la afín vigente. Pura — la usa tanto el hit-test del press como el painter
/// del overlay.
pub(crate) fn puntos_transform(t: &TransformLibre, inv_s: f64) -> PuntosTransform {
    let w2 = (t.bx1 - t.bx0) / 2.0;
    let h2 = (t.by1 - t.by0) / 2.0;
    let ct = t.rot.cos();
    let st = t.rot.sin();
    // Mapeo directo de un offset local `(lhx, lhy)` desde el pivote.
    let fwd = |lhx: f64, lhy: f64| -> (f64, f64) {
        let ex = t.escala_x * lhx;
        let ey = t.escala_y * lhy;
        let rx = ct * ex - st * ey;
        let ry = st * ex + ct * ey;
        (t.piv_x + t.tx + rx, t.piv_y + t.ty + ry)
    };
    let esquinas = [fwd(-w2, -h2), fwd(w2, -h2), fwd(w2, h2), fwd(-w2, h2)];
    // 8 handles: combos de {-w2,0,w2}×{-h2,0,h2} salvo el centro.
    let locs: [(f64, f64); 8] = [
        (-w2, -h2), (0.0, -h2), (w2, -h2),
        (-w2, 0.0),             (w2, 0.0),
        (-w2, h2),  (0.0, h2),  (w2, h2),
    ];
    let mut handles = [(0.0, 0.0, 0.0, 0.0); 8];
    for (k, &(lhx, lhy)) in locs.iter().enumerate() {
        let (wx, wy) = fwd(lhx, lhy);
        handles[k] = (wx, wy, lhx, lhy);
    }
    // Handle de rotación: ancla en el borde superior, separado 26 px-pantalla
    // hacia "arriba" en el marco rotado (sin que la escala lo estire).
    let rot_ancla = fwd(0.0, -h2);
    let up_x = st; // R(rot)·(0,-1)
    let up_y = -ct;
    let sep = 26.0 * inv_s;
    let rot_world = (rot_ancla.0 + up_x * sep, rot_ancla.1 + up_y * sep);
    PuntosTransform { esquinas, handles, rot_world, rot_ancla }
}

/// Entra al modo transformación libre sobre la capa raster seleccionada.
/// Captura su buffer original y el bbox de su contenido (o el lienzo entero si
/// está toda pintada / vacía). No-op con estado descriptivo si no hay capa o
/// es derivada.
pub(crate) fn iniciar_transform(model: &mut Model) -> bool {
    if model.transform.is_some() {
        return false;
    }
    let Some(id) = model.seleccionada else {
        model.estado = "no hay capa seleccionada".into();
        return false;
    };
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let Some(capa) = model.lienzo.capas.iter().find(|c| c.id == id) else {
        return false;
    };
    if !matches!(capa.origen, OrigenCapa::Raster) {
        model.estado = "la capa es derivada — usá la raster madre".into();
        return false;
    }
    let orig = capa.contenido;
    let Some(buf) = model.almacen.obtener(orig) else {
        return false;
    };
    // Pivote = centro del bbox del contenido; si está vacío, el lienzo entero.
    let (bx0, by0, bx1, by1) = match bbox_no_transparente(buf, w, h) {
        Some((x0, y0, x1, y1)) => (x0 as f64, y0 as f64, x1 as f64, y1 as f64),
        None => (0.0, 0.0, w as f64, h as f64),
    };
    model.transform = Some(TransformLibre {
        id,
        orig,
        piv_x: (bx0 + bx1) / 2.0,
        piv_y: (by0 + by1) / 2.0,
        bx0,
        by0,
        bx1,
        by1,
        tx: 0.0,
        ty: 0.0,
        escala_x: 1.0,
        escala_y: 1.0,
        rot: 0.0,
        agarre: None,
    });
    model.estado = "transformar · esquinas escalan · arriba rota · centro mueve · Enter aplica · Esc cancela".into();
    true
}

/// Press en modo transformar: convierte `(lx, ly)` a coords-imagen, hit-testea
/// el handle más cercano (esquinas/lados → escala; handle de arriba → rotar;
/// dentro del cuadro → mover) y arranca el drag. Sin handle bajo el cursor es
/// no-op (el cuadro sigue como estaba).
pub(crate) fn transform_press(model: &mut Model, lx: f32, ly: f32, rw: f32, rh: f32) {
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let Some((s, off_x, off_y)) =
        transform_lienzo(w, h, rw, rh, model.factor_zoom, model.pan_x, model.pan_y)
    else {
        return;
    };
    if s <= 0.0 {
        return;
    }
    let inv_s = 1.0 / s;
    // Cursor en coords-imagen.
    let cmx = (lx as f64 - off_x) / s;
    let cmy = (ly as f64 - off_y) / s;
    let Some(t) = model.transform.as_mut() else { return };
    let pts = puntos_transform(t, inv_s);
    // Umbral de agarre: 12 px-pantalla en coords-imagen.
    let umbral = 12.0 * inv_s;
    let umbral2 = umbral * umbral;
    let dist2 = |ax: f64, ay: f64| (ax - cmx) * (ax - cmx) + (ay - cmy) * (ay - cmy);
    // 1) Handle de rotación (prioridad sobre el resto para que no lo tape un
    //    handle de esquina cuando el cuadro es chico).
    if dist2(pts.rot_world.0, pts.rot_world.1) <= umbral2 {
        t.agarre = Some(Agarre {
            tipo: TipoAgarre::Rotar { rot0: t.rot },
            inv_s,
            acc_x: 0.0,
            acc_y: 0.0,
            q0x: pts.rot_world.0,
            q0y: pts.rot_world.1,
        });
        return;
    }
    // 2) Handles de escala: el más cercano dentro del umbral.
    let mut mejor: Option<(f64, usize)> = None;
    for (k, &(wx, wy, _, _)) in pts.handles.iter().enumerate() {
        let d = dist2(wx, wy);
        if d <= umbral2 && mejor.map(|(md, _)| d < md).unwrap_or(true) {
            mejor = Some((d, k));
        }
    }
    if let Some((_, k)) = mejor {
        let (wx, wy, hx, hy) = pts.handles[k];
        t.agarre = Some(Agarre {
            tipo: TipoAgarre::Escala { hx, hy },
            inv_s,
            acc_x: 0.0,
            acc_y: 0.0,
            q0x: wx,
            q0y: wy,
        });
        return;
    }
    // 3) Dentro del cuadro → mover. Volvemos el cursor al marco local.
    let ct = t.rot.cos();
    let st = t.rot.sin();
    let rx = cmx - t.piv_x - t.tx;
    let ry = cmy - t.piv_y - t.ty;
    let lvx = (ct * rx + st * ry) / t.escala_x;
    let lvy = (-st * rx + ct * ry) / t.escala_y;
    let w2 = (t.bx1 - t.bx0) / 2.0;
    let h2 = (t.by1 - t.by0) / 2.0;
    if lvx.abs() <= w2 && lvy.abs() <= h2 {
        t.agarre = Some(Agarre {
            tipo: TipoAgarre::Mover { tx0: t.tx, ty0: t.ty },
            inv_s,
            acc_x: 0.0,
            acc_y: 0.0,
            q0x: cmx,
            q0y: cmy,
        });
    }
}

/// Move durante el drag de transformación: acumula el delta de pantalla en
/// coords-imagen y recomputa el parámetro afectado (traslación / escala /
/// rotación) desde el ancla del press. Luego remuestrea y recompone en vivo.
pub(crate) fn transform_arrastrar(model: &mut Model, dx: f32, dy: f32) {
    let Some(t) = model.transform.as_mut() else { return };
    let Some(mut ag) = t.agarre else { return };
    ag.acc_x += dx as f64 * ag.inv_s;
    ag.acc_y += dy as f64 * ag.inv_s;
    match ag.tipo {
        TipoAgarre::Mover { tx0, ty0 } => {
            t.tx = tx0 + ag.acc_x;
            t.ty = ty0 + ag.acc_y;
        }
        TipoAgarre::Escala { hx, hy } => {
            // Handle vigente en coords-imagen; lo llevamos al marco pre-escala
            // (quitando pivote, traslación y rotación) → ahí `v = escala·h`.
            let qx = ag.q0x + ag.acc_x;
            let qy = ag.q0y + ag.acc_y;
            let ct = t.rot.cos();
            let st = t.rot.sin();
            let rx = qx - t.piv_x - t.tx;
            let ry = qy - t.piv_y - t.ty;
            let vx = ct * rx + st * ry;
            let vy = -st * rx + ct * ry;
            if hx.abs() > 1e-6 {
                t.escala_x = vx / hx;
            }
            if hy.abs() > 1e-6 {
                t.escala_y = vy / hy;
            }
        }
        TipoAgarre::Rotar { rot0 } => {
            let cx = t.piv_x + t.tx;
            let cy = t.piv_y + t.ty;
            let a0 = (ag.q0y - cy).atan2(ag.q0x - cx);
            let a1 = (ag.q0y + ag.acc_y - cy).atan2(ag.q0x + ag.acc_x - cx);
            t.rot = rot0 + (a1 - a0);
        }
    }
    t.agarre = Some(ag);
    aplicar_preview_transform(model);
}

/// Remuestrea `orig` con la afín vigente y la deja como contenido de la capa
/// (sin snapshot — es preview), recomponiendo el lienzo. No toca el historial.
pub(crate) fn aplicar_preview_transform(model: &mut Model) {
    let Some(t) = model.transform.as_ref() else { return };
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let id = t.id;
    let Some(src) = model.almacen.obtener(t.orig) else { return };
    let out = transformar_afin(
        src, w, h, t.piv_x, t.piv_y, t.escala_x, t.escala_y, t.rot, t.tx, t.ty,
    );
    let new_hash = model.almacen.insertar(out);
    if let Some(c) = model.lienzo.capa_mut(id) {
        c.contenido = new_hash;
    }
    model.lienzo.propagar_stale(id);
    aplicar_y_recomponer(model);
}

/// Confirma la transformación: deja el remuestreo final horneado, snapshotea y
/// sale del modo. No-op si no había sesión.
pub(crate) fn confirmar_transform(model: &mut Model) -> bool {
    let Some(t) = model.transform.take() else { return false };
    // Si nada cambió, sólo salimos (evita una entrada de historial vacía).
    let cambio = t.tx != 0.0
        || t.ty != 0.0
        || (t.escala_x - 1.0).abs() > 1e-6
        || (t.escala_y - 1.0).abs() > 1e-6
        || t.rot.abs() > 1e-6;
    if cambio {
        // Si la capa es vectorial, horneamos la afín en el PATH (queda crisp y
        // editable) en vez de conservar el remuestreo raster del preview.
        let es_vector = model
            .lienzo
            .capa(t.id)
            .map(|c| c.params_vector().is_some())
            .unwrap_or(false);
        if es_vector {
            let m = afin_de_transform(&t);
            model.seleccionada = Some(t.id);
            editar_vector_seleccionado(model, |p| p.transformar(m));
        }
        pushear_snapshot(model, None);
        model.estado = "transformación aplicada".into();
    } else {
        model.estado = "transformar · sin cambios".into();
    }
    true
}

/// Matriz afín `[a, b, c, d, e, f]` equivalente a la transformación libre `t`,
/// con la misma convención que `puntos_transform` (escala·rotación alrededor
/// del pivote + traslación). Aplicarla a un punto reproduce dónde el usuario ve
/// el cuadro de transformación.
fn afin_de_transform(t: &TransformLibre) -> [f32; 6] {
    let (ct, st) = (t.rot.cos(), t.rot.sin());
    let (sx, sy) = (t.escala_x, t.escala_y);
    let (px, py) = (t.piv_x, t.piv_y);
    let a = ct * sx;
    let b = st * sx;
    let c = -st * sy;
    let d = ct * sy;
    let e = px + t.tx - a * px - c * py;
    let f = py + t.ty - b * px - d * py;
    [a as f32, b as f32, c as f32, d as f32, e as f32, f as f32]
}

/// Cancela la transformación: restaura el buffer original de la capa y sale del
/// modo. No-op si no había sesión.
pub(crate) fn cancelar_transform(model: &mut Model) -> bool {
    let Some(t) = model.transform.take() else { return false };
    if let Some(c) = model.lienzo.capa_mut(t.id) {
        c.contenido = t.orig;
    }
    model.lienzo.propagar_stale(t.id);
    aplicar_y_recomponer(model);
    model.estado = "transformación cancelada".into();
    true
}

/// Bounding box de los píxeles `> 0` de una máscara `W·H`. `None` si la
/// máscara está toda en cero (selección vacía).
fn bbox_de_mascara(mascara: &[u8], w: u32, h: u32) -> Option<RectImagen> {
    let w_us = w as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    let mut hay = false;
    for y in 0..h {
        for x in 0..w {
            if mascara[y as usize * w_us + x as usize] > 0 {
                hay = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if hay {
        Some(RectImagen { x0, y0, x1, y1 })
    } else {
        None
    }
}

/// Varita mágica / selección por color contigua: compone el lienzo vigente,
/// inunda desde `(sx, sy)` con tolerancia [`TOL_BALDE`] y guarda el resultado
/// como **máscara de selección** (`model.seleccion_mascara`) más su bounding
/// box en `model.seleccion`. No toca píxeles ni el historial. Devuelve `false`
/// (con estado descriptivo) si la semilla cae fuera o la región sale vacía.
pub(crate) fn seleccionar_por_color(model: &mut Model, sx: u32, sy: u32) -> bool {
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    if sx >= w || sy >= h {
        model.estado = "varita · fuera de la imagen".into();
        return false;
    }
    let img = match tullpu_render::componer(&model.lienzo, &model.almacen) {
        Ok(img) => img,
        Err(_) => {
            model.estado = "varita · no se pudo componer".into();
            return false;
        }
    };
    let Some(mascara) = flood_mascara(img.as_raw(), w, h, sx, sy, TOL_BALDE) else {
        model.estado = "varita · semilla inválida".into();
        return false;
    };
    // Shift sostenido ⇒ suma a la selección vigente (unión).
    fijar_o_sumar_mascara(model, mascara, model.shift_held, "varita")
}

/// Reconstruye el overlay cacheado de la selección desde `seleccion_mascara`:
/// una imagen `W·H` con cian translúcido donde la máscara está marcada y
/// transparente fuera. Si no hay máscara, deja el overlay en `None`. Lo
/// dibuja el painter del lienzo encima del composite.
pub(crate) fn sincronizar_overlay_seleccion(model: &mut Model) {
    use llimphi_ui::llimphi_raster::peniko::{
        Blob, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
    };
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let overlay = model.seleccion_mascara.and_then(|hm| {
        let mascara = model.almacen.obtener(hm)?;
        let n = (w as usize) * (h as usize);
        if mascara.len() != n {
            return None;
        }
        let mut rgba = vec![0u8; n * 4];
        for (i, &m) in mascara.iter().enumerate() {
            if m > 127 {
                // Cian translúcido (premult-agnóstico: alpha straight).
                rgba[i * 4] = 40;
                rgba[i * 4 + 1] = 180;
                rgba[i * 4 + 2] = 255;
                rgba[i * 4 + 3] = 90;
            }
        }
        Some(Image::new(ImageData {
            data: Blob::from(rgba),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: w,
            height: h,
        }))
    });
    model.seleccion_overlay = overlay;
}

/// Fija la máscara `nueva` como selección, o la **suma** (unión por píxel,
/// `max`) a la máscara vigente si `sumar` es `true` (modo Shift). Recalcula el
/// bounding box y guarda todo en el modelo. Devuelve `false` si la unión
/// resultante queda vacía. Es el punto común de varita y lazo.
fn fijar_o_sumar_mascara(model: &mut Model, mut nueva: Vec<u8>, sumar: bool, verbo: &str) -> bool {
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    if sumar {
        if let Some(prev) = cobertura_seleccion(model) {
            for (n, p) in nueva.iter_mut().zip(prev.iter()) {
                *n = (*n).max(*p);
            }
        }
    }
    let Some(bbox) = bbox_de_mascara(&nueva, w, h) else {
        model.estado = format!("{verbo} · región vacía");
        return false;
    };
    let count = nueva.iter().filter(|&&v| v > 0).count();
    let hash = model.almacen.insertar(nueva);
    model.seleccion_mascara = Some(hash);
    model.seleccion = Some(bbox);
    model.seleccion_drag = None;
    model.mover_drag = None;
    sincronizar_overlay_seleccion(model);
    model.estado = format!("{verbo} · {count} px seleccionados");
    true
}

/// Invierte la selección vigente (lo seleccionado pasa a no estarlo y
/// viceversa, dentro del lienzo). Materializa la cobertura actual (máscara o
/// rect), la complementa y la guarda como máscara. No-op si no hay selección.
pub(crate) fn invertir_seleccion(model: &mut Model) -> bool {
    let Some(mut cov) = cobertura_seleccion(model) else {
        model.estado = "no hay selección que invertir".into();
        return false;
    };
    for v in cov.iter_mut() {
        *v = 255 - *v;
    }
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let Some(bbox) = bbox_de_mascara(&cov, w, h) else {
        // El complemento es vacío ⇒ estaba todo seleccionado; ahora nada.
        model.seleccion = None;
        model.seleccion_mascara = None;
        model.seleccion_overlay = None;
        model.estado = "selección invertida · vacía".into();
        return true;
    };
    let count = cov.iter().filter(|&&v| v > 0).count();
    let hash = model.almacen.insertar(cov);
    model.seleccion_mascara = Some(hash);
    model.seleccion = Some(bbox);
    sincronizar_overlay_seleccion(model);
    model.estado = format!("selección invertida · {count} px");
    true
}

/// Lazo: rasteriza el polígono `puntos` (coords-imagen) a una máscara de
/// selección por relleno par-impar y la guarda en `model.seleccion_mascara`
/// con su bounding box. No toca píxeles ni el historial. No-op (con estado)
/// si el polígono tiene < 3 vértices o el área sale vacía.
pub(crate) fn seleccionar_lazo(model: &mut Model, puntos: &[(i32, i32)]) -> bool {
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    if puntos.len() < 3 {
        model.estado = "lazo · trazo muy corto".into();
        return false;
    }
    let mascara = poligono_a_mascara(puntos, w, h);
    fijar_o_sumar_mascara(model, mascara, model.shift_held, "lazo")
}

/// Cobertura de selección como máscara `W·H` (255 = seleccionado). Prefiere
/// `seleccion_mascara` (forma exacta de la varita); si no, sintetiza desde el
/// rect `seleccion`; `None` cuando no hay selección (= lienzo entero). Es el
/// punto único que consultan las ops destructivas para acotar por píxel.
pub(crate) fn cobertura_seleccion(model: &Model) -> Option<Vec<u8>> {
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    let n = (w as usize) * (h as usize);
    if let Some(hm) = model.seleccion_mascara {
        return model.almacen.obtener(hm).filter(|b| b.len() == n).map(|b| b.to_vec());
    }
    let rect = model.seleccion?;
    let mut m = vec![0u8; n];
    let x0 = rect.x0.min(w);
    let y0 = rect.y0.min(h);
    let x1 = rect.x1.min(w);
    let y1 = rect.y1.min(h);
    for y in y0..y1 {
        for x in x0..x1 {
            m[y as usize * w as usize + x as usize] = 255;
        }
    }
    Some(m)
}

/// Aplica un mutador per-píxel `f(&mut [r,g,b,a])` a los píxeles seleccionados
/// (máscara o rect) de la capa raster seleccionada. Comparte la validación
/// entre limpiar y rellenar y soporta selecciones no rectangulares. No-op si
/// no hay selección/capa, la capa es derivada, o el buffer no cambia (mismo
/// hash). `verbo`/`sin_cambio` describen los desenlaces.
fn aplicar_px_en_seleccion(
    model: &mut Model,
    f: impl Fn(&mut [u8]),
    verbo: &str,
    sin_cambio: &str,
) -> bool {
    let Some(cobertura) = cobertura_seleccion(model) else {
        model.estado = "no hay selección — `r` y arrastrar".into();
        return false;
    };
    let Some(id) = model.seleccionada else {
        model.estado = "no hay capa seleccionada".into();
        return false;
    };
    let Some(capa) = model.lienzo.capas.iter().find(|c| c.id == id) else {
        return false;
    };
    if !matches!(capa.origen, OrigenCapa::Raster) {
        model.estado = "la capa seleccionada es derivada — usá la raster madre".into();
        return false;
    }
    let hash_actual = capa.contenido;
    let Some(src) = model.almacen.obtener(hash_actual) else {
        return false;
    };
    let mut buf = src.to_vec();
    let n = cobertura.len().min(buf.len() / 4);
    let mut tocados = 0usize;
    for i in 0..n {
        if cobertura[i] > 127 {
            f(&mut buf[i * 4..i * 4 + 4]);
            tocados += 1;
        }
    }
    let new_hash = model.almacen.insertar(buf);
    if new_hash == hash_actual {
        model.estado = sin_cambio.into();
        return false;
    }
    if let Some(capa_mut) = model.lienzo.capa_mut(id) {
        capa_mut.contenido = new_hash;
    }
    model.lienzo.propagar_stale(id);
    aplicar_y_recomponer(model);
    model.estado = format!("{verbo} ({tocados} px)");
    true
}

/// Aplica una transformación de buffer al **rect** de `model.seleccion` dentro
/// de la capa raster seleccionada (path histórico para selecciones
/// rectangulares; el path por máscara es [`aplicar_px_en_seleccion`]).
/// `transformar(src, w, x0, y0, x1, y1)` produce el buffer nuevo. Re-clampea el
/// rect contra el lienzo vigente. No-op si: no hay selección/capa, la capa es
/// derivada, el rect queda con área cero, o el buffer no cambia (mismo hash).
/// Propaga stale y recompone; la selección se mantiene.
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
        model.estado = "la capa seleccionada es derivada — usá la raster madre".into();
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
/// la capa raster seleccionada (ver [`aplicar_px_en_seleccion`]).
/// La selección se mantiene — encaja con flujos tipo "marquee + Delete
/// + re-pintar"; un Esc la limpia explícitamente.
pub(crate) fn limpiar_seleccion_en_capa(model: &mut Model) -> bool {
    // Con máscara (varita) gateamos por píxel; con rect-solo usamos el path
    // rectangular histórico (mismos mensajes/edge-cases).
    if model.seleccion_mascara.is_some() {
        aplicar_px_en_seleccion(
            model,
            |px| px[3] = 0,
            "limpiada selección",
            "selección ya transparente, nada que limpiar",
        )
    } else {
        aplicar_a_seleccion_en_capa(
            model,
            limpiar_rect_en_buffer,
            "limpiada selección",
            "selección ya transparente, nada que limpiar",
        )
    }
}

/// Rellena los píxeles del rect de `model.seleccion` con el color
/// activo (`color_picked`, o `RELLENO_DEFAULT` si no se leyó ninguno)
/// dentro de la capa raster seleccionada (ver
/// [`aplicar_px_en_seleccion`]). No-op extra si el rect ya tenía
/// ese color exacto (hash sin cambio).
pub(crate) fn rellenar_seleccion_en_capa(model: &mut Model) -> bool {
    let rgba = model.color_picked.unwrap_or(RELLENO_DEFAULT);
    if model.seleccion_mascara.is_some() {
        aplicar_px_en_seleccion(
            model,
            move |px| px.copy_from_slice(&rgba),
            "rellenada selección",
            "selección ya tenía ese color, sin cambio",
        )
    } else {
        aplicar_a_seleccion_en_capa(
            model,
            |src, w, x0, y0, x1, y1| rellenar_rect_en_buffer(src, w, x0, y0, x1, y1, rgba),
            "rellenada selección",
            "selección ya tenía ese color, sin cambio",
        )
    }
}

// =============================================================================
//  Máscaras de capa (fase 52)
// =============================================================================
//
// Una máscara es un buffer de un canal (W·H bytes) que multiplica el alfa
// de la capa al componer (lo aplica `tullpu-render`): 255 = totalmente
// visible, 0 = totalmente oculto. Es no destructiva — vive en el campo
// `Capa::mascara` aparte del contenido y se puede invertir o quitar sin
// tocar los píxeles. "Aplicar" la hornea al alfa del raster y la borra.

/// Agrega una máscara blanca (todo 255 = nada oculto) del tamaño del
/// lienzo a la capa seleccionada. No-op si no hay capa seleccionada o la
/// capa ya tiene máscara (para no pisar una existente).
pub(crate) fn agregar_mascara(model: &mut Model) -> bool {
    let Some(id) = model.seleccionada else {
        model.estado = "no hay capa seleccionada".into();
        return false;
    };
    let w = model.lienzo.width;
    let h = model.lienzo.height;
    if let Some(capa) = model.lienzo.capas.iter().find(|c| c.id == id) {
        if capa.mascara.is_some() {
            model.estado = "la capa ya tiene máscara".into();
            return false;
        }
    } else {
        return false;
    }
    let buffer = tullpu_render::buffer_mascara(w, h, 255);
    let hash = model.almacen.insertar(buffer);
    if let Some(capa) = model.lienzo.capa_mut(id) {
        capa.mascara = Some(hash);
    }
    aplicar_y_recomponer(model);
    model.estado = "máscara agregada (blanca · nada oculto)".into();
    true
}

/// Agrega una máscara construida desde la selección activa: 255 dentro
/// del rect (visible), 0 fuera (oculto). Reemplaza cualquier máscara
/// existente. No-op si no hay selección o capa seleccionada. Es la vía
/// no destructiva equivalente a "recortar a selección" sin perder los
/// píxeles de afuera.
pub(crate) fn agregar_mascara_de_seleccion(model: &mut Model) -> bool {
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
    let mut buffer = vec![0u8; (w as usize) * (h as usize)];
    for y in y0..y1 {
        let fila = (y as usize) * (w as usize);
        for x in x0..x1 {
            buffer[fila + x as usize] = 255;
        }
    }
    let hash = model.almacen.insertar(buffer);
    if let Some(capa) = model.lienzo.capa_mut(id) {
        capa.mascara = Some(hash);
    }
    aplicar_y_recomponer(model);
    model.estado = format!("máscara desde selección {}×{}", x1 - x0, y1 - y0);
    true
}

/// Invierte la máscara de la capa seleccionada (255 ↔ 0): lo visible se
/// oculta y viceversa. No-op si no hay capa o la capa no tiene máscara.
pub(crate) fn invertir_mascara(model: &mut Model) -> bool {
    let Some(id) = model.seleccionada else {
        model.estado = "no hay capa seleccionada".into();
        return false;
    };
    let mh = match model.lienzo.capas.iter().find(|c| c.id == id) {
        Some(capa) => match capa.mascara {
            Some(h) => h,
            None => {
                model.estado = "la capa no tiene máscara que invertir".into();
                return false;
            }
        },
        None => return false,
    };
    let Some(src) = model.almacen.obtener(mh) else {
        return false;
    };
    let inv: Vec<u8> = src.iter().map(|b| 255 - b).collect();
    let hash = model.almacen.insertar(inv);
    if let Some(capa) = model.lienzo.capa_mut(id) {
        capa.mascara = Some(hash);
    }
    aplicar_y_recomponer(model);
    model.estado = "máscara invertida".into();
    true
}

/// Quita la máscara de la capa seleccionada (vuelve a `None` — la capa
/// se compone entera). No destruye píxeles. No-op si no hay máscara.
pub(crate) fn quitar_mascara(model: &mut Model) -> bool {
    let Some(id) = model.seleccionada else {
        model.estado = "no hay capa seleccionada".into();
        return false;
    };
    match model.lienzo.capas.iter().find(|c| c.id == id) {
        Some(capa) if capa.mascara.is_none() => {
            model.estado = "la capa no tiene máscara".into();
            return false;
        }
        Some(_) => {}
        None => return false,
    }
    if let Some(capa) = model.lienzo.capa_mut(id) {
        capa.mascara = None;
    }
    aplicar_y_recomponer(model);
    model.estado = "máscara quitada".into();
    true
}

/// Hornea (aplica) la máscara al alfa del raster seleccionado y la
/// quita: `alfa_nuevo = alfa · mascara / 255` por píxel. Operación
/// destructiva (a diferencia de quitar, que preserva la imagen entera).
/// Sólo para capas raster — el buffer de una derivada es cache y se
/// regeneraría en el próximo recompose. No-op si no hay máscara.
pub(crate) fn aplicar_mascara(model: &mut Model) -> bool {
    let Some(id) = model.seleccionada else {
        model.estado = "no hay capa seleccionada".into();
        return false;
    };
    let (contenido, mh) = match model.lienzo.capas.iter().find(|c| c.id == id) {
        Some(capa) => {
            if !matches!(capa.origen, OrigenCapa::Raster) {
                model.estado =
                    "la capa es derivada — aplicar máscara sólo en raster".into();
                return false;
            }
            match capa.mascara {
                Some(m) => (capa.contenido, m),
                None => {
                    model.estado = "la capa no tiene máscara que aplicar".into();
                    return false;
                }
            }
        }
        None => return false,
    };
    let Some(src) = model.almacen.obtener(contenido).map(|s| s.to_vec()) else {
        return false;
    };
    let Some(mask) = model.almacen.obtener(mh).map(|s| s.to_vec()) else {
        return false;
    };
    // El alfa de cada píxel se escala por el byte de máscara. Buffers
    // siempre del mismo conteo de píxeles (mantenido por crop/rotar).
    let n = src.len() / 4;
    let mut out = src.clone();
    for i in 0..n.min(mask.len()) {
        let a = out[i * 4 + 3] as u16;
        let m = mask[i] as u16;
        out[i * 4 + 3] = ((a * m) / 255) as u8;
    }
    let new_hash = model.almacen.insertar(out);
    if let Some(capa) = model.lienzo.capa_mut(id) {
        capa.contenido = new_hash;
        capa.mascara = None;
    }
    model.lienzo.propagar_stale(id);
    aplicar_y_recomponer(model);
    model.estado = "máscara aplicada al alfa".into();
    true
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
        // La máscara (1 byte/píxel) rota con su capa — si no, quedaría
        // con dims traspuestas y el render fallaría por tamaño.
        if let Some(mh) = capa.mascara {
            if let Some(ms) = model.almacen.obtener(mh) {
                let ms = ms.to_vec();
                let mr = if cw {
                    rotar_buffer_90_cw_bpp(&ms, w_old, h_old, 1)
                } else {
                    rotar_buffer_90_ccw_bpp(&ms, w_old, h_old, 1)
                };
                capa.mascara = Some(model.almacen.insertar(mr));
            }
        }
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

