//! Primitivas agnósticas de pintura — el `DrawCommand` que cada
//! surface (canvas Llimphi o SVG/Canvas2D del WASM) traduce a su API.

use serde::{Deserialize, Serialize};

/// Color RGBA en `[0.0, 1.0]^4`. Independiente del color-space del
/// surface (no es Hsla de la UI nativa ni hex de CSS). El traductor de
/// surface hace la conversión final.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const TRANSPARENT: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub fn opaque(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }
    pub fn with_alpha(mut self, a: f32) -> Self {
        self.a = a;
        self
    }
    /// Helper para serializar como CSS rgba(...).
    pub fn to_css(&self) -> String {
        format!(
            "rgba({},{},{},{})",
            (self.r * 255.0).round() as u8,
            (self.g * 255.0).round() as u8,
            (self.b * 255.0).round() as u8,
            self.a
        )
    }
}

/// Anchor horizontal del texto. Vertical siempre es `middle` para
/// que el texto se centre verticalmente en `(x, y)`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAnchor {
    Start,
    Middle,
    End,
}

/// Primitiva de pintura agnóstica. La lista de comandos describe
/// **qué** dibujar, no **cómo** — cada surface traduce a su API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DrawCommand {
    /// Círculo (stroke + fill opcional).
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        #[serde(default)]
        stroke: Option<Rgba>,
        #[serde(default)]
        fill: Option<Rgba>,
        #[serde(default = "default_stroke_width")]
        stroke_w: f32,
    },
    /// Segmento de línea con dash opcional.
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: Rgba,
        #[serde(default = "default_stroke_width")]
        width: f32,
        /// `Some((on, off))` para dash. None = sólido.
        #[serde(default)]
        dash: Option<(f32, f32)>,
    },
    /// Texto en `(x, y)`, anchor horizontal configurable.
    Text {
        x: f32,
        y: f32,
        content: String,
        color: Rgba,
        size: f32,
        #[serde(default = "default_anchor")]
        anchor: TextAnchor,
    },
    /// Polígono cerrado — lista de vértices, con relleno y/o trazo.
    Polygon {
        points: Vec<(f32, f32)>,
        #[serde(default)]
        fill: Option<Rgba>,
        #[serde(default)]
        stroke: Option<Rgba>,
        #[serde(default = "default_stroke_width")]
        stroke_w: f32,
    },
    /// Path geométrico arbitrario en sintaxis SVG (atributo `d`).
    /// Cada surface lo parsea con su API (kurbo
    /// `BezPath::from_svg` en el canvas Llimphi, atributo `d`
    /// directo en el SVG exporter). Lo usamos para los glyphs
    /// astrológicos — los unicode ☉☽♈♉ tienen cobertura parcial en
    /// fuentes del sistema, así que dibujamos los símbolos como
    /// geometría agnóstica de fuente.
    Path {
        d: String,
        #[serde(default)]
        stroke: Option<Rgba>,
        #[serde(default)]
        fill: Option<Rgba>,
        #[serde(default = "default_stroke_width")]
        stroke_w: f32,
    },
    /// Disco con relleno de **gradiente radial** — `inner` en el centro,
    /// `outer` en el borde (`r`). Pensado para profundidad / vignette: un
    /// `outer` con alpha 0 funde el lienzo con el fondo. El SVG exporter
    /// lo aproxima con un `radialGradient`.
    RadialGradient {
        cx: f32,
        cy: f32,
        r: f32,
        inner: Rgba,
        outer: Rgba,
    },
}

fn default_stroke_width() -> f32 {
    1.0
}
fn default_anchor() -> TextAnchor {
    TextAnchor::Middle
}

/// Opciones para `compose_wheel` — el caller decide tamaño total,
/// rotación visual, palette (dark/light) y qué overlays acompañar.
#[derive(Debug, Clone)]
pub struct CompositionOpts {
    /// Tamaño total del wheel en px (lado del cuadrado contenedor).
    pub size: f32,
    /// Rotación adicional visual (para jog-dial / transformaciones).
    pub rot_offset_deg: f32,
    /// Si `false`, la lista no incluye los glyphs de cuerpos (útil
    /// para previews compactos).
    pub include_bodies: bool,
    /// Paleta — controla todos los colores del lienzo. Default `dark()`.
    pub palette: crate::Palette,
    /// Si `true`, dibuja la cruz ascensional (líneas ASC↔DESC e
    /// IC↔MC a través del centro) + pills con etiquetas.
    pub draw_ascensional_cross: bool,
    /// Si `true`, dibuja coord labels ("DD°MM'♈") al lado de cada
    /// cuerpo natal.
    pub show_coord_labels: bool,
    /// Mostrar líneas de aspectos menores (semisextile, quincunx, etc.).
    pub show_minor_aspects: bool,
    /// Activa relieve 3D del dial (varios strokes concéntricos con
    /// alpha decreciente para emular bevel).
    pub dial_3d: bool,
    /// Cuerpo natal seleccionado (símbolo agnóstico: `"sun"`, `"venus"`,
    /// …). Cuando hay selección activa, los cuerpos no relacionados y
    /// las líneas de aspecto que no involucran al seleccionado se
    /// atenúan (alpha = `0.18`) — el ojo del usuario va al cuerpo y
    /// sus relaciones.
    pub selected_body: Option<String>,
    /// Factor de "detalle" del zoom: escala los **radios** (el aro crece
    /// con el zoom, separando los cuerpos), pero los glyphs/textos crecen
    /// mucho menos (≈ `detail^0.35`) y el grosor de las líneas casi nada.
    /// Así el zoom *redibuja con más detalle* en vez de magnificar la
    /// imagen estática. `1.0` = sin zoom.
    pub detail: f32,
}

