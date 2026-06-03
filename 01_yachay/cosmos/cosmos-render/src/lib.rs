//! `cosmos_app-render` — modelo y matemática de render
//! **agnósticos de surface**. Lo consumen tanto el canvas Llimphi
//! (nativo, render Vulkan/Metal) como el cliente web (WASM, render
//! SVG / Canvas2D). Cualquier mejora del layout / spread / cluster /
//! coords vive acá una sola vez y aparece en ambos clientes.
//!
//! ## Por qué un crate aparte
//!
//! `cosmos_app-engine` arrastra `eternal-sky` (VSOP2013 + I/O de
//! tablas) que **no compila a WASM** sin empaquetar 30+ MB de
//! efemérides. Los tipos del `RenderModel` en sí son serde puro y
//! sí compilan a WASM — extraerlos a este crate libera al cliente
//! web de la dependencia transitiva.
//!
//! ## Capas
//!
//! 1. **Modelo de render** — `RenderModel`, `Layer`, `Glyph`,
//!    `LineSeg`, `Geometry`, `LayerKind`. Estructuras serde-friendly
//!    que el engine emite y los clients consumen.
//! 2. **Matemática agnóstica** *(módulos siguientes, no en esta primera
//!    versión)* — `polar_to_screen`, `spread_angles`, `find_clusters`,
//!    `format_coord_compact`, `Radii`. Migran desde el canvas Llimphi.
//! 3. **`DrawCommand`** *(módulo siguiente)* — primitivas de pintura
//!    (line, circle, glyph, pill) que cada surface traduce a su API.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use serde::{Deserialize, Serialize};

pub use cosmos_model::{Chart, ChartId, ChartKind};

pub mod constellations_data;
pub mod draw;
pub mod glyphs;
pub mod gr;
pub mod harmonic;
pub mod math;
pub mod palette;
pub mod sky_data;
pub mod sphere3d;

pub use draw::{
    compose_wheel, compose_wheel_with_hits, draw_commands_to_svg, CompositionOpts, DrawCommand,
    Rgba, TextAnchor, WheelHits,
};
pub use gr::{compute_gr_triggers, convergencia_minima, GrDirection, GrTrigger};
pub use harmonic::apply_harmonic;
pub use math::{
    find_clusters, format_coord_compact, polar_to_screen, spread_angles, Radii,
};
pub use palette::Palette;
pub use sphere3d::{compose_sphere, SphereOpts, SphereView, OBLICUIDAD_DEG};

// =====================================================================
// RenderModel — lo que el client renderea
// =====================================================================

/// Resultado agnóstico de un cómputo astrológico, listo para renderizar.
/// El canvas Llimphi y el cliente web lo consumen idénticamente: el engine
/// computa (en nativo, con eternal) y publica este struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderModel {
    pub chart_id: ChartId,
    pub chart_kind: ChartKind,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    pub compute_ms: u64,

    // ─── Ángulos del chart (grados eclípticos, 0..360) ───────────────
    /// Ascendente — punto fijo de rotación del lienzo. La rueda se gira
    /// de modo que el Asc cae a las 9 (lado izquierdo).
    pub ascendant_deg: f32,
    pub midheaven_deg: f32,
    pub descendant_deg: f32,
    pub imum_coeli_deg: f32,
    /// Latitud geográfica del lugar, en grados. La vista de esfera 3D
    /// la usa para construir el horizonte local y el cénit del
    /// observador. `default` = 0.0 para compat serde con modelos viejos.
    #[serde(default)]
    pub geo_latitude_deg: f32,
    /// Longitud geográfica del lugar, en grados (este positivo). La
    /// esfera 3D la usa para orientar la Tierra interior — que el
    /// observador caiga en su continente real. `default` = 0.0.
    #[serde(default)]
    pub geo_longitude_deg: f32,

    /// Capas a pintar. Orden = z-order ascendente.
    pub layers: Vec<Layer>,
    /// Metadata humana por overlay activo (transit, progresión,
    /// sinastría, retorno...). Vacío para una carta natal pura. La UI
    /// la pinta como badges en el footer.
    #[serde(default)]
    pub overlays: Vec<OverlayMeta>,
    /// Lista paralela a las LineSeg de aspectos — uno por aspecto
    /// natal o cross. Ordenado por `orb_deg` ascendente (los más
    /// cerrados primero). La UI lo usa para la lista textual.
    #[serde(default)]
    pub aspect_summary: Vec<AspectSummary>,
    /// Grupos uranianos detectados (cuerpos en la misma posición mod 90).
    /// Vacío sino se activó el módulo Uranian.
    #[serde(default)]
    pub uranian_groups: Vec<UranianGroup>,
    /// Triggers del Sistema GR (direcciones primarias). Poblado sólo
    /// cuando el módulo `primary_directions` está activo; ordenado por
    /// `orb_deg` ascendente. La UI lo lista en el HUD de rectificación
    /// y resalta los `event = true` (convergencias directo+converso).
    #[serde(default)]
    pub gr_triggers: Vec<GrTrigger>,
    /// Orden de la carta armónica activa. `1` = carta natal pura.
    #[serde(default = "default_harmonic")]
    pub harmonic: u32,
    /// Espectro de fuerza armónica: índice `i` = fuerza de la armónica
    /// `i + 1`. Vacío salvo en modo armónico (`harmonic > 1`). La UI
    /// lo pinta como histograma para guiar qué armónico mirar.
    #[serde(default)]
    pub harmonic_spectrum: Vec<f32>,
}

