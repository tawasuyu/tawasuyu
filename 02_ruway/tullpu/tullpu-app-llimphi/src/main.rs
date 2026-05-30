//! `tullpu` — app de escritorio Llimphi: lienzo central + panel de capas +
//! paleta de operaciones (locales e IA). MVP del editor de imágenes por
//! capas IA-able.
//!
//! Layout:
//!
//! ```text
//! ┌───────────────────────────────────────────────────────────┐
//! │ header: dimensiones · proveedor IA · estado               │
//! ├──────────────┬─────────────────────────────┬──────────────┤
//! │ capas        │                             │ locales      │
//! │  • fondo     │        LIENZO compuesto     │  + Invertir  │
//! │  • inversión │        (peniko::Image)      │  + Brillo+   │
//! │  • brillo    │                             │  …           │
//! │              │                             │ IA           │
//! │ [+ raster]   │                             │  + Restyle   │
//! │              │                             │  + Segmentar │
//! │              │                             │  + Inpaint   │
//! │              │                             │  + Generar   │
//! └──────────────┴─────────────────────────────┴──────────────┘
//! ```
//!
//! Cada panel de capa es un botón clicable que la selecciona; el panel
//! derecho aplica una op nueva como capa derivada de la seleccionada.
//! Las ops IA se delegan al [`pixel_verbo_core::Proveedor`] que la app
//! resuelve al arranque: si encuentra el daemon `pixel-verbo-daemon` en
//! `$XDG_RUNTIME_DIR/pixel-verbo.sock` lo usa; si no, cae al `ProveedorMock`
//! en proceso — así el botón "Generar" igual funciona sin daemon corriendo.
//! Cada cambio dispara `regenerar_stale_con_ia` + `componer` sincrónicamente.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use llimphi_module_file_picker::{
    self as picker, PickerAction, PickerMsg, PickerPalette, PickerState,
};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, KeyEvent, View};
use llimphi_widget_button::{button_styled, button_view, ButtonPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};

use pixel_verbo_core::{OpPixel, Proveedor};
use pixel_verbo_daemon::ClienteBloqueante;
use pixel_verbo_mock::ProveedorMock;
use tullpu_core::{
    Capa, Frescura, Hash, Lienzo, ModoFusion, OpLocal, OrigenCapa, TransformacionPixel,
};
use tullpu_ops::{regenerar_stale_con_ia, transformacion_ia};
use tullpu_render::{componer, AlmacenEnMemoria, FormatoExport, FuenteBuffers};
use uuid::Uuid;

// =============================================================================
//  Model & Msg
// =============================================================================

struct Model {
    lienzo: Lienzo,
    almacen: AlmacenEnMemoria,
    seleccionada: Option<Uuid>,
    imagen: Option<Image>,
    estado: String,
    proveedor: Box<dyn Proveedor>,
    proveedor_etiqueta: String,
    /// Cache de thumbnails por hash del buffer Rgba8. Una entrada se reusa
    /// mientras el hash siga vivo en alguna capa; tras `regenerar_stale`
    /// hacemos un GC simple sobre los hashes presentes en el lienzo.
    thumbs: HashMap<Hash, Image>,
    /// Raíz desde la que se walkearon los candidatos (CWD al arrancar).
    /// El picker pinta paths relativos a esta raíz.
    raiz: PathBuf,
    /// Lista de archivos imagen detectados bajo `raiz`. Se walkea una vez
    /// al arrancar; reordenar el lienzo no la recalcula.
    imagenes_disponibles: Vec<PathBuf>,
    /// Estado del fuzzy picker. `None` cuando está cerrado.
    picker: Option<PickerState>,
}

#[derive(Clone)]
enum Msg {
    Seleccionar(Uuid),
    ToggleVisible(Uuid),
    BumpOpacidad(Uuid, f32),
    CiclarBlend(Uuid),
    MoverArriba(Uuid),
    MoverAbajo(Uuid),
    Duplicar(Uuid),
    Eliminar(Uuid),
    Agregar(OpLocal),
    AgregarIa(OpPixel),
    Recargar,
    Exportar(FormatoExport),
    Picker(PickerMsg),
    FileDrop(PathBuf),
}

// =============================================================================
//  Inicialización: gradiente procedural como capa fondo
// =============================================================================

