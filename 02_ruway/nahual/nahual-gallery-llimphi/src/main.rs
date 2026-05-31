//! `nahual-gallery-llimphi` — galería de miniaturas tipo gThumb / FastStone.
//!
//! Cose las dos piezas del pipeline de galería:
//! - [`llimphi_widget_grid`] → virtualización 2D (sólo monta la ventana
//!   visible, aunque la carpeta tenga miles de imágenes).
//! - [`nahual_thumb_core`] → genera las miniaturas (decode + downscale),
//!   las cachea en RAM y planifica la cola priorizada al viewport.
//!
//! La concurrencia la pone `Handle::spawn`: por cada path que el
//! planificador entrega, lanzamos un thread que decodifica y reentra con
//! `Msg::ThumbListo`. Mientras tanto la celda muestra un placeholder.
//!
//! Navegación: la grilla mezcla subcarpetas (ícono 📁, primero) e
//! imágenes. Un clic en carpeta entra; en imagen selecciona (⏎/espacio
//! abre el preview). `⌫` sube al padre y el breadcrumb salta a cualquier
//! ancestro. `o` cicla el orden de las imágenes (nombre/tamaño/fecha).
//!
//! Uso: `cargo run -p nahual-gallery-llimphi --release -- <carpeta>`
//! (sin argumento usa el directorio actual).
//!
//! Limitación MVP: el tamaño del viewport se asume fijo (= `initial_size`)
//! porque el trait `App` de Llimphi todavía no expone un hook de resize —
//! mismo atajo que `nahual-file-explorer`. Al achicar/agrandar la ventana
//! las columnas no se recalculan hasta que eso exista.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_widget_breadcrumb::{breadcrumb_view, BreadcrumbPalette};
use llimphi_widget_grid::{grid_view, ventana_visible, GridCell, GridMetrics, GridPalette};
use nahual_image_viewer_llimphi::{
    image_viewer_view, load_image, ImagePreviewState, ImageViewerPalette,
};
use nahual_thumb_core::{
    generar_thumb_de_archivo, obtener_o_generar, CacheDisco, Planificador, ThumbRgba,
};

/// Lado máximo de la miniatura generada (px). Un poco mayor que el tile
/// para que se vea nítida; el grid la reduce al pintar.
const THUMB_LADO: u32 = 224;
/// Cuántas decodificaciones concurrentes permitir (~núcleos).
const MAX_EN_VUELO: usize = 6;
/// Alto reservado para el header en px (descontado del viewport útil).
const HEADER_H: f32 = 44.0;
/// Alto de la barra de breadcrumb (ruta navegable).
const BREADCRUMB_H: f32 = 30.0;

/// Una entrada de la carpeta actual: subcarpeta navegable o archivo de
/// imagen con miniatura. Las carpetas se listan primero.
#[derive(Clone)]
enum Entrada {
    Carpeta(PathBuf),
    Imagen(PathBuf),
}

impl Entrada {
    fn path(&self) -> &Path {
        match self {
            Entrada::Carpeta(p) | Entrada::Imagen(p) => p,
        }
    }
    fn es_carpeta(&self) -> bool {
        matches!(self, Entrada::Carpeta(_))
    }
}

/// Extensiones que tratamos como imagen (alineadas con las features del
/// crate `image` en el workspace: png/jpeg/webp).
const EXTS: &[&str] = &["png", "jpg", "jpeg", "webp"];
/// Tope para abrir una imagen a tamaño completo en el preview (64 MB de
/// archivo — un PNG/JPEG así decodifica a cientos de MB, pero es el límite
/// del visor existente).
const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;

/// Criterio de orden de la grilla.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Orden {
    Nombre,
    Tamano,
    Fecha,
}

impl Orden {
    fn siguiente(self) -> Self {
        match self {
            Orden::Nombre => Orden::Tamano,
            Orden::Tamano => Orden::Fecha,
            Orden::Fecha => Orden::Nombre,
        }
    }
    fn etiqueta(self) -> &'static str {
        match self {
            Orden::Nombre => "nombre",
            Orden::Tamano => "tamaño",
            Orden::Fecha => "fecha",
        }
    }
}