/// Default serde del campo `harmonic`: 1 (carta natal sin transformar).
fn default_harmonic() -> u32 {
    1
}

/// Etiqueta legible de un overlay para el footer del canvas. La engine
/// la pushea desde cada `build_*_overlay`; el canvas solo lee y pinta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayMeta {
    pub module_id: String,
    /// Etiqueta corta — ej. "Tránsito ahora", "Progresión 38.2a",
    /// "Sinastría · Ana", "Saturn return 29a".
    pub label: String,
}

/// Grupo de cuerpos natales que caen en la misma posición del
/// dial uraniano de 90° (su longitud zodiacal módulo 90 es igual o
/// muy cercana). En la astrología uraniana esto es una "fórmula" o
/// "axis" — los cuerpos están en correspondencia simbólica directa
/// porque comparten un cuadrante simétrico.
///
/// Solo se emiten grupos con 2+ miembros (los singletons no son
/// fórmulas). La engine los ordena por proximidad al ε de tolerancia.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UranianGroup {
    /// Identificadores agnósticos de los cuerpos en el grupo
    /// (ej. `["sun", "jupiter", "saturn"]`).
    pub bodies: Vec<String>,
    /// Posición en el dial de 90° (la longitud módulo 90).
    pub mod90_deg: f64,
}

/// Resumen textual de un aspecto para listas legibles. La engine lo
/// emite en paralelo con las `LineSeg` de la capa de aspectos, así
/// el canvas no tiene que re-derivar nombres de cuerpos desde grados.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AspectSummary {
    /// Module al que pertenece — "natal", "transit", "synastry",
    /// "progression", "solar_arc", "planetary_return".
    pub module_id: String,
    /// Identificador agnóstico del cuerpo "a" — "sun", "moon", etc.
    pub from_body: String,
    pub to_body: String,
    /// Identificador del aspecto — "conjunction", "trine", etc.
    pub kind: String,
    pub orb_deg: f64,
    /// `Some(true)` = applying, `Some(false)` = separating. `None` para
    /// cross-aspects (sinastría/return) donde no se computa.
    #[serde(default)]
    pub applying: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub module_id: String,
    pub kind: LayerKind,
    /// Radio normalizado [0, 1] sobre el lienzo — el canvas lo convierte
    /// a píxeles. Permite stack de anillos.
    pub ring: f32,
    #[serde(default)]
    pub z: i32,
    pub geometry: Geometry,
    #[serde(default)]
    pub glyphs: Vec<Glyph>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    SignDial,
    Houses,
    Bodies,
    Aspects,
    Lots,
    FixedStars,
    Midpoints,
    Outer,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Geometry {
    GlyphsOnly,
    /// Anillo dividido en sectores. `cusps_deg` son los grados
    /// zodiacales donde van las divisiones radiales.
    Ring { cusps_deg: Vec<f32> },
    Lines(Vec<LineSeg>),
    Points(Vec<PointMark>),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LineSeg {
    /// Grados zodiacales del extremo "a".
    pub from_deg: f32,
    /// Grados zodiacales del extremo "b".
    pub to_deg: f32,
    /// Categoría simbólica (`"conjunction"`, `"trine"`, …) — el theme la
    /// resuelve a color.
    pub kind: String,
    pub opacity: f32,
    /// Cuerpo en el extremo "a" — populado para LineSegs de aspectos
    /// (natal × natal, cross con overlays). Vacío en `Default::default`
    /// para serde back-compat.
    #[serde(default)]
    pub from_body: String,
    /// Cuerpo en el extremo "b".
    #[serde(default)]
    pub to_body: String,
    /// Orb absoluto en grados (para tooltips).
    #[serde(default)]
    pub orb_deg: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointMark {
    pub deg: f32,
    pub label: String,
    pub tag: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Glyph {
    /// Grado eclíptico [0, 360).
    pub deg: f32,
    /// Glyph simbólico — el theme/canvas lo mapea a unicode o imagen.
    /// Ej: `"sun"`, `"moon"`, `"aries"`, `"asc"`, `"mc"`.
    pub symbol: String,
    #[serde(default)]
    pub annotation: Option<String>,
    #[serde(default)]
    pub retrograde: bool,
    #[serde(default)]
    pub house: Option<u8>,
    /// Marker de dignidad esencial, set solo cuando
    /// `NatalOptions::show_dignities` está activo: `"+"` (domicilio),
    /// `"·"` (exaltación), `"−"` (exilio), `"*"` (caída).
    #[serde(default)]
    pub dignity_marker: Option<String>,
}

/// Módulos overlay que pintan en el mismo slot (outer ring del wheel)
/// y por lo tanto son **mutuamente excluyentes** a nivel de UI: al
/// prender uno, el shell debe apagar los otros. Single source of truth
/// — el shell y el canvas leen de acá en vez de hardcodear listas.
pub const OUTER_RING_MODULES: &[&str] = &["transit", "synastry", "planetary_return"];
