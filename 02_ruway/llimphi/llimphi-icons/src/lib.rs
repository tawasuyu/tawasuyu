//! `llimphi-icons` — set canónico de iconos vectoriales para apps gioser.
//!
//! Cada icono es una función pura que devuelve un `BezPath` definido en
//! un grid lógico de **24×24 unidades**. El renderer escala al rect que
//! reciba, así un mismo icono sirve para 12px (en una fila de lista) y
//! para 64px (en una hero card) sin pérdida de nitidez — es vector,
//! no bitmap.
//!
//! ## Diseño
//!
//! - **Stroke-based, no fill**: los iconos son trazos de ancho 2 (en
//!   unidades del grid) con joins/caps suaves. El stroke se renderiza
//!   con el color que la app elija (típicamente `theme.fg_text` o
//!   `theme.accent`).
//! - **Geometría minimal, no marca**: glifos genéricos universales,
//!   no "marca registrada". Cada uno debe ser reconocible al primer
//!   vistazo aún en 12×12.
//! - **Set acotado**: suficientes para cubrir el grueso de acciones y
//!   tipos que aparecen en cualquier UI gioser. Si una app necesita uno
//!   más, lo agrega aquí (no en su propio crate) — la consistencia
//!   visual importa más que el aislamiento.
//!
//! ## Catálogo
//!
//! | Categoría    | Iconos                                              |
//! |--------------|-----------------------------------------------------|
//! | Documento    | `file`, `folder`, `folder_open`, `save`, `open`     |
//! | Edición      | `plus`, `minus`, `x`, `check`, `edit`, `trash`      |
//! | Navegación   | `chevron_up`, `chevron_down`, `chevron_left`, `chevron_right`, `home`, `search` |
//! | Estado       | `info`, `warning`, `error`, `bell`                  |
//! | Sistema      | `settings`, `more`                                  |
//! | Multimedia   | `play`, `pause`, `stop`, `skip_*`, `volume*`, `repeat`, `shuffle`, `record`, `equalizer`, `camera`, `gauge` |
//! | Archivos     | `image`, `music`, `film`, `archive`, `code`, `file_text`, `link`, `font` |
//!
//! ## Uso
//!
//! ```ignore
//! use llimphi_icons::{Icon, icon_view};
//!
//! // Botón con icono "save":
//! let btn = View::new(style)
//!     .fill(palette.bg_button)
//!     .children(vec![icon_view(Icon::Save, palette.fg_text, 1.6)]);
//! ```
//!
//! El parámetro `stroke_width` (3er arg de `icon_view`) está en unidades
//! del grid (24×24). `1.6` es el default armonioso; `2.0` para énfasis;
//! `1.2` para iconos en tipografías pequeñas.

#![forbid(unsafe_code)]

pub mod app_icons;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{percent, Size, Style},
    Position,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Cap, Join, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;

/// Catálogo de iconos del set canónico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    // --- Documento ---
    File,
    Folder,
    FolderOpen,
    Save,
    Open,
    // --- Edición ---
    Plus,
    Minus,
    X,
    Check,
    Edit,
    Trash,
    // --- Navegación ---
    ChevronUp,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    Home,
    Search,
    // --- Estado ---
    Info,
    Warning,
    Error,
    Bell,
    // --- Sistema ---
    Settings,
    More,
    // --- Multimedia ---
    Play,
    Pause,
    Stop,
    SkipBack,
    SkipForward,
    Rewind,
    FastForward,
    Volume,
    VolumeMute,
    Repeat,
    Shuffle,
    Record,
    Equalizer,
    Camera,
    Gauge,
    // --- Archivos (tipos por extensión, para listados de file manager) ---
    Image,
    Music,
    Film,
    Archive,
    Code,
    FileText,
    Link,
    Font,
}

