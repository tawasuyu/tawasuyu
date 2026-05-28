//! Exporter PNG: rasteriza un [`RenderPlan`] sobre un buffer RGBA y lo
//! codifica como PNG.
//!
//! La estrategia es la misma que `svg.rs`: el painter grabó cada comando
//! en un plan; acá lo replayeamos contra un rasterizador software
//! propio. No depende de `tiny-skia` ni `cairo` — sólo `png` (ya en el
//! workspace) y aritmética f32. Esto mantiene la cadena
//! `core → render → export` libre de stack gráfico nativo.
//!
//! Cobertura:
//! - `FillRect`, `StrokeRect` — clip al canvas, blast directo.
//! - `StrokeLine` / `StrokePolyline` — línea gruesa por expansión
//!   perpendicular + scanline fill del rectángulo orientado.
//! - `FillTriangleStrip` — `N-2` triángulos, scanline fill con color
//!   promedio por triángulo (mismo trade-off que el exporter SVG).
//! - `DrawText` — *no-op*. PNG export es para gráficos densos
//!   (heatmaps, treemaps, traces); el texto se mete después en
//!   composición. SVG sí emite `<text>` cuando se quiere vectorial.
//! - `PushClip` / `PopClip` — stack de clip-rect activo, AND con el
//!   bound del buffer al escribir.
//!
//! Composición: source-over premultiplicado. Suficiente para overlays
//! semitransparentes (radar fill, palette ramps).

use pineal_render::{Color, Rect, RenderCmd, RenderPlan};
use std::io::Cursor;

/// Convierte un `RenderPlan` a PNG (bytes). `bg` es el color con que se
/// inicializa el canvas (alpha incluido — `Color::TRANSPARENT` da fondo
/// transparente). Devuelve `Err` si la codificación PNG falla.
pub fn to_png(
    plan: &RenderPlan,
    width: u32,
    height: u32,
    bg: Color,
) -> Result<Vec<u8>, png::EncodingError> {
    let mut buf = RasterBuffer::new(width, height, bg);
    let mut clip_stack: Vec<Rect> = Vec::new();
    for cmd in &plan.cmds {
        replay(cmd, &mut buf, &mut clip_stack);
    }
    encode_png(&buf)
}

/// Buffer RGBA8 row-major, +Y hacia abajo (igual que `RenderPlan`).
struct RasterBuffer {
    pixels: Vec<u8>,
    w: u32,
    h: u32,
}

impl RasterBuffer {
    fn new(w: u32, h: u32, bg: Color) -> Self {
        let r = (bg.r.clamp(0.0, 1.0) * 255.0).round() as u8;
        let g = (bg.g.clamp(0.0, 1.0) * 255.0).round() as u8;
        let b = (bg.b.clamp(0.0, 1.0) * 255.0).round() as u8;
        let a = (bg.a.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            pixels.extend_from_slice(&[r, g, b, a]);
        }
        Self { pixels, w, h }
    }

