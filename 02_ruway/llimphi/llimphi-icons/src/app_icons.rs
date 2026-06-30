//! `app_icons` — iconos de marca, uno por dominio/app de tawasuyu.
//!
//! A diferencia del set canónico de [`crate::Icon`] (glifos genéricos de
//! acción: file, save, search…), acá vive **un glifo distintivo por app**.
//! Cada app tiene su símbolo y su **color de marca** propios, pero todos
//! comparten el mismo lenguaje visual:
//!
//! - **Mismo grid lógico 24×24**, origen top-left, eje Y hacia abajo.
//! - **Stroke-based, sin fill**: trazos con `Join::Round` + `Cap::Round`.
//! - **Geometría minimal**: reconocible al primer vistazo aún en 16×16.
//! - **Aire de ~3 unidades** en los bordes para que respire dentro de un chip.
//!
//! La idea es que un dock/spotlight/menú pinte `app_icon_view(AppIcon::Pluma)`
//! y obtenga el glifo de la pluma en su color de tinta, sin que la app tenga
//! que cargar un PNG ni declarar su propia geometría.
//!
//! ```ignore
//! use llimphi_icons::app_icons::{AppIcon, app_icon_view};
//!
//! // Resuelve desde el id del registro de apps:
//! if let Some(icon) = AppIcon::from_app_id("cosmos") {
//!     let chip = View::new(style).children(vec![app_icon_view(icon, 1.8)]);
//! }
//! ```

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{percent, Size, Style},
    Position,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Cap, Join, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;

/// Una app de tawasuyu con icono de marca. El identificador (`name`) coincide
/// con el `id` del `AppEntry` en `app-bus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIcon {
    // --- 00_unanchay · PERCIBIR ---
    Chaka,
    Khipu,
    Pineal,
    Pluma,
    Puriy,
    Rimay,
    // --- 01_yachay · CONOCER ---
    Cosmos,
    Dominium,
    Iniy,
    Nakui,
    Tinkuy,
    // --- 02_ruway · HACER ---
    Ayni,
    Cards,
    Chasqui,
    Llimphi,
    Media,
    Mirada,
    Nada,
    Nahual,
    Shuma,
    Supay,
    Takiy,
    Tullpu,
    Wawa,
    // --- 03_ukupacha · RAÍZ ---
    Agora,
    Arje,
    Minga,
    Sandokan,
    WawaExplorer,
}

/// Las 29 apps, en orden de cuadrante. Útil para iterar (galerías, tests).
pub const ALL: [AppIcon; 29] = [
    AppIcon::Chaka,
    AppIcon::Khipu,
    AppIcon::Pineal,
    AppIcon::Pluma,
    AppIcon::Puriy,
    AppIcon::Rimay,
    AppIcon::Cosmos,
    AppIcon::Dominium,
    AppIcon::Iniy,
    AppIcon::Nakui,
    AppIcon::Tinkuy,
    AppIcon::Ayni,
    AppIcon::Cards,
    AppIcon::Chasqui,
    AppIcon::Llimphi,
    AppIcon::Media,
    AppIcon::Mirada,
    AppIcon::Nada,
    AppIcon::Nahual,
    AppIcon::Shuma,
    AppIcon::Supay,
    AppIcon::Takiy,
    AppIcon::Tullpu,
    AppIcon::Wawa,
    AppIcon::Agora,
    AppIcon::Arje,
    AppIcon::Minga,
    AppIcon::Sandokan,
    AppIcon::WawaExplorer,
];