impl Icon {
    /// Identificador estable en lowercase con underscores. Útil para
    /// debugging, persistir choices del usuario, o mapear desde strings
    /// en config.
    pub const fn name(self) -> &'static str {
        match self {
            Icon::File => "file",
            Icon::Folder => "folder",
            Icon::FolderOpen => "folder_open",
            Icon::Save => "save",
            Icon::Open => "open",
            Icon::Plus => "plus",
            Icon::Minus => "minus",
            Icon::X => "x",
            Icon::Check => "check",
            Icon::Edit => "edit",
            Icon::Trash => "trash",
            Icon::ChevronUp => "chevron_up",
            Icon::ChevronDown => "chevron_down",
            Icon::ChevronLeft => "chevron_left",
            Icon::ChevronRight => "chevron_right",
            Icon::Home => "home",
            Icon::Search => "search",
            Icon::Info => "info",
            Icon::Warning => "warning",
            Icon::Error => "error",
            Icon::Bell => "bell",
            Icon::Settings => "settings",
            Icon::More => "more",
            Icon::Play => "play",
            Icon::Pause => "pause",
            Icon::Stop => "stop",
            Icon::SkipBack => "skip_back",
            Icon::SkipForward => "skip_forward",
            Icon::Rewind => "rewind",
            Icon::FastForward => "fast_forward",
            Icon::Volume => "volume",
            Icon::VolumeMute => "volume_mute",
            Icon::Repeat => "repeat",
            Icon::Shuffle => "shuffle",
            Icon::Record => "record",
            Icon::Equalizer => "equalizer",
            Icon::Camera => "camera",
            Icon::Gauge => "gauge",
            Icon::Image => "image",
            Icon::Music => "music",
            Icon::Film => "film",
            Icon::Archive => "archive",
            Icon::Code => "code",
            Icon::FileText => "file_text",
            Icon::Link => "link",
            Icon::Font => "font",
        }
    }

    /// Devuelve el `BezPath` del icono en coords del grid 24×24. La
    /// app raramente lo necesita directamente — es lo que consume
    /// internamente [`icon_view`] / [`paint_icon`].
    pub fn path(self) -> BezPath {
        match self {
            Icon::File => path_file(),
            Icon::Folder => path_folder(),
            Icon::FolderOpen => path_folder_open(),
            Icon::Save => path_save(),
            Icon::Open => path_open(),
            Icon::Plus => path_plus(),
            Icon::Minus => path_minus(),
            Icon::X => path_x(),
            Icon::Check => path_check(),
            Icon::Edit => path_edit(),
            Icon::Trash => path_trash(),
            Icon::ChevronUp => path_chevron(0.0),
            Icon::ChevronDown => path_chevron(180.0),
            Icon::ChevronLeft => path_chevron(270.0),
            Icon::ChevronRight => path_chevron(90.0),
            Icon::Home => path_home(),
            Icon::Search => path_search(),
            Icon::Info => path_info(),
            Icon::Warning => path_warning(),
            Icon::Error => path_error(),
            Icon::Bell => path_bell(),
            Icon::Settings => path_settings(),
            Icon::More => path_more(),
            Icon::Play => path_play(),
            Icon::Pause => path_pause(),
            Icon::Stop => path_stop(),
            Icon::SkipBack => path_skip(true),
            Icon::SkipForward => path_skip(false),
            Icon::Rewind => path_seek(true),
            Icon::FastForward => path_seek(false),
            Icon::Volume => path_volume(false),
            Icon::VolumeMute => path_volume(true),
            Icon::Repeat => path_repeat(),
            Icon::Shuffle => path_shuffle(),
            Icon::Record => path_record(),
            Icon::Equalizer => path_equalizer(),
            Icon::Camera => path_camera(),
            Icon::Gauge => path_gauge(),
            Icon::Image => path_image(),
            Icon::Music => path_music(),
            Icon::Film => path_film(),
            Icon::Archive => path_archive(),
            Icon::Code => path_code(),
            Icon::FileText => path_file_text(),
            Icon::Link => path_link(),
            Icon::Font => path_font(),
        }
    }
}

/// Construye un `View` que pinta el icono ocupando todo el rect del
/// padre. El icono se escala uniformemente al mínimo lado y se centra.
///
/// - `stroke_width` en unidades del grid 24×24 (típico: `1.6`).
pub fn icon_view<Msg: Clone + 'static>(
    icon: Icon,
    color: Color,
    stroke_width: f32,
) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        paint_icon(scene, rect, icon, color, stroke_width);
    })
}