impl Default for CompositionOpts {
    fn default() -> Self {
        Self {
            size: 600.0,
            rot_offset_deg: 0.0,
            include_bodies: true,
            palette: crate::Palette::dark(),
            draw_ascensional_cross: true,
            show_coord_labels: true,
            show_minor_aspects: false,
            dial_3d: true,
            selected_body: None,
            detail: 1.0,
        }
    }
}

/// Regiones hit-testeables del wheel — los discos de los cuerpos
/// natales en sus posiciones de display (post-spread). El canvas las
/// usa para mapear un click en (x, y) local del wheel al cuerpo
/// correspondiente. La lista la emite `compose_wheel_with_hits`
/// junto con los DrawCommands; el caller la guarda y testea contra
/// ella cuando llega un evento de click.
#[derive(Debug, Clone, Default)]
pub struct WheelHits {
    /// Cada entry: `(symbol, cx_screen, cy_screen, hit_radius)` — el
    /// `hit_radius` ya incluye un margen extra sobre el disco visual
    /// (≈ 1.6 × disk) para que el usuario no tenga que apuntar al
    /// pixel exacto.
    pub bodies: Vec<(String, f32, f32, f32)>,
}

impl WheelHits {
    /// Encuentra el cuerpo más cercano a `(x, y)` que esté dentro de
    /// su hit radius. Devuelve `None` si el click cayó en vacío.
    pub fn pick(&self, x: f32, y: f32) -> Option<&str> {
        let mut best: Option<(&str, f32)> = None;
        for (sym, cx, cy, r) in &self.bodies {
            let d2 = (x - cx).powi(2) + (y - cy).powi(2);
            if d2 <= r * r {
                let d = d2.sqrt();
                if best.map(|(_, bd)| d < bd).unwrap_or(true) {
                    best = Some((sym.as_str(), d));
                }
            }
        }
        best.map(|(s, _)| s)
    }
}

/// Compone una lista de `DrawCommand`s a partir de un `RenderModel`.
/// Incluye:
/// - Background panel + dial 3D (bevel via strokes concéntricos)
/// - Anillos A/B/C/D/E con sus cusps
/// - Casas topocéntricas (ring B→C) + geocéntricas (ring C→D) cuando
///   están en el modelo
/// - Glyphs zodiacales con color de su elemento
/// - Cuerpos natales con disco coloreado + spread anti-solapamiento
///   + coord labels en pills
/// - Aspectos: width inversa al orbe, filtrado opcional de minors
/// - Cruz ascensional ASC↔DESC + IC↔MC + pills con etiquetas
pub fn compose_wheel(
    model: &crate::RenderModel,
    opts: &CompositionOpts,
) -> Vec<DrawCommand> {
    compose_wheel_with_hits(model, opts).0
}

