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
//!
//! ## Hotkeys
//!
//! Actúan sobre la capa seleccionada (excepto los de export/picker que son
//! globales). Si el picker está abierto las teclas van al filtro, no acá.
//!
//! - `Ctrl+P`         — abre fuzzy file picker para agregar capa
//! - `Delete` / `Backspace` — eliminar capa
//! - `Ctrl+D`         — duplicar capa
//! - `F2`             — renombrar capa in-situ (Enter confirma · Esc cancela)
//! - `V`              — toggle visibilidad
//! - `B` / `Shift+B`  — ciclar blend forward / reverse
//! - `[` / `]`        — opacidad ∓0.1
//! - `Ctrl+Z` / `Ctrl+Shift+Z` (o `Ctrl+Y`) — undo / redo
//! - `Ctrl+S` / `Ctrl+Shift+S` — exportar PNG / WebP

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use llimphi_module_file_picker::{
    self as picker, PickerAction, PickerMsg, PickerPalette, PickerState,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KurboRect};
use llimphi_ui::llimphi_raster::peniko::Mix;
use llimphi_ui::PaintRect;
use llimphi_ui::WheelDelta;
use llimphi_ui::Modifiers;
use std::sync::{Mutex, OnceLock};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, NamedKey, View};
use llimphi_widget_button::{button_styled, button_view, ButtonPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

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
    /// Renombrado in-situ. `Some((uuid, input))` durante la edición —
    /// la fila correspondiente pinta el text-input en vez del botón de
    /// nombre. F2 entra, Enter confirma, Escape cancela.
    renombrando: Option<(Uuid, TextInputState)>,
    /// Pila de snapshots del [`Lienzo`] para undo/redo. Siempre no vacía:
    /// `historial[0]` es el lienzo al inicializar. `cursor_historial` apunta
    /// al estado vigente: `historial[cursor]` siempre cuadra con `lienzo` en
    /// régimen estable. Una mutación trunca cualquier rama de redo (todo lo
    /// que esté después de `cursor`) y pushea el nuevo estado al tope.
    /// Capado a [`HIST_CAP`] entradas para no inflar RAM en sesiones largas.
    historial: Vec<Lienzo>,
    cursor_historial: usize,
    /// Etiqueta del último snapshot pushado. Se usa para *coalescer* mutaciones
    /// continuas: si la próxima mutación viene con la misma etiqueta y
    /// estamos en el tope del historial, en lugar de agregar otra entrada se
    /// sustituye la del tope. Sirve para que un drag del slider de opacidad
    /// (decenas de eventos por segundo) cuente como una sola operación
    /// reversible. Sin coalesce, deshacer un drag costaría 100 Ctrl+Z.
    ultima_etiqueta_snapshot: Option<(Uuid, &'static str)>,
    /// Multiplicador de zoom sobre el fit-contain natural. 1.0 = fit (la
    /// imagen entra entera en el lienzo); 2.0 = el doble del tamaño fit;
    /// 0.5 = la mitad. Clamp en [`ZOOM_MIN`]..=[`ZOOM_MAX`].
    factor_zoom: f32,
    /// Offset de paneo en px de pantalla desde la posición centrada-fit. La
    /// imagen escala alrededor de `(centro_lienzo + pan)` (matemáticamente
    /// invariante bajo cambios de zoom — el píxel medio de la imagen
    /// permanece en el mismo punto al hacer wheel). Sin clamp: se puede
    /// "perder" la imagen, hotkey `0` resetea.
    pan_x: f32,
    pan_y: f32,
    /// Herramienta activa del lienzo. Cambia el cableado de eventos:
    /// `Mover` ⇒ click-drag panea; `Cuentagotas` ⇒ click lee el píxel
    /// bajo el cursor. El wheel zoom-ea en ambos modos.
    herramienta: Herramienta,
    /// Último color leído por el cuentagotas (RGBA del píxel del lienzo
    /// compuesto). `None` hasta que el usuario clickee con la
    /// herramienta `Cuentagotas` activa.
    color_picked: Option<[u8; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Herramienta {
    /// Click-drag panea el lienzo. Es la herramienta por defecto.
    Mover,
    /// Click sobre el lienzo lee el RGBA del píxel compuesto. No
    /// dragea (el drag se reservaría para una pintura futura).
    Cuentagotas,
}

impl Herramienta {
    fn etiqueta(self) -> &'static str {
        match self {
            Herramienta::Mover => "mover",
            Herramienta::Cuentagotas => "cuentagotas",
        }
    }
}

/// Multiplicador por tick de wheel. 1.1 ≈ +10%, un escalón cómodo. El
/// factor entra como `factor_zoom *= base.powf(-delta.y)` (delta.y > 0 es
/// scroll hacia abajo en convención CSS → zoom out).
const ZOOM_BASE: f32 = 1.1;
const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 32.0;

/// Side-channel para que [`on_wheel`] —que sólo recibe cursor absoluto, no
/// info de layout— pueda saber si el cursor cayó sobre el lienzo. Lo
/// escribe el closure de `paint_with` del lienzo en cada frame; lo lee
/// `on_wheel` antes de despachar. Es lectura-mostly: `Mutex` es OK para
/// los bytes de un `PaintRect` (16 bytes) y evita atomics-por-campo.
static LIENZO_RECT: OnceLock<Mutex<Option<PaintRect>>> = OnceLock::new();

fn lienzo_rect_set(r: PaintRect) {
    let cell = LIENZO_RECT.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = cell.lock() {
        *g = Some(r);
    }
}

fn lienzo_rect_get() -> Option<PaintRect> {
    LIENZO_RECT.get()?.lock().ok().and_then(|g| *g)
}

fn dentro_de_rect(r: PaintRect, cx: f32, cy: f32) -> bool {
    cx >= r.x && cx <= r.x + r.w && cy >= r.y && cy <= r.y + r.h
}

/// Tope de la pila de undo. 64 estados × 32 capas × ~100 B postcard ≈ 200 KB
/// — despreciable. Si se excede, descartamos las entradas más viejas (FIFO).
const HIST_CAP: usize = 64;

#[derive(Clone)]
enum Msg {
    Seleccionar(Uuid),
    ToggleVisible(Uuid),
    BumpOpacidad(Uuid, f32),
    CiclarBlend(Uuid),
    CiclarBlendInverso(Uuid),
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
    IniciarRenombrar(Uuid),
    TeclaRenombrar(KeyEvent),
    ConfirmarRenombrar,
    CancelarRenombrar,
    Undo,
    Redo,
    /// Wheel sobre el lienzo: multiplica `factor_zoom` por `mult` y, si
    /// hay un punto de anclaje conocido, ajusta `pan` para que el punto
    /// quede fijo bajo el cursor (zoom-a-cursor). `(rect, cursor)` son
    /// el último rect del lienzo y la posición global del cursor; ambos
    /// en px de pantalla. `None` ⇒ zoom-alrededor-del-centro.
    Zoom { mult: f32, ancla: Option<(PaintRect, f32, f32)> },
    /// Drag sobre el lienzo: acumula offset de paneo. `dx, dy` en px.
    Pan(f32, f32),
    /// Resetea zoom y pan al estado inicial (fit-contain centrado).
    ResetVista,
    /// Cambia la herramienta activa del lienzo (mover/cuentagotas).
    CambiarHerramienta(Herramienta),
    /// Click sobre el lienzo en modo cuentagotas: `(lx, ly)` relativo al
    /// rect del panel y `(rw, rh)` las dims actuales. El handler
    /// resuelve el píxel-imagen vía `transform_lienzo` y guarda el RGBA.
    RecogerColor {
        lx: f32,
        ly: f32,
        rw: f32,
        rh: f32,
    },
    /// Agrega una capa raster nueva del tamaño del lienzo llena con
    /// `color_picked` (o gris medio si no hay color leído). Cierra el
    /// loop pick→use: pickeás un color con el cuentagotas, después
    /// "+ relleno" aparece como capa nueva encima de la seleccionada.
    AgregarRelleno,
    /// Combina la capa identificada con la que está justo debajo (idx
    /// menor) en una sola capa raster que conserva el composite visual
    /// (respetando blend + opacidad + visibilidad). El par se reemplaza
    /// por una `Capa::raster` con defaults (Normal/1.0/visible). Si la
    /// capa ya está en el fondo (idx 0), no-op + estado descriptivo.
    Combinar(Uuid),
    /// Aplana todas las capas visibles a una sola raster con el
    /// composite del lienzo. Las hidden se preservan en su posición
    /// topológica; el resultado va donde estaba la más alta visible
    /// (Photoshop "Merge Visible"). Sin selección. No-op si hay 0 o 1
    /// visibles.
    AplanarVisibles,
    /// Rota el lienzo entero 90°. `cw=true` ⇒ sentido horario;
    /// `cw=false` ⇒ antihorario. Cada raster gana un buffer nuevo
    /// (rotado), las derivadas quedan stale y se regenan desde la
    /// madre rotada, las dims del lienzo se intercambian.
    RotarLienzo { cw: bool },
    /// Recorta el lienzo al bounding box no-transparente del composite
    /// vigente. Si no hay píxeles opacos, no-op + estado "lienzo
    /// vacío". Si el bbox cubre el lienzo entero, no-op + estado "ya
    /// está justo".
    AutotrimLienzo,
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
    let historial = vec![lienzo.clone()];
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
        historial,
        cursor_historial: 0,
        ultima_etiqueta_snapshot: None,
        factor_zoom: 1.0,
        pan_x: 0.0,
        pan_y: 0.0,
        herramienta: Herramienta::Mover,
        color_picked: None,
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

/// Pushea el estado actual del lienzo a la pila de undo. Si la `etiqueta`
/// (Uuid de capa + categoría) coincide con la del último snapshot Y estamos
/// en el tope, sustituye en lugar de pushear — es el mecanismo de *coalesce*
/// para drags continuos (slider de opacidad disparando decenas de mensajes
/// por segundo). Si no, trunca la rama de redo y agrega entrada nueva.
///
/// Se invoca después de cualquier mutación de `model.lienzo` que el usuario
/// pueda querer revertir (toggle visible, blend, opacidad, mover, dup, elim,
/// agregar, rename, file drop). Las acciones de pura UI (Seleccionar,
/// Recargar, Exportar, Picker abrir/cerrar) no producen snapshot.
fn pushear_snapshot(model: &mut Model, etiqueta: Option<(Uuid, &'static str)>) {
    let en_el_tope = model.cursor_historial + 1 == model.historial.len();
    let coalesce = match (model.ultima_etiqueta_snapshot, etiqueta) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    if en_el_tope && coalesce {
        // Drag continuo: sustituyo el tope con el nuevo estado. Un solo
        // Ctrl+Z deshace el drag completo en vez de N micro-steps.
        model.historial[model.cursor_historial] = model.lienzo.clone();
    } else {
        // Cualquier mutación nueva tras un Ctrl+Z aborta la rama de redo.
        model.historial.truncate(model.cursor_historial + 1);
        model.historial.push(model.lienzo.clone());
        // Cap por memoria: desfilan las entradas más viejas. Si tiramos
        // `n` entradas del frente, el cursor baja `n`.
        while model.historial.len() > HIST_CAP {
            model.historial.remove(0);
        }
        model.cursor_historial = model.historial.len() - 1;
    }
    model.ultima_etiqueta_snapshot = etiqueta;
}

/// Restaura el estado anterior del lienzo (cursor−−). Devuelve `true` si hubo
/// algo que deshacer. El almacén content-addressed nunca borra buffers, así
/// que restaurar a una versión anterior siempre encuentra los hashes — los
/// buffers "huérfanos" de la versión actual quedan dormidos pero accesibles
/// si después se hace redo. Recomposición posterior a cargo del caller.
fn aplicar_undo(model: &mut Model) -> bool {
    if model.cursor_historial == 0 {
        return false;
    }
    model.cursor_historial -= 1;
    model.lienzo = model.historial[model.cursor_historial].clone();
    // Cualquier mutación posterior al undo arranca rama nueva — invalidamos
    // la etiqueta para que el primer push no se coalesce con el último drag
    // que produjo el estado destino.
    model.ultima_etiqueta_snapshot = None;
    true
}

/// Reaplica un estado del que ya habíamos hecho undo (cursor++).
fn aplicar_redo(model: &mut Model) -> bool {
    if model.cursor_historial + 1 >= model.historial.len() {
        return false;
    }
    model.cursor_historial += 1;
    model.lienzo = model.historial[model.cursor_historial].clone();
    model.ultima_etiqueta_snapshot = None;
    true
}

/// Tras restaurar `model.lienzo` desde el historial, la selección puede
/// apuntar a una capa que no existe en ese estado (ej. la creé, le hice
/// Eliminar, ahora Ctrl+Z trae de vuelta una versión ANTERIOR a la creación).
/// Si la seleccionada ya no está, caemos al tope visual del lienzo restaurado.
fn ajustar_seleccion_tras_restaurar(model: &mut Model) {
    let existe = model
        .seleccionada
        .map(|id| model.lienzo.capas.iter().any(|c| c.id == id))
        .unwrap_or(false);
    if !existe {
        model.seleccionada = model.lienzo.capas.last().map(|c| c.id);
    }
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

/// Ciclo canónico de blend modes (orden Photoshop: Normal → catálogo
/// completo → Disolver → Normal). Lo declaramos una vez y derivamos
/// `siguiente_blend` y `blend_anterior` indexando, así Shift+B y B son
/// trivialmente inversos sin dos `match` paralelos que se desincronicen.
const CICLO_BLEND: &[ModoFusion] = &[
    ModoFusion::Normal,
    ModoFusion::Multiplicar,
    ModoFusion::Pantalla,
    ModoFusion::Superponer,
    ModoFusion::Aclarar,
    ModoFusion::Oscurecer,
    ModoFusion::Diferencia,
    ModoFusion::Aditivo,
    ModoFusion::SubExpQuemado,
    ModoFusion::SubLinealQuemado,
    ModoFusion::SobreExpAclarado,
    ModoFusion::LuzFuerte,
    ModoFusion::LuzSuave,
    ModoFusion::LuzViva,
    ModoFusion::LuzLineal,
    ModoFusion::LuzPunto,
    ModoFusion::MezclaDura,
    ModoFusion::Exclusion,
    ModoFusion::Resta,
    ModoFusion::Division,
    ModoFusion::HslTono,
    ModoFusion::HslSaturacion,
    ModoFusion::HslColor,
    ModoFusion::HslLuminosidad,
    ModoFusion::ColorMasOscuro,
    ModoFusion::ColorMasClaro,
    ModoFusion::Disolver,
];

fn indice_blend(b: ModoFusion) -> usize {
    CICLO_BLEND.iter().position(|m| *m == b).unwrap_or(0)
}

fn siguiente_blend(b: ModoFusion) -> ModoFusion {
    let i = indice_blend(b);
    CICLO_BLEND[(i + 1) % CICLO_BLEND.len()]
}

fn blend_anterior(b: ModoFusion) -> ModoFusion {
    let i = indice_blend(b);
    CICLO_BLEND[(i + CICLO_BLEND.len() - 1) % CICLO_BLEND.len()]
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
    factor_zoom: f32,
    herramienta: Herramienta,
    color_picked: Option<[u8; 4]>,
) -> View<Msg> {
    // Indicador discreto: sólo se muestra cuando el usuario tocó zoom
    // o pan; en el caso por defecto (fit) el header queda igual que antes.
    let vista = if (factor_zoom - 1.0).abs() < 1e-4 {
        String::new()
    } else {
        format!(" · vista {:.0}%", factor_zoom * 100.0)
    };
    let tool = format!(" · ⌨ {}", herramienta.etiqueta());
    let color = match color_picked {
        Some([r, g, b, a]) => format!(" · 🎨 #{r:02X}{g:02X}{b:02X} α={a}"),
        None => String::new(),
    };
    let titulo = format!(
        "tullpu · {}×{} · {} capas · IA: {proveedor_etiqueta}{vista}{tool}{color} · {estado}",
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
    renombrando_input: Option<&TextInputState>,
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

    // Si esta capa está siendo renombrada, el bloque del nombre cambia a
    // un text-input enfocado. El resto de los micro-controles (toggle,
    // slider, blend, mover, dup, elim) sigue activo — no bloqueamos el
    // resto de la fila durante la edición porque el modal de teclado ya
    // routea los keypress al input.
    let nombre: View<Msg> = match renombrando_input {
        Some(input) => {
            let tp = TextInputPalette::from_theme(theme);
            View::new(Style {
                flex_grow: 1.0,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(26.0_f32),
                },
                padding: Rect {
                    left: length(2.0_f32),
                    right: length(2.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(vec![text_input_view(
                input,
                "nuevo nombre…",
                true,
                &tp,
                // Click sobre el input cancela cualquier otra interacción
                // ambigua re-foqueando la edición sobre la misma capa.
                Msg::IniciarRenombrar(capa.id),
            )])
        }
        None => button_styled(
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
        ),
    };

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
    // ⊕ = combinar con la de abajo (merge down). Si la capa ya está al
    // fondo (idx 0 en la pila), el handler en `update` lo detecta y
    // emite estado descriptivo — el botón se pinta igual para todas las
    // capas; no escondemos para mantener el layout estable.
    let merge = mini_btn("⊕", Msg::Combinar(capa.id), &btn_pal);
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
        thumb_view, nombre, toggle, opacidad, blend, subir, bajar, dup, merge, elim,
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
        let renombrando = model
            .renombrando
            .as_ref()
            .filter(|(id, _)| *id == capa.id)
            .map(|(_, input)| input);
        hijos.push(fila_capa(theme, capa, sel, thumb, renombrando));
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

    // "herramienta": toggle entre mover (drag panea) y cuentagotas (click
    // lee píxel). Globales — no dependen de selección. Las hotkeys `m` y
    // `i` hacen lo mismo; los botones son por discoverability.
    let mut hijos = vec![subtitulo("herramienta")];
    let pal_tool_activo = ButtonPalette {
        bg: theme.bg_selected,
        fg: theme.fg_text,
        ..pal.clone()
    };
    let etiqueta_mover = if model.herramienta == Herramienta::Mover {
        "● mover (m)"
    } else {
        "○ mover (m)"
    };
    let etiqueta_cuenta = if model.herramienta == Herramienta::Cuentagotas {
        "● cuentagotas (i)"
    } else {
        "○ cuentagotas (i)"
    };
    hijos.push(envolver_fila(button_view(
        etiqueta_mover.to_string(),
        if model.herramienta == Herramienta::Mover {
            &pal_tool_activo
        } else {
            &pal
        },
        Msg::CambiarHerramienta(Herramienta::Mover),
    )));
    hijos.push(envolver_fila(button_view(
        etiqueta_cuenta.to_string(),
        if model.herramienta == Herramienta::Cuentagotas {
            &pal_tool_activo
        } else {
            &pal
        },
        Msg::CambiarHerramienta(Herramienta::Cuentagotas),
    )));

    // "entrada": agregar una capa nueva. Dos vías: relleno sólido del
    // color del cuentagotas, o fuzzy picker de un archivo del workspace.
    // Ninguna requiere selección — siempre activas.
    hijos.push(subtitulo("entrada"));
    // Botón de relleno: muestra el color que va a usar. Si no hay color
    // leído por el cuentagotas, dice "gris" (el RELLENO_DEFAULT).
    let etiqueta_color = match model.color_picked {
        Some(c) => format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]),
        None => "gris".to_string(),
    };
    hijos.push(envolver_fila(button_view(
        format!(
            "+ relleno {} ({}×{})",
            etiqueta_color, model.lienzo.width, model.lienzo.height,
        ),
        &pal,
        Msg::AgregarRelleno,
    )));
    hijos.push(envolver_fila(button_view(
        format!(
            "📂 capa desde archivo · {} candidatos · Ctrl+P",
            model.imagenes_disponibles.len()
        ),
        &pal,
        Msg::Picker(PickerMsg::Open),
    )));

    // "estructura": operaciones sobre el lienzo entero. Aplanar las
    // visibles y rotar el lienzo 90° en cada sentido.
    let n_visibles = model.lienzo.capas.iter().filter(|c| c.visible).count();
    hijos.push(subtitulo("estructura"));
    hijos.push(envolver_fila(button_view(
        format!("⊞ aplanar visibles ({}) · Ctrl+Shift+E", n_visibles),
        &pal,
        Msg::AplanarVisibles,
    )));
    hijos.push(envolver_fila(button_view(
        "⟳ rotar +90° (CW)".to_string(),
        &pal,
        Msg::RotarLienzo { cw: true },
    )));
    hijos.push(envolver_fila(button_view(
        "⟲ rotar −90° (CCW)".to_string(),
        &pal,
        Msg::RotarLienzo { cw: false },
    )));
    hijos.push(envolver_fila(button_view(
        "✂ recortar a visible (auto-trim)".to_string(),
        &pal,
        Msg::AutotrimLienzo,
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
        "+ Espejar ↔",
        OpLocal::EspejarHorizontal,
    )));
    hijos.push(envolver_fila(mk_local(
        "+ Espejar ↕",
        OpLocal::EspejarVertical,
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

/// Construye el transform para pintar `(image_w, image_h)` dentro de un
/// rect `(rw, rh)` con `factor_zoom` y `pan` aplicados. Devuelve la escala
/// absoluta y el offset top-left del rectángulo destino, ambos en px.
/// Pura — testeable sin gráficos.
fn transform_lienzo(
    image_w: u32,
    image_h: u32,
    rw: f32,
    rh: f32,
    factor_zoom: f32,
    pan_x: f32,
    pan_y: f32,
) -> Option<(f64, f64, f64)> {
    if image_w == 0 || image_h == 0 || rw <= 0.0 || rh <= 0.0 {
        return None;
    }
    let sx = rw as f64 / image_w as f64;
    let sy = rh as f64 / image_h as f64;
    let s_fit = sx.min(sy);
    let s = s_fit * factor_zoom as f64;
    let dw = image_w as f64 * s;
    let dh = image_h as f64 * s;
    let off_x = (rw as f64 - dw) * 0.5 + pan_x as f64;
    let off_y = (rh as f64 - dh) * 0.5 + pan_y as f64;
    Some((s, off_x, off_y))
}

/// Calcula el nuevo `(pan_x, pan_y)` para que el punto de pantalla
/// `(cursor_x, cursor_y)` siga apuntando al mismo píxel-imagen tras
/// cambiar `factor_zoom` de `zoom_old` a `zoom_new`. Devuelve los pans
/// sin tocar si la imagen o el rect son degenerados (división por cero).
/// Pura — testeable sin gráficos.
fn pan_para_zoom_a_cursor(
    image_w: u32,
    image_h: u32,
    rect: PaintRect,
    cursor_x: f32,
    cursor_y: f32,
    zoom_old: f32,
    zoom_new: f32,
    pan_x: f32,
    pan_y: f32,
) -> (f32, f32) {
    let Some((s_old, off_x, off_y)) =
        transform_lienzo(image_w, image_h, rect.w, rect.h, zoom_old, pan_x, pan_y)
    else {
        return (pan_x, pan_y);
    };
    if s_old <= 0.0 || image_w == 0 || image_h == 0 {
        return (pan_x, pan_y);
    }
    // Cursor en coords-imagen bajo el zoom anterior.
    let tx_old = rect.x as f64 + off_x;
    let ty_old = rect.y as f64 + off_y;
    let ix = (cursor_x as f64 - tx_old) / s_old;
    let iy = (cursor_y as f64 - ty_old) / s_old;
    // Nueva escala y nuevo top-left exigido para que (ix, iy) caiga bajo
    // el cursor: tx_new = cursor - ix * s_new.
    let s_fit_w = rect.w as f64 / image_w as f64;
    let s_fit_h = rect.h as f64 / image_h as f64;
    let s_new = s_fit_w.min(s_fit_h) * zoom_new as f64;
    let tx_new = cursor_x as f64 - ix * s_new;
    let ty_new = cursor_y as f64 - iy * s_new;
    let dw_new = image_w as f64 * s_new;
    let dh_new = image_h as f64 * s_new;
    let pan_x_nuevo = (tx_new - rect.x as f64 - (rect.w as f64 - dw_new) * 0.5) as f32;
    let pan_y_nuevo = (ty_new - rect.y as f64 - (rect.h as f64 - dh_new) * 0.5) as f32;
    (pan_x_nuevo, pan_y_nuevo)
}

/// Convierte un click en coords-panel `(lx, ly)` con dims `(rw, rh)` a
/// la posición del píxel-imagen bajo el cursor (aplicando zoom + pan) y
/// devuelve el RGBA de ese píxel del buffer `image_data` (Rgba8 fila por
/// fila). Devuelve `None` si las dims son degeneradas, si el píxel cae
/// fuera de la imagen o si el buffer no tiene tamaño suficiente. Pura.
fn recoger_color_en(
    image_data: &[u8],
    image_w: u32,
    image_h: u32,
    lx: f32,
    ly: f32,
    rw: f32,
    rh: f32,
    factor_zoom: f32,
    pan_x: f32,
    pan_y: f32,
) -> Option<[u8; 4]> {
    let (s, off_x, off_y) =
        transform_lienzo(image_w, image_h, rw, rh, factor_zoom, pan_x, pan_y)?;
    if s <= 0.0 {
        return None;
    }
    let ix = ((lx as f64 - off_x) / s).floor() as i64;
    let iy = ((ly as f64 - off_y) / s).floor() as i64;
    if ix < 0 || iy < 0 {
        return None;
    }
    let (ix, iy) = (ix as u32, iy as u32);
    if ix >= image_w || iy >= image_h {
        return None;
    }
    let stride = image_w as usize * 4;
    let idx = iy as usize * stride + ix as usize * 4;
    if idx + 4 > image_data.len() {
        return None;
    }
    Some([
        image_data[idx],
        image_data[idx + 1],
        image_data[idx + 2],
        image_data[idx + 3],
    ])
}

fn panel_lienzo(theme: &llimphi_theme::Theme, model: &Model) -> View<Msg> {
    let cuerpo = match &model.imagen {
        Some(img) => {
            // Clones cheap: peniko::Image internamente es Arc<Blob>, los
            // floats son Copy. Capturadas por valor para que el closure
            // sea 'static + Send + Sync.
            let img = img.clone();
            let factor_zoom = model.factor_zoom;
            let pan_x = model.pan_x;
            let pan_y = model.pan_y;
            let cuerpo_paint = View::new(Style {
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
            .paint_with(move |scene, _ts, r| {
                // Registramos el rect en cada paint para que on_wheel
                // pueda decidir si el cursor cayó sobre el lienzo y, en
                // ese caso, hacer zoom-a-cursor (el closure no muta
                // estado de la app — sólo escribe la cache lateral).
                lienzo_rect_set(r);
                if img.width == 0 || img.height == 0 || r.w <= 0.0 || r.h <= 0.0 {
                    return;
                }
                let Some((s, off_x, off_y)) = transform_lienzo(
                    img.width,
                    img.height,
                    r.w,
                    r.h,
                    factor_zoom,
                    pan_x,
                    pan_y,
                ) else {
                    return;
                };
                let tx = r.x as f64 + off_x;
                let ty = r.y as f64 + off_y;
                let transform = Affine::translate((tx, ty)) * Affine::scale(s);
                // Clip al rect del lienzo: una imagen zoom-in que se sale
                // del panel no debe pintar sobre el panel de ops o capas.
                let node_rect = KurboRect::new(
                    r.x as f64,
                    r.y as f64,
                    (r.x + r.w) as f64,
                    (r.y + r.h) as f64,
                );
                scene.push_layer(Mix::Clip, 1.0, Affine::IDENTITY, &node_rect);
                scene.draw_image(&img, transform);
                scene.pop_layer();
            });
            // El cableado de eventos depende de la herramienta: Mover
            // panea con drag; Cuentagotas recoge color con click. El
            // wheel sigue zoom-eando en ambos modos (vía `on_wheel`).
            match model.herramienta {
                Herramienta::Mover => cuerpo_paint.draggable(|fase, dx, dy| match fase {
                    DragPhase::Move => Some(Msg::Pan(dx, dy)),
                    DragPhase::End => None,
                }),
                Herramienta::Cuentagotas => cuerpo_paint.on_click_at(|lx, ly, rw, rh| {
                    Some(Msg::RecogerColor { lx, ly, rw, rh })
                }),
            }
        }
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
                pushear_snapshot(&mut model, None);
            }
            Msg::BumpOpacidad(id, delta) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.opacidad = (c.opacidad + delta).clamp(0.0, 1.0);
                }
                aplicar_y_recomponer(&mut model);
                // Coalesce: un drag continuo del slider sobre la misma capa
                // colapsa a una sola entrada de historial.
                pushear_snapshot(&mut model, Some((id, "opacidad")));
            }
            Msg::CiclarBlend(id) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.blend = siguiente_blend(c.blend);
                }
                aplicar_y_recomponer(&mut model);
                pushear_snapshot(&mut model, None);
            }
            Msg::CiclarBlendInverso(id) => {
                if let Some(c) = model.lienzo.capa_mut(id) {
                    c.blend = blend_anterior(c.blend);
                }
                aplicar_y_recomponer(&mut model);
                pushear_snapshot(&mut model, None);
            }
            Msg::MoverArriba(id) => {
                // Reordenar no toca dependencias por Uuid, así que basta
                // recomponer — `regenerar_stale_con_ia` corre igual y es
                // barato si nada está stale.
                if model.lienzo.mover_arriba(id) {
                    aplicar_y_recomponer(&mut model);
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::MoverAbajo(id) => {
                if model.lienzo.mover_abajo(id) {
                    aplicar_y_recomponer(&mut model);
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::Duplicar(id) => {
                if let Some(nuevo) = model.lienzo.duplicar(id) {
                    model.seleccionada = Some(nuevo);
                    aplicar_y_recomponer(&mut model);
                    pushear_snapshot(&mut model, None);
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
                pushear_snapshot(&mut model, None);
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
                    pushear_snapshot(&mut model, None);
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
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::Recargar => {
                aplicar_y_recomponer(&mut model);
            }
            Msg::Picker(pm) => {
                model = aplicar_picker(model, pm);
            }
            Msg::IniciarRenombrar(id) => {
                // Pre-cargar el text-input con el nombre actual para que
                // editar sea "tocar el final" en vez de "borrar todo y
                // tipear de nuevo".
                if let Some(c) = model.lienzo.capas.iter().find(|c| c.id == id) {
                    let mut input = TextInputState::new();
                    input.set_text(c.nombre.clone());
                    model.renombrando = Some((id, input));
                    model.seleccionada = Some(id);
                    model.estado = "renombrando · Enter confirma · Esc cancela".into();
                }
            }
            Msg::TeclaRenombrar(ev) => {
                if let Some((_, input)) = model.renombrando.as_mut() {
                    input.apply_key(&ev);
                }
            }
            Msg::ConfirmarRenombrar => {
                if let Some((id, input)) = model.renombrando.take() {
                    let nuevo = input.text();
                    let mut cambio = false;
                    if !nuevo.trim().is_empty() {
                        if let Some(c) = model.lienzo.capa_mut(id) {
                            if c.nombre != nuevo {
                                c.nombre = nuevo;
                                cambio = true;
                            }
                        }
                    }
                    if cambio {
                        pushear_snapshot(&mut model, None);
                    }
                    model.estado = "listo".into();
                }
            }
            Msg::CancelarRenombrar => {
                model.renombrando = None;
                model.estado = "listo".into();
            }
            Msg::FileDrop(path) => {
                // Drag&drop OS-level: reusamos exactamente el mismo path
                // que el picker. Si la extensión no está en el catálogo
                // soportado (PNG/JPEG), `agregar_capa_desde_archivo` falla
                // al decodificar y deja el lienzo intacto con un estado
                // descriptivo — no preflight check para mantener una sola
                // rama de error.
                if agregar_capa_desde_archivo(&mut model, &path) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::Undo => {
                if aplicar_undo(&mut model) {
                    ajustar_seleccion_tras_restaurar(&mut model);
                    aplicar_y_recomponer(&mut model);
                    model.estado = format!(
                        "↶ undo · {}/{}",
                        model.cursor_historial + 1,
                        model.historial.len()
                    );
                } else {
                    model.estado = "↶ nada que deshacer".into();
                }
            }
            Msg::Redo => {
                if aplicar_redo(&mut model) {
                    ajustar_seleccion_tras_restaurar(&mut model);
                    aplicar_y_recomponer(&mut model);
                    model.estado = format!(
                        "↷ redo · {}/{}",
                        model.cursor_historial + 1,
                        model.historial.len()
                    );
                } else {
                    model.estado = "↷ nada que rehacer".into();
                }
            }
            Msg::Zoom { mult, ancla } => {
                let zoom_anterior = model.factor_zoom;
                let zoom_nuevo = (zoom_anterior * mult).clamp(ZOOM_MIN, ZOOM_MAX);
                // Si el cursor está sobre el lienzo (ancla = Some), ajustamos
                // pan para que el píxel bajo el cursor quede fijo
                // (zoom-a-cursor) — la sensación natural de un image editor.
                // Sin ancla, dejamos pan tal cual: el centro de la imagen
                // mostrada permanece fijo (consecuencia de la ecuación de
                // offset).
                if let (Some((rect, cx, cy)), Some(img)) = (ancla, model.imagen.as_ref()) {
                    let (pan_x_nuevo, pan_y_nuevo) = pan_para_zoom_a_cursor(
                        img.width,
                        img.height,
                        rect,
                        cx,
                        cy,
                        zoom_anterior,
                        zoom_nuevo,
                        model.pan_x,
                        model.pan_y,
                    );
                    model.pan_x = pan_x_nuevo;
                    model.pan_y = pan_y_nuevo;
                }
                model.factor_zoom = zoom_nuevo;
            }
            Msg::Pan(dx, dy) => {
                model.pan_x += dx;
                model.pan_y += dy;
            }
            Msg::ResetVista => {
                model.factor_zoom = 1.0;
                model.pan_x = 0.0;
                model.pan_y = 0.0;
                model.estado = "vista reseteada".into();
            }
            Msg::CambiarHerramienta(h) => {
                model.herramienta = h;
                model.estado = format!("herramienta · {}", h.etiqueta());
            }
            Msg::AgregarRelleno => {
                if agregar_capa_relleno(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::Combinar(id) => {
                if combinar_capa_abajo(&mut model, id) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::AplanarVisibles => {
                if aplanar_capas_visibles(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::RotarLienzo { cw } => {
                if rotar_lienzo(&mut model, cw) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::AutotrimLienzo => {
                if recortar_lienzo_a_visible(&mut model) {
                    pushear_snapshot(&mut model, None);
                }
            }
            Msg::RecogerColor { lx, ly, rw, rh } => {
                if let Some(img) = model.imagen.as_ref() {
                    let bytes = img.data.data();
                    match recoger_color_en(
                        bytes,
                        img.width,
                        img.height,
                        lx,
                        ly,
                        rw,
                        rh,
                        model.factor_zoom,
                        model.pan_x,
                        model.pan_y,
                    ) {
                        Some(rgba) => {
                            model.color_picked = Some(rgba);
                            model.estado = format!(
                                "color · #{:02X}{:02X}{:02X} α={}",
                                rgba[0], rgba[1], rgba[2], rgba[3]
                            );
                        }
                        None => {
                            // Click cayó fuera de la imagen (en el pad del
                            // fit-contain o en el borde). Dejamos
                            // `color_picked` tal cual y avisamos.
                            model.estado = "color · fuera de la imagen".into();
                        }
                    }
                }
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
            model.factor_zoom,
            model.herramienta,
            model.color_picked,
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

    fn on_wheel(
        _model: &Model,
        delta: WheelDelta,
        cursor: (f32, f32),
        _modifiers: Modifiers,
    ) -> Option<Msg> {
        // Sólo zoom-eamos si el cursor está sobre el lienzo. Si está en
        // los paneles laterales, dejamos pasar (futuro: scroll vertical
        // del panel de capas si crece). delta.y > 0 ⇒ scroll hacia abajo ⇒
        // zoom out (convención CSS — ver `WheelDelta`).
        let rect = lienzo_rect_get()?;
        if !dentro_de_rect(rect, cursor.0, cursor.1) {
            return None;
        }
        let mult = ZOOM_BASE.powf(-delta.y);
        Some(Msg::Zoom {
            mult,
            ancla: Some((rect, cursor.0, cursor.1)),
        })
    }

    fn on_file_drop(_model: &Model, path: PathBuf) -> Option<Msg> {
        // Cualquier archivo soltado se procesa por la misma vía que el
        // picker. Si no es PNG/JPEG la decodificación falla y el estado
        // refleja el error — sin diálogo modal, sin preflight.
        Some(Msg::FileDrop(path))
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        use llimphi_ui::KeyState;
        // Picker abierto: el módulo decide qué hacer con cada tecla
        // (input, navegación, apply, escape). Tiene prioridad sobre los
        // atajos globales para que escribir en el filtro no abra otro popup.
        if let Some(state) = model.picker.as_ref() {
            if let Some(pm) = picker::on_key(state, event) {
                return Some(Msg::Picker(pm));
            }
            return None;
        }
        // Renombrando una capa: las teclas van al text-input, salvo Enter
        // (confirma) y Escape (cancela). Mismo patrón que el picker: el
        // modo modal absorbe los atajos globales.
        if model.renombrando.is_some() {
            if event.state == KeyState::Pressed {
                match &event.key {
                    Key::Named(NamedKey::Enter) => return Some(Msg::ConfirmarRenombrar),
                    Key::Named(NamedKey::Escape) => return Some(Msg::CancelarRenombrar),
                    _ => {}
                }
            }
            return Some(Msg::TeclaRenombrar(event.clone()));
        }
        // Ctrl+P abre el fuzzy picker (mismo atajo que nada y VS Code).
        if picker::open_shortcut(event) {
            return Some(Msg::Picker(PickerMsg::Open));
        }
        hotkey_a_msg(model, event)
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
        OpLocal::EspejarHorizontal => "espejar ↔",
        OpLocal::EspejarVertical => "espejar ↕",
    }
}

// =============================================================================
//  Hotkeys — atajos globales que actúan sobre la capa seleccionada
// =============================================================================

/// Traduce un `KeyEvent` a un `Msg` según el catálogo de atajos. Se asume
/// que el llamante ya descartó el caso "picker abierto" — acá routeamos
/// libremente sobre el modelo principal. Función pura para que el test
/// pueda cubrir el dispatch sin levantar la app.
///
/// Catálogo:
/// - `Delete` / `Backspace` → eliminar capa seleccionada
/// - `Ctrl+D` → duplicar
/// - `V` → toggle visibilidad
/// - `B` → ciclar blend forward, `Shift+B` ciclar reverse
/// - `[` / `]` → bump opacidad ∓0.1
/// - `Ctrl+S` → export PNG, `Ctrl+Shift+S` → WebP
/// - `Ctrl+Z` → undo, `Ctrl+Shift+Z` o `Ctrl+Y` → redo (globales)
fn hotkey_a_msg(model: &Model, event: &KeyEvent) -> Option<Msg> {
    use llimphi_ui::KeyState;
    if event.state != KeyState::Pressed {
        return None;
    }
    let m = event.modifiers;
    // Atajos globales (no requieren selección).
    match &event.key {
        Key::Character(s) if m.ctrl && !m.shift && s.eq_ignore_ascii_case("s") => {
            return Some(Msg::Exportar(FormatoExport::Png));
        }
        Key::Character(s) if m.ctrl && m.shift && s.eq_ignore_ascii_case("s") => {
            return Some(Msg::Exportar(FormatoExport::Webp));
        }
        Key::Character(s) if m.ctrl && !m.shift && s.eq_ignore_ascii_case("z") => {
            return Some(Msg::Undo);
        }
        Key::Character(s) if m.ctrl && m.shift && s.eq_ignore_ascii_case("z") => {
            return Some(Msg::Redo);
        }
        Key::Character(s) if m.ctrl && !m.shift && s.eq_ignore_ascii_case("y") => {
            return Some(Msg::Redo);
        }
        // Ctrl+Shift+E = aplanar visibles (Photoshop "Merge Visible").
        // Global: no requiere selección — opera sobre todo el lienzo.
        Key::Character(s) if m.ctrl && m.shift && s.eq_ignore_ascii_case("e") => {
            return Some(Msg::AplanarVisibles);
        }
        // Reset de vista: zoom 100% del fit + pan a cero. Global porque
        // no depende de capa seleccionada — es navegación del viewport.
        Key::Character(s) if !m.ctrl && !m.alt && s == "0" => {
            return Some(Msg::ResetVista);
        }
        // Herramientas: `m` mover (pan), `i` cuentagotas (eyedropper —
        // Photoshop standard). Globales porque cambian el modo del
        // lienzo, no operan sobre la capa.
        Key::Character(s) if !m.ctrl && !m.alt && s.eq_ignore_ascii_case("m") => {
            return Some(Msg::CambiarHerramienta(Herramienta::Mover));
        }
        Key::Character(s) if !m.ctrl && !m.alt && s.eq_ignore_ascii_case("i") => {
            return Some(Msg::CambiarHerramienta(Herramienta::Cuentagotas));
        }
        _ => {}
    }
    // El resto opera sobre la capa seleccionada.
    let id = model.seleccionada?;
    match &event.key {
        Key::Named(NamedKey::F2) => Some(Msg::IniciarRenombrar(id)),
        Key::Named(NamedKey::Delete) | Key::Named(NamedKey::Backspace) if !m.ctrl => {
            Some(Msg::Eliminar(id))
        }
        Key::Character(s) if m.ctrl && !m.shift && s.eq_ignore_ascii_case("d") => {
            Some(Msg::Duplicar(id))
        }
        // Ctrl+E = merge down (combinar con la capa de abajo). Sin
        // selección no aplica. Photoshop standard.
        Key::Character(s) if m.ctrl && !m.shift && s.eq_ignore_ascii_case("e") => {
            Some(Msg::Combinar(id))
        }
        Key::Character(s) if !m.ctrl && !m.alt && s.eq_ignore_ascii_case("v") => {
            Some(Msg::ToggleVisible(id))
        }
        Key::Character(s) if !m.ctrl && !m.alt && s.eq_ignore_ascii_case("b") => {
            // El cycle inverso se distingue por shift; sin shift es forward.
            // Reutilizamos `CiclarBlend` para forward; para reverse emitimos
            // un mensaje propio que el update conoce.
            if m.shift {
                Some(Msg::CiclarBlendInverso(id))
            } else {
                Some(Msg::CiclarBlend(id))
            }
        }
        Key::Character(s) if !m.ctrl && !m.alt && s == "[" => {
            Some(Msg::BumpOpacidad(id, -0.1))
        }
        Key::Character(s) if !m.ctrl && !m.alt && s == "]" => {
            Some(Msg::BumpOpacidad(id, 0.1))
        }
        _ => None,
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
            if agregar_capa_desde_archivo(&mut model, &path) {
                pushear_snapshot(&mut model, None);
            }
        }
        PickerAction::None => {}
    }
    model
}

/// Carga `path` como PNG/JPEG, lo ajusta al tamaño del lienzo y apila la
/// capa raster nueva. Se mete justo encima de la capa seleccionada (o al
/// tope si no hay selección). En éxito refresca compositor + thumbs y
/// devuelve `true` (para que el caller decida si snapshotear); en fallo deja
/// el lienzo intacto, escribe el error en el estado y devuelve `false`.
fn agregar_capa_desde_archivo(model: &mut Model, path: &Path) -> bool {
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

/// Color de fallback cuando el cuentagotas todavía no leyó nada — un
/// gris medio opaco es el "neutro" que típicamente se usa como base.
const RELLENO_DEFAULT: [u8; 4] = [128, 128, 128, 255];

/// Construye un buffer Rgba8 de `w × h` lleno con `rgba`. Pura. Salvo
/// errores de overflow (improbables en tamaños sanos), el `w * h * 4`
/// nunca pasa de unos MB para los lienzos típicos de tullpu.
fn buffer_relleno(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
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
fn agregar_capa_relleno(model: &mut Model) -> bool {
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
fn combinar_capa_abajo(model: &mut Model, id: Uuid) -> bool {
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

/// Calcula el bounding box (half-open `(x0, y0, x1, y1)`) de los píxeles
/// con alfa > 0 en un buffer Rgba8 `w × h`. Devuelve `None` si todos
/// los píxeles son transparentes (no hay nada para encerrar). Pura.
fn bbox_no_transparente(data: &[u8], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
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
fn recortar_buffer(src: &[u8], w: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<u8> {
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

/// Recorta el lienzo entero al bbox no-transparente del compuesto. Es
/// el "Trim Transparent Pixels" de Photoshop. La estrategia espeja
/// `rotar_lienzo`: (1) recorta el buffer de cada capa al mismo rect,
/// inserta al almacén content-addressed; (2) actualiza dims del
/// lienzo; (3) marca todas las derivadas Stale (ops como Blur no
/// conmutan exacto con crop por los efectos de borde — se regen desde
/// la madre recortada). No-op si el lienzo está vacío (todo
/// transparente) o si ya estaba justo (bbox = lienzo entero).
fn recortar_lienzo_a_visible(model: &mut Model) -> bool {
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
    // Recortar cada capa: lookup buffer, recortar, insertar nuevo hash.
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
    // Stale para todas las derivadas: Blur, Niveles con clamp, etc. no
    // siempre conmutan exacto con crop (kernel de borde, normalización).
    for capa in model.lienzo.capas.iter_mut() {
        if let OrigenCapa::Derivada { estado, .. } = &mut capa.origen {
            *estado = Frescura::Stale;
        }
    }
    aplicar_y_recomponer(model);
    model.estado = format!(
        "recortado a {}×{} (offset {},{})",
        new_w, new_h, x0, y0
    );
    true
}

/// Rota 90° en sentido horario un buffer Rgba8 `w × h`. El buffer
/// resultante tiene el mismo conteo de bytes pero su layout corresponde
/// a dimensiones `h × w` (el ancho del destino = el alto del origen).
/// Pura. Pre: `src.len() == w*h*4` (la validación va aguas arriba).
///
/// Mapeo: src `(x, y)` → dst `(h-1-y, x)` con `w_new = h`.
fn rotar_buffer_90_cw(src: &[u8], w: u32, h: u32) -> Vec<u8> {
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
fn rotar_buffer_90_ccw(src: &[u8], w: u32, h: u32) -> Vec<u8> {
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
fn rotar_lienzo(model: &mut Model, cw: bool) -> bool {
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
fn aplanar_capas_visibles(model: &mut Model) -> bool {
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

fn main() {
    llimphi_ui::run::<Tullpu>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_ui::{KeyState, Modifiers};

    fn ev_char(s: &str, mods: Modifiers) -> KeyEvent {
        KeyEvent {
            key: Key::Character(s.into()),
            state: KeyState::Pressed,
            text: Some(s.to_string()),
            modifiers: mods,
            repeat: false,
        }
    }
    fn ev_named(k: NamedKey, mods: Modifiers) -> KeyEvent {
        KeyEvent {
            key: Key::Named(k),
            state: KeyState::Pressed,
            text: None,
            modifiers: mods,
            repeat: false,
        }
    }
    fn modelo_minimo() -> Model {
        // Lienzo 4×4 con una capa raster, picker cerrado.
        let mut almacen = AlmacenEnMemoria::nuevo();
        let hash = almacen.insertar(vec![0u8; 4 * 4 * 4]);
        let mut lienzo = Lienzo::nuevo(4, 4);
        let cap = Capa::raster("c", hash);
        let id = cap.id;
        lienzo.apilar(cap);
        let historial = vec![lienzo.clone()];
        Model {
            lienzo,
            almacen,
            seleccionada: Some(id),
            imagen: None,
            estado: "test".into(),
            proveedor: Box::new(pixel_verbo_mock::ProveedorMock::nuevo()),
            proveedor_etiqueta: "test".into(),
            thumbs: HashMap::new(),
            raiz: PathBuf::from("/"),
            imagenes_disponibles: Vec::new(),
            picker: None,
            renombrando: None,
            historial,
            cursor_historial: 0,
            ultima_etiqueta_snapshot: None,
            factor_zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            herramienta: Herramienta::Mover,
            color_picked: None,
        }
    }

    #[test]
    fn blend_anterior_es_inverso_de_siguiente() {
        // Probar 5 modos elegidos a lo largo del ciclo para confirmar
        // que las dos funciones realmente son inversas — protege contra
        // que alguien agregue un modo al `siguiente` y se olvide del otro
        // (ahora son derivados del mismo `CICLO_BLEND` así que es
        // imposible, pero el test guarda la invariante).
        for &m in [
            ModoFusion::Normal,
            ModoFusion::Multiplicar,
            ModoFusion::LuzSuave,
            ModoFusion::HslColor,
            ModoFusion::Disolver,
        ]
        .iter()
        {
            assert_eq!(blend_anterior(siguiente_blend(m)), m);
            assert_eq!(siguiente_blend(blend_anterior(m)), m);
        }
        // El ciclo debe rotar exactamente con la cantidad de variantes:
        // aplicar `siguiente` CICLO_BLEND.len() veces es la identidad.
        let mut x = ModoFusion::Normal;
        for _ in 0..CICLO_BLEND.len() {
            x = siguiente_blend(x);
        }
        assert_eq!(x, ModoFusion::Normal);
    }

    #[test]
    fn hotkey_delete_elimina_capa_seleccionada() {
        let m = modelo_minimo();
        let id = m.seleccionada.unwrap();
        let msg = hotkey_a_msg(&m, &ev_named(NamedKey::Delete, Modifiers::default()));
        assert!(matches!(msg, Some(Msg::Eliminar(x)) if x == id));
    }

    #[test]
    fn hotkey_ctrl_d_duplica() {
        let m = modelo_minimo();
        let id = m.seleccionada.unwrap();
        let mods = Modifiers { ctrl: true, ..Default::default() };
        let msg = hotkey_a_msg(&m, &ev_char("d", mods));
        assert!(matches!(msg, Some(Msg::Duplicar(x)) if x == id));
    }

    #[test]
    fn hotkey_v_toggle_visible() {
        let m = modelo_minimo();
        let id = m.seleccionada.unwrap();
        let msg = hotkey_a_msg(&m, &ev_char("v", Modifiers::default()));
        assert!(matches!(msg, Some(Msg::ToggleVisible(x)) if x == id));
    }

    #[test]
    fn hotkey_b_y_shift_b_son_inversos_de_dispatch() {
        let m = modelo_minimo();
        let id = m.seleccionada.unwrap();
        let fwd = hotkey_a_msg(&m, &ev_char("b", Modifiers::default()));
        assert!(matches!(fwd, Some(Msg::CiclarBlend(x)) if x == id));
        let bwd = hotkey_a_msg(
            &m,
            &ev_char("b", Modifiers { shift: true, ..Default::default() }),
        );
        assert!(matches!(bwd, Some(Msg::CiclarBlendInverso(x)) if x == id));
    }

    #[test]
    fn hotkey_brackets_bump_opacidad_signo_correcto() {
        let m = modelo_minimo();
        let id = m.seleccionada.unwrap();
        let baja = hotkey_a_msg(&m, &ev_char("[", Modifiers::default()));
        let sube = hotkey_a_msg(&m, &ev_char("]", Modifiers::default()));
        match baja {
            Some(Msg::BumpOpacidad(x, d)) if x == id && (d + 0.1).abs() < 1e-6 => {}
            other => panic!("[ no dió −0.1: {other:?}", other = other.is_some()),
        }
        match sube {
            Some(Msg::BumpOpacidad(x, d)) if x == id && (d - 0.1).abs() < 1e-6 => {}
            other => panic!("] no dió +0.1: {other:?}", other = other.is_some()),
        }
    }

    #[test]
    fn hotkey_ctrl_s_y_ctrl_shift_s_exportan_distinto_formato() {
        let m = modelo_minimo();
        let png = hotkey_a_msg(
            &m,
            &ev_char("s", Modifiers { ctrl: true, ..Default::default() }),
        );
        assert!(matches!(png, Some(Msg::Exportar(FormatoExport::Png))));
        let webp = hotkey_a_msg(
            &m,
            &ev_char(
                "s",
                Modifiers { ctrl: true, shift: true, ..Default::default() },
            ),
        );
        assert!(matches!(webp, Some(Msg::Exportar(FormatoExport::Webp))));
    }

    #[test]
    fn hotkey_sin_seleccion_no_dispara_msg_de_capa() {
        let mut m = modelo_minimo();
        m.seleccionada = None;
        // Sin selección, Delete/V/B/[]/Ctrl+D no producen nada.
        for ev in [
            ev_named(NamedKey::Delete, Modifiers::default()),
            ev_char("v", Modifiers::default()),
            ev_char("b", Modifiers::default()),
            ev_char("[", Modifiers::default()),
            ev_char("]", Modifiers::default()),
            ev_char("d", Modifiers { ctrl: true, ..Default::default() }),
        ] {
            assert!(hotkey_a_msg(&m, &ev).is_none());
        }
        // Pero Ctrl+S sí — exporta el lienzo entero, no depende de capa.
        let png = hotkey_a_msg(
            &m,
            &ev_char("s", Modifiers { ctrl: true, ..Default::default() }),
        );
        assert!(matches!(png, Some(Msg::Exportar(FormatoExport::Png))));
    }

    #[test]
    fn hotkey_f2_inicia_renombrado() {
        let m = modelo_minimo();
        let id = m.seleccionada.unwrap();
        let msg = hotkey_a_msg(&m, &ev_named(NamedKey::F2, Modifiers::default()));
        assert!(matches!(msg, Some(Msg::IniciarRenombrar(x)) if x == id));
    }

    #[test]
    fn renombrar_precarga_nombre_y_lo_actualiza_en_confirmar() {
        // Simulo el flujo entero del update sin la UI: IniciarRenombrar
        // crea el TextInputState con el nombre actual; las teclas lo
        // editan; Confirmar lo escribe a la capa.
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        // Renombrar a "fondo nuevo" — la app pone el nombre actual y el
        // user va al final y tipea. Acá lo simplifico: set_text directo.
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarRenombrar(id),
            &Handle::for_test(),
        );
        assert!(model.renombrando.is_some());
        // El input arranca con el nombre actual de la capa.
        let (_, input) = model.renombrando.as_ref().unwrap();
        assert_eq!(input.text(), "c");
        // Edito directamente vía set_text — equivale a borrar + tipear.
        if let Some((_, input)) = model.renombrando.as_mut() {
            input.set_text("fondo nuevo");
        }
        model = <Tullpu as App>::update(model, Msg::ConfirmarRenombrar, &Handle::for_test());
        assert!(model.renombrando.is_none());
        assert_eq!(
            model.lienzo.capas.iter().find(|c| c.id == id).unwrap().nombre,
            "fondo nuevo"
        );
    }

    #[test]
    fn cancelar_renombrado_no_cambia_el_nombre() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let nombre_original = model
            .lienzo
            .capas
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .nombre
            .clone();
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarRenombrar(id),
            &Handle::for_test(),
        );
        if let Some((_, input)) = model.renombrando.as_mut() {
            input.set_text("intento descartado");
        }
        model = <Tullpu as App>::update(model, Msg::CancelarRenombrar, &Handle::for_test());
        assert!(model.renombrando.is_none());
        assert_eq!(
            model.lienzo.capas.iter().find(|c| c.id == id).unwrap().nombre,
            nombre_original
        );
    }

    #[test]
    fn confirmar_renombrado_vacio_no_pisa_el_nombre() {
        // Un input vacío al confirmar no es un nombre válido (rompería
        // la UX — la fila quedaría sin etiqueta). El update lo descarta y
        // mantiene el nombre original.
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarRenombrar(id),
            &Handle::for_test(),
        );
        if let Some((_, input)) = model.renombrando.as_mut() {
            input.set_text("   ");
        }
        model = <Tullpu as App>::update(model, Msg::ConfirmarRenombrar, &Handle::for_test());
        assert_eq!(
            model.lienzo.capas.iter().find(|c| c.id == id).unwrap().nombre,
            "c"
        );
    }

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

    // ---- Fase 23: undo/redo --------------------------------------------------

    #[test]
    fn hotkey_ctrl_z_y_variantes_redo_emiten_msg_correcto() {
        let m = modelo_minimo();
        // Ctrl+Z = undo.
        let undo = hotkey_a_msg(
            &m,
            &ev_char("z", Modifiers { ctrl: true, ..Default::default() }),
        );
        assert!(matches!(undo, Some(Msg::Undo)));
        // Ctrl+Shift+Z = redo.
        let redo_shift = hotkey_a_msg(
            &m,
            &ev_char(
                "z",
                Modifiers { ctrl: true, shift: true, ..Default::default() },
            ),
        );
        assert!(matches!(redo_shift, Some(Msg::Redo)));
        // Ctrl+Y = redo (alias).
        let redo_y = hotkey_a_msg(
            &m,
            &ev_char("y", Modifiers { ctrl: true, ..Default::default() }),
        );
        assert!(matches!(redo_y, Some(Msg::Redo)));
    }

    #[test]
    fn undo_sin_historial_anota_estado_pero_no_panickea() {
        // Modelo recién armado: historial tiene 1 sola entrada (la inicial).
        // Un Undo no debería hacer nada y el estado debe reflejarlo.
        let mut model = modelo_minimo();
        let lienzo_antes = model.lienzo.clone();
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo, lienzo_antes, "lienzo intacto");
        assert!(model.estado.contains("nada que deshacer"));
        assert_eq!(model.cursor_historial, 0);
    }

    #[test]
    fn undo_revierte_toggle_visible() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let visible_original = model.lienzo.capa(id).unwrap().visible;
        model = <Tullpu as App>::update(model, Msg::ToggleVisible(id), &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().visible, !visible_original);
        assert_eq!(model.historial.len(), 2);
        // Undo: volvemos al estado anterior.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().visible, visible_original);
        // Redo: re-aplicamos el toggle.
        model = <Tullpu as App>::update(model, Msg::Redo, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().visible, !visible_original);
    }

    #[test]
    fn nueva_mutacion_tras_undo_trunca_la_rama_de_redo() {
        // Mutación 1, mutación 2, undo (vuelvo a 1), mutación 3: la rama 2
        // queda descartada y un redo posterior debe ser no-op.
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        // M1: invertir visibilidad
        model = <Tullpu as App>::update(model, Msg::ToggleVisible(id), &Handle::for_test());
        // M2: invertirla de nuevo (la deja igual al estado original).
        model = <Tullpu as App>::update(model, Msg::ToggleVisible(id), &Handle::for_test());
        assert_eq!(model.historial.len(), 3);
        assert_eq!(model.cursor_historial, 2);
        // Undo: vuelvo a M1.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.cursor_historial, 1);
        // M3: ciclar blend → debe truncar M2.
        model = <Tullpu as App>::update(model, Msg::CiclarBlend(id), &Handle::for_test());
        assert_eq!(model.historial.len(), 3, "M2 fue truncada");
        assert_eq!(model.cursor_historial, 2);
        // Redo ahora no tiene a dónde ir.
        let snapshot = model.lienzo.clone();
        model = <Tullpu as App>::update(model, Msg::Redo, &Handle::for_test());
        assert_eq!(model.lienzo, snapshot);
        assert!(model.estado.contains("nada que rehacer"));
    }

    #[test]
    fn bump_opacidad_coalesce_drag_en_una_sola_entrada() {
        // Simulo un drag del slider: 50 BumpOpacidad consecutivas sobre la
        // misma capa. El historial debe crecer en 1 sola entrada (la final).
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let len_inicial = model.historial.len();
        for _ in 0..50 {
            model = <Tullpu as App>::update(
                model,
                Msg::BumpOpacidad(id, -0.01),
                &Handle::for_test(),
            );
        }
        assert_eq!(
            model.historial.len(),
            len_inicial + 1,
            "el drag entero debe coalesce a 1 snapshot"
        );
        // Un solo undo revierte el drag completo.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        let cap = model.lienzo.capa(id).unwrap();
        assert!(
            (cap.opacidad - 1.0).abs() < 1e-6,
            "opacidad volvió a 1.0, no quedó atrapada a medio camino: {}",
            cap.opacidad
        );
    }

    #[test]
    fn coalesce_no_cruza_entre_capas_distintas() {
        // Drag de opacidad sobre capa A y luego sobre capa B no deben
        // colapsar en la misma entrada — son operaciones independientes.
        let mut model = modelo_minimo();
        let id_a = model.seleccionada.unwrap();
        // Agrego una segunda capa raster para tener dos targets distintos.
        let mut almacen2 = std::mem::replace(&mut model.almacen, AlmacenEnMemoria::nuevo());
        let h_b = almacen2.insertar(vec![1u8; 4 * 4 * 4]);
        model.almacen = almacen2;
        let cap_b = Capa::raster("b", h_b);
        let id_b = cap_b.id;
        model.lienzo.apilar(cap_b);
        // Forzamos un snapshot manual del estado post-agregado (simulando
        // que la capa B vino vía Agregar/Eliminar — para este test ad-hoc
        // basta con pushear directo).
        pushear_snapshot(&mut model, None);
        let base = model.historial.len();

        // Drag sobre A
        for _ in 0..3 {
            model = <Tullpu as App>::update(
                model,
                Msg::BumpOpacidad(id_a, -0.05),
                &Handle::for_test(),
            );
        }
        assert_eq!(model.historial.len(), base + 1);

        // Drag sobre B (capa distinta → no coalesce con el de A)
        for _ in 0..3 {
            model = <Tullpu as App>::update(
                model,
                Msg::BumpOpacidad(id_b, -0.05),
                &Handle::for_test(),
            );
        }
        assert_eq!(
            model.historial.len(),
            base + 2,
            "drag en B agrega entrada propia"
        );
    }

    #[test]
    fn historial_capado_descarta_entradas_viejas() {
        // Fuerzo HIST_CAP+5 snapshots no-coalescables (sin etiqueta).
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        for _ in 0..(HIST_CAP + 5) {
            model = <Tullpu as App>::update(
                model,
                Msg::ToggleVisible(id),
                &Handle::for_test(),
            );
        }
        assert_eq!(model.historial.len(), HIST_CAP);
        assert_eq!(model.cursor_historial, HIST_CAP - 1);
    }

    #[test]
    fn undo_de_eliminar_resucita_la_capa() {
        // Una capa eliminada debe volver al hacer Ctrl+Z. La selección se
        // reajusta a la capa restaurada (única en el lienzo).
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        assert_eq!(model.lienzo.capas.len(), 1);
        model = <Tullpu as App>::update(model, Msg::Eliminar(id), &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 0);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 1);
        assert_eq!(model.lienzo.capas[0].id, id);
        // Ajusta_seleccion_tras_restaurar la reasigna ya que tras Eliminar
        // la habíamos blanqueado.
        assert_eq!(model.seleccionada, Some(id));
    }

    #[test]
    fn confirmar_renombrar_vacio_no_genera_snapshot() {
        // El path "input vacío" no muta el nombre → no debe ensuciar el
        // historial con una entrada idéntica.
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let len_inicial = model.historial.len();
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarRenombrar(id),
            &Handle::for_test(),
        );
        if let Some((_, input)) = model.renombrando.as_mut() {
            input.set_text("   "); // whitespace only — descartado por update
        }
        model = <Tullpu as App>::update(model, Msg::ConfirmarRenombrar, &Handle::for_test());
        assert_eq!(model.historial.len(), len_inicial);
    }

    // ---- Fase 24: zoom y pan --------------------------------------------------

    #[test]
    fn transform_lienzo_fit_centra_imagen_en_zoom_1() {
        // Imagen 100×100 en un rect 200×200 → s_fit=2, dw=200, off=0,0.
        let (s, off_x, off_y) = transform_lienzo(100, 100, 200.0, 200.0, 1.0, 0.0, 0.0)
            .expect("ok");
        assert!((s - 2.0).abs() < 1e-9);
        assert!(off_x.abs() < 1e-9);
        assert!(off_y.abs() < 1e-9);
    }

    #[test]
    fn transform_lienzo_aspect_distinto_pad_simétrico() {
        // Imagen 100×50 (2:1) en rect 200×200: s_fit=min(2, 4)=2, dw=200,
        // dh=100 → off_y = (200-100)/2 = 50, off_x = 0.
        let (s, off_x, off_y) = transform_lienzo(100, 50, 200.0, 200.0, 1.0, 0.0, 0.0)
            .expect("ok");
        assert!((s - 2.0).abs() < 1e-9);
        assert!(off_x.abs() < 1e-9);
        assert!((off_y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn transform_lienzo_factor_zoom_2_duplica_y_descentra() {
        // Imagen 100×100 fit en 200×200 con zoom=2: s=4, dw=400, off=-100,-100.
        // (la imagen "se sale" del rect — el clip se encarga en paint).
        let (s, off_x, off_y) = transform_lienzo(100, 100, 200.0, 200.0, 2.0, 0.0, 0.0)
            .expect("ok");
        assert!((s - 4.0).abs() < 1e-9);
        assert!((off_x + 100.0).abs() < 1e-9);
        assert!((off_y + 100.0).abs() < 1e-9);
    }

    #[test]
    fn transform_lienzo_pan_solo_traslada() {
        // Cualquier pan se suma directo al offset sin afectar la escala.
        let (s_a, ax, ay) = transform_lienzo(100, 100, 200.0, 200.0, 1.5, 0.0, 0.0).unwrap();
        let (s_b, bx, by) = transform_lienzo(100, 100, 200.0, 200.0, 1.5, 17.0, -23.0).unwrap();
        assert!((s_a - s_b).abs() < 1e-9);
        assert!((bx - ax - 17.0).abs() < 1e-9);
        assert!((by - ay + 23.0).abs() < 1e-9);
    }

    #[test]
    fn transform_lienzo_dims_cero_devuelve_none() {
        assert!(transform_lienzo(0, 10, 100.0, 100.0, 1.0, 0.0, 0.0).is_none());
        assert!(transform_lienzo(10, 10, 0.0, 100.0, 1.0, 0.0, 0.0).is_none());
        assert!(transform_lienzo(10, 10, 100.0, -1.0, 1.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn zoom_a_cursor_mantiene_el_pixel_bajo_el_cursor_fijo() {
        // Imagen 100×100, rect 200×200, zoom_old=1 → s=2, top-left=(0,0).
        // Cursor en (50, 60) → píxel-imagen (25, 30).
        // Zoom a 2: s_new=4, queremos top-left tal que (25,30) caiga en (50,60):
        // tx_new = 50 - 25*4 = -50, ty_new = 60 - 30*4 = -60.
        // dw=400, dh=400 → centered_off = (200-400)/2 = -100.
        // pan = tx_new - centered_off = -50 - (-100) = 50.
        let rect = PaintRect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };
        let (pan_x, pan_y) =
            pan_para_zoom_a_cursor(100, 100, rect, 50.0, 60.0, 1.0, 2.0, 0.0, 0.0);
        assert!((pan_x - 50.0).abs() < 1e-3, "pan_x = {}", pan_x);
        // píxel-imagen y=30 → tx_new=60-30*4=-60, centered_off_y=-100,
        // pan_y = -60 - (-100) = 40.
        assert!((pan_y - 40.0).abs() < 1e-3, "pan_y = {}", pan_y);

        // Verificación cruzada: aplico el transform y reviso que (50,60)
        // corresponde a (25, 30) en coords-imagen al zoom 2.
        let (s_new, off_x, off_y) =
            transform_lienzo(100, 100, rect.w, rect.h, 2.0, pan_x, pan_y).unwrap();
        let tx = rect.x as f64 + off_x;
        let ty = rect.y as f64 + off_y;
        let ix = (50.0 - tx) / s_new;
        let iy = (60.0 - ty) / s_new;
        assert!((ix - 25.0).abs() < 1e-3, "ix = {}", ix);
        assert!((iy - 30.0).abs() < 1e-3, "iy = {}", iy);
    }

    #[test]
    fn dentro_de_rect_es_inclusive_en_bordes() {
        let r = PaintRect { x: 10.0, y: 20.0, w: 100.0, h: 50.0 };
        assert!(dentro_de_rect(r, 10.0, 20.0));
        assert!(dentro_de_rect(r, 110.0, 70.0));
        assert!(dentro_de_rect(r, 60.0, 45.0));
        assert!(!dentro_de_rect(r, 9.99, 50.0));
        assert!(!dentro_de_rect(r, 60.0, 70.01));
        assert!(!dentro_de_rect(r, 110.01, 45.0));
    }

    #[test]
    fn msg_zoom_aplica_clamp_min_max() {
        // factor_zoom inicial = 1.0. Mult = 0.0001 → clamp a ZOOM_MIN.
        let mut model = modelo_minimo();
        model = <Tullpu as App>::update(
            model,
            Msg::Zoom { mult: 0.0001, ancla: None },
            &Handle::for_test(),
        );
        assert!((model.factor_zoom - ZOOM_MIN).abs() < 1e-6);
        // Y al revés: mult grande → ZOOM_MAX.
        model = <Tullpu as App>::update(
            model,
            Msg::Zoom { mult: 1e6, ancla: None },
            &Handle::for_test(),
        );
        assert!((model.factor_zoom - ZOOM_MAX).abs() < 1e-6);
    }

    #[test]
    fn msg_pan_acumula_offsets() {
        let mut model = modelo_minimo();
        model = <Tullpu as App>::update(model, Msg::Pan(10.0, -5.0), &Handle::for_test());
        model = <Tullpu as App>::update(model, Msg::Pan(3.0, 7.0), &Handle::for_test());
        assert!((model.pan_x - 13.0).abs() < 1e-6);
        assert!((model.pan_y - 2.0).abs() < 1e-6);
    }

    #[test]
    fn msg_reset_vista_restaura_zoom_y_pan_default() {
        let mut model = modelo_minimo();
        model.factor_zoom = 3.5;
        model.pan_x = 42.0;
        model.pan_y = -17.0;
        model = <Tullpu as App>::update(model, Msg::ResetVista, &Handle::for_test());
        assert!((model.factor_zoom - 1.0).abs() < 1e-6);
        assert_eq!(model.pan_x, 0.0);
        assert_eq!(model.pan_y, 0.0);
    }

    #[test]
    fn hotkey_cero_emite_reset_vista() {
        let model = modelo_minimo();
        let msg = hotkey_a_msg(&model, &ev_char("0", Modifiers::default()));
        assert!(matches!(msg, Some(Msg::ResetVista)));
        // Con Ctrl no — el 0 estándar es sin modificador.
        let ctrl0 = hotkey_a_msg(
            &model,
            &ev_char("0", Modifiers { ctrl: true, ..Default::default() }),
        );
        assert!(matches!(ctrl0, None));
    }

    // ---- Fase 25: herramientas + cuentagotas ---------------------------------

    /// Construye un buffer Rgba8 4×4 con un patrón conocido: cada píxel
    /// codifica su posición en (R, G), con B fijo y α opaco. Útil para
    /// verificar que el sampler aterriza en la celda correcta.
    fn buffer_patron_4x4() -> Vec<u8> {
        let mut v = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4u8 {
            for x in 0..4u8 {
                v.extend_from_slice(&[x * 60, y * 60, 17, 255]);
            }
        }
        v
    }

    #[test]
    fn recoger_color_pixel_central_a_zoom_1() {
        // Imagen 4×4 en rect 200×200 → s_fit = 50; cada píxel ocupa 50 px.
        // Click en (75, 25) cae en x=1, y=0 → R=60, G=0, B=17, α=255.
        let buf = buffer_patron_4x4();
        let col =
            recoger_color_en(&buf, 4, 4, 75.0, 25.0, 200.0, 200.0, 1.0, 0.0, 0.0).unwrap();
        assert_eq!(col, [60, 0, 17, 255], "(1, 0) esperado");
        // Click en (125, 175) cae en x=2, y=3 → R=120, G=180, B=17, α=255.
        let col2 =
            recoger_color_en(&buf, 4, 4, 125.0, 175.0, 200.0, 200.0, 1.0, 0.0, 0.0).unwrap();
        assert_eq!(col2, [120, 180, 17, 255], "(2, 3) esperado");
    }

    #[test]
    fn recoger_color_fuera_de_imagen_devuelve_none() {
        // Imagen 4×4 en rect 200×100 fit-contain → s=25, dw=100, off_x=50
        // (la imagen está centrada con bandas a izquierda y derecha). Un
        // click en x=10 cae en la banda transparente → fuera de la imagen.
        let buf = buffer_patron_4x4();
        assert!(recoger_color_en(&buf, 4, 4, 10.0, 50.0, 200.0, 100.0, 1.0, 0.0, 0.0).is_none());
        // También fuera por arriba.
        assert!(recoger_color_en(&buf, 4, 4, 100.0, -5.0, 200.0, 100.0, 1.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn recoger_color_respeta_zoom_y_pan() {
        // Mismo buffer 4×4 en rect 200×200. A zoom 2 + pan (0,0) la imagen
        // queda 400×400 centrada en el rect → top-left en (-100, -100).
        // Cada píxel ocupa 100 px. Click en (0, 0) (esquina del rect) cae
        // en píxel (1, 1) — verifico R=60, G=60.
        let buf = buffer_patron_4x4();
        let col = recoger_color_en(&buf, 4, 4, 0.0, 0.0, 200.0, 200.0, 2.0, 0.0, 0.0).unwrap();
        assert_eq!(col, [60, 60, 17, 255], "esquina superior con zoom 2");
    }

    #[test]
    fn recoger_color_buffer_corto_devuelve_none() {
        // Buffer prometido como 4×4 pero sólo trae 2 píxeles → indexar más
        // allá no debe panickear: devolvemos None.
        let buf = vec![10, 20, 30, 255, 40, 50, 60, 255];
        // Click que apuntaría al píxel (3, 3) — fuera del buffer real.
        assert!(recoger_color_en(&buf, 4, 4, 175.0, 175.0, 200.0, 200.0, 1.0, 0.0, 0.0).is_none());
    }

    #[test]
    fn msg_recoger_color_actualiza_color_picked() {
        // Un Model mínimo con una imagen 4×4 conocida; despachamos
        // RecogerColor y verificamos que `color_picked` queda con el RGBA
        // del píxel correcto.
        let mut model = modelo_minimo();
        // Reemplazamos la imagen del modelo por una con buffer conocido.
        let buf = buffer_patron_4x4();
        let blob = Blob::from(buf);
        model.imagen = Some(Image::new(blob, ImageFormat::Rgba8, 4, 4));
        // Click en píxel (2, 3) sobre rect 200×200 a zoom 1 → R=120, G=180.
        model = <Tullpu as App>::update(
            model,
            Msg::RecogerColor { lx: 125.0, ly: 175.0, rw: 200.0, rh: 200.0 },
            &Handle::for_test(),
        );
        assert_eq!(model.color_picked, Some([120, 180, 17, 255]));
        assert!(model.estado.contains("#78B411"), "estado = {}", model.estado);
    }

    #[test]
    fn msg_recoger_color_fuera_no_pisa_color_anterior() {
        let mut model = modelo_minimo();
        let buf = buffer_patron_4x4();
        let blob = Blob::from(buf);
        model.imagen = Some(Image::new(blob, ImageFormat::Rgba8, 4, 4));
        model.color_picked = Some([1, 2, 3, 4]);
        // Click fuera del área de imagen (banda del pad).
        model = <Tullpu as App>::update(
            model,
            Msg::RecogerColor { lx: 5.0, ly: 50.0, rw: 200.0, rh: 100.0 },
            &Handle::for_test(),
        );
        assert_eq!(model.color_picked, Some([1, 2, 3, 4]), "color anterior intacto");
        assert!(model.estado.contains("fuera"));
    }

    #[test]
    fn msg_cambiar_herramienta_actualiza_modo() {
        let mut model = modelo_minimo();
        assert_eq!(model.herramienta, Herramienta::Mover);
        model = <Tullpu as App>::update(
            model,
            Msg::CambiarHerramienta(Herramienta::Cuentagotas),
            &Handle::for_test(),
        );
        assert_eq!(model.herramienta, Herramienta::Cuentagotas);
        model = <Tullpu as App>::update(
            model,
            Msg::CambiarHerramienta(Herramienta::Mover),
            &Handle::for_test(),
        );
        assert_eq!(model.herramienta, Herramienta::Mover);
    }

    #[test]
    fn hotkey_m_e_i_emiten_cambio_de_herramienta() {
        let model = modelo_minimo();
        let mover = hotkey_a_msg(&model, &ev_char("m", Modifiers::default()));
        assert!(matches!(
            mover,
            Some(Msg::CambiarHerramienta(Herramienta::Mover))
        ));
        let cuenta = hotkey_a_msg(&model, &ev_char("i", Modifiers::default()));
        assert!(matches!(
            cuenta,
            Some(Msg::CambiarHerramienta(Herramienta::Cuentagotas))
        ));
        // Con Ctrl o Alt no deben disparar — son hotkeys de tecla suelta.
        assert!(hotkey_a_msg(
            &model,
            &ev_char("m", Modifiers { ctrl: true, ..Default::default() })
        )
        .is_none());
        assert!(hotkey_a_msg(
            &model,
            &ev_char("i", Modifiers { alt: true, ..Default::default() })
        )
        .is_none());
    }

    // ---- Fase 26: capa de relleno sólido ------------------------------------

    #[test]
    fn buffer_relleno_tiene_tamano_y_patron_correctos() {
        let buf = buffer_relleno(3, 2, [10, 20, 30, 40]);
        // 3×2 píxeles × 4 bytes/px = 24 bytes.
        assert_eq!(buf.len(), 24);
        // Cada cuádruple es el RGBA pedido — sin gaps.
        for cuadruple in buf.chunks_exact(4) {
            assert_eq!(cuadruple, &[10, 20, 30, 40]);
        }
    }

    #[test]
    fn agregar_capa_relleno_default_cuando_no_hay_color_picked() {
        // Sin color leído, debe usar RELLENO_DEFAULT (gris medio).
        let mut model = modelo_minimo();
        assert!(model.color_picked.is_none());
        let n_antes = model.lienzo.capas.len();
        agregar_capa_relleno(&mut model);
        assert_eq!(model.lienzo.capas.len(), n_antes + 1);
        let nueva = model.lienzo.capas.last().unwrap();
        assert!(
            nueva.nombre.starts_with("relleno #")
                && nueva.nombre.contains("808080"),
            "nombre {} debe llevar el hex del default",
            nueva.nombre
        );
    }

    #[test]
    fn agregar_capa_relleno_usa_color_picked_si_existe() {
        let mut model = modelo_minimo();
        model.color_picked = Some([200, 100, 50, 255]);
        agregar_capa_relleno(&mut model);
        let nueva = model.lienzo.capas.last().unwrap();
        assert!(
            nueva.nombre.contains("C86432"),
            "nombre {} debe llevar el hex del picked",
            nueva.nombre
        );
    }

    #[test]
    fn agregar_capa_relleno_dos_veces_mismo_color_comparte_hash() {
        // Content-addressing: dos rellenos del mismo color al mismo lienzo
        // producen el mismo Hash y comparten el slot del almacén — no
        // duplican RAM. Las capas tienen Uuid distinto pero contenido = ptr
        // al mismo buffer.
        let mut model = modelo_minimo();
        model.color_picked = Some([42, 42, 42, 255]);
        agregar_capa_relleno(&mut model);
        let h1 = match model.lienzo.capas.last().unwrap().origen {
            tullpu_core::OrigenCapa::Raster => model
                .lienzo
                .capas
                .last()
                .unwrap()
                .contenido,
            _ => panic!("esperaba raster"),
        };
        agregar_capa_relleno(&mut model);
        let h2 = match model.lienzo.capas.last().unwrap().origen {
            tullpu_core::OrigenCapa::Raster => model
                .lienzo
                .capas
                .last()
                .unwrap()
                .contenido,
            _ => panic!("esperaba raster"),
        };
        assert_eq!(h1, h2, "mismo color → mismo hash (dedup)");
        // Pero los Uuid son distintos: son capas independientes.
        let n = model.lienzo.capas.len();
        assert_ne!(
            model.lienzo.capas[n - 1].id,
            model.lienzo.capas[n - 2].id
        );
    }

    #[test]
    fn agregar_capa_relleno_se_inserta_encima_de_la_seleccionada() {
        // Si hay selección, la capa nueva queda en idx_sel + 1.
        let mut model = modelo_minimo();
        let sel = model.seleccionada.unwrap();
        let idx_sel = model
            .lienzo
            .capas
            .iter()
            .position(|c| c.id == sel)
            .unwrap();
        // Agrego una capa B "vieja" para tener una vecina arriba de sel.
        let hash_b = model.almacen.insertar(vec![9u8; 4 * 4 * 4]);
        let cap_b = Capa::raster("vieja", hash_b);
        model.lienzo.apilar(cap_b);
        // Ahora reapunto la selección a sel y agrego el relleno: debe
        // quedar entre sel y "vieja", no al tope.
        model.seleccionada = Some(sel);
        agregar_capa_relleno(&mut model);
        let nueva_idx = model
            .lienzo
            .capas
            .iter()
            .position(|c| c.nombre.starts_with("relleno"))
            .unwrap();
        assert_eq!(nueva_idx, idx_sel + 1, "encima de la seleccionada");
        // Y "vieja" pasó a estar arriba del relleno (idx mayor).
        let vieja_idx = model
            .lienzo
            .capas
            .iter()
            .position(|c| c.nombre == "vieja")
            .unwrap();
        assert!(vieja_idx > nueva_idx);
    }

    #[test]
    fn msg_agregar_relleno_dispatcha_y_snapshotea() {
        // El flujo entero por el update: el historial crece, el lienzo
        // tiene una capa más, y un Undo lo deshace.
        let mut model = modelo_minimo();
        let n_antes = model.lienzo.capas.len();
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(model, Msg::AgregarRelleno, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), n_antes + 1);
        assert_eq!(model.historial.len(), hist_antes + 1);
        // Undo lo revierte.
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), n_antes);
    }

    // ---- Fase 27: combinar capa hacia abajo (merge down) -------------------

    /// Construye un modelo con dos capas raster opacas de colores planos:
    /// debajo `rgba_bajo` ocupando todo el lienzo 2×2; encima `rgba_alto`
    /// también ocupando todo. Devuelve (model, id_abajo, id_arriba).
    fn modelo_dos_capas(rgba_bajo: [u8; 4], rgba_alto: [u8; 4]) -> (Model, Uuid, Uuid) {
        let mut model = modelo_minimo();
        // El minimo trae una capa de 4×4 todo en cero — la usamos como
        // "abajo". Reemplazamos su contenido por el del color pedido.
        model.lienzo = Lienzo::nuevo(2, 2);
        let buf_b = buffer_relleno(2, 2, rgba_bajo);
        let h_b = model.almacen.insertar(buf_b);
        let cap_b = Capa::raster("base", h_b);
        let id_b = cap_b.id;
        model.lienzo.apilar(cap_b);
        let buf_a = buffer_relleno(2, 2, rgba_alto);
        let h_a = model.almacen.insertar(buf_a);
        let cap_a = Capa::raster("sobre", h_a);
        let id_a = cap_a.id;
        model.lienzo.apilar(cap_a);
        model.seleccionada = Some(id_a);
        // Reseteamos el historial para que este sea el estado base.
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        (model, id_b, id_a)
    }

    #[test]
    fn combinar_capa_en_fondo_es_no_op_con_mensaje() {
        // La capa en idx 0 no tiene nada debajo: la merge es un no-op
        // semántico. El lienzo no cambia y el estado avisa.
        let (mut model, id_b, _) = modelo_dos_capas([10, 20, 30, 255], [200, 100, 50, 255]);
        let lienzo_antes = model.lienzo.clone();
        let ok = combinar_capa_abajo(&mut model, id_b);
        assert!(!ok, "no debe reportar éxito");
        assert_eq!(model.lienzo, lienzo_antes, "lienzo intacto");
        assert!(model.estado.contains("no hay capa debajo"));
    }

    #[test]
    fn combinar_capas_normales_aplana_a_la_de_arriba_opaca() {
        // Dos rasters opacos con blend Normal y opacidad 1.0: el composite
        // es exactamente la capa de arriba (la de abajo queda totalmente
        // cubierta). La merge debe producir un buffer de ese color.
        let (mut model, _id_b, id_a) =
            modelo_dos_capas([10, 20, 30, 255], [200, 100, 50, 255]);
        assert_eq!(model.lienzo.capas.len(), 2);
        let ok = combinar_capa_abajo(&mut model, id_a);
        assert!(ok);
        assert_eq!(model.lienzo.capas.len(), 1);
        let nueva = &model.lienzo.capas[0];
        let buf = model.almacen.obtener(nueva.contenido).unwrap();
        // 2×2 píxeles, todos el color de arriba.
        assert_eq!(buf.len(), 16);
        for px in buf.chunks_exact(4) {
            assert_eq!(px, &[200, 100, 50, 255]);
        }
        // El nombre conserva la genealogía con el separador ⊕.
        assert!(nueva.nombre.contains("⊕"), "nombre = {}", nueva.nombre);
        // Selección apuntó a la merged.
        assert_eq!(model.seleccionada, Some(nueva.id));
    }

    #[test]
    fn combinar_capa_con_opacidad_media_mezcla_50_50() {
        // Arriba semitransparente (α=128) sobre fondo opaco: el resultado
        // debe ser aprox. promedio. Tolerancia ±2 por el rounding del
        // compositor (premultiplicación + división).
        let (mut model, _id_b, id_a) =
            modelo_dos_capas([0, 0, 0, 255], [255, 255, 255, 255]);
        // Bajamos la opacidad de la capa de arriba a 0.5.
        let idx_a = model.lienzo.capas.iter().position(|c| c.id == id_a).unwrap();
        model.lienzo.capas[idx_a].opacidad = 0.5;
        let ok = combinar_capa_abajo(&mut model, id_a);
        assert!(ok);
        let nueva = &model.lienzo.capas[0];
        let buf = model.almacen.obtener(nueva.contenido).unwrap();
        for px in buf.chunks_exact(4) {
            for c in 0..3 {
                assert!(
                    (px[c] as i32 - 128).abs() <= 4,
                    "canal {} = {} no está cerca de 128",
                    c,
                    px[c]
                );
            }
            assert_eq!(px[3], 255);
        }
        // Crítico: la merged tiene opacidad 1.0 y blend Normal, no
        // heredando el 0.5 — el 0.5 ya quedó horneado en los píxeles.
        assert!((nueva.opacidad - 1.0).abs() < 1e-6);
        assert_eq!(nueva.blend, ModoFusion::Normal);
    }

    #[test]
    fn combinar_dos_mismos_pares_comparten_hash() {
        // Content-addressing: mergear el mismo par dos veces produce el
        // mismo hash en el almacén (la pintura es función de las capas).
        let (mut m1, _, id_a1) =
            modelo_dos_capas([12, 34, 56, 255], [78, 90, 12, 255]);
        let (mut m2, _, id_a2) =
            modelo_dos_capas([12, 34, 56, 255], [78, 90, 12, 255]);
        combinar_capa_abajo(&mut m1, id_a1);
        combinar_capa_abajo(&mut m2, id_a2);
        let h1 = m1.lienzo.capas[0].contenido;
        let h2 = m2.lienzo.capas[0].contenido;
        assert_eq!(h1, h2, "mismo composite ⇒ mismo hash");
    }

    #[test]
    fn msg_combinar_dispatcha_y_undo_restaura() {
        // El flujo completo por update: tras Combinar hay 1 capa, tras
        // Undo vuelven las 2 originales.
        let (mut model, id_b, id_a) =
            modelo_dos_capas([0, 0, 0, 255], [255, 255, 255, 255]);
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(model, Msg::Combinar(id_a), &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 1);
        assert_eq!(model.historial.len(), hist_antes + 1);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 2);
        // Los Uuid originales vuelven (el historial guarda el Lienzo
        // entero, no rastrea hashes de buffers).
        let ids_post: Vec<Uuid> = model.lienzo.capas.iter().map(|c| c.id).collect();
        assert!(ids_post.contains(&id_b));
        assert!(ids_post.contains(&id_a));
    }

    #[test]
    fn hotkey_ctrl_e_emite_combinar() {
        let (model, _, id_a) = modelo_dos_capas([0; 4], [0; 4]);
        let mods = Modifiers { ctrl: true, ..Default::default() };
        let msg = hotkey_a_msg(&model, &ev_char("e", mods));
        assert!(matches!(msg, Some(Msg::Combinar(x)) if x == id_a));
        // Sin Ctrl, la `e` suelta no debe disparar nada.
        let msg2 = hotkey_a_msg(&model, &ev_char("e", Modifiers::default()));
        assert!(msg2.is_none());
    }

    // ---- Fase 28: aplanar visibles (merge visible) ---------------------------

    /// Helper que mete N capas raster opacas de colores distintos al
    /// modelo mínimo. Devuelve los Uuid en orden de inserción
    /// (capas[0] = primera retornada).
    fn modelo_n_capas(colores: &[[u8; 4]]) -> (Model, Vec<Uuid>) {
        let mut model = modelo_minimo();
        model.lienzo = Lienzo::nuevo(2, 2);
        let mut ids = Vec::new();
        for (i, &c) in colores.iter().enumerate() {
            let buf = buffer_relleno(2, 2, c);
            let h = model.almacen.insertar(buf);
            let cap = Capa::raster(format!("c{}", i), h);
            ids.push(cap.id);
            model.lienzo.apilar(cap);
        }
        model.seleccionada = ids.first().copied();
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        (model, ids)
    }

    #[test]
    fn aplanar_con_cero_visibles_es_no_op() {
        let (mut model, ids) = modelo_n_capas(&[[10, 20, 30, 255]]);
        // Oculto la única capa que hay.
        let idx = model
            .lienzo
            .capas
            .iter()
            .position(|c| c.id == ids[0])
            .unwrap();
        model.lienzo.capas[idx].visible = false;
        let lienzo_antes = model.lienzo.clone();
        let ok = aplanar_capas_visibles(&mut model);
        assert!(!ok);
        assert_eq!(model.lienzo, lienzo_antes);
        assert!(model.estado.contains("nada visible"));
    }

    #[test]
    fn aplanar_con_una_sola_visible_es_no_op() {
        let (mut model, _) = modelo_n_capas(&[[10, 20, 30, 255]]);
        let lienzo_antes = model.lienzo.clone();
        let ok = aplanar_capas_visibles(&mut model);
        assert!(!ok);
        assert_eq!(model.lienzo, lienzo_antes);
        assert!(model.estado.contains("una sola"));
    }

    #[test]
    fn aplanar_dos_visibles_da_una_capa_con_composite() {
        // Dos Normal/opacas: el composite es el color de arriba.
        let (mut model, _) =
            modelo_n_capas(&[[10, 20, 30, 255], [200, 100, 50, 255]]);
        let ok = aplanar_capas_visibles(&mut model);
        assert!(ok);
        assert_eq!(model.lienzo.capas.len(), 1);
        let buf = model.almacen.obtener(model.lienzo.capas[0].contenido).unwrap();
        for px in buf.chunks_exact(4) {
            assert_eq!(px, &[200, 100, 50, 255]);
        }
        // La merged hereda defaults Normal/1.0/visible.
        assert!((model.lienzo.capas[0].opacidad - 1.0).abs() < 1e-6);
        assert_eq!(model.lienzo.capas[0].blend, ModoFusion::Normal);
    }

    #[test]
    fn aplanar_preserva_hidden_intercalado_en_su_posicion_topologica() {
        // Lienzo de 4 capas en orden fondo→tope:
        //   c0 (v)  bg
        //   c1 (h)  hidA — entre dos visibles
        //   c2 (v)  fg
        //   c3 (h)  hidB — encima de la última visible
        // Esperado tras aplanar: [hidA, merged, hidB] (3 capas).
        let (mut model, ids) = modelo_n_capas(&[
            [10, 0, 0, 255],
            [0, 20, 0, 255],
            [0, 0, 30, 255],
            [40, 40, 40, 255],
        ]);
        // Marco c1 y c3 como hidden.
        for &id in &[ids[1], ids[3]] {
            let idx = model
                .lienzo
                .capas
                .iter()
                .position(|c| c.id == id)
                .unwrap();
            model.lienzo.capas[idx].visible = false;
        }
        let ok = aplanar_capas_visibles(&mut model);
        assert!(ok);
        // 4 originales − 2 visibles + 1 merged = 3 capas.
        assert_eq!(model.lienzo.capas.len(), 3);
        // Orden esperado: hidA (idx 0), merged (idx 1), hidB (idx 2).
        assert_eq!(model.lienzo.capas[0].id, ids[1]);
        assert!(model.lienzo.capas[1].nombre.starts_with("aplanado"));
        assert_eq!(model.lienzo.capas[2].id, ids[3]);
        // hidA y hidB siguen siendo invisibles.
        assert!(!model.lienzo.capas[0].visible);
        assert!(!model.lienzo.capas[2].visible);
    }

    #[test]
    fn aplanar_no_visible_arriba_de_todo_inserta_al_tope() {
        // Caso degenerado: una hidden ARRIBA de la última visible.
        // [v0 v, v1 v, hid v0 → no, hidden h]
        // Tras aplanar: [merged, hidden] (merged va donde estaba v1,
        // que era max_visible=1; hidden de 0..=1 = 0, así que insert_idx=0
        // — merged va a idx 0, hidden queda atrás al final).
        let (mut model, ids) = modelo_n_capas(&[
            [10, 0, 0, 255],
            [0, 20, 0, 255],
            [40, 40, 40, 255],
        ]);
        let idx_hidden = model
            .lienzo
            .capas
            .iter()
            .position(|c| c.id == ids[2])
            .unwrap();
        model.lienzo.capas[idx_hidden].visible = false;
        let ok = aplanar_capas_visibles(&mut model);
        assert!(ok);
        assert_eq!(model.lienzo.capas.len(), 2);
        // merged va a 0 (no había hidden por debajo del top visible).
        assert!(model.lienzo.capas[0].nombre.starts_with("aplanado"));
        // La hidden queda en idx 1 (arriba).
        assert_eq!(model.lienzo.capas[1].id, ids[2]);
    }

    #[test]
    fn msg_aplanar_dispatcha_y_undo_restaura() {
        let (mut model, ids) =
            modelo_n_capas(&[[1, 2, 3, 255], [4, 5, 6, 255], [7, 8, 9, 255]]);
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(model, Msg::AplanarVisibles, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 1);
        assert_eq!(model.historial.len(), hist_antes + 1);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capas.len(), 3);
        let ids_post: Vec<Uuid> = model.lienzo.capas.iter().map(|c| c.id).collect();
        for id in ids {
            assert!(ids_post.contains(&id));
        }
    }

    #[test]
    fn hotkey_ctrl_shift_e_emite_aplanar() {
        let m = modelo_minimo();
        let mods = Modifiers { ctrl: true, shift: true, ..Default::default() };
        let msg = hotkey_a_msg(&m, &ev_char("e", mods));
        assert!(matches!(msg, Some(Msg::AplanarVisibles)));
        // Ctrl+E (sin shift) sigue siendo Combinar(id), no AplanarVisibles.
        let solo_ctrl = hotkey_a_msg(
            &m,
            &ev_char("e", Modifiers { ctrl: true, ..Default::default() }),
        );
        assert!(matches!(solo_ctrl, Some(Msg::Combinar(_))));
    }

    // ---- Fase 30: rotar lienzo 90° -----------------------------------------

    fn px_at(buf: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn rotar_buffer_90_cw_mueve_top_left_a_top_right() {
        // src 2×3 con 6 colores distintos. Verifico el mapeo:
        //   src        dst (3×2)
        //   A B        E C A
        //   C D   →    F D B
        //   E F
        let src = vec![
            // row 0:           A             B
            10, 0, 0, 255,   20, 0, 0, 255,
            // row 1:           C             D
            30, 0, 0, 255,   40, 0, 0, 255,
            // row 2:           E             F
            50, 0, 0, 255,   60, 0, 0, 255,
        ];
        let out = rotar_buffer_90_cw(&src, 2, 3);
        // dst dims son 3×2.
        assert_eq!(out.len(), 24);
        // A en (0,0) → (2,0) en dst
        assert_eq!(px_at(&out, 3, 2, 0)[0], 10);
        // B en (1,0) → (2,1) en dst
        assert_eq!(px_at(&out, 3, 2, 1)[0], 20);
        // C en (0,1) → (1,0) en dst
        assert_eq!(px_at(&out, 3, 1, 0)[0], 30);
        // E en (0,2) → (0,0) en dst (top-left de dst era bottom-left de src)
        assert_eq!(px_at(&out, 3, 0, 0)[0], 50);
        // F en (1,2) → (0,1) en dst
        assert_eq!(px_at(&out, 3, 0, 1)[0], 60);
    }

    #[test]
    fn rotar_buffer_90_ccw_es_inversa_de_cw() {
        // Aplicar CW y luego CCW debe devolver el buffer original
        // bit-a-bit. Garantía para que "rotar a un lado y volver" no
        // pierda nada.
        let src = vec![
            // 4×3 con un patrón distinguible.
            1, 2, 3, 255,    4, 5, 6, 255,    7, 8, 9, 255,   10, 11, 12, 255,
            13, 14, 15, 255, 16, 17, 18, 255, 19, 20, 21, 255, 22, 23, 24, 255,
            25, 26, 27, 255, 28, 29, 30, 255, 31, 32, 33, 255, 34, 35, 36, 255,
        ];
        let cw = rotar_buffer_90_cw(&src, 4, 3);
        // cw quedó con dims 3×4. Aplicar CCW debe revertir a 4×3 idéntico.
        let regreso = rotar_buffer_90_ccw(&cw, 3, 4);
        assert_eq!(regreso, src);
    }

    #[test]
    fn rotar_buffer_90_cw_dos_veces_es_rotacion_180() {
        // CW + CW debe equivaler a espejar h + espejar v (rotación 180°).
        // Calculo ambos y comparo.
        let src = vec![
            10, 0, 0, 255,   20, 0, 0, 255,
            30, 0, 0, 255,   40, 0, 0, 255,
            50, 0, 0, 255,   60, 0, 0, 255,
        ];
        let dos_cw = {
            let una = rotar_buffer_90_cw(&src, 2, 3);
            // una es 3×2. CW de nuevo da 2×3.
            rotar_buffer_90_cw(&una, 3, 2)
        };
        // Construyo el espejado 180° vía buffer_relleno + manual:
        // src reversed-byte-wise (en grupos de 4) da el 180°.
        let mut esperado = vec![0u8; src.len()];
        for i in 0..(src.len() / 4) {
            let i_src = i * 4;
            let i_dst = ((src.len() / 4) - 1 - i) * 4;
            esperado[i_dst..i_dst + 4].copy_from_slice(&src[i_src..i_src + 4]);
        }
        assert_eq!(dos_cw, esperado);
    }

    #[test]
    fn rotar_lienzo_cw_intercambia_dimensiones() {
        let (mut model, _) =
            modelo_n_capas(&[[10, 20, 30, 255], [200, 100, 50, 255]]);
        // El lienzo era 2×2 después de modelo_n_capas. Tras rotar +90°
        // sigue siendo 2×2 (cuadrado), así que para verificar el swap
        // armo un lienzo 2×3 explícitamente.
        model.lienzo = Lienzo::nuevo(2, 3);
        // Cargo una capa raster cualquiera; lo que importa es la dim.
        let buf = buffer_relleno(2, 3, [100, 100, 100, 255]);
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("c", h);
        let id = cap.id;
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        let ok = rotar_lienzo(&mut model, true);
        assert!(ok);
        assert_eq!(model.lienzo.width, 3);
        assert_eq!(model.lienzo.height, 2);
    }

    #[test]
    fn rotar_lienzo_ccw_es_inversa_de_cw() {
        // CW seguido de CCW debe restaurar dims (y los buffers de las
        // capas son content-addressed: cada rotación inserta un nuevo
        // hash, pero el FINAL coincide con el hash original).
        let mut model = modelo_minimo();
        // Empiezo con 2×3.
        model.lienzo = Lienzo::nuevo(2, 3);
        let buf = vec![
            10, 0, 0, 255,   20, 0, 0, 255,
            30, 0, 0, 255,   40, 0, 0, 255,
            50, 0, 0, 255,   60, 0, 0, 255,
        ];
        let h_inicial = model.almacen.insertar(buf);
        let cap = Capa::raster("c", h_inicial);
        let id = cap.id;
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        // CW: dims 2×3 → 3×2.
        rotar_lienzo(&mut model, true);
        assert_eq!((model.lienzo.width, model.lienzo.height), (3, 2));
        // CCW: vuelve a 2×3.
        rotar_lienzo(&mut model, false);
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 3));
        // El hash final debe igualar el inicial (content-addressing).
        let h_final = model.lienzo.capa(id).unwrap().contenido;
        assert_eq!(h_final, h_inicial);
    }

    #[test]
    fn rotar_lienzo_sin_capas_es_no_op() {
        let mut model = modelo_minimo();
        model.lienzo.capas.clear();
        let ok = rotar_lienzo(&mut model, true);
        assert!(!ok);
        assert!(model.estado.contains("nada que rotar"));
    }

    #[test]
    fn msg_rotar_lienzo_dispatcha_y_undo_restaura_dims() {
        let mut model = modelo_minimo();
        model.lienzo = Lienzo::nuevo(2, 3);
        let buf = buffer_relleno(2, 3, [50, 50, 50, 255]);
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("c", h);
        model.lienzo.apilar(cap);
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(
            model,
            Msg::RotarLienzo { cw: true },
            &Handle::for_test(),
        );
        assert_eq!((model.lienzo.width, model.lienzo.height), (3, 2));
        assert_eq!(model.historial.len(), hist_antes + 1);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 3));
    }

    // ---- Fase 31: auto-trim del lienzo --------------------------------------

    #[test]
    fn bbox_devuelve_none_si_todo_transparente() {
        // Buffer 3×3 todo a alfa=0.
        let buf = vec![0u8; 3 * 3 * 4];
        assert_eq!(bbox_no_transparente(&buf, 3, 3), None);
    }

    #[test]
    fn bbox_un_solo_pixel_devuelve_rect_de_un_pixel() {
        // 3×3, todo transparente excepto el píxel central (1, 1).
        let mut buf = vec![0u8; 3 * 3 * 4];
        let i = ((1 * 3 + 1) * 4) as usize;
        buf[i] = 100;
        buf[i + 1] = 200;
        buf[i + 2] = 50;
        buf[i + 3] = 255;
        let bb = bbox_no_transparente(&buf, 3, 3).unwrap();
        // Half-open: el píxel (1,1) da (1, 1, 2, 2).
        assert_eq!(bb, (1, 1, 2, 2));
    }

    #[test]
    fn bbox_full_alpha_cubre_el_lienzo_entero() {
        // Buffer 2×3 todo opaco.
        let buf = buffer_relleno(2, 3, [10, 20, 30, 255]);
        assert_eq!(bbox_no_transparente(&buf, 2, 3), Some((0, 0, 2, 3)));
    }

    #[test]
    fn bbox_ignora_pixeles_alfa_cero_aun_con_rgb_no_cero() {
        // Photoshop/PSD a veces deja "pixel data" con alfa=0 — no son
        // tinta visible. El bbox debe ignorarlos.
        let mut buf = vec![0u8; 4 * 4 * 4];
        // Toda la columna izquierda: RGB no-cero pero alfa=0.
        for y in 0..4 {
            let i = (y * 4 * 4) as usize;
            buf[i] = 200;
            buf[i + 3] = 0;
        }
        // Píxel (3, 2) opaco con alfa=255.
        let j = ((2 * 4 + 3) * 4) as usize;
        buf[j + 3] = 255;
        let bb = bbox_no_transparente(&buf, 4, 4).unwrap();
        assert_eq!(bb, (3, 2, 4, 3));
    }

    #[test]
    fn recortar_buffer_extrae_subrect_correcto() {
        // 4×3 con un patrón de gradiente lineal en R.
        let mut buf = Vec::with_capacity(4 * 3 * 4);
        for i in 0..(4 * 3) {
            buf.extend_from_slice(&[i as u8 * 20, 0, 0, 255]);
        }
        // Recorto el rect (1, 1, 4, 3) → 3×2 píxeles.
        // Esperado: filas 1 y 2, columnas 1, 2, 3 del src.
        let out = recortar_buffer(&buf, 4, 1, 1, 4, 3);
        assert_eq!(out.len(), 3 * 2 * 4);
        // Píxel (0, 0) del out = píxel (1, 1) del src = idx 5 lineal.
        // R = 5 * 20 = 100.
        assert_eq!(out[0], 100);
        // Píxel (2, 1) del out = píxel (3, 2) del src = idx 11. R = 220.
        let i = (1 * 3 + 2) * 4;
        assert_eq!(out[i], 220);
    }

    #[test]
    fn autotrim_no_op_si_lienzo_todo_opaco() {
        // El bbox cubre todo el lienzo → no-op + estado.
        let (mut model, _) = modelo_n_capas(&[[10, 20, 30, 255]]);
        // `modelo_n_capas` no recompone — forzamos para que `model.imagen`
        // exista y `recortar_lienzo_a_visible` no caiga en la rama de
        // "no hay composite".
        aplicar_y_recomponer(&mut model);
        let dims_antes = (model.lienzo.width, model.lienzo.height);
        let ok = recortar_lienzo_a_visible(&mut model);
        assert!(!ok);
        assert_eq!((model.lienzo.width, model.lienzo.height), dims_antes);
        assert!(model.estado.contains("ya está justo"));
    }

    #[test]
    fn autotrim_no_op_si_lienzo_todo_transparente() {
        // Capa única con alfa=0 → bbox = None → no-op con mensaje.
        let mut model = modelo_minimo();
        model.lienzo = Lienzo::nuevo(3, 3);
        let buf = buffer_relleno(3, 3, [100, 100, 100, 0]); // RGB pero alfa 0
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("trasparent", h);
        model.lienzo.apilar(cap);
        // Forzamos recompose para llenar model.imagen.
        aplicar_y_recomponer(&mut model);
        let ok = recortar_lienzo_a_visible(&mut model);
        assert!(!ok);
        assert!(model.estado.contains("vacío"));
    }

    #[test]
    fn autotrim_recorta_lienzo_a_la_region_opaca() {
        // Lienzo 4×4 todo transparente excepto un rect interior 2×2
        // (filas 1-2, cols 1-2). Tras autotrim el lienzo debería
        // reducirse a 2×2.
        let mut model = modelo_minimo();
        model.lienzo = Lienzo::nuevo(4, 4);
        // Buffer 4×4 con sólo (1..3, 1..3) opaco rojo.
        let mut buf = vec![0u8; 4 * 4 * 4];
        for y in 1..3 {
            for x in 1..3 {
                let i = (y * 4 + x) * 4;
                buf[i] = 200;
                buf[i + 3] = 255;
            }
        }
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("isla", h);
        let id = cap.id;
        model.lienzo.apilar(cap);
        model.seleccionada = Some(id);
        aplicar_y_recomponer(&mut model);
        let ok = recortar_lienzo_a_visible(&mut model);
        assert!(ok);
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 2));
        // El buffer recortado de la capa: todos los 4 píxeles son la isla
        // roja opaca.
        let nueva_h = model.lienzo.capa(id).unwrap().contenido;
        let buf_post = model.almacen.obtener(nueva_h).unwrap();
        assert_eq!(buf_post.len(), 2 * 2 * 4);
        for px in buf_post.chunks_exact(4) {
            assert_eq!(px, &[200, 0, 0, 255]);
        }
    }

    #[test]
    fn msg_autotrim_dispatcha_y_undo_restaura() {
        // El flujo entero por update: autotrim baja dims, Undo las
        // restaura.
        let mut model = modelo_minimo();
        model.lienzo = Lienzo::nuevo(4, 4);
        let mut buf = vec![0u8; 4 * 4 * 4];
        for y in 1..3 {
            for x in 1..3 {
                let i = (y * 4 + x) * 4;
                buf[i + 3] = 255;
            }
        }
        let h = model.almacen.insertar(buf);
        let cap = Capa::raster("isla", h);
        model.lienzo.apilar(cap);
        aplicar_y_recomponer(&mut model);
        model.historial = vec![model.lienzo.clone()];
        model.cursor_historial = 0;
        let hist_antes = model.historial.len();
        model = <Tullpu as App>::update(model, Msg::AutotrimLienzo, &Handle::for_test());
        assert_eq!((model.lienzo.width, model.lienzo.height), (2, 2));
        assert_eq!(model.historial.len(), hist_antes + 1);
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!((model.lienzo.width, model.lienzo.height), (4, 4));
    }

    #[test]
    fn confirmar_renombrar_con_nuevo_nombre_si_genera_snapshot() {
        let mut model = modelo_minimo();
        let id = model.seleccionada.unwrap();
        let len_inicial = model.historial.len();
        model = <Tullpu as App>::update(
            model,
            Msg::IniciarRenombrar(id),
            &Handle::for_test(),
        );
        if let Some((_, input)) = model.renombrando.as_mut() {
            input.set_text("renombrado");
        }
        model = <Tullpu as App>::update(model, Msg::ConfirmarRenombrar, &Handle::for_test());
        assert_eq!(model.historial.len(), len_inicial + 1);
        // Undo restaura el nombre original ("c").
        model = <Tullpu as App>::update(model, Msg::Undo, &Handle::for_test());
        assert_eq!(model.lienzo.capa(id).unwrap().nombre, "c");
    }
}
