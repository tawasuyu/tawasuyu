//! `dominium-render-plan` — la maqueta isométrica, agnóstica de backend.
//!
//! El último eslabón antes de la pantalla. Toma un [`World`] lógico, lo
//! proyecta con un [`IsoProjector`] y emite una lista plana de
//! [`Quad`]s 2D ya ordenados de atrás hacia adelante: cualquier backend
//! (GPUI, `<canvas>` web, TUI) sólo tiene que pintarlos en orden.
//!
//! Aquí no hay `gpui`, ni `wgpu`, ni `f64`: sólo aritmética `f32` y
//! `dominium-iso`. La regla de la spec —cero dependencias gráficas en el
//! núcleo— se respeta hasta el penúltimo crate.
//!
//! ```text
//!   World ──► build_plan(iso, weights, cfg) ──► RenderPlan { quads }
//!                                                    │
//!                          backend.paint(quad) ◄─────┘  (en orden)
//! ```
//!
//! - Una celda → un quad-rombo aproximado, coloreado por la mezcla de sus
//!   5 capas (la altura sale del `Z` compuesto, el color de la psique del
//!   suelo).
//! - Un Lemming → un quad-marca posado sobre el relieve de su celda.
//! - Todo se ordena por `depth = x + y` (orden de pintor isométrico).

#![forbid(unsafe_code)]

use dominium_core::World;
use dominium_iso::{IsoProjector, ZWeights};
use serde::{Deserialize, Serialize};

/// Color RGBA lineal, componentes en `0.0..=1.0`.
pub type Color = [f32; 4];

/// Un rectángulo 2D en coordenadas de pantalla, listo para pintar. El
/// origen `(0,0)` es el centro de la proyección; el backend traslada.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quad {
    /// Esquina superior-izquierda, eje X de pantalla.
    pub x: f32,
    /// Esquina superior-izquierda, eje Y de pantalla.
    pub y: f32,
    /// Ancho en pixels.
    pub w: f32,
    /// Alto en pixels.
    pub h: f32,
    /// Color RGBA.
    pub color: Color,
    /// Clave de orden de pintor: menor = más al fondo. El plan ya viene
    /// ordenado, pero se conserva por si el backend reordena.
    pub depth: f32,
}

/// Un cuadrilátero arbitrario de 4 vértices en coordenadas de pantalla.
/// Lo usamos para las **caras laterales** del prisma isométrico de cada
/// celda — paralelogramos que no encajan en un `Quad` axis-aligned, y dan
/// la sensación de maqueta 3D / papel cortado.
///
/// Vértices en orden anti-horario empezando por la esquina sup-izq, según
/// la convención del backend (BezPath cierra el path).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Polygon {
    pub vertices: [(f32, f32); 4],
    pub color: Color,
    pub depth: f32,
}

/// Paleta: un color por capa de la grilla, más el de los Lemmings. El
/// color de cada celda es la mezcla de estos pesada por el valor relativo
/// de cada capa.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Palette {
    /// Color de una celda sin ningún campo (terreno desnudo).
    pub floor: Color,
    pub materia: Color,
    pub psique: Color,
    pub poder: Color,
    pub oro: Color,
    pub degradacion: Color,
    /// Color de la marca de un Lemming.
    pub lemming: Color,
    /// Color del aura de influencia de un Concepto (translúcida).
    pub concepto_aura: Color,
    /// Color de la base de un Concepto (la "pared" de la mini-pirámide).
    pub concepto_base: Color,
    /// Color del tope de un Concepto (la "luz" de la mini-pirámide).
    pub concepto: Color,
    /// Color de sombra proyectada (RGBA con alpha bajo).
    pub shadow: Color,
}

impl Default for Palette {
    /// Paleta "tablero psicológico": verde materia, azul psique, rojo
    /// poder, ámbar oro, violeta degradación.
    fn default() -> Self {
        Self {
            floor: [0.10, 0.11, 0.13, 1.0],
            materia: [0.30, 0.72, 0.38, 1.0],
            psique: [0.32, 0.55, 0.86, 1.0],
            poder: [0.84, 0.27, 0.24, 1.0],
            oro: [0.90, 0.74, 0.24, 1.0],
            degradacion: [0.52, 0.30, 0.62, 1.0],
            lemming: [0.96, 0.96, 0.98, 1.0],
            concepto_aura: [0.95, 0.86, 0.55, 0.18],
            concepto_base: [0.58, 0.45, 0.18, 1.0],
            concepto: [0.98, 0.88, 0.42, 1.0],
            shadow: [0.04, 0.04, 0.06, 0.42],
        }
    }
}

/// Una de las 5 capas del Sustrato, para selección de heatmap. Coincide
/// 1:1 con los índices `RELIEVE_*` del core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderLayer {
    Materia,
    Psique,
    Poder,
    Oro,
    Degradacion,
}

impl RenderLayer {
    /// Etiqueta corta para HUD/picker.
    pub fn label(self) -> &'static str {
        match self {
            RenderLayer::Materia => "materia",
            RenderLayer::Psique => "psique",
            RenderLayer::Poder => "poder",
            RenderLayer::Oro => "oro",
            RenderLayer::Degradacion => "degrad.",
        }
    }

    /// Ciclado para pickers de UI: Materia → Psique → … → Materia.
    pub fn next(self) -> RenderLayer {
        match self {
            RenderLayer::Materia => RenderLayer::Psique,
            RenderLayer::Psique => RenderLayer::Poder,
            RenderLayer::Poder => RenderLayer::Oro,
            RenderLayer::Oro => RenderLayer::Degradacion,
            RenderLayer::Degradacion => RenderLayer::Materia,
        }
    }

    /// Devuelve el valor de la capa en la celda `idx`.
    pub fn value_at(self, world: &World, idx: usize) -> f32 {
        let g = &world.grid;
        match self {
            RenderLayer::Materia => g.materia[idx],
            RenderLayer::Psique => g.psique[idx],
            RenderLayer::Poder => g.poder[idx],
            RenderLayer::Oro => g.oro[idx],
            RenderLayer::Degradacion => g.degradacion[idx],
        }
    }
}

/// Cómo colorear las celdas del suelo. `Composite` mezcla las 5 capas
/// según la paleta (modo por defecto, lo que el simulador siempre fue).
/// `Heatmap(layer)` ignora las otras capas y pinta una sola en gradiente
/// `floor → palette[layer]` — útil para ver dónde se concentra una capa
/// específica sin que las otras la enmascaren.
///
/// `PsiCluster` deja el suelo en `Composite` pero los lemmings se colorean
/// según la asignación k-means de `psi_metrics::kmeans_psi`. Los colores por
/// cluster los provee el caller vía [`build_plan_with_overrides`]; si se
/// usa `build_plan` (compat), `PsiCluster` se comporta como `Composite`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderMode {
    Composite,
    Heatmap(RenderLayer),
    PsiCluster,
}

impl Default for RenderMode {
    fn default() -> Self {
        RenderMode::Composite
    }
}