/// Como [`compose_wheel`], pero además devuelve [`WheelHits`] con las
/// regiones hit-testeables de los cuerpos. El canvas (o cualquier
/// otro surface interactivo) usa los hits para resolver clicks a
/// cuerpos sin tener que re-derivar las posiciones de display.
pub fn compose_wheel_with_hits(
    model: &crate::RenderModel,
    opts: &CompositionOpts,
) -> (Vec<DrawCommand>, WheelHits) {
    use crate::math::{find_clusters, polar_to_screen, spread_angles, Radii};
    let mut out = Vec::new();
    let mut hits = WheelHits::default();

    // Vecinos del cuerpo seleccionado: el conjunto de cuerpos que
    // tienen un aspecto natal con él. Vacío si no hay selección o
    // si no hay aspectos en el modelo.
    let related: std::collections::HashSet<String> =
        if let Some(sel) = opts.selected_body.as_deref() {
            let mut s = std::collections::HashSet::new();
            s.insert(sel.to_string());
            for layer in &model.layers {
                if !matches!(layer.kind, crate::LayerKind::Aspects)
                    || layer.module_id != "natal"
                {
                    continue;
                }
                if let crate::Geometry::Lines(segs) = &layer.geometry {
                    for seg in segs {
                        if seg.from_body == sel {
                            s.insert(seg.to_body.clone());
                        }
                        if seg.to_body == sel {
                            s.insert(seg.from_body.clone());
                        }
                    }
                }
            }
            s
        } else {
            std::collections::HashSet::new()
        };
    let dim_alpha = 0.18_f32;
    let dim = |rgba: Rgba, is_related: bool| -> Rgba {
        if opts.selected_body.is_some() && !is_related {
            rgba.with_alpha(rgba.a * dim_alpha)
        } else {
            rgba
        }
    };
    // Coord labels (planetas natales + cusps geo) los recolectamos
    // acá y los emitimos al final en un solo bloque, agrupados por
    // proximidad. Eso permite (a) un solo label compartido cuando
    // dos glyphs caen a < 5 arcmin de distancia (conjunción exacta
    // entre planetas, o planeta-pega-al-cusp), y (b) posicionar el
    // label fuera del disco del planeta — sin pisarlo.
    let mut coord_items: Vec<CoordItem> = Vec::new();

    let cx = opts.size / 2.0;
    let cy = opts.size / 2.0;
    // Zoom = más detalle: el aro crece con `detail` (separa los cuerpos),
    // pero los glyphs/textos crecen con `body_k` (mucho menos) y el grosor
    // de las líneas no escala (queda fino a cualquier zoom).
    let detail = opts.detail.max(0.1);
    let body_k = detail.powf(0.35);
    let margin = opts.size * 0.05;
    let r_outer = ((opts.size / 2.0) - margin) * detail;
    let radii = Radii::from_outer(r_outer);

    let asc = model.ascendant_deg;
    let rot = opts.rot_offset_deg;
    let pal = &opts.palette;

    // === Fondo del lienzo: profundidad + fundido con el fondo ===
    // 1) Halo exterior que se desvanece más allá del aro — funde la rueda
    //    con el fondo del canvas y le da "espacialidad".
    let bg = pal.bg_panel;
    let depth_inner = {
        let d = if pal.is_dark { 0.06 } else { -0.045 };
        Rgba {
            r: (bg.r + d).clamp(0.0, 1.0),
            g: (bg.g + d).clamp(0.0, 1.0),
            b: (bg.b + d).clamp(0.0, 1.0),
            a: 1.0,
        }
    };
    out.push(DrawCommand::RadialGradient {
        cx,
        cy,
        r: (radii.sign_outer + opts.size * 0.02) * 1.22,
        inner: bg.with_alpha(0.85),
        outer: bg.with_alpha(0.0),
    });
    // 2) Disco del panel con gradiente radial (centro algo más claro →
    //    borde = base): da relieve/profundidad a la rueda.
    out.push(DrawCommand::RadialGradient {
        cx,
        cy,
        r: radii.sign_outer + opts.size * 0.02,
        inner: depth_inner,
        outer: bg,
    });

    // === Dial 3D — relieve via strokes concéntricos cerca de aro A ===
    if opts.dial_3d {
        let bevel_steps: [(f32, f32, f32); 4] = [
            (1.012, 0.18, 0.6), // halo externo
            (1.006, 0.32, 0.9),
            (0.994, 0.40, 1.0),
            (0.988, 0.20, 0.7), // halo interno
        ];
        for (factor, alpha, w) in bevel_steps {
            out.push(DrawCommand::Circle {
                cx,
                cy,
                r: radii.sign_outer * factor,
                stroke: Some(pal.dial_ring.with_alpha(alpha)),
                fill: None,
                stroke_w: w,
            });
        }
    }

    // === Aro A (externo zodiaco) + B (interno) ===
    out.push(DrawCommand::Circle {
        cx,
        cy,
        r: radii.sign_outer,
        stroke: Some(pal.dial_ring),
        fill: None,
        stroke_w: 1.6,
    });
    out.push(DrawCommand::Circle {
        cx,
        cy,
        r: radii.sign_inner,
        stroke: Some(pal.dial_ring.with_alpha(0.7)),
        fill: None,
        stroke_w: 1.0,
    });

    // === Cusps zodiacales cada 30°, sub-divisiones cada 10° (más sutiles) ===
    for i in 0..36 {
        let lon = (i as f32) * 10.0;
        let is_sign_boundary = i % 3 == 0;
        let color = if is_sign_boundary {
            pal.dial_ring
        } else {
            pal.dial_ring.with_alpha(0.30)
        };
        let width = if is_sign_boundary { 1.0 } else { 0.4 };
        let (xi, yi) = polar_to_screen(lon, asc, rot, radii.sign_inner);
        let (xo, yo) = polar_to_screen(lon, asc, rot, radii.sign_outer);
        out.push(DrawCommand::Line {
            x1: cx + xi,
            y1: cy + yi,
            x2: cx + xo,
            y2: cy + yo,
            color,
            width,
            dash: None,
        });
    }

    // === Glyphs zodiacales como geometría ─ color elemental ───────
    // Los unicode ♈♉♊… no están en las fuentes default del sistema
    // (LiberationSans/AdwaitaSans), así que dibujamos los signos como
    // path SVG vía `glyphs::sign_commands`. Cada signo aporta
    // múltiples DrawCommand (Line / Path / Circle) — los apilamos.
    let sign_ring_mid = (radii.sign_outer + radii.sign_inner) / 2.0;
    let sign_glyph_size = opts.size * 0.045 * body_k;
    let sign_stroke_w = (opts.size * 0.0030).max(1.2);
    for layer in &model.layers {
        if !matches!(layer.kind, crate::LayerKind::SignDial) {
            continue;
        }
        for g in &layer.glyphs {
            let (gx, gy) = polar_to_screen(g.deg, asc, rot, sign_ring_mid);
            out.extend(crate::glyphs::sign_commands(
                &g.symbol,
                cx + gx,
                cy + gy,
                sign_glyph_size,
                pal.sign(&g.symbol),
                sign_stroke_w,
            ));
        }
    }

    // === Casas topocéntricas (ring B→C) — si están en el modelo ===
    let topo_outer = radii.topo_houses_outer;
    let topo_inner = radii.topo_houses_inner;
    let topo_ring_color = pal.house_ring();
    let has_topo = model
        .layers
        .iter()
        .any(|l| matches!(l.kind, crate::LayerKind::Houses) && l.module_id == "topocentric");
    if has_topo {
        out.push(DrawCommand::Circle {
            cx,
            cy,
            r: topo_inner,
            stroke: Some(topo_ring_color.with_alpha(0.55)),
            fill: None,
            stroke_w: 0.8,
        });
    }

    // === Casas geocéntricas (ring C→D) ===
    let house_outer_r = radii.houses_outer;
    let house_inner_r = radii.houses_inner;
    out.push(DrawCommand::Circle {
        cx,
        cy,
        r: house_outer_r,
        stroke: Some(pal.house_cusp),
        fill: None,
        stroke_w: 1.0,
    });
    out.push(DrawCommand::Circle {
        cx,
        cy,
        r: house_inner_r,
        stroke: Some(pal.house_cusp),
        fill: None,
        stroke_w: 1.0,
    });

    // === Tinte translúcido de las casas geocéntricas ===
    // Cada sector se colorea por el color del lore de SU casa (Casa I =
    // color de Aries, II = Tauro, …), con opacidad baja — da a cada casa
    // su identidad de dominio sin tapar lo que va encima.
    for layer in &model.layers {
        if !matches!(layer.kind, crate::LayerKind::Houses) || layer.module_id == "topocentric" {
            continue;
        }
        if let crate::Geometry::Ring { cusps_deg } = &layer.geometry {
            let n = cusps_deg.len();
            for i in 0..n {
                let from = cusps_deg[i];
                let to = cusps_deg[(i + 1) % n];
                let casa = pal.house(i);
                let pts = house_sector_points(
                    cx, cy, from, to, asc, rot, house_inner_r, house_outer_r,
                );
                out.push(DrawCommand::Polygon {
                    points: pts,
                    fill: Some(casa.with_alpha(0.12)),
                    stroke: None,
                    stroke_w: 0.0,
                });
            }
        }
    }

    // Draws cusps + numbers for both house systems (topo + geo) in their respective rings.
    // Para el sistema geocéntrico además emitimos la coordenada DD°MM'<Sg>
    // de cada cusp justo afuera del aro de casas — así el usuario lee la
    // posición del cusp sin tener que cruzar con el dial zodiacal. Para
    // el topo lo omitimos (compartirían cusps cercanos y duplicarían).
    for layer in &model.layers {
        if !matches!(layer.kind, crate::LayerKind::Houses) {
            continue;
        }
        let (ring_outer, ring_inner, base_color, label_color, is_geo) =
            match layer.module_id.as_str() {
                "topocentric" => {
                    (topo_outer, topo_inner, topo_ring_color, topo_ring_color, false)
                }
                _ => (house_outer_r, house_inner_r, pal.house_cusp, pal.fg_muted, true),
            };
        if let crate::Geometry::Ring { cusps_deg } = &layer.geometry {
            for (i, c) in cusps_deg.iter().enumerate() {
                let is_angle = i == 0 || i == 3 || i == 6 || i == 9;
                let color = if is_angle {
                    pal.angle_highlight
                } else {
                    base_color
                };
                let width = if is_angle { 1.8 } else { 0.8 };
                let (xi, yi) = polar_to_screen(*c, asc, rot, ring_inner);
                let (xo, yo) = polar_to_screen(*c, asc, rot, ring_outer);
                out.push(DrawCommand::Line {
                    x1: cx + xi,
                    y1: cy + yi,
                    x2: cx + xo,
                    y2: cy + yo,
                    color,
                    width,
                    dash: None,
                });
                // Recolectamos el cusp como CoordItem — la emisión
                // del label va al final, con cluster + posicionamiento
                // consciente del disco. Sólo cusps geo (los topo
                // saturarían el aro con duplicados cercanos).
                if is_geo && opts.show_coord_labels {
                    coord_items.push(CoordItem {
                        raw_deg: *c,
                        disp_deg: *c,
                        is_planet: false,
                        body_ring: 0.0,
                        disk_r: 0.0,
                    });
                }
            }
        }
        // House numbers en el centro del ring
        let label_r = (ring_outer + ring_inner) / 2.0;
        for g in &layer.glyphs {
            if let Some(h) = g.house {
                let (gx, gy) = polar_to_screen(g.deg, asc, rot, label_r);
                out.push(DrawCommand::Text {
                    x: cx + gx,
                    y: cy + gy,
                    content: format!("{}", h),
                    color: label_color,
                    size: opts.size * 0.018 * body_k,
                    anchor: TextAnchor::Middle,
                });
            }
        }
    }

    // === Cuerpos por módulo (natal, topocentric, transit, progresión…) ===
    // Spread anti-solapamiento + clusters compartidos + coord labels.
    // Cada body-layer se renderea en su ring canónico.
    let mut natal_display_by_body: std::collections::HashMap<String, f32> =
        std::collections::HashMap::new();
    if opts.include_bodies {
        for layer in &model.layers {
            if !matches!(layer.kind, crate::LayerKind::Bodies) {
                continue;
            }
            let ring = radii.body_ring(&layer.module_id);
            let is_natal = layer.module_id == "natal";
            // Spread: separación mínima ~10°, shift máximo ~12°.
            let raw_degs: Vec<f32> = layer.glyphs.iter().map(|g| g.deg).collect();
            let (display_degs, residual) = spread_angles(&raw_degs, 10.0, 12.0);
            // Clusters para encoger discos
            let clusters = find_clusters(&raw_degs, 9.0);
            let mut cluster_size: Vec<usize> = vec![1; layer.glyphs.len()];
            for c in &clusters {
                for &i in c {
                    cluster_size[i] = c.len();
                }
            }
            // Disco base y escala por cluster size. Proporción reducida
            // respecto a versiones previas: con cuerpos más chicos el zoom
            // desenmaraña mejor las conjunciones apretadas. `body_k` los
            // hace crecer poco con el zoom (el aro crece mucho más).
            let base_disk = opts.size * 0.0175 * body_k;
            let base_font = opts.size * 0.023 * body_k;
            for (i, g) in layer.glyphs.iter().enumerate() {
                let disp_deg = display_degs[i];
                if is_natal {
                    natal_display_by_body.insert(g.symbol.clone(), disp_deg);
                }
                // Encoge un poco si el cluster es denso o quedó presión residual
                let dense_factor = if cluster_size[i] >= 3 {
                    0.78
                } else if cluster_size[i] == 2 {
                    0.88
                } else {
                    1.0
                };
                let stress_factor = (1.0 - residual * 0.5).max(0.6);
                let disk = base_disk * dense_factor * stress_factor;
                let font = (base_font * dense_factor * stress_factor).max(opts.size * 0.018);

                let (gx, gy) = polar_to_screen(disp_deg, asc, rot, ring);

                // Halo del disco — color del planeta, fill oscuro/claro según tema
                let body_is_related = is_natal
                    && (opts.selected_body.is_none() || related.contains(&g.symbol));
                let body_color = dim(pal.planet(&g.symbol), body_is_related);
                let halo_fill = if pal.is_dark {
                    pal.bg_panel.with_alpha(0.92)
                } else {
                    Rgba::opaque(1.0, 1.0, 1.0).with_alpha(0.92)
                };
                out.push(DrawCommand::Circle {
                    cx: cx + gx,
                    cy: cy + gy,
                    r: disk,
                    stroke: Some(body_color),
                    fill: Some(halo_fill),
                    stroke_w: 1.2,
                });
                // Hit region: disco visual con un margen para que el
                // usuario no tenga que apuntar al pixel exacto. Sólo
                // natal — los overlays se ignoran en el hit-test.
                if is_natal {
                    hits.bodies.push((
                        g.symbol.clone(),
                        cx + gx,
                        cy + gy,
                        disk * 1.6,
                    ));
                }
                // Glyph como geometría (path SVG agnóstico de fuente).
                // El tamaño visual del glyph queda inscripto en el
                // disco; el factor 1.3 lo deja ligeramente más grande
                // que el círculo para que se lea bien.
                let glyph_size = (font * 1.05).min(disk * 2.4);
                let glyph_sw = (font * 0.085).max(1.0);
                out.extend(crate::glyphs::planet_commands(
                    &g.symbol,
                    cx + gx,
                    cy + gy,
                    glyph_size,
                    body_color,
                    glyph_sw,
                ));
                if g.retrograde {
                    out.push(crate::glyphs::retrograde_marker(
                        cx + gx,
                        cy + gy,
                        glyph_size,
                        body_color,
                    ));
                }

                // Recolectamos el planeta como CoordItem — el label va
                // al final, con cluster + posición consciente del disco.
                if opts.show_coord_labels && is_natal {
                    coord_items.push(CoordItem {
                        raw_deg: g.deg,
                        disp_deg,
                        is_planet: true,
                        body_ring: ring,
                        disk_r: disk,
                    });
                }
            }
        }
    }

    // === Coord labels: cluster por proximidad + posición sin pisar ===
    // Agrupa items cuya separación angular bruta sea ≤ COORD_CLUSTER_EPS_DEG
    // (≈5 arcmin — conjunciones exactas). Por cada cluster emite UN
    // label posicionado para no pisar discos de planetas:
    //   - Si el cluster tiene al menos un planeta: label radial INTERIOR
    //     al body ring, con margen extra contra el borde del disco más
    //     grande del cluster.
    //   - Si solo cusps: label entre houses_outer y sign_inner.
    if opts.show_coord_labels && !coord_items.is_empty() {
        emit_coord_labels(
            &mut out,
            &coord_items,
            &radii,
            opts,
            pal,
            asc,
            rot,
            cx,
            cy,
        );
    }

    // === Anillo de aspectos + líneas ===
    out.push(DrawCommand::Circle {
        cx,
        cy,
        r: radii.aspects,
        stroke: Some(pal.fg_muted.with_alpha(0.35)),
        fill: None,
        stroke_w: 0.6,
    });
    for layer in &model.layers {
        if !matches!(layer.kind, crate::LayerKind::Aspects) {
            continue;
        }
        let (ring_a, ring_b) = radii.aspect_endpoints(&layer.module_id);
        if let crate::Geometry::Lines(segs) = &layer.geometry {
            for seg in segs {
                // Filtrar menores si opt off
                let is_minor = !matches!(
                    seg.kind.as_str(),
                    "conjunction" | "sextile" | "square" | "trine" | "opposition"
                );
                if is_minor && !opts.show_minor_aspects {
                    continue;
                }
                // Endpoints: si tenemos display_deg natal, usarlo para que la
                // línea apunte al cuerpo "spread", no al "raw". Cae al raw
                // si no hay match (overlays sin natal de un lado).
                let from_deg = natal_display_by_body
                    .get(&seg.from_body)
                    .copied()
                    .unwrap_or(seg.from_deg);
                let to_deg = natal_display_by_body
                    .get(&seg.to_body)
                    .copied()
                    .unwrap_or(seg.to_deg);
                let (ax, ay) = polar_to_screen(from_deg, asc, rot, ring_a);
                let (bx, by) = polar_to_screen(to_deg, asc, rot, ring_b);
                // Intensidad por cercanía del orbe: aspecto exacto (orbe 0)
                // = fuerte (grueso + opaco), aspecto holgado = tenue. Escala
                // sobre un orbe de referencia de 8°.
                let intensity = (1.0 - seg.orb_deg.abs() / 8.0).clamp(0.12, 1.0);
                let alpha = (seg.opacity).clamp(0.0, 1.0) * (0.30 + 0.70 * intensity);
                // Width: 0.5 (holgado) → 3.0 (exacto), contraste marcado.
                let width = 0.5 + 2.5 * intensity;
                // Atenúa cuando hay selección y este aspecto no involucra al cuerpo elegido.
                let aspect_is_related = match opts.selected_body.as_deref() {
                    Some(sel) => seg.from_body == sel || seg.to_body == sel,
                    None => true,
                };
                let line_color = dim(
                    pal.aspect(&seg.kind).with_alpha(alpha),
                    aspect_is_related,
                );
                out.push(DrawCommand::Line {
                    x1: cx + ax,
                    y1: cy + ay,
                    x2: cx + bx,
                    y2: cy + by,
                    color: line_color,
                    width,
                    dash: None,
                });
            }
        }
    }

    // === Cruz ascensional + pills ASC/MC/DESC/IC ===
    if opts.draw_ascensional_cross {
        let cross_r = radii.aspects * 0.96;
        let angles: [(f32, &str); 4] = [
            (model.ascendant_deg, "Asc"),
            (model.descendant_deg, "Desc"),
            (model.midheaven_deg, "MC"),
            (model.imum_coeli_deg, "IC"),
        ];
        // Asc↔Desc + IC↔MC — dos líneas finas a través del centro
        for (a, b) in [
            (model.ascendant_deg, model.descendant_deg),
            (model.imum_coeli_deg, model.midheaven_deg),
        ] {
            let (ax, ay) = polar_to_screen(a, asc, rot, cross_r);
            let (bx, by) = polar_to_screen(b, asc, rot, cross_r);
            out.push(DrawCommand::Line {
                x1: cx + ax,
                y1: cy + ay,
                x2: cx + bx,
                y2: cy + by,
                color: pal.angle_highlight.with_alpha(0.35),
                width: 0.8,
                dash: Some((4.0, 4.0)),
            });
        }
        // Pills — label justo afuera del sign_outer
        let pill_r = radii.sign_outer + opts.size * 0.025;
        for (deg, label) in angles {
            let (gx, gy) = polar_to_screen(deg, asc, rot, pill_r);
            out.push(DrawCommand::Text {
                x: cx + gx,
                y: cy + gy,
                content: label.into(),
                color: pal.angle_highlight,
                size: opts.size * 0.022 * body_k,
                anchor: TextAnchor::Middle,
            });
        }
    }

    (out, hits)
}