#[derive(Clone)]
enum Msg {
    /// Desplaza la grilla por N filas (positivo = hacia abajo).
    Scroll(i32),
    /// Marca una celda como seleccionada.
    Seleccionar(usize),
    /// Miniatura lista (llega desde el thread de decodificación).
    ThumbListo(PathBuf, ThumbRgba),
    /// Falló la generación de una miniatura.
    ThumbFallo(PathBuf, String),
    /// Tecla (navegación con flechas / página).
    Tecla(KeyEvent),
    /// Abre la imagen `i` a tamaño completo (overlay de preview).
    AbrirPreview(usize),
    /// Imagen del preview cargada (llega desde el thread de carga).
    PreviewListo(PathBuf, ImagePreviewState),
    /// Mueve el preview a la imagen vecina (±1) y la carga.
    PreviewVecino(i32),
    /// Cierra el overlay de preview.
    CerrarPreview,
    /// Cambia el criterio de orden de la grilla.
    CiclarOrden,
    /// Activa la entrada `i`: entra a la carpeta o abre el preview.
    Activar(usize),
    /// Sube a la carpeta padre.
    Subir,
    /// Navega al segmento `i` del breadcrumb (raíz → hoja).
    NavegarSegmento(usize),
}

struct Model {
    dir: PathBuf,
    entries: Vec<Entrada>,
    scroll_fila: usize,
    seleccionado: Option<usize>,
    /// Miniaturas ya listas para pintar (peniko). Es el cache RAM del lado
    /// de la app; el cache atado a mtime de `nahual-thumb-core` se enchufa
    /// en el paso 3 (disco).
    thumbs: HashMap<PathBuf, Image>,
    /// Paths cuya generación falló — pintan un ⚠ en vez de reintentar.
    fallidos: HashSet<PathBuf>,
    plan: Planificador,
    /// Cache en disco de miniaturas (reabrir sin re-decodificar). `None`
    /// si no se pudo crear la carpeta — se cae a generar en RAM siempre.
    cache_disco: Option<CacheDisco>,
    metrics: GridMetrics,
    /// Viewport útil asumido (de `initial_size`, menos el header).
    vw: f32,
    vh: f32,
    /// Imagen abierta a tamaño completo (overlay). `None` = grilla normal.
    preview: Option<(PathBuf, ImagePreviewState)>,
    orden: Orden,
    estado: String,
    theme: Theme,
}

struct Gallery;