/// Ajustes de la maqueta: tamaños de quad y paleta. Lo que un panel
/// expondría como controles de presentación (no afectan la simulación).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PlanConfig {
    /// Lado del quad de una celda, en pixels.
    pub tile: f32,
    /// Lado del quad-marca de un Lemming, en pixels.
    pub lemming_size: f32,
    /// Cuánto se eleva la marca del Lemming sobre el relieve de su celda,
    /// en unidades de `Z`.
    pub lemming_lift: f32,
    /// Lado del quad-marca central de un Concepto, en pixels.
    pub concepto_size: f32,
    /// Cuánto se eleva la marca de un Concepto sobre el relieve, en `Z`.
    pub concepto_lift: f32,
    /// Vector en coordenadas de mundo `(dx, dy)` que indica **hacia dónde
    /// cae la sombra** desde el pie de la entidad. Equivalente a la
    /// dirección opuesta al sol. Default: hacia abajo-derecha (luz desde
    /// arriba-izquierda, convención de maqueta clásica).
    pub light_dir: (f32, f32),
    /// Cantidad de capas adicionales que emite cada celda con relieve
    /// significativo, estilo "estampa andina" (mapa topográfico de papel
    /// cortado). Cada capa se apila a una fracción de `z` con un tile
    /// progresivamente más chico y un tono ligeramente más oscuro. 0 = off.
    pub andina_layers: u32,
    /// Umbral mínimo de `z` para activar las capas concéntricas en una
    /// celda — celdas planas no se descomponen.
    pub andina_threshold: f32,
    pub palette: Palette,
    /// Modo de coloreo de las celdas. Default `Composite` = el render
    /// histórico. `Heatmap(L)` aísla una capa.
    #[serde(default)]
    pub render_mode: RenderMode,
    /// Si está activo, cada techo siembra micro-quads procedurales que
    /// insinúan textura según la capa dominante: matorrales en celdas
    /// fértiles, brillos en oro, grietas en degradación. PRNG determinista
    /// por `(cx, cy)` así el patrón no titila entre frames.
    #[serde(default)]
    pub texture: bool,
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            tile: 18.0,
            lemming_size: 9.0,
            lemming_lift: 0.6,
            concepto_size: 14.0,
            concepto_lift: 1.4,
            light_dir: (0.55, 0.35),
            andina_layers: 0,
            andina_threshold: 1.0,
            palette: Palette::default(),
            render_mode: RenderMode::Composite,
            texture: false,
        }
    }
}

/// Un carácter rasterizado por encima de los quads — usado por los
/// glifos de `sprite_id` de Conceptos. El backend lo pinta vía
/// `llimphi-text::draw_block` con tamaño + color del Glyph.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Glyph {
    /// Carácter unicode a pintar.
    pub ch: char,
    /// Esquina sup-izq donde debería caer el bounding box del glifo.
    /// El backend puede centrarlo si quiere.
    pub x: f32,
    pub y: f32,
    pub size_px: f32,
    pub color: Color,
    /// Profundidad (informativa — los glifos se pintan después de los
    /// quads, así que sirve para sub-orden entre glifos si fuera necesario).
    pub depth: f32,
}

/// Lista de quads ordenada de atrás hacia adelante + caja envolvente.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RenderPlan {
    /// Quads ya ordenados por `depth` ascendente: píntalos en orden.
    pub quads: Vec<Quad>,
    /// Polígonos arbitrarios (caras laterales 3D, sombras paralelogramo)
    /// ordenados por `depth` ascendente. El backend los **intercala** con
    /// los quads por depth para mantener el orden de pintor isométrico.
    #[serde(default)]
    pub polygons: Vec<Polygon>,
    /// Glifos a pintar **después** de los quads, en orden de inserción.
    /// El backend usa `llimphi-text` para rasterizarlos. Hoy sólo se usa
    /// como fallback `?` para `sprite_id` desconocidos.
    #[serde(default)]
    pub glyphs: Vec<Glyph>,
    /// Primitivas vectoriales de los sprites de Conceptos, a pintar
    /// **después** de los quads/polígonos, en orden de inserción. El
    /// backend las rasteriza con vello (relleno/trazo/disco).
    #[serde(default)]
    pub sprites: Vec<SpritePrim>,
    /// Caja envolvente de todos los quads — el backend la usa para
    /// centrar o escalar la vista.
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl RenderPlan {
    /// Ancho de la caja envolvente.
    pub fn width(&self) -> f32 {
        self.max_x - self.min_x
    }

    /// Alto de la caja envolvente.
    pub fn height(&self) -> f32 {
        self.max_y - self.min_y
    }
}

/// Mapeo opaco `sprite_id → char`. El motor no le da semántica; sirve
/// para que el panel y el backend gráfico se pongan de acuerdo sobre
/// qué glifo pintar. `0` = sin glifo; `1..=8` definidos; el resto cae
/// a un `?` para feedback visual cuando hay un id desconocido.
pub fn glyph_for_sprite(id: u32) -> Option<char> {
    match id {
        0 => None,
        1 => Some('☩'), // cruz — iglesia
        2 => Some('¤'), // moneda — banco
        3 => Some('⌂'), // casa — comuna
        4 => Some('⚗'), // alambique — laboratorio
        5 => Some('☉'), // sol — centro
        6 => Some('☽'), // luna
        7 => Some('★'), // estrella
        8 => Some('◬'), // triángulo — chacana
        _ => Some('?'),
    }
}

/// Cantidad de sprite_ids con sprite definido (excluye 0 y el fallback).
/// Útil para los pickers de UI que ciclan a través de las opciones.
pub const SPRITE_COUNT: u32 = 8;

/// Nombre legible de un `sprite_id` — para los pickers del panel. `0` no
/// dibuja nada; `1..=8` son la librería; el resto es desconocido.
pub fn sprite_name(id: u32) -> &'static str {
    match id {
        0 => "—",
        1 => "iglesia",
        2 => "banco",
        3 => "casa",
        4 => "laboratorio",
        5 => "sol",
        6 => "luna",
        7 => "estrella",
        8 => "chacana",
        _ => "?",
    }
}

/// Una primitiva vectorial de un sprite procedural, ya resuelta a
/// coordenadas de pantalla. El backend la rasteriza con vello:
/// - `Fill`   — polígono cerrado relleno (≥3 vértices).
/// - `Stroke` — polilínea (abierta) con grosor `width` px.
/// - `Disc`   — disco relleno de centro `(cx, cy)` y radio `r`.
///
/// Es el reemplazo "dibujo de verdad" de los glifos opacos: cada Concepto
/// emite un puñado de estas en vez de un solo carácter unicode. Cero
/// assets en disco, cero shaders — sólo geometría que vello rellena.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SpritePrim {
    Fill {
        points: Vec<(f32, f32)>,
        color: Color,
    },
    Stroke {
        points: Vec<(f32, f32)>,
        width: f32,
        color: Color,
    },
    Disc {
        cx: f32,
        cy: f32,
        r: f32,
        color: Color,
    },
}

/// Tinta + acento + color de recorte de un sprite. `ink` pinta el detalle
/// oscuro (trazos, puertas); `accent` el relleno temático; `carve` es el
/// color de fondo con que se "talla" un hueco (lo usa la luna y el ojo de
/// la chacana, pasándoles el color del tope del Concepto).
struct SpriteInk {
    ink: Color,
    accent: Color,
    carve: Color,
}

