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
        }
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
    use crate::math::{find_clusters, format_coord_compact, polar_to_screen, spread_angles, Radii};
    let mut out = Vec::new();
    // Cuando dos glyphs caen en (casi) la misma coordenada (planeta ↔
    // planeta o planeta ↔ cusp de casa con la misma DD°MM'<Sg>), el
    // coord label de la segunda aparición se suprime — el usuario lee
    // la coordenada una sola vez. Comparamos por el string ya
    // formateado (precisión de minuto) en vez de por grados crudos
    // para que la dedupe ocurra exactamente cuando la etiqueta sería
    // idéntica visualmente.
    let mut emitted_coords: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    let cx = opts.size / 2.0;
    let cy = opts.size / 2.0;
    let margin = opts.size * 0.05;
    let r_outer = (opts.size / 2.0) - margin;
    let radii = Radii::from_outer(r_outer);

    let asc = model.ascendant_deg;
    let rot = opts.rot_offset_deg;
    let pal = &opts.palette;

    // === Background panel ===
    out.push(DrawCommand::Circle {
        cx,
        cy,
        r: radii.sign_outer + opts.size * 0.02,
        stroke: None,
        fill: Some(pal.bg_panel),
        stroke_w: 0.0,
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

    // === Glyphs zodiacales con color elemental ===
    let sign_ring_mid = (radii.sign_outer + radii.sign_inner) / 2.0;
    for layer in &model.layers {
        if !matches!(layer.kind, crate::LayerKind::SignDial) {
            continue;
        }
        for g in &layer.glyphs {
            let (gx, gy) = polar_to_screen(g.deg, asc, rot, sign_ring_mid);
            out.push(DrawCommand::Text {
                x: cx + gx,
                y: cy + gy,
                content: sign_unicode(&g.symbol).into(),
                color: pal.sign(&g.symbol),
                size: opts.size * 0.032,
                anchor: TextAnchor::Middle,
            });
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
                // Coord label del cusp — sólo geocéntrico, sólo cuando
                // `show_coord_labels` está activo. Lo posicionamos entre
                // el aro de casas y el dial zodiacal (zona libre, sin
                // glyphs compitiendo). Dedupe por string formateado.
                if is_geo && opts.show_coord_labels {
                    let coord_str = format_coord_compact(*c);
                    if emitted_coords.insert(coord_str.clone()) {
                        let cusp_label_r = (ring_outer + radii.sign_inner) / 2.0;
                        let (lx, ly) = polar_to_screen(*c, asc, rot, cusp_label_r);
                        out.push(DrawCommand::Text {
                            x: cx + lx,
                            y: cy + ly,
                            content: coord_str,
                            color: label_color,
                            size: opts.size * 0.0165,
                            anchor: TextAnchor::Middle,
                        });
                    }
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
                    size: opts.size * 0.018,
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
            // Disco base y escala por cluster size
            let base_disk = opts.size * 0.022;
            let base_font = opts.size * 0.028;
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
                let body_color = pal.planet(&g.symbol);
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
                // Glyph
                out.push(DrawCommand::Text {
                    x: cx + gx,
                    y: cy + gy,
                    content: planet_unicode_with_retro(&g.symbol, g.retrograde),
                    color: body_color,
                    size: font,
                    anchor: TextAnchor::Middle,
                });

                // Coord label en pill — sólo natal (los overlays se
                // amontonarían con el natal). Dedupe contra las coords
                // ya emitidas por house cusps + previos planetas: si dos
                // glyphs caen en el mismo `DD°MM'<Sg>`, la coordenada se
                // ve una sola vez. La etiqueta va INTERIOR al ring del
                // disco (entre el disco y el aro de aspectos) para no
                // pisar al glyph del planeta ni al cuerpo vecino.
                if opts.show_coord_labels && is_natal {
                    let coord_str = format_coord_compact(g.deg);
                    if emitted_coords.insert(coord_str.clone()) {
                        let label_ring = (ring - disk * 1.8).max(radii.aspects + opts.size * 0.012);
                        let (lx, ly) = polar_to_screen(disp_deg, asc, rot, label_ring);
                        out.push(DrawCommand::Text {
                            x: cx + lx,
                            y: cy + ly,
                            content: coord_str,
                            color: pal.fg_muted,
                            size: opts.size * 0.0155,
                            anchor: TextAnchor::Middle,
                        });
                    }
                }
            }
        }
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
                let alpha = (seg.opacity).clamp(0.0, 1.0);
                // Width inversa al orbe — orbe 0 → 1.6, orbe 10° → 0.5
                let width = (1.6 - seg.orb_deg.abs() * 0.10).clamp(0.45, 1.8);
                out.push(DrawCommand::Line {
                    x1: cx + ax,
                    y1: cy + ay,
                    x2: cx + bx,
                    y2: cy + by,
                    color: pal.aspect(&seg.kind).with_alpha(alpha),
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
                size: opts.size * 0.022,
                anchor: TextAnchor::Middle,
            });
        }
    }

    out
}

/// Sirve los `DrawCommand`s como un documento SVG completo.
/// Devuelve un `String` listo para `innerHTML = ...` o file.
pub fn draw_commands_to_svg(commands: &[DrawCommand], size: f32) -> String {
    let mut s = String::with_capacity(8192);
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{0}\" height=\"{0}\" viewBox=\"0 0 {0} {0}\">",
        size as i32
    ));
    for cmd in commands {
        match cmd {
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
        }
    }
    s.push_str("</svg>");
    s
}

fn svg_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