impl App for Gallery {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "nahual · galería"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 760)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let dir = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let orden = Orden::Nombre;
        let entries = listar(&dir, orden);
        let (w, h) = Self::initial_size();
        let metrics = GridMetrics {
            tile_w: 140.0,
            tile_h: 162.0, // 140 imagen + ~22 label
            gap: 10.0,
            pad: 12.0,
        };
        let estado = format!("{} ítems", entries.len());
        let mut m = Model {
            dir,
            entries,
            scroll_fila: 0,
            seleccionado: None,
            thumbs: HashMap::new(),
            fallidos: HashSet::new(),
            plan: Planificador::nuevo(MAX_EN_VUELO),
            cache_disco: CacheDisco::por_defecto().ok(),
            metrics,
            vw: w as f32,
            vh: h as f32 - HEADER_H - BREADCRUMB_H,
            preview: None,
            orden,
            estado,
            theme: Theme::dark(),
        };
        bombear(&mut m, handle);
        m
    }

    fn on_wheel(_: &Model, delta: WheelDelta, _: (f32, f32), _: Modifiers) -> Option<Msg> {
        // delta.y positivo = rueda hacia arriba ⇒ scroll hacia arriba
        // (filas menores). Convertimos a pasos enteros de fila.
        let pasos = -delta.y.round() as i32;
        if pasos == 0 {
            None
        } else {
            Some(Msg::Scroll(pasos))
        }
    }

    fn on_key(model: &Model, e: &KeyEvent) -> Option<Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Con el preview abierto, las teclas controlan el preview, no la
        // grilla: Esc cierra, ←/→ saltan a la vecina.
        if model.preview.is_some() {
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CerrarPreview),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::PreviewVecino(1)),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::PreviewVecino(-1)),
                _ => None,
            };
        }
        // Enter / Espacio activan la entrada seleccionada (entrar a la
        // carpeta o abrir el preview); Backspace sube a la carpeta padre.
        match &e.key {
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space) => {
                model.seleccionado.map(Msg::Activar)
            }
            Key::Named(NamedKey::Backspace) => Some(Msg::Subir),
            Key::Character(c) if c == "o" => Some(Msg::CiclarOrden),
            _ => Some(Msg::Tecla(e.clone())),
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Scroll(pasos) => {
                let nueva = (m.scroll_fila as i32 + pasos).max(0) as usize;
                m.scroll_fila = nueva;
                bombear(&mut m, handle);
            }
            Msg::Seleccionar(i) => {
                if i < m.entries.len() {
                    m.seleccionado = Some(i);
                    m.estado = nombre(m.entries[i].path());
                }
            }
            Msg::ThumbListo(path, t) => {
                m.plan.completar(&path);
                let img = Image::new(Blob::from(t.rgba), ImageFormat::Rgba8, t.w, t.h);
                m.thumbs.insert(path, img);
                // Encadenar: liberado un cupo, pedir el próximo lote.
                bombear(&mut m, handle);
            }
            Msg::ThumbFallo(path, e) => {
                m.plan.completar(&path);
                m.estado = format!("falló {}: {e}", nombre(&path));
                m.fallidos.insert(path);
                bombear(&mut m, handle);
            }
            Msg::Tecla(ev) => {
                manejar_tecla(&mut m, &ev, handle);
            }
            Msg::AbrirPreview(i) => {
                abrir_preview(&mut m, i, handle);
            }
            Msg::PreviewListo(path, st) => {
                // Sólo aplicar si seguimos viendo la misma imagen (evita que
                // una carga lenta pise a otra más nueva).
                if let Some((actual, _)) = &m.preview {
                    if actual == &path {
                        m.estado = nombre(&path);
                        m.preview = Some((path, st));
                    }
                }
            }
            Msg::PreviewVecino(d) => {
                // Salta a la imagen vecina, ignorando carpetas.
                if let Some((actual, _)) = &m.preview {
                    if let Some(cur) = m.entries.iter().position(|e| e.path() == actual) {
                        let mut i = cur as i32 + d;
                        while i >= 0 && (i as usize) < m.entries.len() {
                            if !m.entries[i as usize].es_carpeta() {
                                let idx = i as usize;
                                m.seleccionado = Some(idx);
                                abrir_preview(&mut m, idx, handle);
                                break;
                            }
                            i += d;
                        }
                    }
                }
            }
            Msg::CerrarPreview => {
                m.preview = None;
            }
            Msg::CiclarOrden => {
                // Conservar la selección por path al re-listar (que reordena).
                let sel_path = m.seleccionado.and_then(|i| m.entries.get(i).map(|e| e.path().to_path_buf()));
                m.orden = m.orden.siguiente();
                m.entries = listar(&m.dir, m.orden);
                m.seleccionado =
                    sel_path.and_then(|p| m.entries.iter().position(|e| e.path() == p));
                m.estado = format!("orden: {}", m.orden.etiqueta());
                bombear(&mut m, handle);
            }
            Msg::Activar(i) => {
                if let Some(e) = m.entries.get(i) {
                    if e.es_carpeta() {
                        let dir = e.path().to_path_buf();
                        navegar_a(&mut m, dir, handle);
                    } else {
                        abrir_preview(&mut m, i, handle);
                    }
                }
            }
            Msg::Subir => {
                if let Some(padre) = m.dir.parent().map(|p| p.to_path_buf()) {
                    navegar_a(&mut m, padre, handle);
                }
            }
            Msg::NavegarSegmento(i) => {
                if let Some(dir) = ancestros(&m.dir).get(i).cloned() {
                    navegar_a(&mut m, dir, handle);
                }
            }
        }
        m
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        let (path, st) = model.preview.as_ref()?;
        let pal = ImageViewerPalette::from_theme(&model.theme);
        let viewer = image_viewer_view::<Msg>(st, Some(path), &pal);
        // Scrim a pantalla completa: cubre la grilla y, al clickear (fuera o
        // sobre la imagen), cierra el preview. Esc y ←/→ los maneja on_key.
        Some(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                ..Default::default()
            })
            .fill(model.theme.bg_app)
            .on_click(Msg::CerrarPreview)
            .children(vec![viewer]),
        )
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let v = ventana_visible(
            model.entries.len(),
            model.vw,
            model.vh,
            model.scroll_fila,
            &model.metrics,
        );

        let header = encabezado(model, &v);
        let ruta = barra_ruta(model);

        let cuerpo: View<Msg> = if model.entries.is_empty() {
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .fill(theme.bg_panel)
            .text(
                format!("sin imágenes ni subcarpetas en {}", model.dir.display()),
                14.0,
                theme.fg_muted,
            )
        } else {
            let cells: Vec<GridCell<Msg>> = (v.first..v.first + v.count)
                .map(|i| {
                    let e = &model.entries[i];
                    GridCell {
                        content: celda_contenido(model, e),
                        label: Some(nombre(e.path())),
                        selected: model.seleccionado == Some(i),
                        // Un clic en carpeta entra; en imagen selecciona
                        // (⏎/espacio la abre en preview).
                        on_click: if e.es_carpeta() {
                            Msg::Activar(i)
                        } else {
                            Msg::Seleccionar(i)
                        },
                    }
                })
                .collect();
            let mostrados = v.first + v.count;
            grid_view(llimphi_widget_grid::GridSpec {
                cells,
                cols: v.cols,
                metrics: model.metrics,
                caption: None,
                truncated_hint: (mostrados < model.entries.len())
                    .then(|| format!("… y {} más abajo", model.entries.len() - mostrados)),
                palette: GridPalette::from_theme(&theme),
            })
        };

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, ruta, cuerpo])
    }
}