/// Sirve los `DrawCommand`s como un documento SVG completo.
/// Devuelve un `String` listo para `innerHTML = ...` o file.
pub fn draw_commands_to_svg(commands: &[DrawCommand], size: f32) -> String {
    let mut s = String::with_capacity(8192);
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{0}\" height=\"{0}\" viewBox=\"0 0 {0} {0}\">",
        size as i32
    ));
    let mut grad_id = 0usize;
    for cmd in commands {
        match cmd {
            DrawCommand::RadialGradient { cx, cy, r, inner, outer } => {
                let id = format!("rg{grad_id}");
                grad_id += 1;
                s.push_str(&format!(
                    "<defs><radialGradient id=\"{id}\" cx=\"{cx:.2}\" cy=\"{cy:.2}\" r=\"{r:.2}\" gradientUnits=\"userSpaceOnUse\"><stop offset=\"0%\" stop-color=\"{}\"/><stop offset=\"100%\" stop-color=\"{}\"/></radialGradient></defs>",
                    inner.to_css(),
                    outer.to_css(),
                ));
                s.push_str(&format!(
                    "<circle cx=\"{cx:.2}\" cy=\"{cy:.2}\" r=\"{r:.2}\" fill=\"url(#{id})\"/>"
                ));
            }
            DrawCommand::Circle { cx, cy, r, stroke, fill, stroke_w } => {
                let stroke_attr = stroke
                    .map(|c| format!(" stroke=\"{}\" stroke-width=\"{}\"", c.to_css(), stroke_w))
                    .unwrap_or_default();
                let fill_attr = match fill {
                    Some(c) => format!(" fill=\"{}\"", c.to_css()),
                    None => " fill=\"none\"".into(),
                };
                s.push_str(&format!(
                    "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"{:.2}\"{}{}/>",
                    cx, cy, r, stroke_attr, fill_attr
                ));
            }
            DrawCommand::Line { x1, y1, x2, y2, color, width, dash } => {
                let dash_attr = match dash {
                    Some((on, off)) => format!(" stroke-dasharray=\"{},{}\"", on, off),
                    None => String::new(),
                };
                s.push_str(&format!(
                    "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{}\" stroke-width=\"{}\"{}/>",
                    x1, y1, x2, y2, color.to_css(), width, dash_attr
                ));
            }
            DrawCommand::Polygon { points, fill, stroke, stroke_w } => {
                let pts: String = points
                    .iter()
                    .map(|(x, y)| format!("{:.2},{:.2} ", x, y))
                    .collect();
                let fill_attr = match fill {
                    Some(c) => format!(" fill=\"{}\"", c.to_css()),
                    None => " fill=\"none\"".into(),
                };
                let stroke_attr = stroke
                    .map(|c| format!(" stroke=\"{}\" stroke-width=\"{}\"", c.to_css(), stroke_w))
                    .unwrap_or_default();
                s.push_str(&format!(
                    "<polygon points=\"{}\"{}{}/>",
                    pts.trim_end(),
                    fill_attr,
                    stroke_attr
                ));
            }
            DrawCommand::Text { x, y, content, color, size: sz, anchor } => {
                let anchor_attr = match anchor {
                    TextAnchor::Start => "start",
                    TextAnchor::Middle => "middle",
                    TextAnchor::End => "end",
                };
                let escaped = svg_escape(content);
                s.push_str(&format!(
                    "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"{:.2}\" fill=\"{}\" text-anchor=\"{}\" dominant-baseline=\"central\">{}</text>",
                    x, y, sz, color.to_css(), anchor_attr, escaped
                ));
            }
            DrawCommand::Path { d, stroke, fill, stroke_w } => {
                let stroke_attr = stroke
                    .map(|c| format!(" stroke=\"{}\" stroke-width=\"{}\"", c.to_css(), stroke_w))
                    .unwrap_or_default();
                let fill_attr = match fill {
                    Some(c) => format!(" fill=\"{}\"", c.to_css()),
                    None => " fill=\"none\"".into(),
                };
                // El `d` ya viene en sintaxis SVG estándar — sólo
                // escapamos las comillas dobles por si acaso (los
                // valores numéricos no las contienen).
                s.push_str(&format!(
                    "<path d=\"{}\"{}{} stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
                    d.replace('"', "&quot;"),
                    stroke_attr,
                    fill_attr
                ));
            }
        }
    }
    s.push_str("</svg>");
    s
}