/// Acento temático por `sprite_id`. La tinta es un gris-tinta común; el
/// acento le da identidad de color a cada icono (oro de iglesia, piedra de
/// banco, terracota de casa, líquido cian de laboratorio, etc.).
fn sprite_palette(id: u32, carve: Color) -> SpriteInk {
    let ink = [0.07, 0.07, 0.10, 1.0];
    let accent = match id {
        1 => [0.86, 0.71, 0.33, 1.0], // iglesia — oro
        2 => [0.78, 0.78, 0.83, 1.0], // banco — piedra
        3 => [0.81, 0.46, 0.31, 1.0], // casa — terracota
        4 => [0.31, 0.71, 0.86, 1.0], // laboratorio — líquido cian
        5 => [0.98, 0.82, 0.26, 1.0], // sol — amarillo
        6 => [0.92, 0.92, 0.80, 1.0], // luna — marfil
        7 => [0.96, 0.81, 0.36, 1.0], // estrella — oro
        8 => [0.82, 0.36, 0.30, 1.0], // chacana — rojo andino
        _ => [0.80, 0.80, 0.80, 1.0],
    };
    SpriteInk { ink, accent, carve }
}

/// Emite las primitivas del sprite `id` centradas en `(cx, cy)`, a tamaño
/// `size` (lado del icono en px), recortando huecos con `carve` (el color
/// del tope del Concepto). Devuelve vacío para `id == 0` o ids
/// desconocidos — esos caen al glifo `?` de feedback. Coordenadas de
/// pantalla, eje Y hacia abajo. Cada icono se autoría en el cuadrado
/// unitario `[-0.5, 0.5]²` y se escala/traslada por `size` y `(cx, cy)`.
pub fn sprite_prims(id: u32, cx: f32, cy: f32, size: f32, carve: Color) -> Vec<SpritePrim> {
    let pal = sprite_palette(id, carve);
    // Punto del cuadrado unitario → pantalla.
    let p = |ux: f32, uy: f32| (cx + ux * size, cy + uy * size);
    // Grosor de trazo base, proporcional al icono.
    let sw = size * 0.085;
    let mut v: Vec<SpritePrim> = Vec::new();

    match id {
        // 1 · IGLESIA — cuerpo + techo a dos aguas + cruz.
        1 => {
            v.push(SpritePrim::Fill {
                points: vec![p(-0.28, 0.5), p(-0.28, -0.05), p(0.28, -0.05), p(0.28, 0.5)],
                color: pal.accent,
            });
            v.push(SpritePrim::Fill {
                points: vec![p(-0.38, -0.05), p(0.0, -0.32), p(0.38, -0.05)],
                color: pal.ink,
            });
            v.push(SpritePrim::Stroke {
                points: vec![p(0.0, -0.32), p(0.0, -0.6)],
                width: sw,
                color: pal.ink,
            });
            v.push(SpritePrim::Stroke {
                points: vec![p(-0.11, -0.5), p(0.11, -0.5)],
                width: sw,
                color: pal.ink,
            });
        }
        // 2 · BANCO — frontón neoclásico + tres columnas + zócalo.
        2 => {
            v.push(SpritePrim::Fill {
                points: vec![p(-0.46, 0.02), p(0.0, -0.3), p(0.46, 0.02)],
                color: pal.accent,
            });
            for &cxn in &[-0.28_f32, 0.0, 0.28] {
                v.push(SpritePrim::Stroke {
                    points: vec![p(cxn, 0.06), p(cxn, 0.42)],
                    width: sw * 1.3,
                    color: pal.accent,
                });
            }
            v.push(SpritePrim::Fill {
                points: vec![p(-0.46, 0.42), p(0.46, 0.42), p(0.46, 0.52), p(-0.46, 0.52)],
                color: pal.ink,
            });
        }
        // 3 · CASA / COMUNA — cuerpo + techo + puerta.
        3 => {
            v.push(SpritePrim::Fill {
                points: vec![p(-0.3, 0.5), p(-0.3, 0.02), p(0.3, 0.02), p(0.3, 0.5)],
                color: pal.accent,
            });
            v.push(SpritePrim::Fill {
                points: vec![p(-0.42, 0.02), p(0.0, -0.34), p(0.42, 0.02)],
                color: pal.ink,
            });
            v.push(SpritePrim::Fill {
                points: vec![p(-0.09, 0.5), p(-0.09, 0.2), p(0.09, 0.2), p(0.09, 0.5)],
                color: pal.ink,
            });
        }
        // 4 · LABORATORIO — matraz: cuello + cuerpo cónico + burbuja.
        4 => {
            v.push(SpritePrim::Fill {
                points: vec![
                    p(-0.1, -0.06),
                    p(0.1, -0.06),
                    p(0.32, 0.48),
                    p(-0.32, 0.48),
                ],
                color: pal.accent,
            });
            v.push(SpritePrim::Stroke {
                points: vec![p(-0.1, -0.42), p(-0.1, -0.06)],
                width: sw,
                color: pal.ink,
            });
            v.push(SpritePrim::Stroke {
                points: vec![p(0.1, -0.42), p(0.1, -0.06)],
                width: sw,
                color: pal.ink,
            });
            v.push(SpritePrim::Stroke {
                points: vec![p(-0.16, -0.42), p(0.16, -0.42)],
                width: sw,
                color: pal.ink,
            });
            v.push(SpritePrim::Disc {
                cx: p(0.05, 0.3).0,
                cy: p(0.05, 0.3).1,
                r: size * 0.07,
                color: pal.carve,
            });
        }
        // 5 · SOL — disco central + ocho rayos.
        5 => {
            let dirs: [(f32, f32); 8] = [
                (1.0, 0.0),
                (0.707, -0.707),
                (0.0, -1.0),
                (-0.707, -0.707),
                (-1.0, 0.0),
                (-0.707, 0.707),
                (0.0, 1.0),
                (0.707, 0.707),
            ];
            for (dx, dy) in dirs {
                v.push(SpritePrim::Stroke {
                    points: vec![p(dx * 0.3, dy * 0.3), p(dx * 0.5, dy * 0.5)],
                    width: sw,
                    color: pal.accent,
                });
            }
            v.push(SpritePrim::Disc {
                cx,
                cy,
                r: size * 0.22,
                color: pal.accent,
            });
        }
        // 6 · LUNA — disco lleno menos un disco de recorte → creciente.
        6 => {
            v.push(SpritePrim::Disc {
                cx,
                cy,
                r: size * 0.34,
                color: pal.accent,
            });
            let (hx, hy) = p(0.18, -0.07);
            v.push(SpritePrim::Disc {
                cx: hx,
                cy: hy,
                r: size * 0.3,
                color: pal.carve,
            });
        }
        // 7 · ESTRELLA — polígono de 5 puntas (10 vértices, radio alterno).
        7 => {
            // Vértices unitarios ordenados por ángulo ascendente (alternan
            // exterior/interior), precomputados para no usar trig en runtime.
            const STAR: [(f32, f32, bool); 10] = [
                (0.951, 0.309, true),   // 18°
                (0.588, 0.809, false),  // 54°
                (0.0, 1.0, true),       // 90°
                (-0.588, 0.809, false), // 126°
                (-0.951, 0.309, true),  // 162°
                (-0.951, -0.309, false),// 198°
                (-0.588, -0.809, true), // 234°
                (0.0, -1.0, false),     // 270°
                (0.588, -0.809, true),  // 306°
                (0.951, -0.309, false), // 342°
            ];
            let (ro, ri) = (0.5_f32, 0.21_f32);
            let pts = STAR
                .iter()
                .map(|&(ux, uy, outer)| {
                    let r = if outer { ro } else { ri };
                    p(ux * r, uy * r)
                })
                .collect();
            v.push(SpritePrim::Fill {
                points: pts,
                color: pal.accent,
            });
        }
        // 8 · CHACANA — cruz andina escalonada (cruz de 12 vértices) + ojo.
        8 => {
            let a = 0.17_f32; // semiancho de brazo
            let b = 0.5_f32; // extensión del brazo
            v.push(SpritePrim::Fill {
                points: vec![
                    p(-a, -b),
                    p(a, -b),
                    p(a, -a),
                    p(b, -a),
                    p(b, a),
                    p(a, a),
                    p(a, b),
                    p(-a, b),
                    p(-a, a),
                    p(-b, a),
                    p(-b, -a),
                    p(-a, -a),
                ],
                color: pal.accent,
            });
            v.push(SpritePrim::Disc {
                cx,
                cy,
                r: size * 0.1,
                color: pal.carve,
            });
        }
        _ => return Vec::new(),
    }
    v
}