/// Cuerpo de una celda: un ícono 📁 para carpetas; para imágenes la
/// miniatura si está lista, un ⚠ si falló, o un placeholder mientras se
/// genera.
fn celda_contenido(model: &Model, e: &Entrada) -> View<Msg> {
    let theme = &model.theme;
    let lado = model.metrics.tile_w - 8.0;
    let base = || Style {
        size: Size {
            width: length(lado),
            height: length(lado),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    };
    if e.es_carpeta() {
        return View::new(base())
            .fill(theme.bg_panel_alt)
            .text("📁".to_string(), 44.0, theme.fg_text);
    }
    let path = e.path();
    if let Some(img) = model.thumbs.get(path) {
        View::new(base()).image(img.clone())
    } else if model.fallidos.contains(path) {
        View::new(base())
            .fill(theme.bg_panel_alt)
            .text("⚠".to_string(), 20.0, theme.fg_muted)
    } else {
        // Placeholder: rectángulo tenue mientras decodifica (MVP — sin
        // animación; el widget-skeleton se puede enchufar acá luego).
        View::new(base()).fill(theme.bg_panel_alt)
    }
}

/// Breadcrumb de la ruta actual: cada segmento (raíz → hoja) es clicable y
/// navega a ese ancestro.
fn barra_ruta(model: &Model) -> View<Msg> {
    let anc = ancestros(&model.dir);
    let segs: Vec<String> = anc
        .iter()
        .map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string_lossy().into_owned())
        })
        .collect();
    let refs: Vec<&str> = segs.iter().map(|s| s.as_str()).collect();
    let pal = BreadcrumbPalette::from_theme(&model.theme);
    let bc = breadcrumb_view::<Msg, _>(&refs, Msg::NavegarSegmento, &pal);
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(BREADCRUMB_H),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(model.theme.bg_panel)
    .children(vec![bc])
}