/// Pintor crudo — útil cuando una app quiere stampear varios iconos
/// dentro del mismo `paint_with` (paneles compuestos, toolbars
/// generadas dinámicamente).
pub fn paint_icon(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: llimphi_ui::PaintRect,
    icon: Icon,
    color: Color,
    stroke_width: f32,
) {
    let side = rect.w.min(rect.h) as f64;
    if side <= 0.0 {
        return;
    }
    let scale = side / 24.0;
    let tx = rect.x as f64 + (rect.w as f64 - side) * 0.5;
    let ty = rect.y as f64 + (rect.h as f64 - side) * 0.5;
    let xform = Affine::translate((tx, ty)) * Affine::scale(scale);

    let stroke = Stroke::new(stroke_width as f64)
        .with_join(Join::Round)
        .with_caps(Cap::Round);
    let path = icon.path();
    scene.stroke(&stroke, xform, color, None, &path);
}

// =====================================================================
// Paths — todos en grid 24×24, origen top-left, eje Y hacia abajo
// =====================================================================
//
// Cada path es geometría minimalista. Los joins y caps son Round (los
// fija el renderer), así que los corners salen suaves sin tener que
// definir curvas extra.

fn path_file() -> BezPath {
    // Documento: rectángulo con esquina superior-derecha plegada.
    let mut p = BezPath::new();
    p.move_to((6.0, 3.0));
    p.line_to((14.0, 3.0));
    p.line_to((19.0, 8.0));
    p.line_to((19.0, 21.0));
    p.line_to((6.0, 21.0));
    p.close_path();
    // Pliegue: línea desde la esquina superior-derecha del file hasta
    // donde "se dobla", luego al borde.
    p.move_to((14.0, 3.0));
    p.line_to((14.0, 8.0));
    p.line_to((19.0, 8.0));
    p
}

fn path_folder() -> BezPath {
    // Folder cerrado: cuerpo + lengüeta arriba-izquierda.
    let mut p = BezPath::new();
    p.move_to((3.0, 8.0));
    p.line_to((3.0, 19.0));
    p.line_to((21.0, 19.0));
    p.line_to((21.0, 8.0));
    p.line_to((11.0, 8.0));
    p.line_to((9.0, 5.0));
    p.line_to((3.0, 5.0));
    p.close_path();
    p
}

fn path_folder_open() -> BezPath {
    // Folder con tapa levantada: el "techo" se inclina hacia la derecha.
    let mut p = BezPath::new();
    // Caja inferior.
    p.move_to((3.0, 8.0));
    p.line_to((3.0, 19.0));
    p.line_to((21.0, 19.0));
    p.line_to((23.0, 11.0));
    p.line_to((7.0, 11.0));
    p.line_to((5.0, 19.0));
    // Lengüeta de la izquierda (sigue ahí).
    p.move_to((3.0, 8.0));
    p.line_to((9.0, 8.0));
    p.line_to((11.0, 11.0));
    p.line_to((21.0, 11.0));
    p
}

fn path_save() -> BezPath {
    // Floppy: cuadrado con muesca top-right y rectángulo de label abajo.
    let mut p = BezPath::new();
    p.move_to((4.0, 4.0));
    p.line_to((17.0, 4.0));
    p.line_to((20.0, 7.0));
    p.line_to((20.0, 20.0));
    p.line_to((4.0, 20.0));
    p.close_path();
    // Slot del shutter arriba.
    p.move_to((8.0, 4.0));
    p.line_to((8.0, 9.0));
    p.line_to((15.0, 9.0));
    p.line_to((15.0, 4.0));
    // Rectángulo de label abajo.
    p.move_to((7.0, 13.0));
    p.line_to((17.0, 13.0));
    p.line_to((17.0, 20.0));
    p.line_to((7.0, 20.0));
    p.close_path();
    p
}

fn path_open() -> BezPath {
    // Carpeta abriéndose hacia arriba con una flecha que entra.
    let mut p = BezPath::new();
    // Folder base.
    p.move_to((3.0, 19.0));
    p.line_to((3.0, 8.0));
    p.line_to((9.0, 8.0));
    p.line_to((11.0, 10.0));
    p.line_to((21.0, 10.0));
    p.line_to((21.0, 19.0));
    p.close_path();
    // Flecha entrando desde arriba al centro.
    p.move_to((12.0, 2.0));
    p.line_to((12.0, 14.0));
    p.move_to((9.0, 11.0));
    p.line_to((12.0, 14.0));
    p.line_to((15.0, 11.0));
    p
}

fn path_plus() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((12.0, 5.0));
    p.line_to((12.0, 19.0));
    p.move_to((5.0, 12.0));
    p.line_to((19.0, 12.0));
    p
}

