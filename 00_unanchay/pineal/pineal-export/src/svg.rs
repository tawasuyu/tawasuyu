//! Exporter SVG: un [`RenderPlan`] → documento SVG completo.
//!
//! El mismo painter que dibuja en pantalla (vía el trait `Canvas`) se
//! graba con un `PlanRecorder` y el plan resultante se vuelca acá. Un
//! solo camino de código para screen y export.
//!
//! v1: los comandos de clip (`PushClip`/`PopClip`) se ignoran — el
//! recorte no es crítico para la mayoría de exports y SVG `clipPath`
//! agrega complejidad de IDs. Se puede agregar después sin romper API.

use pineal_render::{Color, RenderCmd, RenderPlan};
use std::fmt::Write;

/// Convierte un `RenderPlan` a un documento SVG de `width × height`.
pub fn to_svg(plan: &RenderPlan, width: f32, height: f32) -> String {
    let mut s = String::with_capacity(256 + plan.cmds.len() * 80);
    let _ = write!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" \
         height=\"{height}\" viewBox=\"0 0 {width} {height}\">"
    );
    for cmd in &plan.cmds {
        emit_cmd(&mut s, cmd);
    }
    s.push_str("</svg>");
    s
}

fn emit_cmd(s: &mut String, cmd: &RenderCmd) {
    match cmd {
        // v1: clips ignorados (ver doc del módulo).
        RenderCmd::PushClip(_) | RenderCmd::PopClip => {}

        RenderCmd::FillRect { rect, color } => {
            let (c, a) = svg_color(*color);
            let _ = write!(
                s,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" \
                 fill=\"{c}\" fill-opacity=\"{a}\"/>",
                rect.x, rect.y, rect.w, rect.h
            );
        }

        RenderCmd::StrokeRect { rect, stroke } => {
            let (c, a) = svg_color(stroke.color);
            let _ = write!(
                s,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" \
                 fill=\"none\" stroke=\"{c}\" stroke-opacity=\"{a}\" \
                 stroke-width=\"{}\"/>",
                rect.x, rect.y, rect.w, rect.h, stroke.width
            );
        }

        RenderCmd::StrokeLine { a: p0, b: p1, stroke } => {
            let (c, alpha) = svg_color(stroke.color);
            let _ = write!(
                s,
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" \
                 stroke=\"{c}\" stroke-opacity=\"{alpha}\" stroke-width=\"{}\"/>",
                p0.x, p0.y, p1.x, p1.y, stroke.width
            );
        }

        RenderCmd::StrokePolyline { coords, stroke } => {
            let (c, alpha) = svg_color(stroke.color);
            s.push_str("<polyline points=\"");
            emit_points(s, coords);
            let _ = write!(
                s,
                "\" fill=\"none\" stroke=\"{c}\" stroke-opacity=\"{alpha}\" \
                 stroke-width=\"{}\"/>",
                stroke.width
            );
        }

        RenderCmd::FillTriangleStrip { coords, colors } => {
            emit_triangle_strip(s, coords, colors);
        }

        RenderCmd::DrawText { p, text, color, size_px } => {
            let (c, a) = svg_color(*color);
            let _ = write!(
                s,
                "<text x=\"{}\" y=\"{}\" fill=\"{c}\" fill-opacity=\"{a}\" \
                 font-size=\"{size_px}\">{}</text>",
                p.x, p.y, escape_xml(text)
            );
        }
    }
}

/// Emite `x0,y0 x1,y1 …` para el atributo `points` de polyline/polygon.
fn emit_points(s: &mut String, coords: &[f32]) {
    for (i, pair) in coords.chunks_exact(2).enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{},{}", pair[0], pair[1]);
    }
}

/// Un triangle strip de N vértices = N-2 triángulos. Cada triángulo se
/// emite como `<polygon>` con el color promedio de sus 3 vértices (SVG
/// no tiene gradient por-vértice trivial).
fn emit_triangle_strip(s: &mut String, coords: &[f32], colors: &[Color]) {
    let n = coords.len() / 2;
    if n < 3 {
        return;
    }
    for t in 0..n - 2 {
        let (i0, i1, i2) = (t, t + 1, t + 2);
        let avg = avg_color(&[
            colors.get(i0).copied(),
            colors.get(i1).copied(),
            colors.get(i2).copied(),
        ]);
        let (c, a) = svg_color(avg);
        let _ = write!(
            s,
            "<polygon points=\"{},{} {},{} {},{}\" fill=\"{c}\" fill-opacity=\"{a}\"/>",
            coords[i0 * 2], coords[i0 * 2 + 1],
            coords[i1 * 2], coords[i1 * 2 + 1],
            coords[i2 * 2], coords[i2 * 2 + 1],
        );
    }
}

fn avg_color(cs: &[Option<Color>]) -> Color {
    let mut acc = Color::rgba(0.0, 0.0, 0.0, 0.0);
    let mut n = 0.0;
    for c in cs.iter().flatten() {
        acc.r += c.r;
        acc.g += c.g;
        acc.b += c.b;
        acc.a += c.a;
        n += 1.0;
    }
    if n == 0.0 {
        return Color::TRANSPARENT;
    }
    Color::rgba(acc.r / n, acc.g / n, acc.b / n, acc.a / n)
}

/// `Color` f32 → (`rgb(R,G,B)` con enteros 0-255, alpha 0-1).
fn svg_color(c: Color) -> (String, f32) {
    let to255 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    (
        format!("rgb({},{},{})", to255(c.r), to255(c.g), to255(c.b)),
        c.a.clamp(0.0, 1.0),
    )
}

/// Escapa los 5 caracteres especiales de XML en contenido de texto.
fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pineal_render::{Canvas, Point, Rect, StrokeStyle};

    fn sample_plan() -> RenderPlan {
        let mut rec = pineal_render::PlanRecorder::new();
        rec.fill_rect(Rect::new(1.0, 2.0, 30.0, 40.0), Color::from_hex(0xff0000));
        rec.stroke_line(
            Point::new(0.0, 0.0),
            Point::new(100.0, 50.0),
            StrokeStyle::new(2.0, Color::BLACK),
        );
        rec.draw_text(Point::new(5.0, 10.0), "a<b&c", Color::WHITE, 12.0);
        rec.into_plan()
    }

    #[test]
    fn emits_well_formed_svg() {
        let svg = to_svg(&sample_plan(), 200.0, 100.0);
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("width=\"200\""));
        assert!(svg.contains("viewBox=\"0 0 200 100\""));
    }

    #[test]
    fn emits_each_primitive() {
        let svg = to_svg(&sample_plan(), 200.0, 100.0);
        assert!(svg.contains("<rect "));
        assert!(svg.contains("fill=\"rgb(255,0,0)\""));
        assert!(svg.contains("<line "));
        assert!(svg.contains("<text "));
    }

    #[test]
    fn escapes_xml_in_text() {
        let svg = to_svg(&sample_plan(), 200.0, 100.0);
        assert!(svg.contains("a&lt;b&amp;c"));
        assert!(!svg.contains("a<b&c"));
    }

    #[test]
    fn triangle_strip_becomes_polygons() {
        let mut rec = pineal_render::PlanRecorder::new();
        rec.fill_triangle_strip(
            &[0.0, 0.0, 10.0, 0.0, 5.0, 10.0, 15.0, 10.0],
            &[Color::WHITE, Color::WHITE, Color::WHITE, Color::WHITE],
        );
        let svg = to_svg(&rec.into_plan(), 20.0, 20.0);
        // 4 vértices → 2 triángulos → 2 polígonos.
        assert_eq!(svg.matches("<polygon").count(), 2);
    }
}
