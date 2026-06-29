//! Inicialización y carga de la app `tullpu`: arranque del `Model`,
//! ingestión de archivos (PNG/PSD), resolución del proveedor IA, walk
//! del workspace para el picker y el gradiente procedural por defecto.
//!
//! Behavior-preserving split de `main.rs` — sin cambios funcionales.

use std::path::{Path, PathBuf};

use llimphi_clipboard::SystemClipboard;
use pixel_verbo_core::Proveedor;
use pixel_verbo_daemon::ClienteBloqueante;
use pixel_verbo_mock::ProveedorMock;
use tullpu_core::{Capa, Historial, Lienzo};
use tullpu_render::AlmacenEnMemoria;
use uuid::Uuid;

use std::collections::HashMap;
use crate::compose::{histograma_rgb, recomponer, sincronizar_thumbs};
use crate::model::*;

pub(crate) fn buffer_gradiente() -> Vec<u8> {
    let mut v = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let r = (x * 255 / W.max(1)) as u8;
            let g = (y * 255 / H.max(1)) as u8;
            let b = (((x + y) / 2).min(255)) as u8;
            v.extend_from_slice(&[r, g, b, 255]);
        }
    }
    v
}

pub(crate) fn cargar_png(path: &std::path::Path) -> Option<(u32, u32, Vec<u8>)> {
    let reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let dyn_img = reader.decode().ok()?;
    let rgba = dyn_img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Some((w, h, rgba.into_raw()))
}

/// Ajusta un buffer Rgba8 de `src_w × src_h` al tamaño `dst_w × dst_h` del
/// lienzo destino. Si las dimensiones ya cuadran devuelve el buffer
/// directamente (clone barato — `Vec<u8>` move). Si no, hace un *fit-contain*
/// preservando aspect ratio (resize Lanczos3 al rectángulo más grande que
/// entra) y pad transparente centrado para llenar el resto. La capa nueva
/// "asoma" sobre el lienzo en lugar de deformarse — es lo que un editor de
/// imágenes hace cuando arrastrás una textura más chica.
pub(crate) fn ajustar_a_lienzo(
    src: Vec<u8>,
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Option<Vec<u8>> {
    if src_w == dst_w && src_h == dst_h {
        return Some(src);
    }
    let img = image::RgbaImage::from_raw(src_w, src_h, src)?;
    // Factor de escala fit-contain: el menor entre proporción ancho/alto.
    // Si el lienzo es 0×N o N×0 (defensivo), no podemos hacer nada útil.
    if dst_w == 0 || dst_h == 0 || src_w == 0 || src_h == 0 {
        return None;
    }
    let sx = dst_w as f32 / src_w as f32;
    let sy = dst_h as f32 / src_h as f32;
    let s = sx.min(sy);
    let new_w = ((src_w as f32 * s).round() as u32).max(1).min(dst_w);
    let new_h = ((src_h as f32 * s).round() as u32).max(1).min(dst_h);
    let escalada = image::imageops::resize(
        &img,
        new_w,
        new_h,
        image::imageops::FilterType::Lanczos3,
    );
    // Lienzo destino lleno de transparente, después blit centrado.
    let mut destino = image::RgbaImage::from_pixel(dst_w, dst_h, image::Rgba([0, 0, 0, 0]));
    let off_x = ((dst_w - new_w) / 2) as i64;
    let off_y = ((dst_h - new_h) / 2) as i64;
    image::imageops::overlay(&mut destino, &escalada, off_x, off_y);
    Some(destino.into_raw())
}

pub(crate) fn socket_pixel_verbo() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(xdg).join("pixel-verbo.sock");
    }
    PathBuf::from("/tmp/pixel-verbo.sock")
}

/// Intenta hablar con el daemon; si no responde, cae al mock. La elección
/// queda visible en el header — el usuario sabe contra qué está pegando.
pub(crate) fn resolver_proveedor() -> (Box<dyn Proveedor>, String) {
    let sock = socket_pixel_verbo();
    match ClienteBloqueante::conectar(&sock) {
        Ok(cli) => {
            let etiqueta = format!("daemon {} @ {}", cli.model_id(), sock.display());
            (Box::new(cli), etiqueta)
        }
        Err(_) => {
            let mock = ProveedorMock::nuevo();
            let etiqueta = format!("mock {}", mock.model_id());
            (Box::new(mock), etiqueta)
        }
    }
}

