//! `llimphi-mesh` — render de mallas deformables a una `vello::Scene`.
//!
//! La matemática de huesos y skinning vive en [`llimphi_anim::skel`]; este crate
//! sólo **pinta** la malla ya deformada. Recibe la malla ([`Mesh`]) + las
//! posiciones deformadas de sus vértices (lo que devuelve [`Mesh::deform`]) y un
//! `xform` model→pantalla, y ofrece las dos rutas que el spike validó contra
//! vello 0.7:
//!
//! - [`paint_solid`] — rellena cada triángulo deformado con un color (malla
//!   vectorial / debug). Trivial.
//! - [`paint_textured`] — malla **texturizada**: por triángulo, recorta al
//!   triángulo deformado y dibuja la imagen con el afín que mapea sus UV a la
//!   posición deformada (warp piecewise-affine). Costo = un clip-layer por
//!   triángulo; cuidado con mallas de miles de triángulos.
//! - [`paint_wireframe`] — traza los bordes de los triángulos (ver la
//!   deformación; ideal para demos/debug).
//!
//! Helpers de encuadre: [`rest_bounds`] (bbox de la malla en reposo) +
//! [`fit_transform`] (afín que encaja esos bounds, centrados, en un `PaintRect`).

#![forbid(unsafe_code)]

use llimphi_anim::skel::Mesh;
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Point, Rect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Brush, Color, Fill, ImageBrush};
use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::PaintRect;

/// Afín que mapea el triángulo `src` al triángulo `dst` (3 correspondencias).
/// Es la base del warp piecewise-affine de la malla texturizada. Devuelve la
/// identidad si `src` es degenerado (área nula).
pub fn affine_from_tri(src: [Point; 3], dst: [Point; 3]) -> Affine {
    let e1 = (src[1].x - src[0].x, src[1].y - src[0].y);
    let e2 = (src[2].x - src[0].x, src[2].y - src[0].y);
    let f1 = (dst[1].x - dst[0].x, dst[1].y - dst[0].y);
    let f2 = (dst[2].x - dst[0].x, dst[2].y - dst[0].y);
    let det = e1.0 * e2.1 - e2.0 * e1.1;
    if det.abs() < 1e-9 {
        return Affine::IDENTITY;
    }
    let inv = 1.0 / det;
    let l00 = (f1.0 * e2.1 - f2.0 * e1.1) * inv;
    let l01 = (-f1.0 * e2.0 + f2.0 * e1.0) * inv;
    let l10 = (f1.1 * e2.1 - f2.1 * e1.1) * inv;
    let l11 = (-f1.1 * e2.0 + f2.1 * e1.0) * inv;
    let tx = dst[0].x - (l00 * src[0].x + l01 * src[0].y);
    let ty = dst[0].y - (l10 * src[0].x + l11 * src[0].y);
    Affine::new([l00, l10, l01, l11, tx, ty])
}

/// Bounding box de las posiciones de **reposo** de la malla. Útil para
/// `fit_transform`. Vacío (`Rect::ZERO`) si la malla no tiene vértices.
pub fn rest_bounds(mesh: &Mesh) -> Rect {
    let mut it = mesh.vertices.iter();
    let Some(first) = it.next() else {
        return Rect::ZERO;
    };
    let (mut x0, mut y0, mut x1, mut y1) = (first.rest.x, first.rest.y, first.rest.x, first.rest.y);
    for v in it {
        x0 = x0.min(v.rest.x);
        y0 = y0.min(v.rest.y);
        x1 = x1.max(v.rest.x);
        y1 = y1.max(v.rest.y);
    }
    Rect::new(x0, y0, x1, y1)
}

/// Afín que encaja `bounds` (espacio de la malla) dentro de `rect` (pantalla),
/// escalando uniforme al mínimo lado y centrando (preserva aspecto).
pub fn fit_transform(bounds: Rect, rect: PaintRect) -> Affine {
    let bw = bounds.width();
    let bh = bounds.height();
    if bw <= 0.0 || bh <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return Affine::IDENTITY;
    }
    let s = (rect.w as f64 / bw).min(rect.h as f64 / bh);
    let used_w = bw * s;
    let used_h = bh * s;
    let tx = rect.x as f64 + (rect.w as f64 - used_w) * 0.5 - bounds.x0 * s;
    let ty = rect.y as f64 + (rect.h as f64 - used_h) * 0.5 - bounds.y0 * s;
    Affine::translate((tx, ty)) * Affine::scale(s)
}