fn path_minus() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((5.0, 12.0));
    p.line_to((19.0, 12.0));
    p
}

fn path_x() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((6.0, 6.0));
    p.line_to((18.0, 18.0));
    p.move_to((18.0, 6.0));
    p.line_to((6.0, 18.0));
    p
}

fn path_check() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((5.0, 13.0));
    p.line_to((10.0, 18.0));
    p.line_to((20.0, 6.0));
    p
}

fn path_edit() -> BezPath {
    // Lápiz: cuerpo diagonal + punta + tag de borrador.
    let mut p = BezPath::new();
    p.move_to((4.0, 20.0));
    p.line_to((8.0, 19.0));
    p.line_to((20.0, 7.0));
    p.line_to((17.0, 4.0));
    p.line_to((5.0, 16.0));
    p.close_path();
    p.move_to((14.0, 7.0));
    p.line_to((17.0, 10.0));
    p
}

fn path_trash() -> BezPath {
    // Tacho: tapa con manijita + cuerpo con tres barras verticales.
    let mut p = BezPath::new();
    // Tapa.
    p.move_to((4.0, 6.0));
    p.line_to((20.0, 6.0));
    // Manijita.
    p.move_to((9.0, 6.0));
    p.line_to((9.0, 4.0));
    p.line_to((15.0, 4.0));
    p.line_to((15.0, 6.0));
    // Cuerpo.
    p.move_to((6.0, 6.0));
    p.line_to((7.0, 21.0));
    p.line_to((17.0, 21.0));
    p.line_to((18.0, 6.0));
    // Barras internas.
    p.move_to((10.0, 10.0));
    p.line_to((10.0, 17.0));
    p.move_to((14.0, 10.0));
    p.line_to((14.0, 17.0));
    p
}

/// Chevron apuntando hacia arriba (default) o rotado por `angle_deg`
/// alrededor del centro del grid (12, 12). 90° = derecha, 180° = abajo,
/// 270° = izquierda.
fn path_chevron(angle_deg: f64) -> BezPath {
    let mut p = BezPath::new();
    // Chevron base: forma de ^ apuntando arriba.
    p.move_to((6.0, 14.0));
    p.line_to((12.0, 8.0));
    p.line_to((18.0, 14.0));
    let theta = angle_deg.to_radians();
    let center = (12.0, 12.0);
    Affine::translate(center)
        * Affine::rotate(theta)
        * Affine::translate((-center.0, -center.1))
        * p
}

fn path_home() -> BezPath {
    // Casa: triángulo de techo + caja rectangular.
    let mut p = BezPath::new();
    p.move_to((3.0, 12.0));
    p.line_to((12.0, 4.0));
    p.line_to((21.0, 12.0));
    // Cuerpo.
    p.move_to((5.0, 11.0));
    p.line_to((5.0, 20.0));
    p.line_to((19.0, 20.0));
    p.line_to((19.0, 11.0));
    // Puerta.
    p.move_to((10.0, 20.0));
    p.line_to((10.0, 14.0));
    p.line_to((14.0, 14.0));
    p.line_to((14.0, 20.0));
    p
}

fn path_search() -> BezPath {
    // Lupa: círculo (poligonal 16 segmentos) + mango diagonal.
    let mut p = BezPath::new();
    let cx = 10.5;
    let cy = 10.5;
    let r = 5.5;
    let segments = 24;
    for i in 0..=segments {
        let theta = std::f64::consts::TAU * (i as f64) / (segments as f64);
        let x = cx + r * theta.cos();
        let y = cy + r * theta.sin();
        if i == 0 {
            p.move_to((x, y));
        } else {
            p.line_to((x, y));
        }
    }
    // Mango.
    p.move_to((14.5, 14.5));
    p.line_to((20.0, 20.0));
    p
}

fn path_info() -> BezPath {
    // i: círculo + punto arriba + barra abajo.
    let mut p = path_circle(12.0, 12.0, 9.0, 32);
    // Punto.
    p.move_to((12.0, 7.0));
    p.line_to((12.0, 8.5));
    // Barra.
    p.move_to((12.0, 11.0));
    p.line_to((12.0, 17.0));
    p
}