/// Ancestros de un directorio en orden raíz → hoja (incluye el propio dir).
fn ancestros(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = dir.ancestors().map(|p| p.to_path_buf()).collect();
    v.reverse();
    v
}

/// Cambia la carpeta actual: re-lista, resetea scroll/selección/preview y
/// arranca la generación de miniaturas de la nueva ventana.
fn navegar_a(m: &mut Model, dir: PathBuf, handle: &Handle<Msg>) {
    m.dir = dir;
    m.entries = listar(&m.dir, m.orden);
    m.scroll_fila = 0;
    m.seleccionado = None;
    m.preview = None;
    m.estado = format!("{} ítems", m.entries.len());
    bombear(m, handle);
}

/// Barra superior: carpeta, conteo, fila actual y estado.
fn encabezado(model: &Model, v: &llimphi_widget_grid::VisibleWindow) -> View<Msg> {
    let theme = &model.theme;
    let texto = format!(
        "{} ítems  ·  fila {}/{}  ·  orden: {} (o)  ·  ⏎/espacio: abrir  ·  ⌫ subir  ·  {}",
        model.entries.len(),
        v.first_row + 1,
        v.total_rows.max(1),
        model.orden.etiqueta(),
        model.estado,
    );
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_H),
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
    .fill(theme.bg_panel_alt)
    .text_aligned(texto, 12.0, theme.fg_text, Alignment::Start)
}

/// Maneja navegación por teclado: flechas mueven la selección (y arrastran
/// el scroll para mantenerla visible); Re Pág / Av Pág saltan una pantalla.
fn manejar_tecla(m: &mut Model, ev: &KeyEvent, handle: &Handle<Msg>) {
    if m.entries.is_empty() {
        return;
    }
    let v = ventana_visible(m.entries.len(), m.vw, m.vh, m.scroll_fila, &m.metrics);
    let cols = v.cols.max(1);
    let filas_pantalla = v.filas_visibles.saturating_sub(1).max(1);
    let ultimo = m.entries.len() - 1;
    let cur = m.seleccionado.unwrap_or(0);

    let nuevo: Option<usize> = match &ev.key {
        Key::Named(NamedKey::ArrowRight) => Some((cur + 1).min(ultimo)),
        Key::Named(NamedKey::ArrowLeft) => Some(cur.saturating_sub(1)),
        Key::Named(NamedKey::ArrowDown) => Some((cur + cols).min(ultimo)),
        Key::Named(NamedKey::ArrowUp) => Some(cur.saturating_sub(cols)),
        Key::Named(NamedKey::Home) => Some(0),
        Key::Named(NamedKey::End) => Some(ultimo),
        Key::Named(NamedKey::PageDown) => {
            m.scroll_fila = (m.scroll_fila + filas_pantalla).min(v.total_rows.saturating_sub(1));
            bombear(m, handle);
            return;
        }
        Key::Named(NamedKey::PageUp) => {
            m.scroll_fila = m.scroll_fila.saturating_sub(filas_pantalla);
            bombear(m, handle);
            return;
        }
        _ => None,
    };

    if let Some(sel) = nuevo {
        m.seleccionado = Some(sel);
        m.estado = nombre(m.entries[sel].path());
        asegurar_visible(m, sel, cols, filas_pantalla);
        bombear(m, handle);
    }
}

/// Ajusta `scroll_fila` para que el índice `sel` quede dentro de la
/// ventana visible (lo arrastra mínimamente arriba o abajo).
fn asegurar_visible(m: &mut Model, sel: usize, cols: usize, filas_pantalla: usize) {
    let fila_sel = sel / cols;
    if fila_sel < m.scroll_fila {
        m.scroll_fila = fila_sel;
    } else if fila_sel >= m.scroll_fila + filas_pantalla {
        m.scroll_fila = fila_sel + 1 - filas_pantalla;
    }
}

