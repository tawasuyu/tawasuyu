//! Exporter PDF mínimo — un `RenderPlan` → documento PDF de 1 página.
//!
//! Estrategia: PDF es texto + binary stream. Para los primitivos que
//! produce pineal (rect, polyline, polygon de triangle-strip) basta con
//! emitir un content stream con operadores básicos:
//!
//! - `r g b rg` — set fill color (RGB, 0..1).
//! - `r g b RG` — set stroke color.
//! - `w w` — set line width.
//! - `x y w h re` — append rectangle to path.
//! - `x y m` — moveto.
//! - `x y l` — lineto.
//! - `f` — fill (non-zero).
//! - `S` — stroke.
//! - `q` / `Q` — save / restore graphics state (para clip).
//! - `W n` — clip non-zero + no-op end-path.
//!
//! PDF tiene origen en bottom-left con +Y hacia arriba; pineal trabaja
//! en screen-space (top-left, +Y abajo). Convertimos en el writer:
//! `pdf_y = page_height - y`. El alto/ancho del rect quedan iguales.
//!
//! Texto se omite a propósito (igual que el PNG exporter) — para labels
//! vectoriales usar SVG, que sí emite `<text>`. PDF embedeable de texto
//! requiere font subsetting, complejo y no es lo que se necesita para
//! reportes de chart.
//!
//! Sin compresión flate / sin streams comprimidos: cada export queda
//! human-readable y debugeable. Tamaño aceptable para los volúmenes
//! que un chart produce.

use pineal_render::{Color, Rect, RenderCmd, RenderPlan};
use std::fmt::Write;

/// Convierte un `RenderPlan` a un PDF de página única `width × height`
/// (en puntos PDF, 72 dpi). Devuelve los bytes del documento.
pub fn to_pdf(plan: &RenderPlan, width: f32, height: f32) -> Vec<u8> {
    let content = build_content(plan, height);
    assemble(width, height, &content)
}

fn build_content(plan: &RenderPlan, page_h: f32) -> String {
    let mut s = String::with_capacity(plan.cmds.len() * 64 + 128);
    // Línea por default.
    let mut current_fill = Color::TRANSPARENT;
    let mut current_stroke = Color::TRANSPARENT;
    let mut current_width = -1.0f32;
    for cmd in &plan.cmds {
        match cmd {
            RenderCmd::PushClip(r) => {
                let (x, y, w, h) = rect_to_pdf(*r, page_h);
                let _ = writeln!(s, "q");
                let _ = writeln!(s, "{} {} {} {} re W n", x, y, w, h);
            }
            RenderCmd::PopClip => {
                let _ = writeln!(s, "Q");
            }
            RenderCmd::FillRect { rect, color } => {
                set_fill(&mut s, *color, &mut current_fill);
                let (x, y, w, h) = rect_to_pdf(*rect, page_h);
                let _ = writeln!(s, "{} {} {} {} re f", x, y, w, h);
            }
            RenderCmd::StrokeRect { rect, stroke } => {
                set_stroke(&mut s, stroke.color, &mut current_stroke);
                set_width(&mut s, stroke.width, &mut current_width);
                let (x, y, w, h) = rect_to_pdf(*rect, page_h);
                let _ = writeln!(s, "{} {} {} {} re S", x, y, w, h);
            }
            RenderCmd::StrokeLine { a, b, stroke } => {
                set_stroke(&mut s, stroke.color, &mut current_stroke);
                set_width(&mut s, stroke.width, &mut current_width);
                let _ = writeln!(s, "{} {} m {} {} l S", a.x, page_h - a.y, b.x, page_h - b.y);
            }
            RenderCmd::StrokePolyline { coords, stroke } => {
                if coords.len() < 4 {
                    continue;
                }
                set_stroke(&mut s, stroke.color, &mut current_stroke);
                set_width(&mut s, stroke.width, &mut current_width);
                let _ = writeln!(s, "{} {} m", coords[0], page_h - coords[1]);
                let mut i = 2;
                while i + 1 < coords.len() {
                    let _ = writeln!(s, "{} {} l", coords[i], page_h - coords[i + 1]);
                    i += 2;
                }
                let _ = writeln!(s, "S");
            }
            RenderCmd::FillTriangleStrip { coords, colors } => {
                let n = coords.len() / 2;
                if n < 3 {
                    continue;
                }
                for t in 0..n - 2 {
                    let avg = avg_color(&[
                        colors.get(t).copied(),
                        colors.get(t + 1).copied(),
                        colors.get(t + 2).copied(),
                    ]);
                    set_fill(&mut s, avg, &mut current_fill);
                    let p0 = (coords[t * 2], coords[t * 2 + 1]);
                    let p1 = (coords[(t + 1) * 2], coords[(t + 1) * 2 + 1]);
                    let p2 = (coords[(t + 2) * 2], coords[(t + 2) * 2 + 1]);
                    let _ = writeln!(
                        s,
                        "{} {} m {} {} l {} {} l h f",
                        p0.0,
                        page_h - p0.1,
                        p1.0,
                        page_h - p1.1,
                        p2.0,
                        page_h - p2.1
                    );
                }
            }
            // DrawText: skip a propósito. SVG cubre vectorial-con-texto.
            RenderCmd::DrawText { .. } => {}
        }
    }
    s
}