/// Mezcla `n` colores con pesos: `Σ wᵢ·colorᵢ / Σ wᵢ`. Alpha del primero.
fn blend(parts: &[(f32, Color)]) -> Color {
    let total: f32 = parts.iter().map(|(w, _)| *w).sum();
    if total <= f32::EPSILON {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let mut out = [0.0f32; 4];
    for (w, c) in parts {
        let k = w / total;
        for ch in 0..3 {
            out[ch] += k * c[ch];
        }
    }
    out[3] = 1.0;
    out
}

/// Color de una celda: mezcla de la paleta pesada por el valor relativo
/// de sus 5 capas. Una celda vacía cae al color `floor`.
fn cell_color(world: &World, idx: usize, pal: &Palette) -> Color {
    let g = &world.grid;
    let layers = [
        (g.materia[idx].max(0.0), pal.materia),
        (g.psique[idx].max(0.0), pal.psique),
        (g.poder[idx].max(0.0), pal.poder),
        (g.oro[idx].max(0.0), pal.oro),
        (g.degradacion[idx].max(0.0), pal.degradacion),
    ];
    let total: f32 = layers.iter().map(|(v, _)| *v).sum();
    if total <= f32::EPSILON {
        return pal.floor;
    }
    blend(&layers)
}

/// Oscurece un color por un factor multiplicativo (mantiene alpha).
/// Útil para sombrear caras laterales: el techo va en color base, las
/// caras visibles a la luz en factor 0.72, las en sombra en 0.55.
fn shade(c: Color, k: f32) -> Color {
    [c[0] * k, c[1] * k, c[2] * k, c[3]]
}

/// PRNG determinista a partir de `(cx, cy)`. Hash xorshift de 32 bits,
/// suficiente para sembrar texturas que no titilen entre frames.
fn cell_hash(cx: usize, cy: usize, salt: u32) -> u32 {
    let mut h = (cx as u32)
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add((cy as u32).wrapping_mul(0x85EB_CA6B))
        .wrapping_add(salt.wrapping_mul(0xC2B2_AE35));
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

/// Float ∈ [0, 1) determinista para la celda.
fn cell_rand(cx: usize, cy: usize, salt: u32) -> f32 {
    (cell_hash(cx, cy, salt) >> 8) as f32 / (1u32 << 24) as f32
}

/// Siembra micro-quads sobre el techo de una celda según la capa
/// dominante. Cada decoración cae dentro del rombo del techo —
/// aproximamos como un rect axis-aligned para mantener costo bajo:
/// son detalles pequeños donde el sesgo visual es despreciable.
///
/// Sólo emite quads cuando la celda tiene "algo que mostrar" — si está
/// vacía o todas las capas son insignificantes, no agrega ruido.
fn add_texture(
    out: &mut Vec<Quad>,
    world: &World,
    idx: usize,
    cx: usize,
    cy: usize,
    sx_center: f32,
    sy_center: f32,
    half_extent: f32,
    depth: f32,
    pal: &Palette,
) {
    let g = &world.grid;
    let m = g.materia[idx];
    let p = g.psique[idx];
    let pw = g.poder[idx];
    let o = g.oro[idx];
    let d = g.degradacion[idx];

    // Layout: las decoraciones se ubican en posiciones pseudo-aleatorias
    // dentro del rectángulo (sx ± half_extent, sy ± half_extent).
    let put = |out: &mut Vec<Quad>, salt: u32, size: f32, color: Color| {
        let rx = cell_rand(cx, cy, salt) * 2.0 - 1.0;
        let ry = cell_rand(cx, cy, salt + 1) * 2.0 - 1.0;
        // Margen para que el dot no caiga pisando el borde.
        let r = half_extent * 0.7;
        out.push(Quad {
            x: sx_center + rx * r - size * 0.5,
            y: sy_center + ry * r - size * 0.5,
            w: size,
            h: size,
            color,
            depth: depth + 0.05,
        });
    };

    // Matorrales: 1 a 3 puntos verde oscuro según cuánta materia.
    if m > 8.0 {
        let n = if m > 50.0 { 3 } else if m > 20.0 { 2 } else { 1 };
        let dark_green = shade(pal.materia, 0.45);
        for k in 0..n {
            put(out, 11 + k as u32 * 7, half_extent * 0.18, dark_green);
        }
    }
    // Brillo dorado: 1 mota pequeña amarilla cuando hay oro.
    if o > 4.0 {
        let glow = [
            (pal.oro[0] + 0.2).min(1.0),
            (pal.oro[1] + 0.2).min(1.0),
            (pal.oro[2] * 0.4).max(0.0),
            1.0,
        ];
        put(out, 41, half_extent * 0.16, glow);
    }
    // Grietas: para degradacion alta, una mota violeta oscura.
    if d > 0.5 {
        let scar = shade(pal.degradacion, 0.55);
        put(out, 57, half_extent * 0.22, scar);
        if d > 2.0 {
            put(out, 71, half_extent * 0.16, scar);
        }
    }
    // Halo psíquico: cuando psique es muy alta, una mota azulada
    // semitransparente — sugiere niebla espiritual.
    if p > 8.0 {
        let mist = [pal.psique[0], pal.psique[1], pal.psique[2], 0.55];
        put(out, 83, half_extent * 0.22, mist);
    }
    // Vena de poder: trazo rojizo cuando poder concentrado.
    if pw > 6.0 {
        let vein = shade(pal.poder, 0.85);
        put(out, 97, half_extent * 0.14, vein);
    }
}

/// Color de una celda en modo Heatmap de una sola capa: gradiente
/// `pal.floor → color de capa` saturando a `1.0` cuando el valor llega al
/// rango de referencia. Las celdas vacías quedan `floor`; las saturadas
/// quedan exactamente el color de la capa.
///
/// `scale` define qué valor cuenta como "saturado" — algo flexible porque
/// las capas viven en escalas distintas (materia llega a centenares,
/// degradacion suele ser sub-unidad). Default razonable para una grilla
/// 40×40 con SimParams default.
fn heatmap_color(world: &World, idx: usize, pal: &Palette, layer: RenderLayer) -> Color {
    let v = layer.value_at(world, idx).max(0.0);
    // Escala de referencia por capa: el orden de magnitud típico en una
    // corrida normal. Saturación lineal.
    let scale = match layer {
        RenderLayer::Materia => 60.0,
        RenderLayer::Psique => 20.0,
        RenderLayer::Poder => 15.0,
        RenderLayer::Oro => 25.0,
        RenderLayer::Degradacion => 5.0,
    };
    let t = (v / scale).clamp(0.0, 1.0);
    let tip = match layer {
        RenderLayer::Materia => pal.materia,
        RenderLayer::Psique => pal.psique,
        RenderLayer::Poder => pal.poder,
        RenderLayer::Oro => pal.oro,
        RenderLayer::Degradacion => pal.degradacion,
    };
    let base = pal.floor;
    [
        base[0] + (tip[0] - base[0]) * t,
        base[1] + (tip[1] - base[1]) * t,
        base[2] + (tip[2] - base[2]) * t,
        1.0,
    ]
}

/// Construye la maqueta isométrica de un `World`.
///
/// Wrapper sobre [`build_plan_with_overrides`] que pinta todos los lemmings
/// con `cfg.palette.lemming`. Equivalente a la firma histórica del crate.
pub fn build_plan(
    world: &World,
    iso: &IsoProjector,
    weights: &ZWeights,
    cfg: &PlanConfig,
) -> RenderPlan {
    let default_color = cfg.palette.lemming;
    build_plan_with_overrides(world, iso, weights, cfg, |_| default_color)
}

/// Versión de [`build_plan`] que permite teñir cada lemming individualmente
/// vía la closure `lemming_color`. Indispensable para `RenderMode::PsiCluster`:
/// el caller corre `kmeans_psi` y devuelve el color del cluster del lemming
/// `i`. Para el resto de modos, basta pasar `|_| cfg.palette.lemming`.
pub fn build_plan_with_overrides(
    world: &World,
    iso: &IsoProjector,
    weights: &ZWeights,
    cfg: &PlanConfig,
    lemming_color: impl Fn(usize) -> Color,
) -> RenderPlan {
    let g = &world.grid;
    let mut quads: Vec<Quad> = Vec::with_capacity(world.lemmings.len() * 2);
    // Cada celda emite 1 techo + hasta 2 caras laterales (este, sur).
    let mut polygons: Vec<Polygon> = Vec::with_capacity(g.cells() * 3);
    let mut glyphs: Vec<Glyph> = Vec::with_capacity(world.conceptos.len());
    let mut sprites: Vec<SpritePrim> = Vec::new();

    // Lookup de la altura de una celda — None para fuera de bounds.
    let z_at = |cx: i64, cy: i64| -> Option<f32> {
        if cx < 0 || cy < 0 || cx >= g.width as i64 || cy >= g.height as i64 {
            None
        } else {
            Some(weights.z_of(g, g.idx(cx as usize, cy as usize)))
        }
    };

    // Half-side de una celda en coordenadas de mundo. El tile (en pixels)
    // queda aplicado por el `IsoProjector.scale` — acá pensamos en mundo.
    let hs = 0.5_f32;

    // --- Celdas: techo (rombo iso) + caras laterales visibles (paralelogramos) ---
    for cy in 0..g.height {
        for cx in 0..g.width {
            let idx = g.idx(cx, cy);
            let z = weights.z_of(g, idx);
            let color = match cfg.render_mode {
                RenderMode::Composite | RenderMode::PsiCluster => {
                    cell_color(world, idx, &cfg.palette)
                }
                RenderMode::Heatmap(layer) => {
                    heatmap_color(world, idx, &cfg.palette, layer)
                }
            };
            let depth = cx as f32 + cy as f32;
            let fx = cx as f32;
            let fy = cy as f32;

            // 4 esquinas del techo proyectadas: NW, NE, SE, SW (sentido
            // horario porque la proyección iso invierte el eje Y).
            let p_nw = iso.project(fx - hs, fy - hs, z);
            let p_ne = iso.project(fx + hs, fy - hs, z);
            let p_se = iso.project(fx + hs, fy + hs, z);
            let p_sw = iso.project(fx - hs, fy + hs, z);

            // Estampa andina: capas previas como rombos concéntricos.
            if cfg.andina_layers > 0 && z > cfg.andina_threshold {
                let n = cfg.andina_layers as f32;
                for k in 0..cfg.andina_layers {
                    let frac = (k as f32) / n;
                    let z_k = z * frac;
                    let s = 1.0 - frac * 0.18;
                    let dark = 0.6 + frac * 0.35;
                    let color_k = shade(color, dark);
                    let p_nw_k = iso.project(fx - hs * s, fy - hs * s, z_k);
                    let p_ne_k = iso.project(fx + hs * s, fy - hs * s, z_k);
                    let p_se_k = iso.project(fx + hs * s, fy + hs * s, z_k);
                    let p_sw_k = iso.project(fx - hs * s, fy + hs * s, z_k);
                    polygons.push(Polygon {
                        vertices: [p_nw_k, p_ne_k, p_se_k, p_sw_k],
                        color: color_k,
                        depth: depth - 0.001 * (cfg.andina_layers - k) as f32,
                    });
                }
            }

            // Techo (cima a `z`).
            polygons.push(Polygon {
                vertices: [p_nw, p_ne, p_se, p_sw],
                color,
                depth,
            });

            // Textura procedural: micro-quads sobre el techo según capa
            // dominante. Determinista por (cx, cy).
            if cfg.texture {
                // Centro del techo y extensión ≈ media diagonal del rombo.
                let sx_top = (p_nw.0 + p_se.0) * 0.5;
                let sy_top = (p_nw.1 + p_se.1) * 0.5;
                let dx = (p_ne.0 - p_nw.0).abs();
                let dy = (p_se.1 - p_ne.1).abs();
                let extent = (dx + dy) * 0.25;
                add_texture(
                    &mut quads,
                    world,
                    idx,
                    cx,
                    cy,
                    sx_top,
                    sy_top,
                    extent,
                    depth,
                    &cfg.palette,
                );
            }

            // Caras laterales — sólo si hay borde "abierto": la vecina no
            // existe o está más abajo. Sino, su propio techo ya oculta
            // esta pared y emitirla sería trabajo perdido.
            //
            // Las caras bajan hasta la altura de la vecina (no hasta 0),
            // así celdas escalonadas se ven como escalones reales. Si no
            // hay vecina (borde del grid), bajan hasta 0.
            let east_z = z_at(cx as i64 + 1, cy as i64).unwrap_or(0.0);
            if east_z < z {
                let p_ne_b = iso.project(fx + hs, fy - hs, east_z);
                let p_se_b = iso.project(fx + hs, fy + hs, east_z);
                polygons.push(Polygon {
                    // Orden cerrado: arriba-norte, arriba-sur, abajo-sur, abajo-norte.
                    vertices: [p_ne, p_se, p_se_b, p_ne_b],
                    color: shade(color, 0.72),
                    depth: depth + 0.30,
                });
            }
            let south_z = z_at(cx as i64, cy as i64 + 1).unwrap_or(0.0);
            if south_z < z {
                let p_se_b = iso.project(fx + hs, fy + hs, south_z);
                let p_sw_b = iso.project(fx - hs, fy + hs, south_z);
                polygons.push(Polygon {
                    // Arriba-este, abajo-este, abajo-oeste, arriba-oeste.
                    vertices: [p_se, p_se_b, p_sw_b, p_sw],
                    color: shade(color, 0.55),
                    depth: depth + 0.40,
                });
            }
        }
    }

    // --- Conceptos: aura + sombra proyectada + base + tope ---
    // Cuatro quads cuentan una mini-estructura volumétrica:
    //   1) aura: halo translúcido en el suelo (depth -0.5)
    //   2) sombra: rect oscuro al pie de la luz (depth -0.4, antes de cells)
    //   3) base: cuadro ancho al ras del relieve (depth +0.5, "pared")
    //   4) tope: cuadro chico elevado por `concepto_lift` (depth +0.75)
    for c in &world.conceptos.items {
        let (cx, cy) = g.clamp_cell(c.pos_x, c.pos_y);
        let z_floor = weights.z_of(g, g.idx(cx, cy));

        // Aura al ras del suelo.
        let (ax, ay) = iso.project(c.pos_x, c.pos_y, 0.0);
        let aura = c.radius * 2.0 * cfg.tile;
        quads.push(Quad {
            x: ax - aura * 0.5,
            y: ay - aura * 0.5,
            w: aura,
            h: aura,
            color: cfg.palette.concepto_aura,
            depth: c.pos_x + c.pos_y - 0.5,
        });

        // Sombra proyectada en la dirección opuesta a la luz, largo
        // proporcional a la altura del tope.
        let z_top = z_floor + cfg.concepto_lift;
        let (sx, sy) = iso.shadow(c.pos_x, c.pos_y, z_top, cfg.light_dir);
        quads.push(Quad {
            x: sx - cfg.concepto_size * 0.7,
            y: sy - cfg.concepto_size * 0.35,
            w: cfg.concepto_size * 1.4,
            h: cfg.concepto_size * 0.7,
            color: cfg.palette.shadow,
            depth: c.pos_x + c.pos_y - 0.4,
        });

        // Base apoyada en el relieve — más ancha y oscura: la "pared".
        let (bx, by) = iso.project(c.pos_x, c.pos_y, z_floor);
        let base_size = cfg.concepto_size * 1.35;
        quads.push(Quad {
            x: bx - base_size * 0.5,
            y: by - base_size * 0.5,
            w: base_size,
            h: base_size,
            color: cfg.palette.concepto_base,
            depth: c.pos_x + c.pos_y + 0.5,
        });

        // Tope elevado — más chico y brillante: la "luz".
        let (tx, ty) = iso.project(c.pos_x, c.pos_y, z_top);
        quads.push(Quad {
            x: tx - cfg.concepto_size * 0.5,
            y: ty - cfg.concepto_size * 0.5,
            w: cfg.concepto_size,
            h: cfg.concepto_size,
            color: cfg.palette.concepto,
            depth: c.pos_x + c.pos_y + 0.75,
        });

        // Sprite vectorial del sprite_id, posado sobre el tope. La librería
        // (`sprite_prims`) cubre 1..=8 con iconos reales; `0` no dibuja
        // nada; un id desconocido cae al glifo `?` para feedback visual.
        let sprite_size = cfg.concepto_size * 1.7;
        let prims = sprite_prims(c.sprite_id, tx, ty - sprite_size * 0.08, sprite_size, cfg.palette.concepto);
        if prims.is_empty() {
            if c.sprite_id != 0 {
                let glyph_size = cfg.concepto_size * 1.15;
                glyphs.push(Glyph {
                    ch: '?',
                    // Aproximamos el centrado: parley pinta desde la esquina sup-izq.
                    x: tx - glyph_size * 0.4,
                    y: ty - glyph_size * 0.6,
                    size_px: glyph_size,
                    color: [0.05, 0.05, 0.08, 1.0],
                    depth: c.pos_x + c.pos_y + 0.85,
                });
            }
        } else {
            sprites.extend(prims);
        }
    }

    // --- Lemmings: sombra al ras + marca posada sobre el relieve ---
    let lem = &world.lemmings;
    for i in 0..lem.len() {
        let (px, py) = (lem.pos_x[i], lem.pos_y[i]);
        let (cx, cy) = g.clamp_cell(px, py);
        let z = weights.z_of(g, g.idx(cx, cy)) + cfg.lemming_lift;

        // Sombra proyectada — pequeña, plana, al suelo de su celda.
        let (sx, sy) = iso.shadow(px, py, z, cfg.light_dir);
        quads.push(Quad {
            x: sx - cfg.lemming_size * 0.45,
            y: sy - cfg.lemming_size * 0.25,
            w: cfg.lemming_size * 0.9,
            h: cfg.lemming_size * 0.5,
            color: cfg.palette.shadow,
            depth: px + py + 0.3,
        });

        // Marca del lemming — color por override (PsiCluster pinta por
        // cluster k-means; los demás modos pasan `cfg.palette.lemming`).
        let (mx, my) = iso.project(px, py, z);
        quads.push(Quad {
            x: mx - cfg.lemming_size * 0.5,
            y: my - cfg.lemming_size * 0.5,
            w: cfg.lemming_size,
            h: cfg.lemming_size,
            color: lemming_color(i),
            // +0.5 → la marca se pinta después de su celda y de las
            // celdas con su misma diagonal.
            depth: px + py + 0.5,
        });
    }

    // --- Orden de pintor: atrás (depth bajo) primero ---
    quads.sort_by(|a, b| {
        a.depth.partial_cmp(&b.depth).unwrap_or(core::cmp::Ordering::Equal)
    });
    polygons.sort_by(|a, b| {
        a.depth.partial_cmp(&b.depth).unwrap_or(core::cmp::Ordering::Equal)
    });

    // --- Caja envolvente: cubre quads + polygons + glifos ---
    let mut plan = RenderPlan { quads, polygons, glyphs, sprites, ..Default::default() };
    let mut have_bounds = false;
    let bump = |plan: &mut RenderPlan, have: &mut bool, x: f32, y: f32, w: f32, h: f32| {
        if !*have {
            plan.min_x = x;
            plan.min_y = y;
            plan.max_x = x + w;
            plan.max_y = y + h;
            *have = true;
        } else {
            plan.min_x = plan.min_x.min(x);
            plan.min_y = plan.min_y.min(y);
            plan.max_x = plan.max_x.max(x + w);
            plan.max_y = plan.max_y.max(y + h);
        }
    };
    // Snapshot las refs en variables locales para no chocar con el mut borrow.
    let q_iter: Vec<(f32, f32, f32, f32)> = plan
        .quads
        .iter()
        .map(|q| (q.x, q.y, q.w, q.h))
        .collect();
    for (x, y, w, h) in q_iter {
        bump(&mut plan, &mut have_bounds, x, y, w, h);
    }
    let pg_iter: Vec<[(f32, f32); 4]> = plan.polygons.iter().map(|p| p.vertices).collect();
    for v in pg_iter {
        for (vx, vy) in v {
            bump(&mut plan, &mut have_bounds, vx, vy, 0.0, 0.0);
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iso() -> IsoProjector {
        IsoProjector::new(1.0, 10.0)
    }

    #[test]
    fn empty_world_yields_one_top_polygon_per_cell() {
        // Plano (z=0 en todos lados) → solo techos, sin caras laterales.
        let world = World::new(5, 4);
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        assert_eq!(plan.polygons.len(), 20);
        // Sin lemmings ni conceptos → sin quads.
        assert_eq!(plan.quads.len(), 0);
    }

    #[test]
    fn each_lemming_adds_two_quads_shadow_and_marker() {
        let mut world = World::new(8, 8);
        world.lemmings.spawn(2.0, 3.0, 50.0, [1.0, 0.0, 0.0, 0.0]);
        world.lemmings.spawn(5.0, 5.0, 50.0, [0.0, 1.0, 0.0, 0.0]);
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        // 2 lemmings × 2 quads (sombra + marca) — las celdas ahora son polygons.
        assert_eq!(plan.quads.len(), 4);
        assert_eq!(plan.polygons.len(), 64);
    }

    #[test]
    fn polygons_are_depth_sorted_back_to_front() {
        let mut world = World::new(6, 6);
        world.lemmings.spawn(3.0, 3.0, 50.0, [0.0; 4]);
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        for w in plan.polygons.windows(2) {
            assert!(w[0].depth <= w[1].depth, "techos van de atrás hacia adelante");
        }
        for w in plan.quads.windows(2) {
            assert!(w[0].depth <= w[1].depth);
        }
    }

    #[test]
    fn lemming_draws_after_its_cell() {
        // Lemming en la celda (2,2): su marca (depth 4.5) debe ir tras la
        // celda (2,2) (depth 4.0).
        let mut world = World::new(6, 6);
        world.lemmings.spawn(2.0, 2.0, 50.0, [0.0; 4]);
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        let cfg = PlanConfig::default();
        let marca = plan
            .quads
            .iter()
            .find(|q| q.w == cfg.lemming_size)
            .expect("hay una marca");
        assert_eq!(marca.depth, 4.5);
    }

    #[test]
    fn empty_cell_uses_floor_color() {
        let world = World::new(3, 3);
        let cfg = PlanConfig::default();
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &cfg);
        // El primer polygon es el techo de la celda más al fondo: floor.
        assert_eq!(plan.polygons[0].color, cfg.palette.floor);
    }

    #[test]
    fn high_materia_cell_leans_green() {
        // Una celda con materia → su techo en color `materia`; las otras
        // celdas vacías van en `floor`. (La celda alta también emite caras
        // laterales sombreadas — las filtramos por color exacto.)
        let mut world = World::new(3, 3);
        let idx = world.grid.idx(1, 1);
        world.grid.materia[idx] = 100.0;
        let cfg = PlanConfig::default();
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &cfg);
        let painted: Vec<_> = plan
            .polygons
            .iter()
            .filter(|p| p.color == cfg.palette.materia)
            .collect();
        assert_eq!(painted.len(), 1, "un solo techo con color materia");
    }

    #[test]
    fn cell_color_blends_two_layers() {
        let mut world = World::new(3, 3);
        let idx = world.grid.idx(0, 0);
        world.grid.materia[idx] = 50.0;
        world.grid.poder[idx] = 50.0;
        let pal = Palette::default();
        let c = cell_color(&world, idx, &pal);
        // Mezcla 50/50 de verde materia y rojo poder → canal por canal.
        for ch in 0..3 {
            let expected = 0.5 * pal.materia[ch] + 0.5 * pal.poder[ch];
            assert!((c[ch] - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn bounding_box_encloses_every_quad_and_polygon() {
        let mut world = World::new(7, 5);
        world.lemmings.spawn(3.0, 2.0, 50.0, [0.0; 4]);
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        for q in &plan.quads {
            assert!(q.x >= plan.min_x - 1e-3);
            assert!(q.y >= plan.min_y - 1e-3);
            assert!(q.x + q.w <= plan.max_x + 1e-3);
            assert!(q.y + q.h <= plan.max_y + 1e-3);
        }
        for p in &plan.polygons {
            for (vx, vy) in p.vertices {
                assert!(vx >= plan.min_x - 1e-3);
                assert!(vy >= plan.min_y - 1e-3);
                assert!(vx <= plan.max_x + 1e-3);
                assert!(vy <= plan.max_y + 1e-3);
            }
        }
        assert!(plan.width() > 0.0 && plan.height() > 0.0);
    }

    #[test]
    fn heatmap_isolates_one_layer() {
        let mut world = World::new(3, 3);
        let i_mat = world.grid.idx(0, 0);
        let i_pow = world.grid.idx(2, 0);
        world.grid.materia[i_mat] = 60.0;
        world.grid.poder[i_pow] = 15.0;
        let mut cfg = PlanConfig::default();
        cfg.render_mode = RenderMode::Heatmap(RenderLayer::Materia);
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &cfg);
        // El techo de (0,0) (depth 0) y (2,0) (depth 2) son los que
        // queremos. En heatmap(materia), (0,0) llega a saturación
        // (color materia) y (2,0) queda en floor porque no tiene materia.
        let p_mat = plan
            .polygons
            .iter()
            .find(|p| p.depth == 0.0)
            .expect("techo (0,0)");
        let p_pow = plan
            .polygons
            .iter()
            .find(|p| p.depth == 2.0)
            .expect("techo (2,0)");
        assert_eq!(p_mat.color, cfg.palette.materia);
        assert_eq!(p_pow.color, cfg.palette.floor);
    }

    #[test]
    fn plan_is_deterministic() {
        let mut world = World::new(10, 10);
        world.lemmings.spawn(4.0, 6.0, 50.0, [0.5, 0.2, 0.1, 0.7]);
        let idx = world.grid.idx(2, 2);
        world.grid.materia[idx] = 33.0;
        let a = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        let b = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        assert_eq!(a.quads, b.quads);
        assert_eq!(a.polygons, b.polygons);
    }

    #[test]
    fn each_concepto_adds_four_quads_aura_shadow_base_top() {
        use dominium_core::{Concepto, LayerMods};
        let mut world = World::new(8, 8);
        world.conceptos.add(Concepto {
            id: "iglesia".into(),
            sprite_id: 0,
            pos_x: 4.0,
            pos_y: 4.0,
            radius: 2.0,
            mods: LayerMods::default(),
            hack: None,
            persuasion: None,
        });
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        // 4 quads del concepto (aura + sombra + base + tope). Las celdas
        // viven en polygons ahora.
        assert_eq!(plan.quads.len(), 4);
        assert_eq!(plan.polygons.len(), 64);
    }

    #[test]
    fn concepto_top_paints_after_its_lemming_neighbors() {
        use dominium_core::{Concepto, LayerMods};
        let mut world = World::new(8, 8);
        // Lemming en (4,4), concepto también en (4,4): el tope del concepto
        // (depth 8.75) debe ir tras la marca del lemming (depth 8.5).
        world.lemmings.spawn(4.0, 4.0, 50.0, [0.0; 4]);
        world.conceptos.add(Concepto {
            id: "iglesia".into(),
            sprite_id: 0,
            pos_x: 4.0,
            pos_y: 4.0,
            radius: 1.5,
            mods: LayerMods::default(),
            hack: None,
            persuasion: None,
        });
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        let cfg = PlanConfig::default();
        let lemming_marker_depth = plan
            .quads
            .iter()
            .find(|q| q.w == cfg.lemming_size && q.color == cfg.palette.lemming)
            .expect("hay un lemming")
            .depth;
        let concepto_top_depth = plan
            .quads
            .iter()
            .find(|q| q.w == cfg.concepto_size && q.color == cfg.palette.concepto)
            .expect("hay un tope de concepto")
            .depth;
        assert!(concepto_top_depth > lemming_marker_depth);
    }

    #[test]
    fn shadow_falls_along_light_dir_world_x() {
        use dominium_core::{Concepto, LayerMods};
        // light_dir = (1, 0) → la sombra cae +x en mundo → en pantalla iso
        // x' = (x - y)*cos30 crece. La sombra queda a la derecha del tope.
        let mut world = World::new(8, 8);
        world.conceptos.add(Concepto {
            id: "torre".into(),
            sprite_id: 0,
            pos_x: 4.0,
            pos_y: 4.0,
            radius: 1.0,
            mods: LayerMods::default(),
            hack: None,
            persuasion: None,
        });
        let cfg = PlanConfig { light_dir: (1.0, 0.0), ..Default::default() };
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &cfg);
        let shadow = plan
            .quads
            .iter()
            .find(|q| q.color == cfg.palette.shadow)
            .expect("hay una sombra del concepto");
        let top = plan
            .quads
            .iter()
            .find(|q| q.w == cfg.concepto_size && q.color == cfg.palette.concepto)
            .expect("hay un tope");
        let shadow_cx = shadow.x + shadow.w * 0.5;
        let top_cx = top.x + top.w * 0.5;
        assert!(shadow_cx > top_cx, "centro de sombra debe quedar a la derecha del tope");
    }

    #[test]
    fn andina_disabled_keeps_one_top_per_cell() {
        // Con andina_layers = 0, una celda alta emite techo + caras
        // laterales hacia las vecinas bajas — pero ningún rombo extra
        // andino.
        let mut world = World::new(3, 3);
        let center = world.grid.idx(1, 1);
        world.grid.materia[center] = 100.0;
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        // 9 techos + caras laterales (la celda alta emite 2 caras: este
        // hacia (2,1) y sur hacia (1,2), ambas tienen z=0).
        let tops = plan
            .polygons
            .iter()
            .filter(|p| p.depth.fract() == 0.0)
            .count();
        assert_eq!(tops, 9);
    }

    #[test]
    fn andina_enabled_stacks_extra_layers_on_high_relief() {
        let mut world = World::new(3, 3);
        let center = world.grid.idx(1, 1);
        world.grid.materia[center] = 100.0; // z = 100 >> threshold 1.0
        let cfg = PlanConfig {
            andina_layers: 3,
            andina_threshold: 1.0,
            ..Default::default()
        };
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &cfg);
        // Las 3 capas andinas tienen depths estrictamente menores que el
        // depth del techo (2.0), pero mayores que 1.99 (micro-shift -0.001).
        let andina = plan
            .polygons
            .iter()
            .filter(|p| p.depth > 1.99 && p.depth < 2.0)
            .count();
        assert_eq!(andina, 3, "tres capas andinas extras en la celda alta");
    }

    #[test]
    fn andina_skips_flat_cells_below_threshold() {
        let world = World::new(4, 4); // todas las celdas en z = 0
        let cfg = PlanConfig {
            andina_layers: 3,
            andina_threshold: 1.0,
            ..Default::default()
        };
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &cfg);
        // 16 techos + 0 capas andinas + 0 caras laterales (todo plano).
        assert_eq!(plan.polygons.len(), 16);
    }

    #[test]
    fn z_weights_raise_the_terrain() {
        // Con materia alta y peso de relieve, el techo de la celda sube.
        let mut world = World::new(3, 3);
        let idx = world.grid.idx(1, 1);
        world.grid.materia[idx] = 50.0;
        let flat = build_plan(
            &world,
            &iso(),
            &ZWeights { materia: 0.0, ..ZWeights::default() },
            &PlanConfig::default(),
        );
        let raised = build_plan(
            &world,
            &iso(),
            &ZWeights { materia: 1.0, ..ZWeights::default() },
            &PlanConfig::default(),
        );
        let cfg = PlanConfig::default();
        // El techo coloreado `materia` es único: comparo su Y promedio.
        let pick = |p: &RenderPlan| {
            let top = p
                .polygons
                .iter()
                .find(|pg| pg.color == cfg.palette.materia)
                .unwrap();
            (top.vertices[0].1
                + top.vertices[1].1
                + top.vertices[2].1
                + top.vertices[3].1)
                / 4.0
        };
        assert!(pick(&raised) < pick(&flat), "el relieve sube el techo");
    }

    #[test]
    fn side_face_emitted_when_neighbor_is_lower() {
        // Pico aislado en (1,1) → emite 2 caras laterales (este+sur)
        // hacia las celdas (2,1) y (1,2) que están en z=0.
        let mut world = World::new(3, 3);
        let idx = world.grid.idx(1, 1);
        world.grid.materia[idx] = 50.0;
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        let cfg = PlanConfig::default();
        // Caras: color shade(materia, 0.72) y shade(materia, 0.55).
        let east_color = shade(cfg.palette.materia, 0.72);
        let south_color = shade(cfg.palette.materia, 0.55);
        let east_count = plan
            .polygons
            .iter()
            .filter(|p| p.color == east_color)
            .count();
        let south_count = plan
            .polygons
            .iter()
            .filter(|p| p.color == south_color)
            .count();
        assert_eq!(east_count, 1, "una cara este");
        assert_eq!(south_count, 1, "una cara sur");
    }

    #[test]
    fn side_face_skipped_when_neighbor_is_same_or_higher() {
        // Todas las celdas planas (z=0): ninguna emite caras.
        let world = World::new(4, 4);
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        // Solo techos.
        for p in &plan.polygons {
            assert_eq!(p.depth.fract(), 0.0, "polygon no-techo emitido en plano");
        }
    }

    #[test]
    fn sprite_library_covers_one_to_eight() {
        // Cada id 1..=8 produce ≥1 primitiva; 0 y un desconocido, ninguna.
        for id in 1..=SPRITE_COUNT {
            assert!(
                !sprite_prims(id, 0.0, 0.0, 10.0, [1.0; 4]).is_empty(),
                "sprite {id} debería emitir primitivas"
            );
        }
        assert!(sprite_prims(0, 0.0, 0.0, 10.0, [1.0; 4]).is_empty());
        assert!(sprite_prims(99, 0.0, 0.0, 10.0, [1.0; 4]).is_empty());
    }

    #[test]
    fn known_sprite_emits_vectors_not_glyph() {
        use dominium_core::{Concepto, LayerMods};
        let mut world = World::new(8, 8);
        world.conceptos.add(Concepto {
            id: "iglesia".into(),
            sprite_id: 1, // iglesia → librería vectorial
            pos_x: 4.0,
            pos_y: 4.0,
            radius: 1.5,
            mods: LayerMods::default(),
            hack: None,
            persuasion: None,
        });
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        assert!(!plan.sprites.is_empty(), "el concepto emite sprites");
        assert!(plan.glyphs.is_empty(), "ya no se usa glifo para ids conocidos");
    }

    #[test]
    fn unknown_sprite_falls_back_to_question_glyph() {
        use dominium_core::{Concepto, LayerMods};
        let mut world = World::new(8, 8);
        world.conceptos.add(Concepto {
            id: "raro".into(),
            sprite_id: 99, // desconocido → fallback '?'
            pos_x: 4.0,
            pos_y: 4.0,
            radius: 1.5,
            mods: LayerMods::default(),
            hack: None,
            persuasion: None,
        });
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        assert!(plan.sprites.is_empty());
        assert_eq!(plan.glyphs.len(), 1);
        assert_eq!(plan.glyphs[0].ch, '?');
    }

    #[test]
    fn sprite_id_zero_draws_nothing() {
        use dominium_core::{Concepto, LayerMods};
        let mut world = World::new(8, 8);
        world.conceptos.add(Concepto {
            id: "mudo".into(),
            sprite_id: 0,
            pos_x: 4.0,
            pos_y: 4.0,
            radius: 1.5,
            mods: LayerMods::default(),
            hack: None,
            persuasion: None,
        });
        let plan = build_plan(&world, &iso(), &ZWeights::default(), &PlanConfig::default());
        assert!(plan.sprites.is_empty() && plan.glyphs.is_empty());
    }
}
