//! Primitivas agnósticas de pintura — el `DrawCommand` que cada
//! surface (gpui canvas o SVG/Canvas2D del WASM) traduce a su API.

use serde::{Deserialize, Serialize};

/// Color RGBA en `[0.0, 1.0]^4`. Independiente del color-space del
/// surface (no es Hsla de gpui ni hex de CSS). El traductor de surface
/// hace la conversión final.
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
}

fn default_stroke_width() -> f32 {
    1.0
}
fn default_anchor() -> TextAnchor {
    TextAnchor::Middle
}

/// Opciones para `compose_wheel` — el caller decide tamaño total del
/// wheel y rotación visual. Los colores son simples por ahora;
/// extender después con una palette completa.
#[derive(Debug, Clone)]
pub struct CompositionOpts {
    /// Tamaño total del wheel en px (lado del cuadrado contenedor).
    pub size: f32,
    /// Rotación adicional visual (para jog-dial / transformaciones).
    pub rot_offset_deg: f32,
    /// Si `false`, la lista no incluye los glyphs de cuerpos (útil
    /// para previews compactos).
    pub include_bodies: bool,
}

impl Default for CompositionOpts {
    fn default() -> Self {
        Self {
            size: 600.0,
            rot_offset_deg: 0.0,
            include_bodies: true,
        }
    }
}