/// Decide cómo materializar el path recibido por CLI según extensión:
/// `.psd` → import multi-capa via `foreign-psd`; el resto cae al loader PNG
/// existente y se convierte en una capa raster única. Devuelve el lienzo
/// armado, su almacén poblado, el `Uuid` que la UI debe seleccionar al
/// arrancar y una etiqueta corta para el header.
pub(crate) fn cargar_arg(path: &std::path::Path) -> Option<(Lienzo, AlmacenEnMemoria, Uuid, String)> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_lowercase);
    match ext.as_deref() {
        Some("psd") => cargar_psd(path),
        Some("svg") => cargar_svg(path),
        _ => cargar_png_como_capa(path),
    }
}

/// `.svg` → import a capas vectoriales nativas (vía `foreign-svg`). Cada path
/// del SVG se rasteriza al tamaño del lienzo y se cuelga como `Capa::vector`,
/// conservando sus params (re-editables). El lienzo toma el tamaño que declara
/// el SVG.
pub(crate) fn cargar_svg(path: &std::path::Path) -> Option<(Lienzo, AlmacenEnMemoria, Uuid, String)> {
    let bytes = std::fs::read(path).ok()?;
    let imp = match foreign_svg::importar_svg(&bytes) {
        Ok(imp) => imp,
        Err(e) => {
            eprintln!("tullpu: error importando '{}': {e}", path.display());
            return None;
        }
    };
    let (w, h) = (imp.width.max(1), imp.height.max(1));
    let mut almacen = AlmacenEnMemoria::nuevo();
    let mut lienzo = Lienzo::nuevo(w, h);
    let n = imp.capas.len();
    for (i, params) in imp.capas.into_iter().enumerate() {
        let buffer = tullpu_ops::rasterizar_vector(&params, w, h);
        let hash = almacen.insertar(buffer);
        lienzo.apilar(Capa::vector(format!("path {}", i + 1), hash, params));
    }
    let id = lienzo.capas.first()?.id;
    Some((lienzo, almacen, id, format!("svg · {n} paths · {w}×{h}")))
}

pub(crate) fn cargar_png_como_capa(
    path: &std::path::Path,
) -> Option<(Lienzo, AlmacenEnMemoria, Uuid, String)> {
    let (w, h, bytes) = cargar_png(path)?;
    let mut almacen = AlmacenEnMemoria::nuevo();
    let hash = almacen.insertar(bytes);
    let mut lienzo = Lienzo::nuevo(w, h);
    let fondo = Capa::raster("fondo", hash);
    let id = fondo.id;
    lienzo.apilar(fondo);
    Some((lienzo, almacen, id, format!("png cargado · {}×{}", w, h)))
}

pub(crate) fn cargar_psd(path: &std::path::Path) -> Option<(Lienzo, AlmacenEnMemoria, Uuid, String)> {
    let bytes = std::fs::read(path).ok()?;
    let imp = match foreign_psd::importar_psd(&bytes) {
        Ok(imp) => imp,
        Err(e) => {
            eprintln!("tullpu: error importando '{}': {e}", path.display());
            return None;
        }
    };
    // Vuelco todos los buffers (uno por capa, dedup ya hecho por hash) al
    // almacén de la app — `tullpu-render::componer` los va a pedir por hash.
    let mut almacen = AlmacenEnMemoria::nuevo();
    almacen.buffers.extend(imp.buffers);
    let n_capas = imp.lienzo.capas.len();
    let n_degradadas = imp.informe.caidas_a_normal.len();
    let n_grupos = imp.informe.grupos_detectados;
    let n_rasterizados = imp.informe.grupos_rasterizados.len();
    // El primer Uuid (fondo) sirve como selección inicial — si el PSD vino
    // vacío (foreign-psd ya lo rechaza, pero por defensa), caemos a default.
    let id = imp.lienzo.capas.first()?.id;
    // Etiqueta progresiva: prefijo base + anotaciones sólo si hubo
    // divergencias o composición intermedia.
    let mut etiqueta = format!("psd · {} capas", n_capas);
    if n_grupos > 0 {
        etiqueta.push_str(&format!(" · {} grupos", n_grupos));
    }
    let mut anotaciones: Vec<String> = Vec::new();
    if n_degradadas > 0 {
        anotaciones.push(format!("{} blend→Normal", n_degradadas));
    }
    if n_rasterizados > 0 {
        anotaciones.push(format!("{} rasterizados", n_rasterizados));
    }
    if !anotaciones.is_empty() {
        etiqueta.push_str(&format!(" ({})", anotaciones.join(", ")));
    }
    Some((imp.lienzo, almacen, id, etiqueta))
}

