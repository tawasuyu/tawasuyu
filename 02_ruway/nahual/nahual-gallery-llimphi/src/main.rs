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
use llimphi_widget_grid::{grid_view, ventana_visible, GridCell, GridMetrics, GridPalette};
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

/// Extensiones que tratamos como imagen (alineadas con las features del
/// crate `image` en el workspace: png/jpeg/webp).
const EXTS: &[&str] = &["png", "jpg", "jpeg", "webp"];

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
}

struct Model {
    dir: PathBuf,
    entries: Vec<PathBuf>,
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
        let entries = listar_imagenes(&dir);
        let (w, h) = Self::initial_size();
        let metrics = GridMetrics {
            tile_w: 140.0,
            tile_h: 162.0, // 140 imagen + ~22 label
            gap: 10.0,
            pad: 12.0,
        };
        let estado = if entries.is_empty() {
            format!("sin imágenes en {}", dir.display())
        } else {
            format!("{} imágenes", entries.len())
        };
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
            vh: h as f32 - HEADER_H,
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

    fn on_key(_: &Model, e: &KeyEvent) -> Option<Msg> {
        (e.state == KeyState::Pressed).then(|| Msg::Tecla(e.clone()))
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
                    m.estado = nombre(&m.entries[i]);
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
        }
        m
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
                format!("sin imágenes en {}", model.dir.display()),
                14.0,
                theme.fg_muted,
            )
        } else {
            let cells: Vec<GridCell<Msg>> = (v.first..v.first + v.count)
                .map(|i| {
                    let path = &model.entries[i];
                    GridCell {
                        content: celda_contenido(model, path),
                        label: Some(nombre(path)),
                        selected: model.seleccionado == Some(i),
                        on_click: Msg::Seleccionar(i),
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
        .children(vec![header, cuerpo])
    }
}

/// Cuerpo de una celda: la miniatura si está lista, un ⚠ si falló, o un
/// placeholder mientras se genera.
fn celda_contenido(model: &Model, path: &Path) -> View<Msg> {
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

/// Barra superior: carpeta, conteo, fila actual y estado.
fn encabezado(model: &Model, v: &llimphi_widget_grid::VisibleWindow) -> View<Msg> {
    let theme = &model.theme;
    let texto = format!(
        "📁 {}  ·  {} img  ·  fila {}/{}  ·  {}",
        model.dir.display(),
        model.entries.len(),
        v.first_row + 1,
        v.total_rows.max(1),
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
        m.estado = nombre(&m.entries[sel]);
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

/// Recalcula la ventana visible, encola los thumbs que falten, olvida los
/// que se fueron de pantalla y lanza el próximo lote de generación.
fn bombear(m: &mut Model, handle: &Handle<Msg>) {
    let v = ventana_visible(m.entries.len(), m.vw, m.vh, m.scroll_fila, &m.metrics);
    // Persistir el clamp (no scrollear más allá del fondo).
    m.scroll_fila = v.first_row;

    let mut visibles: HashSet<PathBuf> = HashSet::with_capacity(v.count);
    for (orden, i) in (v.first..v.first + v.count).enumerate() {
        let path = m.entries[i].clone();
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

/// Lista los archivos de imagen de un directorio, ordenados por nombre.
fn listar_imagenes(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && es_imagen(p))
            .collect(),
        Err(_) => Vec::new(),
    };
    v.sort();
    v
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
