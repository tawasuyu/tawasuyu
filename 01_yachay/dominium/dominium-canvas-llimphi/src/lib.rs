//! `dominium-canvas-llimphi` — el único crate de dominium que importa
//! `llimphi-ui`.
//!
//! Toda la cadena `dominium-core → physics → iso → render-plan` es
//! agnóstica de backend. Este crate cierra el circuito: una función
//! [`canvas_view`] que recibe un [`RenderPlan`] ya resuelto y devuelve
//! un `View<Msg>` que lo pinta, centrando la maqueta en los bounds
//! asignados por taffy.
//!
//! ## Dos caminos de rasterizado (Tier 0 / Tier 1)
//!
//! - **Camino GPU (default, Tier 1)** — la geometría OPACA intercalada
//!   por profundidad (techos rombo + caras laterales + quads de
//!   lemmings/conceptos/textura/trails) se emite a un [`GpuBatch`] como
//!   **triángulos** (cada quad/rombo/paralelogramo → 2 tris) en el MISMO
//!   orden back-to-front que usaba vello. Como `GpuBatch` dibuja los tris
//!   en orden de inserción, una sola pasada preserva el algoritmo del
//!   pintor isométrico (no hay z-buffer). Resultado: ~115 k fills
//!   vectoriales/frame → 1 draw call de triángulos. Los **íconos/sprites
//!   AA de Concepto y los glifos** van en una pasada vello "over"
//!   ([`View::paint_over`]) para conservar el antialiasing y quedar
//!   ENCIMA de la capa GPU.
//!
//! - **Camino vello (legacy, Tier 0 puro)** — el render histórico, todo
//!   por vello en `paint_with`. Se activa con la env `DOMINIUM_RENDER_LEGACY`
//!   y existe para comparar pixel-a-pixel contra el camino GPU. La caché
//!   por huella del `RenderPlan` (en `dominium-app-llimphi`) cubre los
//!   frames sin cambio en ambos caminos.
//!
//! El `View` no guarda estado entre frames — el host reconstruye el View
//! con el `RenderPlan` (o su `Arc` cacheado) del frame actual.

#![forbid(unsafe_code)]

use dominium_render_plan::{Color as PlanColor, Polygon, Quad, RenderPlan, SpritePrim};
use llimphi_ui::llimphi_hal::wgpu;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::gpu::{GpuBatch, GpuPipelines};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Point, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{draw_block, TextBlock};
use llimphi_ui::{PaintRect, View};

/// Convierte el RGBA lineal del plan (`[f32;4]` en [0,1]) al `Color`
/// de peniko. Mantiene la convención sin gamma del backend GPUI.
fn plan_color(c: PlanColor) -> Color {
    let to_byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::from_rgba8(to_byte(c[0]), to_byte(c[1]), to_byte(c[2]), to_byte(c[3]))
}

/// `true` si el camino legacy (vello puro) está forzado por env. Sólo para
/// comparación visual / bench A/B — el default es el camino GPU.
fn legacy_path() -> bool {
    std::env::var_os("DOMINIUM_RENDER_LEGACY").is_some()
}

/// Construye un View que pinta `plan` en su rect. Ver [`canvas_view_arc`].
///
/// `pan` es el desplazamiento de cámara en píxeles de pantalla (sumado al
/// centrado automático): `(0.0, 0.0)` deja la maqueta centrada.
pub fn canvas_view<Msg>(plan: RenderPlan, background: Option<Color>, pan: (f32, f32)) -> View<Msg>
where
    Msg: Clone + 'static,
{
    canvas_view_arc(std::sync::Arc::new(plan), background, pan)
}