impl AppIcon {
    /// Id estable de la app (coincide con `AppEntry.id` / nombre del dominio).
    pub const fn name(self) -> &'static str {
        match self {
            AppIcon::Chaka => "chaka",
            AppIcon::Khipu => "khipu",
            AppIcon::Pineal => "pineal",
            AppIcon::Pluma => "pluma",
            AppIcon::Puriy => "puriy",
            AppIcon::Rimay => "rimay",
            AppIcon::Cosmos => "cosmos",
            AppIcon::Dominium => "dominium",
            AppIcon::Iniy => "iniy",
            AppIcon::Nakui => "nakui",
            AppIcon::Tinkuy => "tinkuy",
            AppIcon::Ayni => "ayni",
            AppIcon::Cards => "cards",
            AppIcon::Chasqui => "chasqui",
            AppIcon::Llimphi => "llimphi",
            AppIcon::Media => "media",
            AppIcon::Mirada => "mirada",
            AppIcon::Nada => "nada",
            AppIcon::Nahual => "nahual",
            AppIcon::Shuma => "shuma",
            AppIcon::Supay => "supay",
            AppIcon::Takiy => "takiy",
            AppIcon::Tullpu => "tullpu",
            AppIcon::Wawa => "wawa",
            AppIcon::Agora => "agora",
            AppIcon::Arje => "arje",
            AppIcon::Minga => "minga",
            AppIcon::Sandokan => "sandokan",
            AppIcon::WawaExplorer => "wawa-explorer",
        }
    }

    /// Resuelve una app desde su `id` del registro. Acepta tanto
    /// `"wawa-explorer"` como `"wawa_explorer"`.
    pub fn from_app_id(id: &str) -> Option<AppIcon> {
        let id = id.trim().to_ascii_lowercase();
        let id = id.replace('_', "-");
        ALL.into_iter().find(|a| a.name() == id)
    }

    /// Color de marca de la app — el que el dock/menú debería usar para
    /// pintar el glifo por default.
    pub const fn brand(self) -> Color {
        let (r, g, b) = match self {
            AppIcon::Chaka => (43, 166, 164),
            AppIcon::Khipu => (181, 101, 29),
            AppIcon::Pineal => (108, 79, 216),
            AppIcon::Pluma => (61, 59, 142),
            AppIcon::Puriy => (63, 163, 77),
            AppIcon::Rimay => (232, 131, 58),
            AppIcon::Cosmos => (230, 184, 0),
            AppIcon::Dominium => (74, 111, 165),
            AppIcon::Iniy => (124, 179, 66),
            AppIcon::Nakui => (194, 84, 157),
            AppIcon::Tinkuy => (217, 83, 79),
            AppIcon::Ayni => (42, 168, 196),
            AppIcon::Cards => (142, 99, 206),
            AppIcon::Chasqui => (52, 179, 106),
            AppIcon::Llimphi => (229, 91, 122),
            AppIcon::Media => (226, 62, 87),
            AppIcon::Mirada => (45, 125, 210),
            AppIcon::Nada => (136, 147, 160),
            AppIcon::Nahual => (124, 77, 191),
            AppIcon::Shuma => (224, 165, 38),
            AppIcon::Supay => (155, 63, 181),
            AppIcon::Takiy => (229, 99, 155),
            AppIcon::Tullpu => (224, 96, 58),
            AppIcon::Wawa => (91, 141, 239),
            AppIcon::Agora => (47, 158, 143),
            AppIcon::Arje => (176, 141, 87),
            AppIcon::Minga => (224, 123, 57),
            AppIcon::Sandokan => (192, 57, 43),
            AppIcon::WawaExplorer => (110, 160, 240),
        };
        Color::from_rgb8(r, g, b)
    }

    /// `BezPath` del glifo en coords del grid 24×24.
    pub fn path(self) -> BezPath {
        match self {
            AppIcon::Chaka => path_chaka(),
            AppIcon::Khipu => path_khipu(),
            AppIcon::Pineal => path_pineal(),
            AppIcon::Pluma => path_pluma(),
            AppIcon::Puriy => path_puriy(),
            AppIcon::Rimay => path_rimay(),
            AppIcon::Cosmos => path_cosmos(),
            AppIcon::Dominium => path_dominium(),
            AppIcon::Iniy => path_iniy(),
            AppIcon::Nakui => path_nakui(),
            AppIcon::Tinkuy => path_tinkuy(),
            AppIcon::Ayni => path_ayni(),
            AppIcon::Cards => path_cards(),
            AppIcon::Chasqui => path_chasqui(),
            AppIcon::Llimphi => path_llimphi(),
            AppIcon::Media => path_media(),
            AppIcon::Mirada => path_mirada(),
            AppIcon::Nada => path_nada(),
            AppIcon::Nahual => path_nahual(),
            AppIcon::Shuma => path_shuma(),
            AppIcon::Supay => path_supay(),
            AppIcon::Takiy => path_takiy(),
            AppIcon::Tullpu => path_tullpu(),
            AppIcon::Wawa => path_wawa(),
            AppIcon::Agora => path_agora(),
            AppIcon::Arje => path_arje(),
            AppIcon::Minga => path_minga(),
            AppIcon::Sandokan => path_sandokan(),
            AppIcon::WawaExplorer => path_wawa_explorer(),
        }
    }
}