/// Vértices del polígono de un sector de casa (anillo entre `r_in` y
/// `r_out`, del cusp `from_deg` al `to_deg` en sentido zodiacal). Aproxima
/// los arcos con segmentos cada ~4°.
#[allow(clippy::too_many_arguments)]
fn house_sector_points(
    cx: f32,
    cy: f32,
    from_deg: f32,
    to_deg: f32,
    asc: f32,
    rot: f32,
    r_in: f32,
    r_out: f32,
) -> Vec<(f32, f32)> {
    use crate::math::polar_to_screen;
    let span = (to_deg - from_deg).rem_euclid(360.0);
    let steps = ((span / 4.0).ceil() as usize).max(2);
    let mut pts = Vec::with_capacity(steps * 2 + 2);
    for k in 0..=steps {
        let d = from_deg + span * (k as f32 / steps as f32);
        let (x, y) = polar_to_screen(d, asc, rot, r_out);
        pts.push((cx + x, cy + y));
    }
    for k in 0..=steps {
        let d = from_deg + span * (1.0 - k as f32 / steps as f32);
        let (x, y) = polar_to_screen(d, asc, rot, r_in);
        pts.push((cx + x, cy + y));
    }
    pts
}

fn svg_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// =====================================================================
// Coord labels — cluster y emisión
// =====================================================================