/// Compone una lista de `DrawCommand`s a partir de un `RenderModel`.
/// Versión inicial: anillo de signos + cusps cada 30° + house numbers
/// + cuerpos natales. Sin clusters/spread/aspectos (extiende en
/// commits siguientes).
pub fn compose_wheel(
    model: &crate::RenderModel,
    opts: &CompositionOpts,
) -> Vec<DrawCommand> {
    use crate::math::{polar_to_screen, Radii};
    let mut out = Vec::new();

    let cx = opts.size / 2.0;
    let cy = opts.size / 2.0;
    let margin = opts.size * 0.05;
    let r_outer = (opts.size / 2.0) - margin;
    let radii = Radii::from_outer(r_outer);

    let asc = model.ascendant_deg;
    let rot = opts.rot_offset_deg;

    // Colores neutros (en fase próxima los reemplazo por palette real)
    let ink_strong = Rgba::opaque(0.15, 0.15, 0.20);
    let ink_mid = Rgba::opaque(0.45, 0.45, 0.50).with_alpha(0.85);
    let ink_soft = Rgba::opaque(0.55, 0.55, 0.60).with_alpha(0.55);
    let house_color = Rgba::opaque(0.30, 0.55, 0.50).with_alpha(0.85);
    let angle_color = Rgba::opaque(0.85, 0.55, 0.20);

    // === Aro A (externo zodiaco) + B (interno) ===
    out.push(DrawCommand::Circle {
        cx,
        cy,
        r: radii.sign_outer,
        stroke: Some(ink_strong),
        fill: None,
        stroke_w: 1.5,
    });
    out.push(DrawCommand::Circle {
        cx,
        cy,
        r: radii.sign_inner,
        stroke: Some(ink_mid),
        fill: None,
        stroke_w: 1.0,
    });

    // === Cusps zodiacales (12 radios entre sign_inner y sign_outer) ===
    for i in 0..12 {
        let lon = (i as f32) * 30.0;
        let (xi, yi) = polar_to_screen(lon, asc, rot, radii.sign_inner);
        let (xo, yo) = polar_to_screen(lon, asc, rot, radii.sign_outer);
        out.push(DrawCommand::Line {
            x1: cx + xi,
            y1: cy + yi,
            x2: cx + xo,
            y2: cy + yo,
            color: ink_mid,
            width: 1.0,
            dash: None,
        });
    }

    // === Casas: aros + cusps + glyph número ===
    let house_outer_r = radii.houses_outer;
    let house_inner_r = radii.houses_inner;
    out.push(DrawCommand::Circle {
        cx,
        cy,
        r: house_outer_r,
        stroke: Some(house_color),
        fill: None,
        stroke_w: 1.0,
    });
    out.push(DrawCommand::Circle {
        cx,
        cy,
        r: house_inner_r,
        stroke: Some(house_color),
        fill: None,
        stroke_w: 1.0,
    });
    for layer in &model.layers {
        if !matches!(layer.kind, crate::LayerKind::Houses) {
            continue;
        }
        if layer.module_id != "natal" {
            continue;
        }
        if let crate::Geometry::Ring { cusps_deg } = &layer.geometry {
            for (i, c) in cusps_deg.iter().enumerate() {
                let is_angle = i == 0 || i == 3 || i == 6 || i == 9;
                let color = if is_angle { angle_color } else { house_color };
                let width = if is_angle { 2.0 } else { 0.8 };
                let (xi, yi) = polar_to_screen(*c, asc, rot, house_inner_r);
                let (xo, yo) = polar_to_screen(*c, asc, rot, house_outer_r);
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
        }
        // House numbers
        let label_r = (house_outer_r + house_inner_r) / 2.0;
        for g in &layer.glyphs {
            if let Some(h) = g.house {
                let (gx, gy) = polar_to_screen(g.deg, asc, rot, label_r);
                out.push(DrawCommand::Text {
                    x: cx + gx,
                    y: cy + gy,
                    content: format!("{}", h),
                    color: ink_mid,
                    size: opts.size * 0.018,
                    anchor: TextAnchor::Middle,
                });
            }
        }
    }

    // === Glyphs zodiacales ===
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
                color: ink_strong,
                size: opts.size * 0.03,
                anchor: TextAnchor::Middle,
            });
        }
    }

    // === Cuerpos natales (sin spread/cluster — minimal) ===
    if opts.include_bodies {
        for layer in &model.layers {
            if !matches!(layer.kind, crate::LayerKind::Bodies) {
                continue;
            }
            if layer.module_id != "natal" {
                continue;
            }
            let ring = radii.bodies;
            for g in &layer.glyphs {
                let (gx, gy) = polar_to_screen(g.deg, asc, rot, ring);
                // Disco halo
                out.push(DrawCommand::Circle {
                    cx: cx + gx,
                    cy: cy + gy,
                    r: opts.size * 0.022,
                    stroke: Some(ink_strong),
                    fill: Some(Rgba::opaque(0.97, 0.97, 0.97).with_alpha(0.92)),
                    stroke_w: 1.0,
                });
                // Glyph del cuerpo
                out.push(DrawCommand::Text {
                    x: cx + gx,
                    y: cy + gy,
                    content: planet_unicode(&g.symbol).into(),
                    color: ink_strong,
                    size: opts.size * 0.028,
                    anchor: TextAnchor::Middle,
                });
            }
        }
    }

    // === Anillo de aspectos + líneas ===
    out.push(DrawCommand::Circle {
        cx,
        cy,
        r: radii.aspects,
        stroke: Some(ink_soft),
        fill: None,
        stroke_w: 0.7,
    });
    for layer in &model.layers {
        if !matches!(layer.kind, crate::LayerKind::Aspects) {
            continue;
        }
        if let crate::Geometry::Lines(segs) = &layer.geometry {
            for seg in segs {
                let (ax, ay) = polar_to_screen(seg.from_deg, asc, rot, radii.aspects);
                let (bx, by) = polar_to_screen(seg.to_deg, asc, rot, radii.aspects);
                let alpha = (seg.opacity).clamp(0.0, 1.0);
                out.push(DrawCommand::Line {
                    x1: cx + ax,
                    y1: cy + ay,
                    x2: cx + bx,
                    y2: cy + by,
                    color: aspect_color(&seg.kind).with_alpha(alpha),
                    width: 0.9,
                    dash: None,
                });
            }
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

fn sign_unicode(name: &str) -> &'static str {
    match name {
        "aries" => "♈",
        "taurus" => "♉",
        "gemini" => "♊",
        "cancer" => "♋",
        "leo" => "♌",
        "virgo" => "♍",
        "libra" => "♎",
        "scorpio" => "♏",
        "sagittarius" => "♐",
        "capricorn" => "♑",
        "aquarius" => "♒",
        "pisces" => "♓",
        _ => "?",
    }
}

fn planet_unicode(name: &str) -> &'static str {
    match name {
        "sun" => "☉",
        "moon" => "☽",
        "mercury" => "☿",
        "venus" => "♀",
        "mars" => "♂",
        "jupiter" => "♃",
        "saturn" => "♄",
        "uranus" => "♅",
        "neptune" => "♆",
        "pluto" => "♇",
        "north_node" => "☊",
        "south_node" => "☋",
        "chiron" => "⚷",
        "lilith" => "⚸",
        _ => "•",
    }
}

fn aspect_color(kind: &str) -> Rgba {
    match kind {
        "conjunction" => Rgba::opaque(0.85, 0.65, 0.20),
        "sextile" => Rgba::opaque(0.20, 0.55, 0.80),
        "square" => Rgba::opaque(0.90, 0.30, 0.30),
        "trine" => Rgba::opaque(0.30, 0.70, 0.40),
        "opposition" => Rgba::opaque(0.55, 0.30, 0.75),
        _ => Rgba::opaque(0.55, 0.55, 0.60).with_alpha(0.55),
    }
}