/// Posiciones deformadas (model space) → pantalla, en un buffer reusable.
fn to_screen(positions: &[Point], xform: Affine, out: &mut Vec<Point>) {
    out.clear();
    out.extend(positions.iter().map(|p| xform * *p));
}

fn tri_path(a: Point, b: Point, c: Point) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(a);
    p.line_to(b);
    p.line_to(c);
    p.close_path();
    p
}

/// Rellena cada triángulo deformado con un color sólido. `positions` son las
/// posiciones deformadas (de `Mesh::deform`), `xform` las lleva a pantalla.
pub fn paint_solid(
    scene: &mut Scene,
    mesh: &Mesh,
    positions: &[Point],
    xform: Affine,
    color: Color,
) {
    let mut screen = Vec::new();
    to_screen(positions, xform, &mut screen);
    let brush = Brush::Solid(color);
    for t in &mesh.triangles {
        let (Some(&a), Some(&b), Some(&c)) = (
            screen.get(t[0] as usize),
            screen.get(t[1] as usize),
            screen.get(t[2] as usize),
        ) else {
            continue;
        };
        scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &tri_path(a, b, c));
    }
}

/// Malla texturizada: por triángulo, recorta al triángulo deformado y dibuja la
/// imagen con el afín que mapea las UV (espacio de imagen) a la posición
/// deformada en pantalla — warp piecewise-affine. `image` es la textura; las UV
/// de cada vértice (`0..1`) indexan en ella.
pub fn paint_textured(
    scene: &mut Scene,
    mesh: &Mesh,
    positions: &[Point],
    xform: Affine,
    image: &ImageBrush,
) {
    let mut screen = Vec::new();
    to_screen(positions, xform, &mut screen);
    let iw = image.image.width as f64;
    let ih = image.image.height as f64;
    for t in &mesh.triangles {
        let idx = [t[0] as usize, t[1] as usize, t[2] as usize];
        let (Some(&da), Some(&db), Some(&dc)) =
            (screen.get(idx[0]), screen.get(idx[1]), screen.get(idx[2]))
        else {
            continue;
        };
        // src = UV·tamaño-de-imagen (espacio de imagen); dst = pantalla deformada.
        let uv = |i: usize| {
            let v = &mesh.vertices[idx[i]];
            Point::new(v.uv.0 * iw, v.uv.1 * ih)
        };
        let aff = affine_from_tri([uv(0), uv(1), uv(2)], [da, db, dc]);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &tri_path(da, db, dc));
        scene.draw_image(image.as_ref(), aff);
        scene.pop_layer();
    }
}