/// Tolerancia angular para fusionar items en el mismo cluster de
/// label. 5 arcmin = `5/60` grados ≈ 0.083°. Captura conjunciones
/// "exactas" (planetas dentro de unos minutos de arco entre sí), y
/// planeta-pega-al-cusp cuando un cuerpo está justo sobre una casa.
const COORD_CLUSTER_EPS_DEG: f32 = 5.0 / 60.0;

/// Item a etiquetar — puede ser un planeta (con su ring y disco) o
/// un cusp de casa. Se acumulan en compose_wheel y se procesan al
/// final con [`emit_coord_labels`].
struct CoordItem {
    /// Grado real (sin spread anti-solapamiento). Se usa para el
    /// texto del label (`format_coord_compact`) y para el clustering.
    raw_deg: f32,
    /// Grado de display (post-spread) — se usa para posicionar el
    /// label cerca del glyph efectivo, no del crudo. Para cusps =
    /// raw_deg (no hay spread).
    disp_deg: f32,
    is_planet: bool,
    /// Ring radial donde vive el cuerpo (sólo si is_planet).
    body_ring: f32,
    /// Radio del disco del cuerpo (sólo si is_planet) — el label
    /// se aleja del disco al menos `2.0 * disk_r` para no pisarlo.
    disk_r: f32,
}