const W: u32 = 512;
const H: u32 = 320;

fn buffer_gradiente() -> Vec<u8> {
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

fn cargar_png(path: &std::path::Path) -> Option<(u32, u32, Vec<u8>)> {
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
fn ajustar_a_lienzo(
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

fn socket_pixel_verbo() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(xdg).join("pixel-verbo.sock");
    }
    PathBuf::from("/tmp/pixel-verbo.sock")
}

/// Intenta hablar con el daemon; si no responde, cae al mock. La elección
/// queda visible en el header — el usuario sabe contra qué está pegando.
fn resolver_proveedor() -> (Box<dyn Proveedor>, String) {
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
fn cargar_arg(path: &std::path::Path) -> Option<(Lienzo, AlmacenEnMemoria, Uuid, String)> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_lowercase);
    match ext.as_deref() {
        Some("psd") => cargar_psd(path),
        _ => cargar_png_como_capa(path),
    }
}

fn cargar_png_como_capa(
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

fn cargar_psd(path: &std::path::Path) -> Option<(Lienzo, AlmacenEnMemoria, Uuid, String)> {
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

fn inicializar() -> Model {
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
    };
    sincronizar_thumbs(&mut model);
    model
}

/// Walk recursivo bajo `raiz` quedándose con extensiones de imagen
/// (`.png`, `.jpg`, `.jpeg`). Salta dotfiles, `target/`, `node_modules/`
/// y `pkg/` (artefactos wasm) — los mismos exclusions que `nada`. Cap a
/// 50k entries para que un directorio gigante no funda RAM. Devuelve
/// paths absolutos ordenados.
const PICKER_FILE_CAP: usize = 50_000;
fn walk_imagenes(raiz: &Path) -> Vec<PathBuf> {
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

fn es_imagen_soportada(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png") | Some("jpg") | Some("jpeg")
    )
}

fn lienzo_default() -> (Lienzo, AlmacenEnMemoria, Uuid, String) {
    let mut almacen = AlmacenEnMemoria::nuevo();
    let hash = almacen.insertar(buffer_gradiente());
    let mut lienzo = Lienzo::nuevo(W, H);
    let fondo = Capa::raster("fondo", hash);
    let id = fondo.id;
    lienzo.apilar(fondo);
    (lienzo, almacen, id, "listo · gradiente demo".into())
}

fn recomponer(l: &Lienzo, alm: &impl FuenteBuffers) -> Option<Image> {
    let img = componer(l, alm).ok()?;
    let (w, h) = (img.width(), img.height());
    let blob = Blob::from(img.into_raw());
    Some(Image::new(blob, ImageFormat::Rgba8, w, h))
}

fn aplicar_y_recomponer(model: &mut Model) {
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
    sincronizar_thumbs(model);
}

/// Lado del thumb en píxeles. Pequeño a propósito — la fila de capa es de
/// 28 px de alto y conviene dejar aire arriba/abajo.
const THUMB_LADO: u32 = 22;

/// Asegura que cada capa del lienzo tenga su thumbnail en el cache, y
/// descarta entries cuyos hashes ya no están en uso. La regeneración por
/// op (vía `regenerar_stale_con_ia`) cambia `Capa.contenido` para las
/// derivadas; el hash nuevo entra al cache, el viejo se barre.
fn sincronizar_thumbs(model: &mut Model) {
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
fn thumbnail_de_buffer(
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


// =============================================================================
//  Ciclar blend modes (no hay dropdown todavía — clic cicla)
// =============================================================================

fn siguiente_blend(b: ModoFusion) -> ModoFusion {
    match b {
        ModoFusion::Normal => ModoFusion::Multiplicar,
        ModoFusion::Multiplicar => ModoFusion::Pantalla,
        ModoFusion::Pantalla => ModoFusion::Superponer,
        ModoFusion::Superponer => ModoFusion::Aclarar,
        ModoFusion::Aclarar => ModoFusion::Oscurecer,
        ModoFusion::Oscurecer => ModoFusion::Diferencia,
        ModoFusion::Diferencia => ModoFusion::Aditivo,
        ModoFusion::Aditivo => ModoFusion::SubExpQuemado,
        ModoFusion::SubExpQuemado => ModoFusion::SubLinealQuemado,
        ModoFusion::SubLinealQuemado => ModoFusion::SobreExpAclarado,
        ModoFusion::SobreExpAclarado => ModoFusion::LuzFuerte,
        ModoFusion::LuzFuerte => ModoFusion::LuzSuave,
        ModoFusion::LuzSuave => ModoFusion::LuzViva,
        ModoFusion::LuzViva => ModoFusion::LuzLineal,
        ModoFusion::LuzLineal => ModoFusion::LuzPunto,
        ModoFusion::LuzPunto => ModoFusion::MezclaDura,
        ModoFusion::MezclaDura => ModoFusion::Exclusion,
        ModoFusion::Exclusion => ModoFusion::Resta,
        ModoFusion::Resta => ModoFusion::Division,
        ModoFusion::Division => ModoFusion::HslTono,
        ModoFusion::HslTono => ModoFusion::HslSaturacion,
        ModoFusion::HslSaturacion => ModoFusion::HslColor,
        ModoFusion::HslColor => ModoFusion::HslLuminosidad,
        ModoFusion::HslLuminosidad => ModoFusion::ColorMasOscuro,
        ModoFusion::ColorMasOscuro => ModoFusion::ColorMasClaro,
        ModoFusion::ColorMasClaro => ModoFusion::Disolver,
        ModoFusion::Disolver => ModoFusion::Normal,
    }
}

fn etiqueta_blend(b: ModoFusion) -> &'static str {
    match b {
        ModoFusion::Normal => "normal",
        ModoFusion::Multiplicar => "multiplicar",
        ModoFusion::Pantalla => "pantalla",
        ModoFusion::Superponer => "superponer",
        ModoFusion::Aclarar => "aclarar",
        ModoFusion::Oscurecer => "oscurecer",
        ModoFusion::Diferencia => "diferencia",
        ModoFusion::Aditivo => "aditivo",
        ModoFusion::SubExpQuemado => "subexp-quemado",
        ModoFusion::SubLinealQuemado => "sublineal-quemado",
        ModoFusion::SobreExpAclarado => "sobreexp-aclarado",
        ModoFusion::LuzFuerte => "luz-fuerte",
        ModoFusion::LuzSuave => "luz-suave",
        ModoFusion::LuzViva => "luz-viva",
        ModoFusion::LuzLineal => "luz-lineal",
        ModoFusion::LuzPunto => "luz-punto",
        ModoFusion::MezclaDura => "mezcla-dura",
        ModoFusion::Exclusion => "exclusión",
        ModoFusion::Resta => "resta",
        ModoFusion::Division => "división",
        ModoFusion::HslTono => "hsl-tono",
        ModoFusion::HslSaturacion => "hsl-saturación",
        ModoFusion::HslColor => "hsl-color",
        ModoFusion::HslLuminosidad => "hsl-luminosidad",
        ModoFusion::ColorMasOscuro => "color-más-oscuro",
        ModoFusion::ColorMasClaro => "color-más-claro",
        ModoFusion::Disolver => "disolver",
    }
}

// =============================================================================
//  Vista
// =============================================================================

fn header(
    theme: &llimphi_theme::Theme,
    lienzo: &Lienzo,
    estado: &str,
    proveedor_etiqueta: &str,
) -> View<Msg> {
    let titulo = format!(
        "tullpu · {}×{} · {} capas · IA: {proveedor_etiqueta} · {estado}",
        lienzo.width,
        lienzo.height,
        lienzo.capas.len()
    );
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(titulo, 12.0, theme.fg_muted, Alignment::Start)
}

fn fila_capa(
    theme: &llimphi_theme::Theme,
    capa: &Capa,
    seleccionada: bool,
    thumb: Option<&Image>,
) -> View<Msg> {
    let btn_pal = ButtonPalette::from_theme(theme);
    let nombre_op = match &capa.origen {
        OrigenCapa::Raster => "raster".to_string(),
        OrigenCapa::Derivada { op, estado, .. } => {
            let suf = match estado {
                Frescura::Fresca => "",
                Frescura::Stale => " · stale",
            };
            format!("{}{suf}", op.etiqueta())
        }
    };
    let etiqueta = format!(
        "{}  ·  {}  ·  α {:.2}  ·  {}",
        capa.nombre,
        nombre_op,
        capa.opacidad,
        etiqueta_blend(capa.blend)
    );
    let fila_bg = if seleccionada {
        theme.bg_panel_alt
    } else {
        theme.bg_panel
    };
    let fg = if capa.visible {
        theme.fg_text
    } else {
        theme.fg_muted
    };

    // Botón principal: selección de la capa, ocupa la mayor parte de la fila.
    let nombre = button_styled(
        etiqueta,
        Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: length(26.0_f32),
            },
            padding: Rect {
                left: length(10.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        },
        Alignment::Start,
        &ButtonPalette {
            bg: fila_bg,
            bg_hover: theme.bg_button_hover,
            fg,
            radius: 4.0,
        },
        Msg::Seleccionar(capa.id),
    );

    // Botones de control compactos a la derecha.
    let toggle = mini_btn(if capa.visible { "👁" } else { "—" }, Msg::ToggleVisible(capa.id), &btn_pal);
    // Slider de opacidad in-situ: reemplaza los botones α−/α+ (saltos de
    // 0.1) con drag continuo en [0.0, 1.0]. El widget devuelve `dv` (delta
    // de valor) por evento; `BumpOpacidad` ya acumula deltas, así que el
    // hook es directo. Sólo emitimos en `DragPhase::Move` — `End` no aporta
    // nuevo delta y duplicaría el último.
    let cap_id = capa.id;
    let opacidad = slider_view(
        "",
        capa.opacidad,
        0.0,
        1.0,
        &slider_pal_compacto(theme),
        move |phase, dv| match phase {
            DragPhase::Move => Some(Msg::BumpOpacidad(cap_id, dv)),
            DragPhase::End => None,
        },
    );
    let blend = mini_btn("blnd", Msg::CiclarBlend(capa.id), &btn_pal);
    // En la lista la pintamos top→fondo: "↑" visualmente sube en la lista,
    // lo que equivale a bajar el índice en la pila (más cerca del fondo).
    // Mantengo la semántica visual para que el usuario haga lo que ve.
    let subir = mini_btn("↑", Msg::MoverArriba(capa.id), &btn_pal);
    let bajar = mini_btn("↓", Msg::MoverAbajo(capa.id), &btn_pal);
    let dup = mini_btn("⎘", Msg::Duplicar(capa.id), &btn_pal);
    let elim = mini_btn("✕", Msg::Eliminar(capa.id), &btn_pal);

    // Thumbnail a la izquierda (slot fijo aun si el cache aún no lo tiene
    // — evita reflow). 24×24 con un margen interno para respirar.
    let thumb_view = match thumb {
        Some(img) => View::new(Style {
            size: Size {
                width: length(24.0_f32),
                height: length(24.0_f32),
            },
            padding: Rect {
                left: length(1.0_f32),
                right: length(3.0_f32),
                top: length(1.0_f32),
                bottom: length(1.0_f32),
            },
            ..Default::default()
        })
        .image(img.clone()),
        None => View::new(Style {
            size: Size {
                width: length(24.0_f32),
                height: length(24.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_panel_alt),
    };

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(2.0_f32),
            right: length(2.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![
        thumb_view, nombre, toggle, opacidad, blend, subir, bajar, dup, elim,
    ])
}

/// Slider compacto pensado para vivir embedded en la fila de capa: sin
/// bloque de label (el nombre de la capa ya lo identifica), track
/// estrecho, valor a la derecha para feedback numérico inmediato.
fn slider_pal_compacto(theme: &llimphi_theme::Theme) -> SliderPalette {
    let mut p = SliderPalette::from_theme(theme);
    p.label_width = 0.0;
    p.track_width = 56.0;
    p.value_width = 36.0;
    p.row_height = 24.0;
    p
}

fn mini_btn(label: &str, msg: Msg, pal: &ButtonPalette) -> View<Msg> {
    button_styled(
        label.to_string(),
        Style {
            size: Size {
                width: length(34.0_f32),
                height: length(24.0_f32),
            },
            padding: Rect {
                left: length(2.0_f32),
                right: length(2.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        pal,
        msg,
    )
}

fn panel_capas(theme: &llimphi_theme::Theme, model: &Model) -> View<Msg> {
    let mut hijos: Vec<View<Msg>> = Vec::new();
    let titulo = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned("capas (top→fondo)".to_string(), 11.0, theme.fg_muted, Alignment::Start);
    hijos.push(titulo);
    // Las pintamos top → fondo (al revés que el orden visual interno).
    for capa in model.lienzo.capas.iter().rev() {
        let sel = model.seleccionada == Some(capa.id);
        let thumb = model.thumbs.get(&capa.contenido);
        hijos.push(fila_capa(theme, capa, sel, thumb));
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(400.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(hijos)
}

fn panel_ops(theme: &llimphi_theme::Theme, model: &Model) -> View<Msg> {
    let pal = ButtonPalette::from_theme(theme);
    let bloqueado = model.seleccionada.is_none();
    let mk_local = |label: &str, op: OpLocal| -> View<Msg> {
        let msg = if bloqueado { Msg::Recargar } else { Msg::Agregar(op) };
        button_view(label.to_string(), &pal, msg)
    };
    let mk_ia = |label: &str, op: OpPixel| -> View<Msg> {
        let msg = if bloqueado {
            Msg::Recargar
        } else {
            Msg::AgregarIa(op)
        };
        button_view(label.to_string(), &pal, msg)
    };

    let subtitulo = |s: &str| {
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(22.0_f32),
            },
            padding: Rect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(8.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(s.to_string(), 11.0, theme.fg_muted, Alignment::Start)
    };

    // "entrada" primero: abrir el fuzzy picker para agregar una capa nueva
    // desde un PNG/JPEG del workspace. No requiere selección — siempre activo.
    let mut hijos = vec![subtitulo("entrada")];
    hijos.push(envolver_fila(button_view(
        format!(
            "📂 capa desde archivo · {} candidatos · Ctrl+P",
            model.imagenes_disponibles.len()
        ),
        &pal,
        Msg::Picker(PickerMsg::Open),
    )));

    // "salida": no requiere selección, siempre activa.
    hijos.push(subtitulo("salida"));
    hijos.push(envolver_fila(button_view(
        "💾 PNG (lossless · α)".to_string(),
        &pal,
        Msg::Exportar(FormatoExport::Png),
    )));
    hijos.push(envolver_fila(button_view(
        "💾 JPEG q90 (sin α)".to_string(),
        &pal,
        Msg::Exportar(FormatoExport::Jpeg { calidad: 90 }),
    )));
    hijos.push(envolver_fila(button_view(
        "💾 WebP (lossless · α)".to_string(),
        &pal,
        Msg::Exportar(FormatoExport::Webp),
    )));

    hijos.push(subtitulo("locales"));
    hijos.push(envolver_fila(mk_local("+ Invertir", OpLocal::Invertir)));
    hijos.push(envolver_fila(mk_local(
        "+ Brillo +0.15",
        OpLocal::Brillo { delta: 0.15 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Brillo −0.15",
        OpLocal::Brillo { delta: -0.15 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Contraste ×1.3",
        OpLocal::Contraste { factor: 1.3 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Contraste ×0.7",
        OpLocal::Contraste { factor: 0.7 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Saturación ×0.0",
        OpLocal::Saturacion { factor: 0.0 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Saturación ×1.5",
        OpLocal::Saturacion { factor: 1.5 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Tonalidad 90°",
        OpLocal::Tonalidad { grados: 90.0 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Blur radio 4",
        OpLocal::Blur { radio: 4.0 },
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Niveles 0.1–0.9 γ1.2",
        OpLocal::Niveles {
            entrada_min: 0.1,
            entrada_max: 0.9,
            gamma: 1.2,
        },
    )));

    hijos.push(subtitulo("ia"));
    hijos.push(envolver_fila(mk_ia(
        "+ Restyle 'tropical'",
        OpPixel::Restyle {
            prompt: "tropical".into(),
        },
    )));
    hijos.push(envolver_fila(mk_ia(
        "+ Restyle 'frío'",
        OpPixel::Restyle {
            prompt: "frío".into(),
        },
    )));
    hijos.push(envolver_fila(mk_ia(
        "+ Segmentar centro",
        OpPixel::Segmentar { prompt: None },
    )));
    hijos.push(envolver_fila(mk_ia(
        "+ Inpaint huecos",
        OpPixel::Inpaint { prompt: None },
    )));
    hijos.push(envolver_fila(mk_ia(
        "+ Generar 'atardecer'",
        OpPixel::Generar {
            prompt: "atardecer".into(),
            ancho: model.lienzo.width,
            alto: model.lienzo.height,
        },
    )));

    if bloqueado {
        hijos.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(20.0_f32),
                },
                padding: Rect {
                    left: length(10.0_f32),
                    right: length(10.0_f32),
                    top: length(8.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(
                "(seleccioná una capa primero)".to_string(),
                10.0,
                theme.fg_muted,
                Alignment::Start,
            ),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(240.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(hijos)
}

fn envolver_fila(boton: View<Msg>) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(32.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(3.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![boton])
}

fn panel_lienzo(theme: &llimphi_theme::Theme, model: &Model) -> View<Msg> {
    let cuerpo = match &model.imagen {
        Some(img) => View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(12.0_f32),
                bottom: length(12.0_f32),
            },
            ..Default::default()
        })
        .image(img.clone()),
        None => View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            "(sin composición)".to_string(),
            12.0,
            theme.fg_muted,
            Alignment::Center,
        ),
    };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![cuerpo])
}

// =============================================================================
//  App
// =============================================================================

struct Tullpu;

impl App for Tullpu {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "tullpu · editor de imágenes por capas"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        inicializar()
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Seleccionar(id) => {
                model.seleccionada = Some(id);
            }
            Msg::ToggleVisible(id) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.visible = !c.visible;
                }
                aplicar_y_recomponer(&mut model);
            }
            Msg::BumpOpacidad(id, delta) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.opacidad = (c.opacidad + delta).clamp(0.0, 1.0);
                }
                aplicar_y_recomponer(&mut model);
            }
            Msg::CiclarBlend(id) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.blend = siguiente_blend(c.blend);
                }
                aplicar_y_recomponer(&mut model);
            }
            Msg::MoverArriba(id) => {
                // Reordenar no toca dependencias por Uuid, así que basta
                // recomponer — `regenerar_stale_con_ia` corre igual y es
                // barato si nada está stale.
                if model.lienzo.mover_arriba(id) {
                    aplicar_y_recomponer(&mut model);
                }
            }
            Msg::MoverAbajo(id) => {
                if model.lienzo.mover_abajo(id) {
                    aplicar_y_recomponer(&mut model);
                }
            }
            Msg::Duplicar(id) => {
                if let Some(nuevo) = model.lienzo.duplicar(id) {
                    model.seleccionada = Some(nuevo);
                    aplicar_y_recomponer(&mut model);
                }
            }
            Msg::Eliminar(id) => {
                model.lienzo.capas.retain(|c| c.id != id);
                if model.seleccionada == Some(id) {
                    model.seleccionada = model.lienzo.capas.last().map(|c| c.id);
                }
                // Las capas derivadas que quedaron huérfanas se marcan stale
                // — su regeneración fallará silenciosamente (BufferFaltante).
                aplicar_y_recomponer(&mut model);
            }
            Msg::Agregar(op) => {
                if let Some(madre_id) = model.seleccionada {
                    // El contenido_cache inicial lo dejamos en ceros — el
                    // orquestador lo rellena en la siguiente regeneración.
                    let nueva = Capa::derivada(
                        format!("{}", op_etiqueta(&op)),
                        madre_id,
                        TransformacionPixel::Local(op),
                        [0u8; 32],
                    );
                    let nuevo_id = nueva.id;
                    model.lienzo.apilar(nueva);
                    model.seleccionada = Some(nuevo_id);
                    aplicar_y_recomponer(&mut model);
                }
            }
            Msg::AgregarIa(op) => {
                if let Some(madre_id) = model.seleccionada {
                    let modelo = model.proveedor.model_id().name.clone();
                    let nombre = format!("ia:{}", op.etiqueta());
                    let trans = transformacion_ia(modelo, &op);
                    let nueva = Capa::derivada(nombre, madre_id, trans, [0u8; 32]);
                    let nuevo_id = nueva.id;
                    model.lienzo.apilar(nueva);
                    model.seleccionada = Some(nuevo_id);
                    aplicar_y_recomponer(&mut model);
                }
            }
            Msg::Recargar => {
                aplicar_y_recomponer(&mut model);
            }
            Msg::Picker(pm) => {
                model = aplicar_picker(model, pm);
            }
            Msg::FileDrop(path) => {
                // Drag&drop OS-level: reusamos exactamente el mismo path
                // que el picker. Si la extensión no está en el catálogo
                // soportado (PNG/JPEG), `agregar_capa_desde_archivo` falla
                // al decodificar y deja el lienzo intacto con un estado
                // descriptivo — no preflight check para mantener una sola
                // rama de error.
                agregar_capa_desde_archivo(&mut model, &path);
            }
            Msg::Exportar(formato) => {
                // Path en CWD con timestamp Unix — sin file picker (la app
                // todavía no tiene). La extensión la elige el formato; el
                // usuario ve el path final en el header.
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let ext = extension_export(formato);
                let ruta = std::path::PathBuf::from(format!("tullpu-export-{ts}.{ext}"));
                model.estado = match tullpu_render::exportar(
                    &model.lienzo,
                    &model.almacen,
                    &ruta,
                    formato,
                ) {
                    Ok(_) => format!("exportado → {}", ruta.display()),
                    Err(e) => format!("export falló: {e}"),
                };
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = llimphi_theme::Theme::dark();
        let cabecera = header(
            &theme,
            &model.lienzo,
            &model.estado,
            &model.proveedor_etiqueta,
        );
        let centro = View::new(Style {
            flex_direction: FlexDirection::Row,
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![
            panel_capas(&theme, model),
            panel_lienzo(&theme, model),
            panel_ops(&theme, model),
        ]);
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![cabecera, centro])
    }

    fn on_file_drop(_model: &Model, path: PathBuf) -> Option<Msg> {
        // Cualquier archivo soltado se procesa por la misma vía que el
        // picker. Si no es PNG/JPEG la decodificación falla y el estado
        // refleja el error — sin diálogo modal, sin preflight.
        Some(Msg::FileDrop(path))
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        // Picker abierto: el módulo decide qué hacer con cada tecla
        // (input, navegación, apply, escape). Tiene prioridad sobre los
        // atajos globales para que escribir en el filtro no abra otro popup.
        if let Some(state) = model.picker.as_ref() {
            if let Some(pm) = picker::on_key(state, event) {
                return Some(Msg::Picker(pm));
            }
            return None;
        }
        // Ctrl+P abre el fuzzy picker (mismo atajo que nada y VS Code).
        if picker::open_shortcut(event) {
            return Some(Msg::Picker(PickerMsg::Open));
        }
        None
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        let state = model.picker.as_ref()?;
        let theme = llimphi_theme::Theme::dark();
        let palette = PickerPalette::from_theme(&theme);
        let panel = picker::view(
            state,
            &model.imagenes_disponibles,
            &model.raiz,
            &palette,
            Msg::Picker,
        );
        // Envuelvo el panel en un contenedor con padding lateral generoso
        // para centrarlo visualmente sobre el lienzo — el módulo devuelve
        // un View de `100% × 220px` que sin esto se pegaría al borde.
        Some(
            View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                padding: Rect {
                    left: length(120.0_f32),
                    right: length(120.0_f32),
                    top: length(80.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(vec![panel]),
        )
    }
}

fn extension_export(f: FormatoExport) -> &'static str {
    match f {
        FormatoExport::Png => "png",
        FormatoExport::Jpeg { .. } => "jpg",
        FormatoExport::Webp => "webp",
    }
}

fn op_etiqueta(op: &OpLocal) -> &'static str {
    match op {
        OpLocal::Invertir => "invertir",
        OpLocal::Brillo { .. } => "brillo",
        OpLocal::Contraste { .. } => "contraste",
        OpLocal::Niveles { .. } => "niveles",
        OpLocal::Blur { .. } => "blur",
        OpLocal::Opacidad { .. } => "opacidad",
        OpLocal::Saturacion { .. } => "saturación",
        OpLocal::Tonalidad { .. } => "tonalidad",
    }
}

// =============================================================================
//  Picker — agregar capa desde archivo PNG/JPEG
// =============================================================================

/// Routea un `PickerMsg` al módulo y traduce el `PickerAction` resultante:
/// `Open(path)` decodea el PNG/JPEG, lo ajusta al tamaño del lienzo y lo
/// apila como capa raster nueva encima de la seleccionada (o al tope).
fn aplicar_picker(mut model: Model, pm: PickerMsg) -> Model {
    if matches!(pm, PickerMsg::Open) && model.picker.is_none() {
        model.picker = Some(PickerState::new(
            &model.imagenes_disponibles,
            &model.raiz,
        ));
        model.estado = format!(
            "picker · {} imágenes · ↓↑ navega · Enter agrega · Esc cierra",
            model.imagenes_disponibles.len(),
        );
        return model;
    }
    let action = match model.picker.as_mut() {
        Some(state) => picker::apply(state, pm, &model.imagenes_disponibles, &model.raiz),
        None => return model,
    };
    match action {
        PickerAction::Close => {
            model.picker = None;
            model.estado = "listo".into();
        }
        PickerAction::Open(path) => {
            model.picker = None;
            agregar_capa_desde_archivo(&mut model, &path);
        }
        PickerAction::None => {}
    }
    model
}

/// Carga `path` como PNG/JPEG, lo ajusta al tamaño del lienzo y apila la
/// capa raster nueva. Se mete justo encima de la capa seleccionada (o al
/// tope si no hay selección). En éxito refresca compositor + thumbs; en
/// fallo deja el lienzo intacto y escribe el error en el estado.
fn agregar_capa_desde_archivo(model: &mut Model, path: &Path) {
    let Some((w, h, bytes)) = cargar_png(path) else {
        model.estado = format!("error decodificando {}", path.display());
        return;
    };
    let dst_w = model.lienzo.width;
    let dst_h = model.lienzo.height;
    let Some(buffer) = ajustar_a_lienzo(bytes, w, h, dst_w, dst_h) else {
        model.estado = format!("error ajustando {}×{} → {}×{}", w, h, dst_w, dst_h);
        return;
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
}

fn main() {
    llimphi_ui::run::<Tullpu>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ajustar_dims_iguales_devuelve_sin_tocar() {
        let src = vec![1, 2, 3, 4, 5, 6, 7, 8]; // 1×2 px Rgba8
        let copia = src.clone();
        let out = ajustar_a_lienzo(src, 1, 2, 1, 2).expect("dims iguales");
        assert_eq!(out, copia);
    }

    #[test]
    fn ajustar_contain_pad_transparente_centrado() {
        // 100×50 (2:1) → 200×200: cabe perfecto a 200×100, padding vertical
        // de 50 px arriba y abajo. Verifico que las esquinas son
        // transparentes y que la franja del medio tiene color.
        let mut src = Vec::with_capacity(100 * 50 * 4);
        for _ in 0..(100 * 50) {
            src.extend_from_slice(&[200, 100, 50, 255]); // naranja opaco
        }
        let out = ajustar_a_lienzo(src, 100, 50, 200, 200).expect("ajuste ok");
        assert_eq!(out.len(), 200 * 200 * 4);

        // Esquina superior izquierda: en el pad → transparente.
        assert_eq!(&out[0..4], &[0, 0, 0, 0]);
        // Píxel (100, 100) ≈ centro → opaco con color cercano al naranja.
        let i = (100 * 200 + 100) * 4;
        assert_eq!(out[i + 3], 255, "centro opaco");
        assert!(out[i] > 100, "rojo presente: {}", out[i]);
        // Esquina inferior derecha: en el pad → transparente.
        let j = (199 * 200 + 199) * 4;
        assert_eq!(&out[j..j + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn ajustar_dst_cero_devuelve_none() {
        let src = vec![0u8; 4];
        assert!(ajustar_a_lienzo(src, 1, 1, 0, 1).is_none());
    }

    #[test]
    fn es_imagen_soportada_filtra_extensiones() {
        assert!(es_imagen_soportada(Path::new("foo.png")));
        assert!(es_imagen_soportada(Path::new("foo.PNG")));
        assert!(es_imagen_soportada(Path::new("foo.jpg")));
        assert!(es_imagen_soportada(Path::new("foo.jpeg")));
        assert!(!es_imagen_soportada(Path::new("foo.psd")));
        assert!(!es_imagen_soportada(Path::new("foo.txt")));
        assert!(!es_imagen_soportada(Path::new("foo")));
    }
}
