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

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use llimphi_theme::{motion, Theme};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::Affine;
use llimphi_ui::llimphi_raster::peniko::{
    Blob, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_icons::Icon;
use llimphi_widget_empty::{empty_view, EmptyPalette};
use llimphi_widget_skeleton::{skeleton_view, SkeletonPalette};
use llimphi_widget_breadcrumb::{breadcrumb_view, BreadcrumbPalette};
use llimphi_widget_grid::{grid_view, ventana_visible, GridCell, GridMetrics, GridPalette};
use nahual_image_viewer_llimphi::{
    image_viewer_view, load_image, ImagePreviewState, ImageViewerPalette,
};
use nahual_thumb_core::{
    generar_thumb_de_archivo, obtener_o_generar, CacheDisco, Planificador, ThumbRgba,
};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{
    menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H,
};
use app_bus::{AppMenu, Menu, MenuItem};

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
/// imagen con miniatura (y su tamaño en disco, para el badge). Las carpetas
/// se listan primero.
#[derive(Clone)]
enum Entrada {
    Carpeta(PathBuf),
    Imagen { path: PathBuf, size: u64 },
}

impl Entrada {
    fn path(&self) -> &Path {
        match self {
            Entrada::Carpeta(p) => p,
            Entrada::Imagen { path, .. } => path,
        }
    }
    fn es_carpeta(&self) -> bool {
        matches!(self, Entrada::Carpeta(_))
    }
}

/// Formatea un tamaño en bytes de forma humana (B/KB/MB/GB).
fn humano(n: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < 3 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// Hash estable de una cadena → `key` para animaciones implícitas (la misma
/// ruta/escena produce siempre la misma key entre rebuilds, así el fade-in
/// corre sólo la primera vez que el nodo aparece).
fn key_of(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Intervalo del tick de animación: fuerza repaint para que el shimmer del
/// skeleton corra mientras haya miniaturas decodificándose.
const TICK_MS: u64 = 50;

/// Extensiones que tratamos como imagen (alineadas con las features del
/// crate `image` en el workspace: png/jpeg/webp).
const EXTS: &[&str] = &["png", "jpg", "jpeg", "webp"];
/// Tope para abrir una imagen a tamaño completo en el preview (64 MB de
/// archivo — un PNG/JPEG así decodifica a cientos de MB, pero es el límite
/// del visor existente).
const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;
/// Tope de miniaturas en RAM. Al pasarlo, se descartan las más lejanas a la
/// ventana visible (con margen) — acota la memoria en carpetas enormes.
const CAP_THUMBS: usize = 800;
/// Rango de tile permitido al hacer zoom con +/−.
const TILE_MIN: f32 = 80.0;
const TILE_MAX: f32 = 280.0;
/// Alto del label bajo la miniatura (nombre + tamaño).
const LABEL_H: f32 = 20.0;
/// Intervalo del slideshow.
const SLIDE_SECS: u64 = 3;

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
    fn etiqueta(self) -> String {
        rimay_localize::t(match self {
            Orden::Nombre => "nahual-gallery-sort-name",
            Orden::Tamano => "nahual-gallery-sort-size",
            Orden::Fecha => "nahual-gallery-sort-date",
        })
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
    /// Ajusta el tamaño del tile (zoom de grilla) por el delta dado en px.
    Zoom(f32),
    /// Alterna el slideshow (sólo con el preview abierto).
    ToggleSlideshow,
    /// Tick del slideshow: avanza a la imagen siguiente.
    SlideAvanzar,
    /// Abre/cierra un menú raíz de la barra (índice del menú).
    MenuOpen(Option<usize>),
    /// Comando del menú principal (command id → acción real).
    MenuCommand(String),
    /// Cierra todo overlay de menú (raíz y contextual).
    CloseMenus,
    /// Abre el menú contextual en coords de ventana, sobre la selección.
    ContextMenuOpen(f32, f32),
    /// Cicla la paleta de tema.
    CiclarTema,
    /// Tick de animación: fuerza repaint para el shimmer del skeleton mientras
    /// haya miniaturas en vuelo. Se auto-rearma sólo si siguen faltando.
    Tick,
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
    /// Slideshow activo (avanza solo en el preview).
    slideshow: bool,
    orden: Orden,
    estado: String,
    theme: Theme,
    /// Menú raíz abierto en la barra (índice), si lo hay.
    menu_open: Option<usize>,
    /// Menú contextual abierto en (x, y) de ventana, si lo hay.
    context_menu: Option<(f32, f32)>,
    /// Hay una cadena de `Msg::Tick` en vuelo (evita rearmar dos).
    ticking: bool,
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
        rimay_localize::init();
        let dir = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let orden = Orden::Nombre;
        let entries = listar(&dir, orden);
        let (w, h) = Self::initial_size();
        let metrics = GridMetrics {
            tile_w: 140.0,
            tile_h: 140.0 + LABEL_H,
            gap: 10.0,
            pad: 12.0,
        };
        let estado = rimay_localize::t_args("nahual-gallery-items", &[("n", entries.len().to_string().into())]);
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
            vh: h as f32 - MENU_H - HEADER_H - BREADCRUMB_H,
            preview: None,
            slideshow: false,
            orden,
            estado,
            theme: Theme::dark(),
            menu_open: None,
            context_menu: None,
            ticking: false,
        };
        bombear(&mut m, handle);
        ensure_tick(&mut m, handle);
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
                Key::Character(c) if c == "s" => Some(Msg::ToggleSlideshow),
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
            // Zoom de grilla: + / = agrandan, - achica.
            Key::Character(c) if c == "+" || c == "=" => Some(Msg::Zoom(20.0)),
            Key::Character(c) if c == "-" => Some(Msg::Zoom(-20.0)),
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
                let img = Image::new(ImageData {
                    data: Blob::from(t.rgba),
                    format: ImageFormat::Rgba8,
                    alpha_type: ImageAlphaType::Alpha,
                    width: t.w,
                    height: t.h,
                });
                m.thumbs.insert(path, img);
                // Encadenar: liberado un cupo, pedir el próximo lote.
                bombear(&mut m, handle);
            }
            Msg::ThumbFallo(path, e) => {
                m.plan.completar(&path);
                m.estado = rimay_localize::t_args(
                    "nahual-gallery-thumb-fail",
                    &[("name", nombre(&path).into()), ("err", e.to_string().into())],
                );
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
                avanzar_imagen(&mut m, d, false, handle);
            }
            Msg::CerrarPreview => {
                m.preview = None;
                m.slideshow = false;
            }
            Msg::Zoom(d) => {
                let w = (m.metrics.tile_w + d).clamp(TILE_MIN, TILE_MAX);
                m.metrics.tile_w = w;
                m.metrics.tile_h = w + LABEL_H;
                m.estado = format!("tile {:.0}px", w);
                bombear(&mut m, handle);
            }
            Msg::ToggleSlideshow => {
                if m.preview.is_some() {
                    m.slideshow = !m.slideshow;
                    m.estado = if m.slideshow {
                        "slideshow ▶".into()
                    } else {
                        "slideshow ⏸".into()
                    };
                    if m.slideshow {
                        programar_slide(handle);
                    }
                }
            }
            Msg::SlideAvanzar => {
                // Sólo avanza si el slideshow sigue activo y hay preview.
                if m.slideshow && m.preview.is_some() {
                    avanzar_imagen(&mut m, 1, true, handle);
                    programar_slide(handle);
                }
            }
            Msg::CiclarOrden => {
                // Conservar la selección por path al re-listar (que reordena).
                let sel_path = m.seleccionado.and_then(|i| m.entries.get(i).map(|e| e.path().to_path_buf()));
                m.orden = m.orden.siguiente();
                m.entries = listar(&m.dir, m.orden);
                m.seleccionado =
                    sel_path.and_then(|p| m.entries.iter().position(|e| e.path() == p));
                m.estado = rimay_localize::t_args("nahual-gallery-order", &[("order", m.orden.etiqueta().into())]);
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
            Msg::MenuOpen(which) => {
                m.menu_open = which;
                m.context_menu = None;
            }
            Msg::MenuCommand(cmd) => {
                m.menu_open = None;
                return handle_menu_command(m, &cmd, handle);
            }
            Msg::CloseMenus => {
                m.menu_open = None;
                m.context_menu = None;
            }
            Msg::ContextMenuOpen(x, y) => {
                m.menu_open = None;
                m.context_menu = Some((x, y));
            }
            Msg::CiclarTema => {
                m.theme = Theme::next_after(m.theme.name);
            }
            Msg::Tick => {
                // El thread durmió TICK_MS; sólo rearmamos si siguen faltando
                // miniaturas (lo hace `ensure_tick` más abajo).
                m.ticking = false;
            }
        }
        // Si quedaron miniaturas decodificándose, mantené el shimmer animado.
        ensure_tick(&mut m, handle);
        m
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // El preview a pantalla completa manda mientras esté abierto.
        if let Some((path, st)) = model.preview.as_ref() {
            let pal = ImageViewerPalette::from_theme(&model.theme);
            let viewer = image_viewer_view::<Msg>(st, Some(path), &pal);
            // Scrim a pantalla completa: cubre la grilla y, al clickear (fuera o
            // sobre la imagen), cierra el preview. Esc y ←/→ los maneja on_key.
            return Some(
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
            );
        }

        // Menú contextual sobre la entrada seleccionada: prioridad sobre el
        // dropdown del menú principal.
        if let Some((x, y)) = model.context_menu {
            let sel = model.seleccionado.and_then(|i| model.entries.get(i));
            let header = sel
                .map(|e| nombre(e.path()))
                .unwrap_or_else(|| rimay_localize::t("nahual-gallery-header-default"));
            let es_carpeta = sel.map(|e| e.es_carpeta()).unwrap_or(false);
            // Acciones reales según la entrada: carpeta ⇒ entrar; imagen ⇒
            // abrir preview. Más reset de zoom / ciclar orden, siempre útiles.
            let zoom_reset = rimay_localize::t("nahual-gallery-zoom-reset");
            let cycle_order = rimay_localize::t("nahual-gallery-cycle-order");
            let items = if es_carpeta {
                vec![
                    ContextMenuItem::action(rimay_localize::t("nahual-gallery-enter-folder")),
                    ContextMenuItem::action(zoom_reset.clone()),
                    ContextMenuItem::action(cycle_order.clone()),
                ]
            } else if sel.is_some() {
                vec![
                    ContextMenuItem::action(rimay_localize::t("nahual-gallery-open-image")),
                    ContextMenuItem::action(zoom_reset.clone()),
                    ContextMenuItem::action(cycle_order.clone()),
                ]
            } else {
                vec![
                    ContextMenuItem::action(zoom_reset),
                    ContextMenuItem::action(cycle_order),
                ]
            };
            let idx = model.seleccionado;
            let tile_w = model.metrics.tile_w;
            let tiene_sel = sel.is_some();
            let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> =
                Arc::new(move |i: usize| {
                    if tiene_sel {
                        match i {
                            0 => idx.map(Msg::Activar).unwrap_or(Msg::CloseMenus),
                            1 => Msg::Zoom(140.0 - tile_w),
                            _ => Msg::CiclarOrden,
                        }
                    } else {
                        match i {
                            0 => Msg::Zoom(140.0 - tile_w),
                            _ => Msg::CiclarOrden,
                        }
                    }
                });
            return Some(context_menu_view(ContextMenuSpec {
                anchor: (x, y),
                viewport: viewport_of(model),
                header: Some(header),
                items,
                active: usize::MAX,
                on_pick,
                on_dismiss: Msg::CloseMenus,
                palette: ContextMenuPalette::from_theme(&model.theme),
            }));
        }

        // Si no, el dropdown del menú principal.
        let menu = app_menu();
        menubar_overlay(&menubar_spec(&menu, model, &model.theme))
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

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
        let header = encabezado(model, &v);
        let ruta = barra_ruta(model);

        let cuerpo: View<Msg> = if model.entries.is_empty() {
            // Carpeta vacía: empty-state con orientación en vez de un hueco.
            let pal = EmptyPalette::from_theme(&theme);
            let desc = rimay_localize::t_args(
                "nahual-gallery-empty-desc",
                &[("path", model.dir.display().to_string().into())],
            );
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                ..Default::default()
            })
            .fill(theme.bg_panel)
            .children(vec![empty_view(Icon::Image, rimay_localize::t("nahual-fe-empty"), Some(&desc), &pal)])
        } else {
            let cells: Vec<GridCell<Msg>> = (v.first..v.first + v.count)
                .map(|i| {
                    let e = &model.entries[i];
                    let label = match e {
                        Entrada::Imagen { path, size } => {
                            format!("{}  ·  {}", nombre(path), humano(*size))
                        }
                        Entrada::Carpeta(p) => nombre(p),
                    };
                    GridCell {
                        content: celda_contenido(model, e),
                        label: Some(label),
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
                truncated_hint: (mostrados < model.entries.len()).then(|| {
                    rimay_localize::t_args(
                        "nahual-gallery-more",
                        &[("n", (model.entries.len() - mostrados).to_string().into())],
                    )
                }),
                palette: GridPalette::from_theme(&theme),
            })
        };

        // Transición de escena: al navegar de carpeta o cambiar el orden, la
        // `scene_key` cambia y el cuerpo entra con un fade + slide-up suave en
        // vez de saltar. Estable durante la carga de miniaturas de la misma
        // escena, así no refadea por cada thumb que llega.
        let scene_key = key_of(&format!("{}|{}", model.dir.display(), model.orden.etiqueta()));
        let cuerpo = cuerpo.animated_enter_from(scene_key, motion::SLOW, Affine::translate((0.0, 24.0)));

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        // Right-click en la raíz (origen 0,0 ⇒ local == ventana) abre el
        // menú contextual sobre la entrada seleccionada.
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, header, ruta, cuerpo])
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
        // La miniatura entra con fade-in la primera vez que aparece su key —
        // no salta de golpe sobre el skeleton.
        View::new(base())
            .image(img.clone())
            .animated_enter(key_of(&path.to_string_lossy()), motion::NORMAL)
    } else if model.fallidos.contains(path) {
        View::new(base())
            .fill(theme.bg_panel_alt)
            .text("⚠".to_string(), 20.0, theme.fg_muted)
    } else {
        // Placeholder con shimmer mientras decodifica: el usuario ve la forma
        // de la miniatura que viene, no un rectángulo muerto.
        let pal = SkeletonPalette::from_theme(theme);
        View::new(base())
            .radius(6.0)
            .clip(true)
            .children(vec![skeleton_view(&pal)])
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
    let texto = rimay_localize::t_args(
        "nahual-gallery-header",
        &[
            ("items", model.entries.len().to_string().into()),
            ("row", (v.first_row + 1).to_string().into()),
            ("total", v.total_rows.max(1).to_string().into()),
            ("order", model.orden.etiqueta().into()),
            ("estado", model.estado.clone().into()),
        ],
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

/// Mueve el preview a la imagen anterior/siguiente (`d` = ±1), saltando
/// carpetas. Con `wrap`, da la vuelta en los extremos (para el slideshow).
fn avanzar_imagen(m: &mut Model, d: i32, wrap: bool, handle: &Handle<Msg>) {
    let Some((actual, _)) = m.preview.as_ref() else {
        return;
    };
    let Some(cur) = m.entries.iter().position(|e| e.path() == actual) else {
        return;
    };
    let n = m.entries.len() as i32;
    let mut i = cur as i32 + d;
    // A lo sumo `n` pasos: evita un loop infinito si todo son carpetas.
    for _ in 0..n {
        if i < 0 || i >= n {
            if wrap {
                i = (i % n + n) % n;
            } else {
                return;
            }
        }
        if !m.entries[i as usize].es_carpeta() {
            let idx = i as usize;
            m.seleccionado = Some(idx);
            abrir_preview(m, idx, handle);
            return;
        }
        i += d;
    }
}

/// Programa un tick de slideshow: un thread duerme `SLIDE_SECS` y reentra
/// con `SlideAvanzar` (que reprograma el siguiente si el slideshow sigue).
fn programar_slide(handle: &Handle<Msg>) {
    handle.spawn(move || {
        std::thread::sleep(Duration::from_secs(SLIDE_SECS));
        Msg::SlideAvanzar
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

/// ¿Hay miniaturas visibles aún sin decodificar (ni fallidas)? Es decir,
/// celdas que ahora mismo pintan un skeleton — mientras sea cierto, el
/// shimmer necesita repaints periódicos.
fn hay_pendientes(m: &Model) -> bool {
    if m.entries.is_empty() {
        return false;
    }
    let v = ventana_visible(m.entries.len(), m.vw, m.vh, m.scroll_fila, &m.metrics);
    (v.first..v.first + v.count).any(|i| match &m.entries[i] {
        Entrada::Imagen { path, .. } => {
            !m.thumbs.contains_key(path) && !m.fallidos.contains(path)
        }
        Entrada::Carpeta(_) => false,
    })
}

/// Arranca la cadena de ticks de animación si hay miniaturas en vuelo y no
/// hay ya una corriendo. La cadena se auto-detiene cuando todo lo visible
/// quedó decodificado, así no queda un loop de repaint ocioso.
fn ensure_tick(m: &mut Model, handle: &Handle<Msg>) {
    if m.ticking || !hay_pendientes(m) {
        return;
    }
    m.ticking = true;
    handle.spawn(move || {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        Msg::Tick
    });
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

    // Eviction: si el cache RAM creció demasiado, conservar sólo las
    // miniaturas de un rango de entradas alrededor de la ventana visible
    // (con margen de varias pantallas) — el resto se regenera al volver.
    if m.thumbs.len() > CAP_THUMBS {
        let margen = v.cols.saturating_mul(30).max(60);
        let lo = v.first.saturating_sub(margen);
        let hi = (v.first + v.count + margen).min(m.entries.len());
        let mantener: HashSet<PathBuf> = m.entries[lo..hi]
            .iter()
            .filter(|e| !e.es_carpeta())
            .map(|e| e.path().to_path_buf())
            .collect();
        m.thumbs.retain(|p, _| mantener.contains(p));
    }

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
        .chain(imagenes.into_iter().map(|path| {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            Entrada::Imagen { path, size }
        }))
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

/// Viewport para clampear overlays. La galería trackea `vw` pero `vh` es
/// el área útil bajo las barras; reconstruimos el alto total de ventana.
fn viewport_of(model: &Model) -> (f32, f32) {
    (model.vw, model.vh + MENU_H + HEADER_H + BREADCRUMB_H)
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(menu: &'a AppMenu, model: &Model, theme: &'a Theme) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: viewport_of(model),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// Menú principal de la galería. Archivo / Ver / Ayuda — sólo comandos que
/// mapean a `Msg` reales. Sin "Editar": no hay campos de texto editables.
fn app_menu() -> AppMenu {
    use rimay_localize::t;
    AppMenu::new()
        .menu(
            Menu::new(t("nahual-gallery-menu-file"))
                .item(MenuItem::new(t("nahual-gallery-up"), "file.up").shortcut("Backspace"))
                .item(MenuItem::new(t("exit"), "file.quit").shortcut("Ctrl+Q").separated()),
        )
        .menu(
            Menu::new(t("nahual-gallery-menu-view"))
                .item(MenuItem::new(t("nahual-gallery-zoom-in"), "view.zoom_in").shortcut("+"))
                .item(MenuItem::new(t("nahual-gallery-zoom-out"), "view.zoom_out").shortcut("-"))
                .item(MenuItem::new(t("nahual-gallery-zoom-reset"), "view.zoom_reset"))
                .item(MenuItem::new(t("nahual-gallery-cycle-order"), "view.orden").shortcut("o").separated())
                .item(MenuItem::new(t("cycle-theme"), "view.theme")),
        )
        .menu(Menu::new(t("help")).item(MenuItem::new(t("about"), "help.about")))
}

/// Traduce un command id del menú principal a su efecto real.
fn handle_menu_command(model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    match cmd {
        "file.up" => {
            handle.dispatch(Msg::Subir);
            model
        }
        "file.quit" => std::process::exit(0),
        "view.zoom_in" => {
            handle.dispatch(Msg::Zoom(20.0));
            model
        }
        "view.zoom_out" => {
            handle.dispatch(Msg::Zoom(-20.0));
            model
        }
        "view.zoom_reset" => {
            // 140px es el tile inicial; pedimos el delta hasta ahí.
            handle.dispatch(Msg::Zoom(140.0 - model.metrics.tile_w));
            model
        }
        "view.orden" => {
            handle.dispatch(Msg::CiclarOrden);
            model
        }
        "view.theme" => {
            handle.dispatch(Msg::CiclarTema);
            model
        }
        // help.about y desconocidos: no-op.
        _ => model,
    }
}

fn main() {
    llimphi_ui::run::<Gallery>();
}