/// Posiciona los coord labels en clusters de proximidad. Toma la
/// lista cruda, la sortea y agrupa por proximidad angular ≤
/// `COORD_CLUSTER_EPS_DEG` (con wrap-around 0°↔360°). Por cluster
/// emite **un** Text command — la coord aparece una sola vez aún
/// si hay varios glyphs ahí.
#[allow(clippy::too_many_arguments)]
fn emit_coord_labels(
    out: &mut Vec<DrawCommand>,
    items: &[CoordItem],
    radii: &crate::math::Radii,
    opts: &CompositionOpts,
    pal: &crate::palette::Palette,
    asc: f32,
    rot: f32,
    cx: f32,
    cy: f32,
) {
    use crate::math::{format_coord_compact, polar_to_screen};

    // Sortear por raw_deg, manteniendo índices originales.
    let mut sorted: Vec<&CoordItem> = items.iter().collect();
    sorted.sort_by(|a, b| {
        a.raw_deg
            .partial_cmp(&b.raw_deg)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Clusters consecutivos por proximidad.
    let mut groups: Vec<Vec<&CoordItem>> = Vec::new();
    for it in sorted {
        let push_new = match groups.last() {
            Some(g) => {
                let last_deg = g.last().unwrap().raw_deg;
                (it.raw_deg - last_deg).abs() > COORD_CLUSTER_EPS_DEG
            }
            None => true,
        };
        if push_new {
            groups.push(vec![it]);
        } else {
            groups.last_mut().unwrap().push(it);
        }
    }
    // Wrap-around: fusionar primer y último cluster si tocan a través
    // del 0°/360°.
    if groups.len() >= 2 {
        let first_deg = groups.first().unwrap().first().unwrap().raw_deg;
        let last_deg = groups.last().unwrap().last().unwrap().raw_deg;
        if (360.0 - last_deg + first_deg).abs() <= COORD_CLUSTER_EPS_DEG {
            let tail = groups.pop().unwrap();
            let head = groups.remove(0);
            let merged: Vec<&CoordItem> = tail.into_iter().chain(head).collect();
            groups.insert(0, merged);
        }
    }

    // Emisión por cluster.
    for group in &groups {
        // Coord string: usamos el grado del primer item del cluster
        // — todos están a ≤5 arcmin, el formato a precisión de minuto
        // es idéntico para todos.
        let coord_str = format_coord_compact(group[0].raw_deg);

        // Ángulo de display: promedio de los disp_deg (que ya
        // incorporan el spread anti-solape de los planetas).
        let disp_deg = mean_angle(group.iter().map(|i| i.disp_deg));

        let has_planet = group.iter().any(|i| i.is_planet);
        let label_ring = if has_planet {
            // Posición: justo bajo el borde inferior (radial-interior)
            // del disco más grande del cluster + un margen visual
            // (≈ medio alto de texto + 2 px). El gap natal-aspects es
            // estrecho (~0.08·r), así que dejamos al label rozar
            // levemente el aro de aspectos antes que pisar el disco
            // del planeta.
            let body_ring = group
                .iter()
                .filter(|i| i.is_planet)
                .map(|i| i.body_ring)
                .fold(0.0_f32, f32::max);
            let max_disk = group
                .iter()
                .filter(|i| i.is_planet)
                .map(|i| i.disk_r)
                .fold(0.0_f32, f32::max);
            let target = body_ring - max_disk - opts.size * 0.015;
            target.max(radii.aspects - opts.size * 0.005)
        } else {
            // Cusp-only: zona libre entre house ring y dial zodiacal.
            (radii.houses_outer + radii.sign_inner) * 0.5
        };

        let (lx, ly) = polar_to_screen(disp_deg, asc, rot, label_ring);
        // Texto a mayor contraste (fg_text) y un poco más grande, sobre una
        // píldora de fondo semitransparente para que se lea sobre cualquier
        // anillo o línea de aspecto que pase por detrás.
        let color = if has_planet { pal.fg_text } else { pal.house_cusp };
        let fsize = opts.size * 0.018 * opts.detail.max(0.1).powf(0.35);
        let lcx = cx + lx;
        let lcy = cy + ly;
        let half_w = coord_str.chars().count() as f32 * fsize * 0.32 + fsize * 0.3;
        let half_h = fsize * 0.62;
        out.push(DrawCommand::Polygon {
            points: vec![
                (lcx - half_w, lcy - half_h),
                (lcx + half_w, lcy - half_h),
                (lcx + half_w, lcy + half_h),
                (lcx - half_w, lcy + half_h),
            ],
            fill: Some(pal.bg_panel.with_alpha(0.72)),
            stroke: None,
            stroke_w: 0.0,
        });
        out.push(DrawCommand::Text {
            x: lcx,
            y: lcy,
            content: coord_str,
            color,
            size: fsize,
            anchor: TextAnchor::Middle,
        });
    }
}

/// Promedio circular de ángulos en grados — convierte a vectores
/// unitarios, suma, y vuelve a polar. Imprescindible para promediar
/// 359° y 1° y obtener 0°, no 180°.
fn mean_angle<I: IntoIterator<Item = f32>>(iter: I) -> f32 {
    let (mut sx, mut sy, mut n) = (0.0_f32, 0.0_f32, 0_u32);
    for a in iter {
        let r = a.to_radians();
        sx += r.cos();
        sy += r.sin();
        n += 1;
    }
    if n == 0 {
        return 0.0;
    }
    let mean_rad = sy.atan2(sx);
    let deg = mean_rad.to_degrees();
    if deg < 0.0 { deg + 360.0 } else { deg }
}

/// Etiqueta corta de un signo zodiacal — 3 letras ASCII en mayúscula.
///
/// **Por qué letras y no unicode** (♈♉…): muchas fuentes del sistema
/// (LiberationSans / AdwaitaSans del default Arch/Linux) **no traen**
/// el bloque `U+2648..U+2653`, así que el glyph caía como `.notdef`
/// invisible o como cuadrito; parley/fontique no encuentra fallback
/// porque tampoco hay font de símbolos instalada. Las letras renderan
/// en cualquier fuente sans-serif y mantienen el grado de
/// identificación (`ARI` lee igual que ♈ para un astrólogo).
pub(crate) fn sign_unicode(name: &str) -> &'static str {
    match name {
        "aries" => "ARI",
        "taurus" => "TAU",
        "gemini" => "GEM",
        "cancer" => "CAN",
        "leo" => "LEO",
        "virgo" => "VIR",
        "libra" => "LIB",
        "scorpio" => "SCO",
        "sagittarius" => "SAG",
        "capricorn" => "CAP",
        "aquarius" => "AQU",
        "pisces" => "PIS",
        _ => "?",
    }
}

/// Etiqueta corta de un cuerpo — código alfabético (Su/Mo/Me/Ve/Ma/
/// Ju/Sa/Ur/Ne/Pl/Ch/NN/SN/Li). Misma razón que [`sign_unicode`]:
/// los símbolos planetarios unicode tienen cobertura parcial en
/// fuentes del sistema (Liberation tiene ♀♂♃♄♅♆♇ pero no ☉☽), así
/// que el usuario veía sólo Venus y Marte. Letras = visible siempre.
fn planet_unicode(name: &str) -> &'static str {
    match name {
        "sun" => "Su",
        "moon" => "Mo",
        "mercury" => "Me",
        "venus" => "Ve",
        "mars" => "Ma",
        "jupiter" => "Ju",
        "saturn" => "Sa",
        "uranus" => "Ur",
        "neptune" => "Ne",
        "pluto" => "Pl",
        "north_node" => "NN",
        "south_node" => "SN",
        "chiron" => "Ch",
        "lilith" => "Li",
        _ => "·",
    }
}

/// Glyph del cuerpo con sufijo "R" si está retrógrado — concatenación
/// directa en el text para no agregar más comandos por planeta.
pub(crate) fn planet_unicode_with_retro(name: &str, retrograde: bool) -> String {
    if retrograde {
        format!("{}R", planet_unicode(name))
    } else {
        planet_unicode(name).to_string()
    }
}