fn path_warning() -> BezPath {
    // Triángulo con ! adentro.
    let mut p = BezPath::new();
    p.move_to((12.0, 3.0));
    p.line_to((22.0, 21.0));
    p.line_to((2.0, 21.0));
    p.close_path();
    p.move_to((12.0, 10.0));
    p.line_to((12.0, 15.0));
    p.move_to((12.0, 17.5));
    p.line_to((12.0, 18.5));
    p
}

fn path_error() -> BezPath {
    // Círculo con X adentro.
    let mut p = path_circle(12.0, 12.0, 9.0, 32);
    p.move_to((8.5, 8.5));
    p.line_to((15.5, 15.5));
    p.move_to((15.5, 8.5));
    p.line_to((8.5, 15.5));
    p
}

fn path_bell() -> BezPath {
    // Campana: domo + base + badajo.
    let mut p = BezPath::new();
    // Cuerpo con curva suave.
    p.move_to((5.0, 17.0));
    p.curve_to((5.0, 8.0), (8.0, 5.0), (12.0, 5.0));
    p.curve_to((16.0, 5.0), (19.0, 8.0), (19.0, 17.0));
    p.close_path();
    // Base.
    p.move_to((3.5, 17.0));
    p.line_to((20.5, 17.0));
    // Badajo.
    p.move_to((10.5, 20.0));
    p.line_to((13.5, 20.0));
    p
}

fn path_settings() -> BezPath {
    // Engranaje: 8 dientes radiales + agujero central.
    let mut p = BezPath::new();
    let cx = 12.0;
    let cy = 12.0;
    let inner_r = 6.5;
    let outer_r = 9.5;
    let teeth = 8;
    for i in 0..teeth * 2 {
        let theta = std::f64::consts::TAU * (i as f64) / (teeth as f64 * 2.0);
        // Cada paso alterna entre inner y outer para formar los dientes.
        let r = if i % 2 == 0 { outer_r } else { inner_r };
        let x = cx + r * theta.cos();
        let y = cy + r * theta.sin();
        if i == 0 {
            p.move_to((x, y));
        } else {
            p.line_to((x, y));
        }
    }
    p.close_path();
    // Agujero central.
    let inner = path_circle(cx, cy, 3.0, 16);
    for el in inner.elements() {
        p.push(*el);
    }
    p
}

fn path_more() -> BezPath {
    // Tres puntos horizontales (cada "punto" es un círculo pequeño).
    let mut p = BezPath::new();
    for (cx, cy) in &[(6.0, 12.0), (12.0, 12.0), (18.0, 12.0)] {
        let dot = path_circle(*cx, *cy, 1.5, 12);
        for el in dot.elements() {
            p.push(*el);
        }
    }
    p
}

/// Helper: aproxima un círculo con `segments` lados rectos. Para iconos
/// stroke esto se ve liso a partir de ~16 segmentos por la suavidad del
/// Cap::Round. Más barato y más predecible que cubic Beziers para los
/// glifos chiquitos donde vivimos.
fn path_circle(cx: f64, cy: f64, r: f64, segments: usize) -> BezPath {
    let mut p = BezPath::new();
    for i in 0..=segments {
        let theta = std::f64::consts::TAU * (i as f64) / (segments as f64);
        let x = cx + r * theta.cos();
        let y = cy + r * theta.sin();
        if i == 0 {
            p.move_to((x, y));
        } else {
            p.line_to((x, y));
        }
    }
    p
}

// ---------------------------------------------------------------------
// Multimedia — transporte de reproductor (media-app y demás)
// ---------------------------------------------------------------------

fn append(dst: &mut BezPath, src: &BezPath) {
    for el in src.elements() {
        dst.push(*el);
    }
}

fn path_play() -> BezPath {
    // Triángulo apuntando a la derecha.
    let mut p = BezPath::new();
    p.move_to((8.0, 5.0));
    p.line_to((8.0, 19.0));
    p.line_to((18.0, 12.0));
    p.close_path();
    p
}

fn path_pause() -> BezPath {
    // Dos barras verticales.
    let mut p = BezPath::new();
    p.move_to((9.0, 6.0));
    p.line_to((9.0, 18.0));
    p.move_to((15.0, 6.0));
    p.line_to((15.0, 18.0));
    p
}

fn path_stop() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((7.0, 7.0));
    p.line_to((17.0, 7.0));
    p.line_to((17.0, 17.0));
    p.line_to((7.0, 17.0));
    p.close_path();
    p
}