/// Abre el preview de la imagen `i`: marca el placeholder de carga y lanza
/// la decodificación full-res en un thread, que reentra con `PreviewListo`.
fn abrir_preview(m: &mut Model, i: usize, handle: &Handle<Msg>) {
    let Some(path) = m.entries.get(i).map(|e| e.path().to_path_buf()) else {
        return;
    };
    m.seleccionado = Some(i);
    m.estado = format!("cargando {}…", nombre(&path));
    // Placeholder mientras carga (el viewer pinta "—").
    m.preview = Some((path.clone(), ImagePreviewState::Empty));
    let p = path.clone();
    handle.spawn(move || {
        let st = load_image(&p, MAX_PREVIEW_BYTES);
        Msg::PreviewListo(p, st)
    });
}

/// Ordena una lista de paths según el criterio. `Tamano`/`Fecha` consultan
/// `metadata` (un `stat` por archivo — se paga una vez por reorden).
fn ordenar_paths(paths: &mut [PathBuf], orden: Orden) {
    match orden {
        Orden::Nombre => paths.sort(),
        Orden::Tamano => {
            paths.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        }
        Orden::Fecha => {
            paths.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        }
    }
}

/// Recalcula la ventana visible, encola los thumbs que falten, olvida los
/// que se fueron de pantalla y lanza el próximo lote de generación.
fn bombear(m: &mut Model, handle: &Handle<Msg>) {
    let v = ventana_visible(m.entries.len(), m.vw, m.vh, m.scroll_fila, &m.metrics);
    // Persistir el clamp (no scrollear más allá del fondo).
    m.scroll_fila = v.first_row;

    let mut visibles: HashSet<PathBuf> = HashSet::with_capacity(v.count);
    for (orden, i) in (v.first..v.first + v.count).enumerate() {
        // Las carpetas no generan miniatura (pintan un ícono).
        if m.entries[i].es_carpeta() {
            continue;
        }
        let path = m.entries[i].path().to_path_buf();
        visibles.insert(path.clone());
        if !m.thumbs.contains_key(&path) && !m.fallidos.contains(&path) {
            // Prioridad = orden de aparición en la ventana (lo de arriba
            // primero).
            m.plan.solicitar(path, orden as u64);
        }
    }
    m.plan.olvidar_excepto(&visibles);

    for path in m.plan.proximos() {
        let p = path.clone();
        let cache = m.cache_disco.clone();
        handle.spawn(move || {
            // Pasa por el cache en disco si lo hay: hit ⇒ sin decodificar;
            // miss ⇒ genera y puebla el disco para la próxima sesión.
            let res = match &cache {
                Some(c) => obtener_o_generar(c, &p, THUMB_LADO),
                None => generar_thumb_de_archivo(&p, THUMB_LADO),
            };
            match res {
                Ok(t) => Msg::ThumbListo(p, t),
                Err(e) => Msg::ThumbFallo(p, e.to_string()),
            }
        });
    }
}

/// Lista el contenido navegable de un directorio: subcarpetas (ordenadas
/// por nombre) primero, luego los archivos de imagen (ordenados por
/// `orden`). Lo demás se ignora.
fn listar(dir: &Path, orden: Orden) -> Vec<Entrada> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };
    let mut carpetas: Vec<PathBuf> = Vec::new();
    let mut imagenes: Vec<PathBuf> = Vec::new();
    for entrada in rd.flatten() {
        let p = entrada.path();
        if p.is_dir() {
            carpetas.push(p);
        } else if es_imagen(&p) {
            imagenes.push(p);
        }
    }
    carpetas.sort();
    ordenar_paths(&mut imagenes, orden);
    carpetas
        .into_iter()
        .map(Entrada::Carpeta)
        .chain(imagenes.into_iter().map(Entrada::Imagen))
        .collect()
}

fn es_imagen(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn nombre(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .to_string()
}

fn main() {
    llimphi_ui::run::<Gallery>();
}