/// Traza los bordes de los triángulos (para ver la deformación / debug).
pub fn paint_wireframe(
    scene: &mut Scene,
    mesh: &Mesh,
    positions: &[Point],
    xform: Affine,
    color: Color,
    width: f64,
) {
    let mut screen = Vec::new();
    to_screen(positions, xform, &mut screen);
    let brush = Brush::Solid(color);
    let stroke = Stroke::new(width);
    for t in &mesh.triangles {
        let (Some(&a), Some(&b), Some(&c)) = (
            screen.get(t[0] as usize),
            screen.get(t[1] as usize),
            screen.get(t[2] as usize),
        ) else {
            continue;
        };
        scene.stroke(&stroke, Affine::IDENTITY, &brush, None, &tri_path(a, b, c));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_anim::skel::{Mesh, Pose, Skeleton, Vertex};
    use llimphi_ui::llimphi_raster::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
    use std::sync::Arc;

    /// Una malla quad (2 triángulos) atada a un hueso, con UVs de esquina.
    fn quad_mesh(bone: usize) -> Mesh {
        let mut m = Mesh::new();
        m.vertices = vec![
            Vertex::rigid(Point::new(0.0, 0.0), (0.0, 0.0), bone),
            Vertex::rigid(Point::new(100.0, 0.0), (1.0, 0.0), bone),
            Vertex::rigid(Point::new(100.0, 100.0), (1.0, 1.0), bone),
            Vertex::rigid(Point::new(0.0, 100.0), (0.0, 1.0), bone),
        ];
        m.triangles = vec![[0, 1, 2], [0, 2, 3]];
        m
    }

    fn skel_one_bone() -> (Skeleton, usize) {
        let mut s = Skeleton::new();
        let b = s.add_bone(None, Pose::identity());
        s.bind();
        s.update();
        (s, b)
    }

    fn img_2x2() -> ImageBrush {
        let px: Vec<u8> = vec![
            200, 60, 60, 255, 60, 200, 60, 255, 60, 60, 200, 255, 220, 200, 60, 255,
        ];
        ImageBrush::new(ImageData {
            data: Blob::new(Arc::new(px)),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: 2,
            height: 2,
        })
    }

    const RECT: PaintRect = PaintRect {
        x: 0.0,
        y: 0.0,
        w: 200.0,
        h: 200.0,
    };

    #[test]
    fn fit_transform_centra_y_escala() {
        let m = quad_mesh(0);
        let xf = fit_transform(rest_bounds(&m), RECT);
        // El quad 100×100 en un rect 200×200 escala ×2 y centra (sin offset, ya
        // que llena el lado). Esquina (0,0) → (0,0), (100,100) → (200,200).
        let p0 = xf * Point::new(0.0, 0.0);
        let p1 = xf * Point::new(100.0, 100.0);
        assert!((p0.x).abs() < 1e-9 && (p0.y).abs() < 1e-9, "{p0:?}");
        assert!((p1.x - 200.0).abs() < 1e-9 && (p1.y - 200.0).abs() < 1e-9, "{p1:?}");
    }

    #[test]
    fn paint_solid_emite_geometria() {
        let (s, b) = skel_one_bone();
        let m = quad_mesh(b);
        let pos = m.deform(&s);
        let xf = fit_transform(rest_bounds(&m), RECT);
        let mut sc = Scene::new();
        paint_solid(&mut sc, &m, &pos, xf, Color::from_rgba8(200, 100, 50, 255));
        assert!(!sc.encoding().is_empty());
    }

    #[test]
    fn paint_textured_emite_geometria() {
        let (s, b) = skel_one_bone();
        let m = quad_mesh(b);
        let pos = m.deform(&s);
        let xf = fit_transform(rest_bounds(&m), RECT);
        let img = img_2x2();
        let mut sc = Scene::new();
        paint_textured(&mut sc, &m, &pos, xf, &img);
        assert!(!sc.encoding().is_empty(), "la malla texturizada debe emitir geometría");
    }

    #[test]
    fn paint_wireframe_emite_geometria() {
        let (s, b) = skel_one_bone();
        let m = quad_mesh(b);
        let pos = m.deform(&s);
        let xf = fit_transform(rest_bounds(&m), RECT);
        let mut sc = Scene::new();
        paint_wireframe(&mut sc, &m, &pos, xf, Color::from_rgba8(255, 255, 255, 255), 1.5);
        assert!(!sc.encoding().is_empty());
    }

    /// Deformar un hueso mueve la geometría pintada: las posiciones deformadas
    /// cambian respecto al reposo (certifica que el render usa la deformación).
    #[test]
    fn deformar_cambia_las_posiciones() {
        let mut s = Skeleton::new();
        let b = s.add_bone(None, Pose::identity());
        s.bind();
        s.update();
        let m = quad_mesh(b);
        let reposo = m.deform(&s);

        s.set_pose(b, Pose::rotate(0.5));
        s.update();
        let deformado = m.deform(&s);

        let movido = reposo
            .iter()
            .zip(&deformado)
            .any(|(a, d)| (a.x - d.x).abs() > 1e-6 || (a.y - d.y).abs() > 1e-6);
        assert!(movido, "rotar el hueso debe mover los vértices");
    }
}