/// Saltar pista: barra + triángulo (a la izquierda si `back`).
fn path_skip(back: bool) -> BezPath {
    let mut p = BezPath::new();
    if back {
        p.move_to((7.0, 6.0));
        p.line_to((7.0, 18.0));
        p.move_to((17.0, 6.0));
        p.line_to((17.0, 18.0));
        p.line_to((8.0, 12.0));
        p.close_path();
    } else {
        p.move_to((7.0, 6.0));
        p.line_to((7.0, 18.0));
        p.line_to((16.0, 12.0));
        p.close_path();
        p.move_to((17.0, 6.0));
        p.line_to((17.0, 18.0));
    }
    p
}

/// Avance rápido: dos triángulos (a la izquierda si `rewind`).
fn path_seek(rewind: bool) -> BezPath {
    let mut p = BezPath::new();
    if rewind {
        p.move_to((11.0, 6.0));
        p.line_to((11.0, 18.0));
        p.line_to((4.0, 12.0));
        p.close_path();
        p.move_to((20.0, 6.0));
        p.line_to((20.0, 18.0));
        p.line_to((13.0, 12.0));
        p.close_path();
    } else {
        p.move_to((4.0, 6.0));
        p.line_to((4.0, 18.0));
        p.line_to((11.0, 12.0));
        p.close_path();
        p.move_to((13.0, 6.0));
        p.line_to((13.0, 18.0));
        p.line_to((20.0, 12.0));
        p.close_path();
    }
    p
}

/// Altavoz; con ondas (normal) o con una X (mute).
fn path_volume(mute: bool) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((3.0, 9.0));
    p.line_to((8.0, 9.0));
    p.line_to((12.0, 5.0));
    p.line_to((12.0, 19.0));
    p.line_to((8.0, 15.0));
    p.line_to((3.0, 15.0));
    p.close_path();
    if mute {
        p.move_to((15.0, 9.0));
        p.line_to((21.0, 15.0));
        p.move_to((21.0, 9.0));
        p.line_to((15.0, 15.0));
    } else {
        p.move_to((15.0, 9.0));
        p.quad_to((17.5, 12.0), (15.0, 15.0));
        p.move_to((17.5, 7.0));
        p.quad_to((21.5, 12.0), (17.5, 17.0));
    }
    p
}

fn path_repeat() -> BezPath {
    // Dos flechas horizontales opuestas (loop compacto).
    let mut p = BezPath::new();
    p.move_to((6.0, 9.0));
    p.line_to((16.0, 9.0));
    p.move_to((14.0, 7.0));
    p.line_to((17.0, 9.0));
    p.line_to((14.0, 11.0));
    p.move_to((18.0, 15.0));
    p.line_to((8.0, 15.0));
    p.move_to((10.0, 13.0));
    p.line_to((7.0, 15.0));
    p.line_to((10.0, 17.0));
    p
}

fn path_shuffle() -> BezPath {
    // Dos flechas que se cruzan.
    let mut p = BezPath::new();
    p.move_to((5.0, 8.0));
    p.line_to((19.0, 16.0));
    p.move_to((16.0, 15.5));
    p.line_to((20.0, 16.5));
    p.line_to((17.5, 13.0));
    p.move_to((5.0, 16.0));
    p.line_to((19.0, 8.0));
    p.move_to((17.5, 11.0));
    p.line_to((20.0, 7.5));
    p.line_to((16.0, 8.5));
    p
}

fn path_record() -> BezPath {
    path_circle(12.0, 12.0, 5.0, 20)
}

fn path_equalizer() -> BezPath {
    let mut p = BezPath::new();
    // Tres deslizadores verticales.
    p.move_to((7.0, 5.0));
    p.line_to((7.0, 19.0));
    p.move_to((12.0, 5.0));
    p.line_to((12.0, 19.0));
    p.move_to((17.0, 5.0));
    p.line_to((17.0, 19.0));
    // Perillas a distinta altura.
    p.move_to((5.0, 9.0));
    p.line_to((9.0, 9.0));
    p.move_to((10.0, 14.0));
    p.line_to((14.0, 14.0));
    p.move_to((15.0, 8.0));
    p.line_to((19.0, 8.0));
    p
}