/// `View` que pinta el icono de app en su **color de marca**, ocupando todo
/// el rect del padre, escalado uniforme y centrado.
///
/// - `stroke_width` en unidades del grid 24×24 (típico de marca: `1.8`).
pub fn app_icon_view<Msg: Clone + 'static>(icon: AppIcon, stroke_width: f32) -> View<Msg> {
    app_icon_view_colored(icon, icon.brand(), stroke_width)
}

/// Igual que [`app_icon_view`] pero forzando un color (p.ej. monocromo
/// `theme.fg_text` para un menú denso donde el color distrae).
pub fn app_icon_view_colored<Msg: Clone + 'static>(
    icon: AppIcon,
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
        paint_app_icon(scene, rect, icon, color, stroke_width);
    })
}

/// Exporta el icono de app como **SVG** (string), en su color de marca.
/// Mismo dibujo que [`app_icon_view`] pero en un formato de archivo: sirve para
/// los `.desktop` (freedesktop `scalable/apps/<id>.svg`), la web, o cualquier
/// consumidor que no renderice con Llimphi. El path es stroke-only con remates
/// redondos, igual que el pintor vectorial. `viewBox 0 0 24 24` (la grilla).
pub fn app_icon_svg(icon: AppIcon, stroke_width: f32) -> String {
    let d = icon.path().to_svg();
    let [r, g, b, _] = icon.brand().components;
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let hex = format!("#{:02x}{:02x}{:02x}", q(r), q(g), q(b));
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" \
fill=\"none\" stroke=\"{hex}\" stroke-width=\"{sw}\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\
<path d=\"{d}\"/></svg>\n",
        sw = fmt_num(stroke_width),
    )
}