/// Igual que [`canvas_view`] pero recibe el plan envuelto en `Arc`. Pensado
/// para hosts que cachean el `RenderPlan` entre frames (memoización por
/// huella): clonar el `Arc` para meterlo en las closures de paint es O(1),
/// en vez de copiar el `Vec` de ~115 k primitivas por frame.
///
/// Adjunta DOS closures al mismo View:
/// 1. `gpu_paint_with` — emite la geometría opaca como triángulos GPU.
/// 2. `paint_over` — pinta sprites AA + glifos en vello, encima del GPU.
///
/// Con `DOMINIUM_RENDER_LEGACY` puesta, en cambio adjunta un único
/// `paint_with` con el render histórico de vello (para comparar).
///
/// `pan` (px de pantalla) se suma al centrado automático en TODOS los
/// caminos (GPU, legacy, over). El mapeo inverso del host (clicks/drags)
/// debe restar el mismo `pan` o el click queda desalineado al panear.
pub fn canvas_view_arc<Msg>(
    plan: std::sync::Arc<RenderPlan>,
    background: Option<Color>,
    pan: (f32, f32),
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let view = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    });
    let view = if let Some(bg) = background {
        view.fill(bg)
    } else {
        view
    };

    if legacy_path() {
        // Camino histórico: TODO por vello en una sola closure.
        return view.paint_with(move |scene, ts, rect: PaintRect| {
            paint_vello_full(&plan, scene, ts, rect, pan);
        });
    }

    // ── Camino GPU (Tier 1) ─────────────────────────────────────────────
    // Geometría opaca → tris GPU; sprites AA + glifos → vello "over".
    let plan_gpu = plan.clone();
    let plan_over = plan;
    view.gpu_paint_with(move |device, queue, encoder, tex_view, rect, viewport| {
        if plan_gpu.quads.is_empty() && plan_gpu.polygons.is_empty() {
            return;
        }
        let pipelines = pipelines_for(device);
        let mut batch = GpuBatch::new(pipelines);
        emit_geometry_tris(&plan_gpu, rect, &mut batch, pan);
        batch.flush(
            device,
            queue,
            encoder,
            tex_view,
            (viewport.0 as f32, viewport.1 as f32),
            wgpu::LoadOp::Load,
        );
    })
    .paint_over(move |scene, ts, rect: PaintRect| {
        let (off_x, off_y) = plan_offset(&plan_over, rect, pan);
        paint_sprites(&plan_over, scene, off_x, off_y);
        paint_glyphs(&plan_over, scene, ts, off_x, off_y);
    })
}

/// Pipelines GPU cacheadas (una sola vez, sobre el primer `device`). La
/// intermedia del runtime es `Rgba8Unorm` (ver `llimphi-ui` redraw.rs).
fn pipelines_for(device: &wgpu::Device) -> &'static GpuPipelines {
    use std::sync::OnceLock;
    static SLOT: OnceLock<GpuPipelines> = OnceLock::new();
    SLOT.get_or_init(|| GpuPipelines::new(device, wgpu::TextureFormat::Rgba8Unorm))
}

/// Offset de centrado + pan de cámara: el centro de la caja envolvente del
/// plan se alinea con el centro del rect del nodo, desplazado por `pan` (px
/// de pantalla). Idéntico en ambos caminos (vello/GPU) y debe coincidir con
/// el mapeo inverso de la app (clicks/drags) — que también resta `pan`.
fn plan_offset(plan: &RenderPlan, rect: PaintRect, pan: (f32, f32)) -> (f64, f64) {
    let plan_cx = (plan.min_x + plan.max_x) * 0.5;
    let plan_cy = (plan.min_y + plan.max_y) * 0.5;
    let off_x = (rect.x + rect.w * 0.5 - plan_cx + pan.0) as f64;
    let off_y = (rect.y + rect.h * 0.5 - plan_cy + pan.1) as f64;
    (off_x, off_y)
}

// ════════════════════════════════════════════════════════════════════════
// Camino GPU: geometría opaca como triángulos
// ════════════════════════════════════════════════════════════════════════