pub(crate) fn inicializar() -> Model {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    let (lienzo, almacen, id_inicial, estado) = arg
        .as_ref()
        .and_then(|p| cargar_arg(p))
        .unwrap_or_else(lienzo_default);

    let (proveedor, proveedor_etiqueta) = resolver_proveedor();

    // Raíz para el fuzzy picker: CWD del proceso. Si `getcwd` falla (raro),
    // caemos al directorio del arg de CLI o a "/".
    let raiz = std::env::current_dir()
        .ok()
        .or_else(|| arg.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)))
        .unwrap_or_else(|| PathBuf::from("/"));
    let imagenes_disponibles = walk_imagenes(&raiz);

    let imagen = recomponer(&lienzo, &almacen);
    let hist = Historial::nuevo(lienzo.clone(), HIST_CAP);
    let mut model = Model {
        lienzo,
        almacen,
        seleccionada: Some(id_inicial),
        imagen,
        estado,
        proveedor,
        proveedor_etiqueta,
        thumbs: HashMap::new(),
        raiz,
        imagenes_disponibles,
        picker: None,
        renombrando: None,
        hist,
        factor_zoom: 1.0,
        pan_x: 0.0,
        pan_y: 0.0,
        herramienta: Herramienta::Mover,
        color_picked: None,
        histograma: None,
        seleccion: None,
        seleccion_mascara: None,
        seleccion_overlay: None,
        seleccion_drag: None,
        mover_drag: None,
        pincel_drag: None,
        radio_pincel: RADIO_PINCEL,
        dureza_pincel: DUREZA_PINCEL,
        shift_held: false,
        alt_held: false,
        clon_ancla: None,
        clon_offset: None,
        ultimo_pincel: None,
        simetria: Simetria::Ninguna,
        gradiente_drag: None,
        lazo_drag: None,
        editando_texto: None,
        portapapeles: None,
        editando_mascara: false,
        valor_mascara: 255,
        thumbs_mascara: HashMap::new(),
        curva_arrastrando: None,
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: llimphi_motion::Tween::idle(1.0),
        context_menu: None,
        edit_menu: None,
        edit_active: usize::MAX,
        edit_anim: llimphi_motion::Tween::idle(1.0),
        clipboard: SystemClipboard::new(),
        toasts: Vec::new(),
        next_toast: 0,
        transform: None,
    };
    sincronizar_thumbs(&mut model);
    // Cómputo inicial del histograma desde el composite recién armado.
    model.histograma = model
        .imagen
        .as_ref()
        .map(|img| histograma_rgb(img.image.data.data()));
    model
}

/// Walk recursivo bajo `raiz` quedándose con extensiones de imagen
/// (`.png`, `.jpg`, `.jpeg`). Salta dotfiles, `target/`, `node_modules/`
/// y `pkg/` (artefactos wasm) — los mismos exclusions que `nada`. Cap a
/// 50k entries para que un directorio gigante no funda RAM. Devuelve
/// paths absolutos ordenados.
pub(crate) fn walk_imagenes(raiz: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![raiz.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= PICKER_FILE_CAP {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else {
                continue;
            };
            if name_str.starts_with('.')
                || name_str == "target"
                || name_str == "node_modules"
                || name_str == "pkg"
            {
                continue;
            }
            let path = entry.path();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                stack.push(path);
            } else if es_imagen_soportada(&path) {
                out.push(path);
                if out.len() >= PICKER_FILE_CAP {
                    break;
                }
            }
        }
    }
    out.sort();
    out
}

pub(crate) fn es_imagen_soportada(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png") | Some("jpg") | Some("jpeg")
    )
}

pub(crate) fn lienzo_default() -> (Lienzo, AlmacenEnMemoria, Uuid, String) {
    let mut almacen = AlmacenEnMemoria::nuevo();
    let hash = almacen.insertar(buffer_gradiente());
    let mut lienzo = Lienzo::nuevo(W, H);
    let fondo = Capa::raster("fondo", hash);
    let id = fondo.id;
    lienzo.apilar(fondo);
    (lienzo, almacen, id, "listo · gradiente demo".into())
}