fn rect_to_pdf(r: Rect, page_h: f32) -> (f32, f32, f32, f32) {
    // PDF rect: x, y, w, h donde (x, y) es la esquina inferior-izquierda.
    let pdf_y = page_h - r.y - r.h;
    (r.x, pdf_y, r.w, r.h)
}

fn set_fill(s: &mut String, c: Color, current: &mut Color) {
    if !same_color(*current, c) {
        let _ = writeln!(s, "{} {} {} rg", c.r.clamp(0.0, 1.0), c.g.clamp(0.0, 1.0), c.b.clamp(0.0, 1.0));
        *current = c;
    }
}

fn set_stroke(s: &mut String, c: Color, current: &mut Color) {
    if !same_color(*current, c) {
        let _ = writeln!(s, "{} {} {} RG", c.r.clamp(0.0, 1.0), c.g.clamp(0.0, 1.0), c.b.clamp(0.0, 1.0));
        *current = c;
    }
}

fn set_width(s: &mut String, w: f32, current: &mut f32) {
    let w = w.max(0.0);
    if (*current - w).abs() > 1e-4 {
        let _ = writeln!(s, "{} w", w);
        *current = w;
    }
}

fn same_color(a: Color, b: Color) -> bool {
    (a.r - b.r).abs() < 1e-4
        && (a.g - b.g).abs() < 1e-4
        && (a.b - b.b).abs() < 1e-4
        && (a.a - b.a).abs() < 1e-4
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

/// Ensambla el documento PDF: header, 4 objetos (catalog, pages, page,
/// content stream), xref y trailer. Cada offset se va anotando para el
/// xref final.
fn assemble(width: f32, height: f32, content: &str) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::with_capacity(content.len() + 512);
    let mut offsets: Vec<usize> = Vec::with_capacity(5);

    buf.extend_from_slice(b"%PDF-1.4\n");
    // Comentario binario obligatorio para PDFs que mezclan binario.
    buf.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");

    offsets.push(buf.len());
    buf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    offsets.push(buf.len());
    buf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

    offsets.push(buf.len());
    let page = format!(
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents 4 0 R \
         /Resources << >> >>\nendobj\n",
        w = width,
        h = height,
    );
    buf.extend_from_slice(page.as_bytes());

    offsets.push(buf.len());
    let content_bytes = content.as_bytes();
    let stream_header = format!("4 0 obj\n<< /Length {} >>\nstream\n", content_bytes.len());
    buf.extend_from_slice(stream_header.as_bytes());
    buf.extend_from_slice(content_bytes);
    buf.extend_from_slice(b"\nendstream\nendobj\n");

    // xref.
    let xref_offset = buf.len();
    buf.extend_from_slice(b"xref\n0 5\n");
    buf.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        let line = format!("{:010} 00000 n \n", off);
        buf.extend_from_slice(line.as_bytes());
    }
    // Trailer.
    let trailer = format!(
        "trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        xref_offset
    );
    buf.extend_from_slice(trailer.as_bytes());
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use pineal_render::{Canvas, Color, Point, StrokeStyle};

    fn sample_plan() -> RenderPlan {
        let mut rec = pineal_render::PlanRecorder::new();
        rec.fill_rect(Rect::new(10.0, 20.0, 100.0, 60.0), Color::from_hex(0xff0000));
        rec.stroke_line(
            Point::new(0.0, 0.0),
            Point::new(200.0, 100.0),
            StrokeStyle::new(2.0, Color::BLACK),
        );
        rec.into_plan()
    }

    #[test]
    fn pdf_starts_with_header() {
        let pdf = to_pdf(&sample_plan(), 300.0, 200.0);
        assert!(pdf.starts_with(b"%PDF-1.4"));
    }

    #[test]
    fn pdf_ends_with_eof() {
        let pdf = to_pdf(&sample_plan(), 300.0, 200.0);
        let tail = &pdf[pdf.len().saturating_sub(8)..];
        assert!(tail.windows(5).any(|w| w == b"%%EOF"));
    }

    #[test]
    fn pdf_contains_required_objects() {
        let pdf = to_pdf(&sample_plan(), 300.0, 200.0);
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains("/Type /Catalog"));
        assert!(s.contains("/Type /Pages"));
        assert!(s.contains("/Type /Page "));
        assert!(s.contains("/MediaBox [0 0 300 200]"));
        assert!(s.contains("xref"));
        assert!(s.contains("startxref"));
    }

    #[test]
    fn fill_rect_emits_re_f() {
        let pdf = to_pdf(&sample_plan(), 300.0, 200.0);
        let s = String::from_utf8_lossy(&pdf);
        // rg para fill, re y f para rect+fill.
        assert!(s.contains("1 0 0 rg"));
        assert!(s.contains("re f"));
    }

    #[test]
    fn stroke_line_emits_m_l_s() {
        let pdf = to_pdf(&sample_plan(), 300.0, 200.0);
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains(" m"));
        assert!(s.contains(" l"));
        assert!(s.contains("S\n") || s.contains("S\r"));
    }

    #[test]
    fn empty_plan_still_well_formed() {
        let pdf = to_pdf(&RenderPlan::default(), 100.0, 50.0);
        assert!(pdf.starts_with(b"%PDF"));
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains("%%EOF"));
    }

    #[test]
    fn triangle_strip_emits_polygon_per_triangle() {
        let mut rec = pineal_render::PlanRecorder::new();
        rec.fill_triangle_strip(
            &[0.0, 0.0, 10.0, 0.0, 5.0, 10.0, 15.0, 10.0],
            &[Color::WHITE; 4],
        );
        let pdf = to_pdf(&rec.into_plan(), 50.0, 50.0);
        let s = String::from_utf8_lossy(&pdf);
        // 4 vértices → 2 triángulos → 2 polígonos (m + 2 l + h + f).
        assert_eq!(s.matches(" h f").count(), 2);
    }

    #[test]
    fn clip_uses_q_and_Q_blocks() {
        let mut rec = pineal_render::PlanRecorder::new();
        rec.push_clip(Rect::new(0.0, 0.0, 50.0, 50.0));
        rec.fill_rect(Rect::new(10.0, 10.0, 20.0, 20.0), Color::BLACK);
        rec.pop_clip();
        let pdf = to_pdf(&rec.into_plan(), 100.0, 100.0);
        let s = String::from_utf8_lossy(&pdf);
        // q al inicio del clip + W n para fijarlo + Q al cerrar.
        assert!(s.contains("q\n"));
        assert!(s.contains(" W n"));
        assert!(s.contains("Q\n"));
    }
}