/// Emite toda la geometría opaca intercalada (quads + polygons) al
/// `GpuBatch` como triángulos, en el MISMO orden back-to-front que usaba
/// vello — preservando el algoritmo del pintor. Cada quad axis-aligned y
/// cada polígono de 4 vértices se triangula en 2 tris; los polígonos de 3
/// vértices (frontones, techos a dos aguas) en 1 tri.
///
/// **No** se emiten sprites ni glifos: esos van en el over-layer vello.
///
/// Clipping: como la pasada GPU escribe sobre toda la intermedia (no hay
/// scissor en `GpuBatch`), descartamos en CPU las primitivas que caen
/// enteramente fuera del rect del canvas. Sin esto, la maqueta (mayor que
/// el canvas a grid alto) sangraría sobre el panel lateral. El test es un
/// solapamiento de bounding boxes — barato frente al build del plan.
fn emit_geometry_tris(plan: &RenderPlan, rect: PaintRect, batch: &mut GpuBatch, pan: (f32, f32)) {
    let (off_x, off_y) = plan_offset(plan, rect, pan);
    let ox = off_x as f32;
    let oy = off_y as f32;
    // Rect del canvas en coordenadas de frame (margen de holgura cero: las
    // primitivas que tocan el borde se emiten enteras, el AA del borde lo da
    // el recorte del compositor en el blit).
    let (lo_x, lo_y, hi_x, hi_y) = (rect.x, rect.y, rect.x + rect.w, rect.y + rect.h);
    let quad_visible = |q: &Quad| -> bool {
        let x = q.x + ox;
        let y = q.y + oy;
        x + q.w >= lo_x && x <= hi_x && y + q.h >= lo_y && y <= hi_y
    };
    let poly_visible = |p: &Polygon| -> bool {
        let (mut nx, mut ny, mut xx, mut xy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (vx, vy) in p.vertices {
            nx = nx.min(vx);
            ny = ny.min(vy);
            xx = xx.max(vx);
            xy = xy.max(vy);
        }
        xx + ox >= lo_x && nx + ox <= hi_x && xy + oy >= lo_y && ny + oy <= hi_y
    };

    // Merge lineal por depth (ambos inputs ya vienen ordenados), idéntico al
    // del camino vello, así el orden de inserción de tris = orden de pintor.
    let mut qi = 0usize;
    let mut pi = 0usize;
    while qi < plan.quads.len() || pi < plan.polygons.len() {
        let q_d = plan.quads.get(qi).map(|q| q.depth);
        let p_d = plan.polygons.get(pi).map(|p| p.depth);
        let take_quad = match (q_d, p_d) {
            (Some(q), Some(p)) => q <= p,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if take_quad {
            let q = &plan.quads[qi];
            if quad_visible(q) {
                emit_quad_tris(batch, q, ox, oy);
            }
            qi += 1;
        } else {
            let p = &plan.polygons[pi];
            if poly_visible(p) {
                emit_polygon_tris(batch, p, ox, oy);
            }
            pi += 1;
        }
    }
}

/// Un quad axis-aligned → 2 triángulos (color uniforme por vértice).
fn emit_quad_tris(batch: &mut GpuBatch, q: &Quad, ox: f32, oy: f32) {
    let c = plan_color(q.color);
    let x0 = q.x + ox;
    let y0 = q.y + oy;
    let x1 = x0 + q.w;
    let y1 = y0 + q.h;
    // (x0,y0)──(x1,y0)
    //    │   ╲    │
    // (x0,y1)──(x1,y1)
    batch.add_tri((x0, y0), (x1, y0), (x1, y1), c, c, c);
    batch.add_tri((x0, y0), (x1, y1), (x0, y1), c, c, c);
}

/// Un polígono (3 o 4 vértices) → triangle fan desde el vértice 0.
fn emit_polygon_tris(batch: &mut GpuBatch, p: &Polygon, ox: f32, oy: f32) {
    let c = plan_color(p.color);
    let v = &p.vertices;
    let v0 = (v[0].0 + ox, v[0].1 + oy);
    let v1 = (v[1].0 + ox, v[1].1 + oy);
    let v2 = (v[2].0 + ox, v[2].1 + oy);
    batch.add_tri(v0, v1, v2, c, c, c);
    // Los polígonos del plan son siempre [4] (los frontones de sprite son
    // SpritePrim::Fill, no Polygon). El 4º vértice cierra el quad/rombo;
    // como degenera (v3==v0 jamás), siempre emitimos el segundo tri.
    let v3 = (v[3].0 + ox, v[3].1 + oy);
    batch.add_tri(v0, v2, v3, c, c, c);
}

// ════════════════════════════════════════════════════════════════════════
// Over-layer vello: sprites AA + glifos
// ════════════════════════════════════════════════════════════════════════

/// Sprites vectoriales de los Conceptos (relleno / trazo / disco) por
/// vello — conservan AA y quedan ENCIMA de la capa GPU.
fn paint_sprites(plan: &RenderPlan, scene: &mut llimphi_ui::llimphi_raster::vello::Scene, off_x: f64, off_y: f64) {
    for prim in &plan.sprites {
        match prim {
            SpritePrim::Fill { points, color } => {
                if points.len() < 3 {
                    continue;
                }
                let mut path = BezPath::new();
                path.move_to(Point::new(points[0].0 as f64 + off_x, points[0].1 as f64 + off_y));
                for pt in &points[1..] {
                    path.line_to(Point::new(pt.0 as f64 + off_x, pt.1 as f64 + off_y));
                }
                path.close_path();
                scene.fill(Fill::NonZero, Affine::IDENTITY, plan_color(*color), None, &path);
            }
            SpritePrim::Stroke { points, width, color } => {
                if points.len() < 2 {
                    continue;
                }
                let mut path = BezPath::new();
                path.move_to(Point::new(points[0].0 as f64 + off_x, points[0].1 as f64 + off_y));
                for pt in &points[1..] {
                    path.line_to(Point::new(pt.0 as f64 + off_x, pt.1 as f64 + off_y));
                }
                scene.stroke(
                    &Stroke::new(*width as f64),
                    Affine::IDENTITY,
                    plan_color(*color),
                    None,
                    &path,
                );
            }
            SpritePrim::Disc { cx, cy, r, color } => {
                let circle =
                    Circle::new(Point::new(*cx as f64 + off_x, *cy as f64 + off_y), *r as f64);
                scene.fill(Fill::NonZero, Affine::IDENTITY, plan_color(*color), None, &circle);
            }
        }
    }
}

/// Glifos de fallback (`?` para sprite_id desconocido) por vello.
fn paint_glyphs(
    plan: &RenderPlan,
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    off_x: f64,
    off_y: f64,
) {
    for gl in &plan.glyphs {
        let s = gl.ch.to_string();
        let block = TextBlock::simple(
            &s,
            gl.size_px,
            plan_color(gl.color),
            (gl.x as f64 + off_x, gl.y as f64 + off_y),
        );
        draw_block(scene, ts, &block);
    }
}

// ════════════════════════════════════════════════════════════════════════
// Camino legacy: render histórico completo por vello (para comparación A/B)
// ════════════════════════════════════════════════════════════════════════

/// El render histórico: quads + polygons intercalados, sprites y glifos,
/// todo por vello. Idéntico al `paint_with` anterior a Tier 1 — sólo se
/// usa con `DOMINIUM_RENDER_LEGACY` para comparar pixel-a-pixel.
fn paint_vello_full(
    plan: &RenderPlan,
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: PaintRect,
    pan: (f32, f32),
) {
    if plan.quads.is_empty()
        && plan.polygons.is_empty()
        && plan.glyphs.is_empty()
        && plan.sprites.is_empty()
    {
        return;
    }
    let (off_x, off_y) = plan_offset(plan, rect, pan);

    let mut qi = 0usize;
    let mut pi = 0usize;
    while qi < plan.quads.len() || pi < plan.polygons.len() {
        let q_d = plan.quads.get(qi).map(|q| q.depth);
        let p_d = plan.polygons.get(pi).map(|p| p.depth);
        let take_quad = match (q_d, p_d) {
            (Some(q), Some(p)) => q <= p,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if take_quad {
            let q = &plan.quads[qi];
            let x0 = q.x as f64 + off_x;
            let y0 = q.y as f64 + off_y;
            let r = KurboRect::new(x0, y0, x0 + q.w as f64, y0 + q.h as f64);
            scene.fill(Fill::NonZero, Affine::IDENTITY, plan_color(q.color), None, &r);
            qi += 1;
        } else {
            let p = &plan.polygons[pi];
            let mut path = BezPath::new();
            let v = &p.vertices;
            path.move_to(Point::new(v[0].0 as f64 + off_x, v[0].1 as f64 + off_y));
            path.line_to(Point::new(v[1].0 as f64 + off_x, v[1].1 as f64 + off_y));
            path.line_to(Point::new(v[2].0 as f64 + off_x, v[2].1 as f64 + off_y));
            path.line_to(Point::new(v[3].0 as f64 + off_x, v[3].1 as f64 + off_y));
            path.close_path();
            scene.fill(Fill::NonZero, Affine::IDENTITY, plan_color(p.color), None, &path);
            pi += 1;
        }
    }
    paint_sprites(plan, scene, off_x, off_y);
    paint_glyphs(plan, scene, ts, off_x, off_y);
}

// ════════════════════════════════════════════════════════════════════════
// Hooks de bench / example (headless A/B). No forman parte de la API
// estable; existen para que `examples/ab_compare.rs` ejercite los DOS
// caminos sobre la misma escena sin levantar la app.
// ════════════════════════════════════════════════════════════════════════

#[doc(hidden)]
pub mod bench {
    use super::*;
    use llimphi_ui::llimphi_raster::vello::Scene;
    use llimphi_ui::llimphi_text::Typesetter;

    /// Emite la geometría opaca del plan como tris en `batch` (camino GPU).
    /// `pan` = desplazamiento de cámara en px de pantalla.
    pub fn emit_tris(plan: &RenderPlan, rect: PaintRect, batch: &mut GpuBatch, pan: (f32, f32)) {
        emit_geometry_tris(plan, rect, batch, pan);
    }

    /// Pinta sprites AA + glifos en `scene` (over-layer del camino GPU).
    pub fn over_layer(
        plan: &RenderPlan,
        scene: &mut Scene,
        ts: &mut Typesetter,
        rect: PaintRect,
        pan: (f32, f32),
    ) {
        let (ox, oy) = plan_offset(plan, rect, pan);
        paint_sprites(plan, scene, ox, oy);
        paint_glyphs(plan, scene, ts, ox, oy);
    }

    /// Render histórico completo por vello (camino legacy, Tier 0 puro).
    pub fn vello_full(
        plan: &RenderPlan,
        scene: &mut Scene,
        ts: &mut Typesetter,
        rect: PaintRect,
        pan: (f32, f32),
    ) {
        paint_vello_full(plan, scene, ts, rect, pan);
    }

    /// Re-export del constructor de pipelines para el example headless
    /// (necesita su propio `GpuPipelines` sobre el device del demo).
    pub use llimphi_ui::llimphi_raster::gpu::GpuPipelines as Pipelines;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_red_round_trips() {
        let c = plan_color([1.0, 0.0, 0.0, 1.0]).to_rgba8();
        assert_eq!((c.r, c.g, c.b, c.a), (255, 0, 0, 255));
    }

    #[test]
    fn alpha_passes_through() {
        let c = plan_color([0.0, 0.0, 1.0, 0.25]).to_rgba8();
        assert_eq!(c.b, 255);
        assert_eq!(c.a, 64); // 0.25 * 255 = 63.75 ~> 64
    }

    #[test]
    fn out_of_range_clamps() {
        let c = plan_color([1.5, -0.2, 0.5, 1.0]).to_rgba8();
        assert_eq!((c.r, c.g, c.b), (255, 0, 128));
    }

    /// La triangulación de un quad cubre el rect entero: los 4 corners del
    /// rect aparecen entre los 6 vértices de los 2 tris emitidos.
    #[test]
    fn quad_triangulation_covers_corners() {
        // No podemos inspeccionar el GpuBatch (buffers opacos), pero sí
        // verificar la geometría de los tris que emitiríamos manualmente:
        // dos tris (0,1,2)+(0,2,3) sobre [(x0,y0),(x1,y0),(x1,y1),(x0,y1)]
        // tocan las 4 esquinas. Test de la convención, no del batch.
        let q = Quad { x: 10.0, y: 20.0, w: 4.0, h: 6.0, color: [1.0; 4], depth: 0.0 };
        let corners = [
            (q.x, q.y),
            (q.x + q.w, q.y),
            (q.x + q.w, q.y + q.h),
            (q.x, q.y + q.h),
        ];
        // tri A = corners[0,1,2]; tri B = corners[0,2,3] → unión = los 4.
        let used = [corners[0], corners[1], corners[2], corners[0], corners[2], corners[3]];
        for c in corners {
            assert!(used.contains(&c), "corner {c:?} cubierto por la triangulación");
        }
    }
}