    /// Source-over con premultiplicación. `x`/`y` ya deben caer dentro.
    #[inline]
    fn blend(&mut self, x: u32, y: u32, c: Color) {
        let idx = ((y * self.w + x) * 4) as usize;
        let sa = c.a.clamp(0.0, 1.0);
        if sa <= 0.0 {
            return;
        }
        let sr = c.r.clamp(0.0, 1.0) * sa;
        let sg = c.g.clamp(0.0, 1.0) * sa;
        let sb = c.b.clamp(0.0, 1.0) * sa;
        let inv = 1.0 - sa;
        let dr = self.pixels[idx] as f32 / 255.0;
        let dg = self.pixels[idx + 1] as f32 / 255.0;
        let db = self.pixels[idx + 2] as f32 / 255.0;
        let da = self.pixels[idx + 3] as f32 / 255.0;
        let or = sr + dr * inv;
        let og = sg + dg * inv;
        let ob = sb + db * inv;
        let oa = sa + da * inv;
        self.pixels[idx] = (or.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.pixels[idx + 1] = (og.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.pixels[idx + 2] = (ob.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.pixels[idx + 3] = (oa.clamp(0.0, 1.0) * 255.0).round() as u8;
    }

    /// Llena `rect` (en pixels) con `color`, intersectándolo primero con
    /// el clip actual y el bound del buffer.
    fn fill_rect(&mut self, rect: Rect, color: Color, clip: Option<Rect>) {
        let bound = self.bound();
        let mut r = intersect(rect, bound);
        if let Some(c) = clip {
            r = intersect(r, c);
        }
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let x0 = r.x.floor() as u32;
        let y0 = r.y.floor() as u32;
        let x1 = (r.x + r.w).ceil() as u32;
        let y1 = (r.y + r.h).ceil() as u32;
        for y in y0..x1_clamped(y1, self.h) {
            for x in x0..x1_clamped(x1, self.w) {
                self.blend(x, y, color);
            }
        }
    }

    fn bound(&self) -> Rect {
        Rect::new(0.0, 0.0, self.w as f32, self.h as f32)
    }
}

fn x1_clamped(v: u32, max: u32) -> u32 {
    if v > max {
        max
    } else {
        v
    }
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

fn replay(cmd: &RenderCmd, buf: &mut RasterBuffer, clip: &mut Vec<Rect>) {
    let active_clip = clip.last().copied();
    match cmd {
        RenderCmd::PushClip(r) => {
            // Push el AND con el clip anterior.
            let new_clip = match active_clip {
                Some(prev) => intersect(*r, prev),
                None => *r,
            };
            clip.push(new_clip);
        }
        RenderCmd::PopClip => {
            clip.pop();
        }
        RenderCmd::FillRect { rect, color } => {
            buf.fill_rect(*rect, *color, active_clip);
        }
        RenderCmd::StrokeRect { rect, stroke } => {
            let w = stroke.width.max(1.0);
            // Top, bottom, left, right como rects rellenos.
            buf.fill_rect(Rect::new(rect.x, rect.y, rect.w, w), stroke.color, active_clip);
            buf.fill_rect(
                Rect::new(rect.x, rect.y + rect.h - w, rect.w, w),
                stroke.color,
                active_clip,
            );
            buf.fill_rect(Rect::new(rect.x, rect.y, w, rect.h), stroke.color, active_clip);
            buf.fill_rect(
                Rect::new(rect.x + rect.w - w, rect.y, w, rect.h),
                stroke.color,
                active_clip,
            );
        }
        RenderCmd::StrokeLine { a, b, stroke } => {
            stroke_segment(buf, (a.x, a.y), (b.x, b.y), stroke.color, stroke.width, active_clip);
        }
        RenderCmd::StrokePolyline { coords, stroke } => {
            for w in coords.chunks_exact(2).collect::<Vec<_>>().windows(2) {
                stroke_segment(
                    buf,
                    (w[0][0], w[0][1]),
                    (w[1][0], w[1][1]),
                    stroke.color,
                    stroke.width,
                    active_clip,
                );
            }
        }
        RenderCmd::FillTriangleStrip { coords, colors } => {
            let n = coords.len() / 2;
            if n < 3 {
                return;
            }
            for t in 0..n - 2 {
                let p0 = (coords[t * 2], coords[t * 2 + 1]);
                let p1 = (coords[(t + 1) * 2], coords[(t + 1) * 2 + 1]);
                let p2 = (coords[(t + 2) * 2], coords[(t + 2) * 2 + 1]);
                let avg = avg_color(&[
                    colors.get(t).copied(),
                    colors.get(t + 1).copied(),
                    colors.get(t + 2).copied(),
                ]);
                fill_triangle(buf, p0, p1, p2, avg, active_clip);
            }
        }
        // DrawText: PNG export deliberadamente skipea texto. Ver doc del módulo.
        RenderCmd::DrawText { .. } => {}
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

/// Segmento expandido a un quad orientado (perpendicular ± width/2),
/// rasterizado como dos triángulos.
fn stroke_segment(
    buf: &mut RasterBuffer,
    a: (f32, f32),
    b: (f32, f32),
    color: Color,
    width: f32,
    clip: Option<Rect>,
) {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-6 {
        return;
    }
    let half = width.max(1.0) * 0.5;
    // Perpendicular unitario.
    let nx = -dy / len * half;
    let ny = dx / len * half;
    let p0 = (a.0 + nx, a.1 + ny);
    let p1 = (a.0 - nx, a.1 - ny);
    let p2 = (b.0 + nx, b.1 + ny);
    let p3 = (b.0 - nx, b.1 - ny);
    fill_triangle(buf, p0, p1, p2, color, clip);
    fill_triangle(buf, p1, p3, p2, color, clip);
}

/// Scanline fill de un triángulo (sin antialiasing — el resultado se ve
/// igual a un GPU sin MSAA. Aceptable para charts densos).
fn fill_triangle(
    buf: &mut RasterBuffer,
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    color: Color,
    clip: Option<Rect>,
) {
    let bound = buf.bound();
    let active = match clip {
        Some(c) => intersect(c, bound),
        None => bound,
    };
    let min_x = a.0.min(b.0).min(c.0).max(active.x).floor() as i32;
    let max_x = a.0.max(b.0).max(c.0).min(active.x + active.w).ceil() as i32;
    let min_y = a.1.min(b.1).min(c.1).max(active.y).floor() as i32;
    let max_y = a.1.max(b.1).max(c.1).min(active.y + active.h).ceil() as i32;
    if max_x <= min_x || max_y <= min_y {
        return;
    }
    let area = edge(a, b, c);
    if area.abs() < 1e-6 {
        return;
    }
    let sign = area.signum();
    for y in min_y..max_y {
        for x in min_x..max_x {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = edge(b, c, p) * sign;
            let w1 = edge(c, a, p) * sign;
            let w2 = edge(a, b, p) * sign;
            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                buf.blend(x as u32, y as u32, color);
            }
        }
    }
}

#[inline]
fn edge(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

fn encode_png(buf: &RasterBuffer) -> Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::with_capacity(buf.pixels.len() / 4);
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut out), buf.w, buf.h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&buf.pixels)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pineal_render::{Canvas, Color, Point, StrokeStyle};

    fn record_one_rect() -> RenderPlan {
        let mut rec = pineal_render::PlanRecorder::new();
        rec.fill_rect(Rect::new(2.0, 2.0, 6.0, 6.0), Color::from_hex(0xff0000));
        rec.into_plan()
    }

    #[test]
    fn png_starts_with_magic() {
        let bytes = to_png(&record_one_rect(), 16, 16, Color::WHITE).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn rect_is_painted_red_on_white_background() {
        let bytes = to_png(&record_one_rect(), 16, 16, Color::WHITE).unwrap();
        // Decodificar de vuelta y verificar un pixel dentro del rect (≈ 255,0,0)
        // y otro fuera (255,255,255).
        let decoder = png::Decoder::new(Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut img = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut img).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        let idx_in = ((4u32 * 16 + 4) * 4) as usize;
        let idx_out = ((0u32 * 16 + 0) * 4) as usize;
        assert_eq!(img[idx_in], 255);
        assert!(img[idx_in + 1] < 5);
        assert!(img[idx_in + 2] < 5);
        assert_eq!(img[idx_out], 255);
        assert_eq!(img[idx_out + 1], 255);
        assert_eq!(img[idx_out + 2], 255);
    }

    #[test]
    fn stroke_line_writes_pixels_along_segment() {
        let mut rec = pineal_render::PlanRecorder::new();
        rec.stroke_line(
            Point::new(2.0, 2.0),
            Point::new(13.0, 13.0),
            StrokeStyle::new(2.0, Color::BLACK),
        );
        let bytes = to_png(&rec.into_plan(), 16, 16, Color::WHITE).unwrap();
        let decoder = png::Decoder::new(Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut img = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut img).unwrap();
        // Algún pixel cerca de la diagonal debería ser negro o casi.
        let idx = ((7u32 * 16 + 7) * 4) as usize;
        assert!(img[idx] < 40, "esperaba algo oscuro en (7,7), got R={}", img[idx]);
    }

    #[test]
    fn triangle_strip_fills_pixels() {
        let mut rec = pineal_render::PlanRecorder::new();
        rec.fill_triangle_strip(
            &[0.0, 0.0, 16.0, 0.0, 0.0, 16.0, 16.0, 16.0],
            &[Color::BLACK; 4],
        );
        let bytes = to_png(&rec.into_plan(), 16, 16, Color::WHITE).unwrap();
        let decoder = png::Decoder::new(Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut img = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut img).unwrap();
        // Casi todos los pixels deberían ser negros (1 strip cubre el cuadrado).
        let black = img
            .chunks_exact(4)
            .filter(|p| p[0] < 30 && p[1] < 30 && p[2] < 30)
            .count();
        assert!(black > 240, "se esperaban >240 pixels negros, hubo {black}");
    }

    #[test]
    fn clip_blocks_writes_outside_rect() {
        let mut rec = pineal_render::PlanRecorder::new();
        rec.push_clip(Rect::new(0.0, 0.0, 8.0, 16.0));
        rec.fill_rect(Rect::new(0.0, 0.0, 16.0, 16.0), Color::BLACK);
        rec.pop_clip();
        let bytes = to_png(&rec.into_plan(), 16, 16, Color::WHITE).unwrap();
        let decoder = png::Decoder::new(Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut img = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut img).unwrap();
        // Lado izquierdo (x=2) negro; lado derecho (x=12) blanco.
        let idx_l = ((8u32 * 16 + 2) * 4) as usize;
        let idx_r = ((8u32 * 16 + 12) * 4) as usize;
        assert!(img[idx_l] < 30);
        assert!(img[idx_r] > 230);
    }
}