fn path_camera() -> BezPath {
    let mut p = BezPath::new();
    p.move_to((4.0, 8.0));
    p.line_to((7.0, 8.0));
    p.line_to((9.0, 6.0));
    p.line_to((15.0, 6.0));
    p.line_to((17.0, 8.0));
    p.line_to((20.0, 8.0));
    p.line_to((20.0, 18.0));
    p.line_to((4.0, 18.0));
    p.close_path();
    append(&mut p, &path_circle(12.0, 13.0, 3.5, 16));
    p
}

fn path_gauge() -> BezPath {
    // Esfera + aguja (velocidad).
    let mut p = path_circle(12.0, 13.0, 6.0, 20);
    p.move_to((12.0, 13.0));
    p.line_to((16.0, 9.0));
    p
}

// ---------------------------------------------------------------------
// Archivos — tipos por extensión (listados de file manager / shell)
// ---------------------------------------------------------------------

fn path_image() -> BezPath {
    // Marco con una montaña y un sol (el clásico "imagen").
    let mut p = BezPath::new();
    p.move_to((4.0, 5.0));
    p.line_to((20.0, 5.0));
    p.line_to((20.0, 19.0));
    p.line_to((4.0, 19.0));
    p.close_path();
    // Sol.
    append(&mut p, &path_circle(8.5, 9.5, 1.6, 12));
    // Montaña (línea quebrada hasta el borde derecho).
    p.move_to((4.0, 17.0));
    p.line_to((10.0, 12.0));
    p.line_to((14.0, 15.0));
    p.line_to((17.0, 12.0));
    p.line_to((20.0, 15.0));
    p
}

fn path_music() -> BezPath {
    // Nota musical: dos cabezas redondas unidas por una plica con bandera.
    let mut p = BezPath::new();
    // Plicas.
    p.move_to((9.0, 18.0));
    p.line_to((9.0, 6.0));
    p.line_to((19.0, 4.0));
    p.line_to((19.0, 16.0));
    // Cabeza izquierda.
    append(&mut p, &path_circle(7.0, 18.0, 2.0, 14));
    // Cabeza derecha.
    append(&mut p, &path_circle(17.0, 16.0, 2.0, 14));
    p
}

fn path_film() -> BezPath {
    // Tira de película: rectángulo con perforaciones a los lados.
    let mut p = BezPath::new();
    p.move_to((4.0, 5.0));
    p.line_to((20.0, 5.0));
    p.line_to((20.0, 19.0));
    p.line_to((4.0, 19.0));
    p.close_path();
    // Rieles internos (separan perforaciones del cuadro central).
    p.move_to((8.0, 5.0));
    p.line_to((8.0, 19.0));
    p.move_to((16.0, 5.0));
    p.line_to((16.0, 19.0));
    // Perforaciones (cuatro tics por lado).
    for y in [7.5, 11.0, 14.5] {
        p.move_to((4.0, y));
        p.line_to((8.0, y));
        p.move_to((16.0, y));
        p.line_to((20.0, y));
    }
    p
}

fn path_archive() -> BezPath {
    // Caja/paquete: tapa arriba + cuerpo + tirador del cierre.
    let mut p = BezPath::new();
    // Tapa.
    p.move_to((3.0, 5.0));
    p.line_to((21.0, 5.0));
    p.line_to((21.0, 9.0));
    p.line_to((3.0, 9.0));
    p.close_path();
    // Cuerpo.
    p.move_to((4.5, 9.0));
    p.line_to((4.5, 20.0));
    p.line_to((19.5, 20.0));
    p.line_to((19.5, 9.0));
    // Pestaña del cierre.
    p.move_to((10.0, 12.0));
    p.line_to((14.0, 12.0));
    p
}

fn path_code() -> BezPath {
    // Corchetes angulares </> — universal para "código".
    let mut p = BezPath::new();
    // Chevron izquierdo.
    p.move_to((9.0, 7.0));
    p.line_to((4.0, 12.0));
    p.line_to((9.0, 17.0));
    // Chevron derecho.
    p.move_to((15.0, 7.0));
    p.line_to((20.0, 12.0));
    p.line_to((15.0, 17.0));
    // Barra diagonal central.
    p.move_to((13.0, 6.0));
    p.line_to((11.0, 18.0));
    p
}