/// Formatea un f32 sin ceros de cola (`1.8` no `1.8000`).
fn fmt_num(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// Pintor crudo — para stampear varios iconos de app dentro del mismo
/// `paint_with` (una grilla de launcher, por ejemplo).
pub fn paint_app_icon(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: llimphi_ui::PaintRect,
    icon: AppIcon,
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
// Helpers
// =====================================================================

/// Círculo aproximado con `segments` lados rectos (liso por el Cap::Round).
fn circle(cx: f64, cy: f64, r: f64, segments: usize) -> BezPath {
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

/// Empuja todos los elementos de `src` dentro de `dst` (para componer
/// glifos hechos de varias subformas).
fn push_all(dst: &mut BezPath, src: BezPath) {
    for el in src.elements() {
        dst.push(*el);
    }
}

// =====================================================================
// Glifos — uno por app. Grid 24×24, margen ~3.
// =====================================================================

// --- 00_unanchay · PERCIBIR ---

fn path_chaka() -> BezPath {
    // chaka = puente: tablero recto + arco + dos pilotes.
    let mut p = BezPath::new();
    // Tablero.
    p.move_to((3.0, 9.0));
    p.line_to((21.0, 9.0));
    // Arco bajo el tablero.
    p.move_to((5.0, 18.0));
    p.curve_to((5.0, 11.0), (19.0, 11.0), (19.0, 18.0));
    // Pilotes que conectan tablero y arco.
    p.move_to((9.0, 9.0));
    p.line_to((9.0, 12.5));
    p.move_to((15.0, 9.0));
    p.line_to((15.0, 12.5));
    p
}

fn path_khipu() -> BezPath {
    // khipu: cordón principal + tres ramales con nudos (puntos).
    let mut p = BezPath::new();
    // Cordón superior.
    p.move_to((4.0, 6.0));
    p.line_to((20.0, 6.0));
    // Ramales.
    p.move_to((7.0, 6.0));
    p.line_to((7.0, 19.0));
    p.move_to((12.0, 6.0));
    p.line_to((12.0, 20.0));
    p.move_to((17.0, 6.0));
    p.line_to((17.0, 18.0));
    // Nudos.
    push_all(&mut p, circle(7.0, 12.0, 1.3, 10));
    push_all(&mut p, circle(12.0, 10.0, 1.3, 10));
    push_all(&mut p, circle(12.0, 16.0, 1.3, 10));
    push_all(&mut p, circle(17.0, 11.0, 1.3, 10));
    p
}

fn path_pineal() -> BezPath {
    // pineal = tercer ojo: párpado almendrado + iris + antena/rayo arriba.
    let mut p = BezPath::new();
    p.move_to((4.0, 12.0));
    p.curve_to((8.0, 7.0), (16.0, 7.0), (20.0, 12.0));
    p.curve_to((16.0, 17.0), (8.0, 17.0), (4.0, 12.0));
    push_all(&mut p, circle(12.0, 12.0, 2.6, 14));
    p.move_to((12.0, 3.0));
    p.line_to((12.0, 5.5));
    p
}

fn path_pluma() -> BezPath {
    // pluma = plumín: rombo apuntando abajo + ranura + ojal.
    let mut p = BezPath::new();
    p.move_to((12.0, 3.0));
    p.line_to((16.0, 9.0));
    p.line_to((13.5, 20.0));
    p.line_to((10.5, 20.0));
    p.line_to((8.0, 9.0));
    p.close_path();
    // Ranura.
    p.move_to((12.0, 11.5));
    p.line_to((12.0, 19.0));
    // Ojal.
    push_all(&mut p, circle(12.0, 9.5, 1.2, 10));
    p
}

fn path_puriy() -> BezPath {
    // puriy = caminar/recorrido: senda curva ascendente con flecha.
    let mut p = BezPath::new();
    p.move_to((6.0, 20.0));
    p.curve_to((6.0, 12.0), (18.0, 12.0), (18.0, 4.0));
    // Cabeza de flecha.
    p.move_to((15.0, 6.0));
    p.line_to((18.0, 4.0));
    p.line_to((20.5, 6.5));
    p
}

fn path_rimay() -> BezPath {
    // rimay = palabra/habla: globo de diálogo con cola + dos renglones.
    let mut p = BezPath::new();
    p.move_to((4.0, 6.0));
    p.line_to((20.0, 6.0));
    p.line_to((20.0, 15.0));
    p.line_to((11.0, 15.0));
    p.line_to((8.0, 19.0));
    p.line_to((8.0, 15.0));
    p.line_to((4.0, 15.0));
    p.close_path();
    // Renglones.
    p.move_to((8.0, 9.5));
    p.line_to((16.0, 9.5));
    p.move_to((8.0, 12.0));
    p.line_to((13.0, 12.0));
    p
}

// --- 01_yachay · CONOCER ---

fn path_cosmos() -> BezPath {
    // cosmos = destello de 4 puntas + dos estrellas pequeñas.
    let mut p = BezPath::new();
    p.move_to((12.0, 4.0));
    p.line_to((13.4, 10.6));
    p.line_to((20.0, 12.0));
    p.line_to((13.4, 13.4));
    p.line_to((12.0, 20.0));
    p.line_to((10.6, 13.4));
    p.line_to((4.0, 12.0));
    p.line_to((10.6, 10.6));
    p.close_path();
    // Estrellas chicas.
    push_all(&mut p, circle(19.0, 6.0, 0.8, 8));
    push_all(&mut p, circle(5.5, 18.0, 0.8, 8));
    p
}

fn path_dominium() -> BezPath {
    // dominium = ERP/libro mayor: barras de distinta altura sobre una base.
    let mut p = BezPath::new();
    // Base.
    p.move_to((3.0, 20.0));
    p.line_to((21.0, 20.0));
    // Columnas.
    p.move_to((6.0, 14.0));
    p.line_to((9.0, 14.0));
    p.line_to((9.0, 20.0));
    p.line_to((6.0, 20.0));
    p.close_path();
    p.move_to((10.5, 8.0));
    p.line_to((13.5, 8.0));
    p.line_to((13.5, 20.0));
    p.line_to((10.5, 20.0));
    p.close_path();
    p.move_to((15.0, 11.0));
    p.line_to((18.0, 11.0));
    p.line_to((18.0, 20.0));
    p.line_to((15.0, 20.0));
    p.close_path();
    p
}

fn path_iniy() -> BezPath {
    // iniy = aliento/creer: brote con tallo y dos hojas.
    let mut p = BezPath::new();
    // Tallo.
    p.move_to((12.0, 20.0));
    p.line_to((12.0, 10.0));
    // Hoja izquierda.
    p.move_to((12.0, 14.0));
    p.curve_to((8.0, 14.0), (6.0, 11.0), (7.0, 8.0));
    p.curve_to((10.0, 9.0), (12.0, 11.0), (12.0, 14.0));
    // Hoja derecha.
    p.move_to((12.0, 12.0));
    p.curve_to((15.5, 12.0), (17.0, 9.0), (16.5, 6.0));
    p.curve_to((14.0, 7.0), (12.0, 9.0), (12.0, 12.0));
    p
}

fn path_nakui() -> BezPath {
    // nakui = grafo de morfismos: tres nodos + aristas.
    let mut p = BezPath::new();
    // Aristas (primero, para que queden bajo los nodos).
    p.move_to((7.5, 9.0));
    p.line_to((16.5, 9.0));
    p.move_to((7.5, 9.8));
    p.line_to((10.8, 16.0));
    p.move_to((16.5, 9.8));
    p.line_to((13.2, 16.0));
    // Nodos.
    push_all(&mut p, circle(6.0, 8.0, 2.2, 14));
    push_all(&mut p, circle(18.0, 8.0, 2.2, 14));
    push_all(&mut p, circle(12.0, 18.0, 2.2, 14));
    p
}

fn path_tinkuy() -> BezPath {
    // tinkuy = encuentro/choque: dos flechas que convergen + chispa.
    let mut p = BezPath::new();
    // Flecha izquierda →
    p.move_to((3.0, 12.0));
    p.line_to((9.5, 12.0));
    p.move_to((7.5, 10.0));
    p.line_to((9.5, 12.0));
    p.line_to((7.5, 14.0));
    // Flecha derecha ←
    p.move_to((21.0, 12.0));
    p.line_to((14.5, 12.0));
    p.move_to((16.5, 10.0));
    p.line_to((14.5, 12.0));
    p.line_to((16.5, 14.0));
    // Chispa central.
    push_all(&mut p, circle(12.0, 12.0, 1.6, 10));
    p
}

// --- 02_ruway · HACER ---

fn path_ayni() -> BezPath {
    // ayni = reciprocidad: dos flechas curvas en ciclo.
    let mut p = BezPath::new();
    // Arco superior, flecha hacia la derecha-abajo.
    p.move_to((6.0, 8.0));
    p.curve_to((9.0, 4.0), (15.0, 4.0), (18.0, 8.5));
    p.move_to((15.5, 8.0));
    p.line_to((18.0, 8.5));
    p.line_to((18.5, 5.8));
    // Arco inferior, flecha hacia la izquierda-arriba.
    p.move_to((18.0, 16.0));
    p.curve_to((15.0, 20.0), (9.0, 20.0), (6.0, 15.5));
    p.move_to((8.5, 16.0));
    p.line_to((6.0, 15.5));
    p.line_to((5.5, 18.2));
    p
}

fn path_cards() -> BezPath {
    // cards = naipes apilados: carta frontal + borde de la de atrás.
    let mut p = BezPath::new();
    // Carta de atrás (asoma arriba y a la derecha).
    p.move_to((8.0, 5.0));
    p.line_to((19.0, 5.0));
    p.line_to((19.0, 16.0));
    // Carta frontal.
    p.move_to((5.0, 9.0));
    p.line_to((15.0, 9.0));
    p.line_to((15.0, 20.0));
    p.line_to((5.0, 20.0));
    p.close_path();
    p
}

fn path_chasqui() -> BezPath {
    // chasqui = mensajero: avión de papel.
    let mut p = BezPath::new();
    p.move_to((4.0, 11.0));
    p.line_to((20.0, 4.0));
    p.line_to((13.0, 20.0));
    p.line_to((11.0, 13.0));
    p.close_path();
    // Pliegue central.
    p.move_to((11.0, 13.0));
    p.line_to((20.0, 4.0));
    p
}

fn path_llimphi() -> BezPath {
    // llimphi = pintura/color: paleta con apoyo para el pulgar + 3 gotas.
    let mut p = BezPath::new();
    p.move_to((4.0, 12.0));
    p.curve_to((4.0, 6.0), (11.0, 4.0), (15.0, 5.0));
    p.curve_to((20.0, 6.5), (21.0, 12.0), (18.0, 15.0));
    p.curve_to((16.0, 16.5), (16.5, 13.5), (14.0, 14.0));
    p.curve_to((11.5, 14.5), (12.5, 18.0), (9.0, 18.0));
    p.curve_to((5.5, 18.0), (4.0, 15.0), (4.0, 12.0));
    p.close_path();
    // Gotas de pintura.
    push_all(&mut p, circle(8.0, 9.0, 1.1, 10));
    push_all(&mut p, circle(12.0, 8.0, 1.1, 10));
    push_all(&mut p, circle(15.5, 10.0, 1.1, 10));
    p
}

fn path_media() -> BezPath {
    // media = reproducción: marco + triángulo de play.
    let mut p = BezPath::new();
    p.move_to((4.0, 6.0));
    p.line_to((20.0, 6.0));
    p.line_to((20.0, 18.0));
    p.line_to((4.0, 18.0));
    p.close_path();
    // Play.
    p.move_to((10.0, 9.0));
    p.line_to((10.0, 15.0));
    p.line_to((16.0, 12.0));
    p.close_path();
    p
}

fn path_mirada() -> BezPath {
    // mirada = ojo: párpado + iris + pupila.
    let mut p = BezPath::new();
    p.move_to((3.0, 12.0));
    p.curve_to((8.0, 6.0), (16.0, 6.0), (21.0, 12.0));
    p.curve_to((16.0, 18.0), (8.0, 18.0), (3.0, 12.0));
    p.close_path();
    push_all(&mut p, circle(12.0, 12.0, 3.4, 18));
    push_all(&mut p, circle(12.0, 12.0, 1.0, 8));
    p
}

fn path_nada() -> BezPath {
    // nada = vacío: conjunto vacío ∅ (anillo + diagonal).
    let mut p = circle(12.0, 12.0, 8.0, 28);
    p.move_to((6.5, 17.5));
    p.line_to((17.5, 6.5));
    p
}

fn path_nahual() -> BezPath {
    // nahual = máscara/mutación de forma: antifaz con dos ojos.
    let mut p = BezPath::new();
    p.move_to((4.0, 9.0));
    p.curve_to((4.0, 6.5), (8.0, 6.0), (10.0, 7.5));
    p.curve_to((11.0, 8.2), (13.0, 8.2), (14.0, 7.5));
    p.curve_to((16.0, 6.0), (20.0, 6.5), (20.0, 9.0));
    p.curve_to((20.0, 13.5), (16.0, 16.5), (12.0, 15.5));
    p.curve_to((8.0, 16.5), (4.0, 13.5), (4.0, 9.0));
    p.close_path();
    push_all(&mut p, circle(9.0, 10.0, 1.3, 10));
    push_all(&mut p, circle(15.0, 10.0, 1.3, 10));
    p
}

fn path_shuma() -> BezPath {
    // shuma = discernir: embudo/filtro.
    let mut p = BezPath::new();
    p.move_to((4.0, 6.0));
    p.line_to((20.0, 6.0));
    p.line_to((13.0, 14.0));
    p.line_to((13.0, 19.0));
    p.line_to((11.0, 20.0));
    p.line_to((11.0, 14.0));
    p.close_path();
    p
}

fn path_supay() -> BezPath {
    // supay = espíritu del ukhupacha: llama doble.
    let mut p = BezPath::new();
    // Llama exterior.
    p.move_to((12.0, 3.0));
    p.curve_to((17.0, 9.0), (16.0, 14.0), (12.0, 21.0));
    p.curve_to((8.0, 14.0), (7.0, 9.0), (12.0, 3.0));
    p.close_path();
    // Llama interior.
    p.move_to((12.0, 9.0));
    p.curve_to((14.0, 12.0), (13.0, 16.0), (12.0, 18.0));
    p.curve_to((11.0, 16.0), (10.0, 12.0), (12.0, 9.0));
    p.close_path();
    p
}

fn path_takiy() -> BezPath {
    // takiy = cantar: corchea + ondas de sonido.
    let mut p = BezPath::new();
    // Cabeza de nota.
    push_all(&mut p, circle(8.0, 18.0, 2.4, 16));
    // Plica.
    p.move_to((10.4, 18.0));
    p.line_to((10.4, 6.0));
    // Banderola.
    p.move_to((10.4, 6.0));
    p.curve_to((13.5, 7.0), (14.5, 9.0), (13.5, 11.0));
    // Ondas.
    p.move_to((16.0, 9.0));
    p.curve_to((18.0, 11.0), (18.0, 13.0), (16.0, 15.0));
    p
}

fn path_tullpu() -> BezPath {
    // tullpu = tinte/color: tres gotas.
    let mut p = BezPath::new();
    // Gota 1.
    p.move_to((8.0, 5.0));
    p.curve_to((11.0, 9.0), (11.0, 11.0), (8.0, 12.0));
    p.curve_to((5.0, 11.0), (5.0, 9.0), (8.0, 5.0));
    p.close_path();
    // Gota 2.
    p.move_to((16.0, 6.0));
    p.curve_to((19.0, 10.0), (19.0, 12.0), (16.0, 13.0));
    p.curve_to((13.0, 12.0), (13.0, 10.0), (16.0, 6.0));
    p.close_path();
    // Gota 3.
    p.move_to((12.0, 13.0));
    p.curve_to((15.0, 17.0), (15.0, 19.0), (12.0, 20.0));
    p.curve_to((9.0, 19.0), (9.0, 17.0), (12.0, 13.0));
    p.close_path();
    p
}

fn path_wawa() -> BezPath {
    // wawa = célula/semilla (el SO en gestación): membrana + núcleo.
    let mut p = circle(12.0, 12.0, 8.0, 28);
    push_all(&mut p, circle(12.0, 12.0, 3.0, 16));
    p
}

// --- 03_ukupacha · RAÍZ ---

fn path_agora() -> BezPath {
    // agora = firma/confianza: escudo con check.
    let mut p = BezPath::new();
    p.move_to((12.0, 3.0));
    p.line_to((20.0, 6.0));
    p.line_to((20.0, 12.0));
    p.curve_to((20.0, 17.0), (16.0, 20.0), (12.0, 21.0));
    p.curve_to((8.0, 20.0), (4.0, 17.0), (4.0, 12.0));
    p.line_to((4.0, 6.0));
    p.close_path();
    // Check.
    p.move_to((8.5, 12.0));
    p.line_to((11.0, 14.5));
    p.line_to((16.0, 8.5));
    p
}

fn path_arje() -> BezPath {
    // arje = arché/raíz de confianza: ancla.
    let mut p = BezPath::new();
    // Anillo.
    push_all(&mut p, circle(12.0, 5.0, 2.2, 14));
    // Caña.
    p.move_to((12.0, 7.2));
    p.line_to((12.0, 19.0));
    // Travesaño.
    p.move_to((8.0, 10.0));
    p.line_to((16.0, 10.0));
    // Uñas/brazos.
    p.move_to((6.0, 14.0));
    p.curve_to((6.0, 18.5), (9.0, 20.0), (12.0, 20.0));
    p.move_to((18.0, 14.0));
    p.curve_to((18.0, 18.5), (15.0, 20.0), (12.0, 20.0));
    p
}

fn path_minga() -> BezPath {
    // minga = trabajo comunal: tres figuras.
    let mut p = BezPath::new();
    // Figura central.
    push_all(&mut p, circle(12.0, 7.0, 2.2, 14));
    p.move_to((8.0, 18.0));
    p.curve_to((8.0, 13.0), (16.0, 13.0), (16.0, 18.0));
    // Figura izquierda.
    push_all(&mut p, circle(5.5, 10.0, 1.6, 12));
    p.move_to((2.5, 18.0));
    p.curve_to((2.5, 14.5), (6.0, 13.5), (7.5, 15.0));
    // Figura derecha.
    push_all(&mut p, circle(18.5, 10.0, 1.6, 12));
    p.move_to((21.5, 18.0));
    p.curve_to((21.5, 14.5), (18.0, 13.5), (16.5, 15.0));
    p
}

fn path_sandokan() -> BezPath {
    // sandokan = caja/contenedor aislado: cubo isométrico.
    let mut p = BezPath::new();
    // Cara frontal.
    p.move_to((5.0, 8.0));
    p.line_to((14.0, 8.0));
    p.line_to((14.0, 18.0));
    p.line_to((5.0, 18.0));
    p.close_path();
    // Tapa.
    p.move_to((5.0, 8.0));
    p.line_to((9.0, 4.0));
    p.line_to((18.0, 4.0));
    p.line_to((14.0, 8.0));
    // Cara lateral.
    p.move_to((14.0, 8.0));
    p.line_to((18.0, 4.0));
    p.line_to((18.0, 14.0));
    p.line_to((14.0, 18.0));
    p
}

fn path_wawa_explorer() -> BezPath {
    // wawa-explorer = launchpad: grilla 2×2.
    let mut p = BezPath::new();
    for (x, y) in &[(5.0, 5.0), (13.0, 5.0), (5.0, 13.0), (13.0, 13.0)] {
        p.move_to((*x, *y));
        p.line_to((*x + 6.0, *y));
        p.line_to((*x + 6.0, *y + 6.0));
        p.line_to((*x, *y + 6.0));
        p.close_path();
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_app_icons_have_nonempty_path() {
        for icon in ALL {
            let p = icon.path();
            assert!(
                p.elements().len() > 0,
                "icono de app {} produjo path vacío",
                icon.name()
            );
        }
    }

    #[test]
    fn svg_export_es_valido() {
        for icon in ALL {
            let svg = app_icon_svg(icon, 1.8);
            assert!(svg.starts_with("<svg"), "{} svg sin cabecera", icon.name());
            assert!(svg.contains("<path d=\"M"), "{} svg sin path", icon.name());
            assert!(svg.contains("stroke=\"#"), "{} svg sin color de marca", icon.name());
            assert!(svg.trim_end().ends_with("</svg>"), "{} svg sin cierre", icon.name());
        }
    }

    #[test]
    fn app_names_are_unique() {
        let mut names: Vec<&str> = ALL.iter().map(|i| i.name()).collect();
        let n = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), n, "nombres duplicados en AppIcon::name()");
    }

    #[test]
    fn from_app_id_roundtrips() {
        for icon in ALL {
            assert_eq!(AppIcon::from_app_id(icon.name()), Some(icon));
        }
        // Tolera underscores y mayúsculas.
        assert_eq!(AppIcon::from_app_id("WAWA_EXPLORER"), Some(AppIcon::WawaExplorer));
        assert_eq!(AppIcon::from_app_id("desconocida"), None);
    }
}