fn path_file_text() -> BezPath {
    // Documento (como `file`) con líneas de texto adentro.
    let mut p = path_file();
    p.move_to((8.5, 12.0));
    p.line_to((16.5, 12.0));
    p.move_to((8.5, 15.0));
    p.line_to((16.5, 15.0));
    p.move_to((8.5, 18.0));
    p.line_to((13.5, 18.0));
    p
}

fn path_link() -> BezPath {
    // Symlink: dos eslabones de cadena en diagonal.
    let mut p = BezPath::new();
    // Eslabón superior-izquierdo (cápsula inclinada).
    p.move_to((10.0, 14.0));
    p.line_to((7.0, 11.0));
    p.curve_to((5.0, 9.0), (5.0, 7.0), (7.0, 5.0));
    p.curve_to((9.0, 3.0), (11.0, 3.0), (13.0, 5.0));
    p.line_to((15.0, 7.0));
    // Eslabón inferior-derecho.
    p.move_to((14.0, 10.0));
    p.line_to((17.0, 13.0));
    p.curve_to((19.0, 15.0), (19.0, 17.0), (17.0, 19.0));
    p.curve_to((15.0, 21.0), (13.0, 21.0), (11.0, 19.0));
    p.line_to((9.0, 17.0));
    p
}

fn path_font() -> BezPath {
    // Letra "A" serif — glifo de fuente tipográfica.
    let mut p = BezPath::new();
    // Astas de la A.
    p.move_to((6.0, 20.0));
    p.line_to((12.0, 4.0));
    p.line_to((18.0, 20.0));
    // Travesaño.
    p.move_to((8.5, 14.0));
    p.line_to((15.5, 14.0));
    // Serifas inferiores.
    p.move_to((4.5, 20.0));
    p.line_to((7.5, 20.0));
    p.move_to((16.5, 20.0));
    p.line_to((19.5, 20.0));
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_icons_have_nonempty_path() {
        let all = [
            Icon::File, Icon::Folder, Icon::FolderOpen, Icon::Save, Icon::Open,
            Icon::Plus, Icon::Minus, Icon::X, Icon::Check, Icon::Edit, Icon::Trash,
            Icon::ChevronUp, Icon::ChevronDown, Icon::ChevronLeft, Icon::ChevronRight,
            Icon::Home, Icon::Search, Icon::Info, Icon::Warning, Icon::Error,
            Icon::Bell, Icon::Settings, Icon::More,
            Icon::Play, Icon::Pause, Icon::Stop, Icon::SkipBack, Icon::SkipForward,
            Icon::Rewind, Icon::FastForward, Icon::Volume, Icon::VolumeMute,
            Icon::Repeat, Icon::Shuffle, Icon::Record, Icon::Equalizer,
            Icon::Camera, Icon::Gauge,
            Icon::Image, Icon::Music, Icon::Film, Icon::Archive,
            Icon::Code, Icon::FileText, Icon::Link, Icon::Font,
        ];
        for icon in all {
            let p = icon.path();
            assert!(
                p.elements().len() > 0,
                "icono {} produjo path vacío",
                icon.name()
            );
        }
    }

    #[test]
    fn icon_names_are_unique() {
        let all = [
            Icon::File, Icon::Folder, Icon::FolderOpen, Icon::Save, Icon::Open,
            Icon::Plus, Icon::Minus, Icon::X, Icon::Check, Icon::Edit, Icon::Trash,
            Icon::ChevronUp, Icon::ChevronDown, Icon::ChevronLeft, Icon::ChevronRight,
            Icon::Home, Icon::Search, Icon::Info, Icon::Warning, Icon::Error,
            Icon::Bell, Icon::Settings, Icon::More,
            Icon::Play, Icon::Pause, Icon::Stop, Icon::SkipBack, Icon::SkipForward,
            Icon::Rewind, Icon::FastForward, Icon::Volume, Icon::VolumeMute,
            Icon::Repeat, Icon::Shuffle, Icon::Record, Icon::Equalizer,
            Icon::Camera, Icon::Gauge,
            Icon::Image, Icon::Music, Icon::Film, Icon::Archive,
            Icon::Code, Icon::FileText, Icon::Link, Icon::Font,
        ];
        let mut names: Vec<&str> = all.iter().map(|i| i.name()).collect();
        let n = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), n, "nombres duplicados en Icon::name()");
    }
}
