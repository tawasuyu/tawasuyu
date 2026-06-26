//! Demo **chévere**: leyes físicas sobre varios esqueletos.
//!
//! Cinco tentáculos: cada uno es una cadena de huesos con una malla texturizada
//! skinneada, manejada por **física Verlet** (`llimphi_anim::physics`) — cuelgan
//! del techo, se balancean bajo gravedad y rebotan en el piso. **Mové el mouse**
//! y el cursor los **empuja** (campo de repulsión). Cero keyframes: la física
//! posa los esqueletos, el skinning deforma las mallas.
//!
//! Todo Rust nativo sobre vello 0.7. Pensado también como fondo vivo de
//! lock/greeter.
//!
//! Corre con:
//!   `cargo run -p llimphi-mesh --example physics_skeletons_demo --release`

use std::sync::Arc;
use std::time::Duration;

use llimphi_anim::physics::{pose_chain_from_points, Physics};
use llimphi_anim::skel::{BoneId, Mesh, Pose, Skeleton, Vertex};
use llimphi_mesh::paint_textured;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, Vec2};
use llimphi_ui::llimphi_raster::peniko::{
    Blob, Color, ImageAlphaType, ImageBrush, ImageData, ImageFormat,
};
use llimphi_ui::{App, Handle, PaintRect, View};

const SEGS: usize = 9;
const SEG_LEN: f64 = 26.0;
const THICK: f64 = 22.0;
const FLOOR_Y: f64 = 372.0;

/// Un tentáculo: física (partículas en cadena) + esqueleto + malla skinneada.
struct Rope {
    phys: Physics,
    skel: Skeleton,
    mesh: Mesh,
    bones: Vec<BoneId>,
    image: ImageBrush,
}

fn checker(a: (u8, u8, u8), b: (u8, u8, u8)) -> ImageBrush {
    let n = 8u32;
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let (r, g, bl) = if (x + y) % 2 == 0 { a } else { b };
            px.extend_from_slice(&[r, g, bl, 255]);
        }
    }
    ImageBrush::new(ImageData {
        data: Blob::new(Arc::new(px)),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: n,
        height: n,
    })
}

fn make_rope(anchor: Point, image: ImageBrush) -> Rope {
    // Física: partículas desde el ancla (fija) hacia abajo.
    let mut phys = Physics::new();
    phys.floor_y = Some(FLOOR_Y);
    let mut prev = phys.particle(anchor, true);
    for i in 1..=SEGS {
        let p = phys.particle(Point::new(anchor.x, anchor.y + i as f64 * SEG_LEN), false);
        phys.link(prev, p);
        prev = p;
    }

    // Esqueleto en bind pose recto desde el ORIGEN (a lo largo de +x); la física
    // lo reubica/orienta cada frame con pose_chain_from_points.
    let mut skel = Skeleton::new();
    let mut bones = vec![skel.add_bone(None, Pose::identity())];
    for _ in 1..=SEGS {
        bones.push(skel.add_bone(Some(*bones.last().unwrap()), Pose::translate(Vec2::new(SEG_LEN, 0.0))));
    }
    skel.bind();

    // Malla-tira en reposo (recta desde el origen), rigid por columna al hueso i.
    let mut mesh = Mesh::new();
    for i in 0..=SEGS {
        let x = i as f64 * SEG_LEN;
        let u = i as f64 / SEGS as f64;
        // La punta se afina (tentáculo).
        let half = THICK * 0.5 * (1.0 - 0.6 * (i as f64 / SEGS as f64));
        mesh.vertices.push(Vertex::rigid(Point::new(x, -half), (u, 0.0), bones[i]));
        mesh.vertices.push(Vertex::rigid(Point::new(x, half), (u, 1.0), bones[i]));
    }
    for i in 0..SEGS {
        let (t0, t1) = ((2 * i) as u32, (2 * (i + 1)) as u32);
        let (b0, b1) = ((2 * i + 1) as u32, (2 * (i + 1) + 1) as u32);
        mesh.triangles.push([t0, t1, b1]);
        mesh.triangles.push([t0, b1, b0]);
    }

    Rope { phys, skel, mesh, bones, image }
}

#[derive(Clone)]
enum Msg {
    Tick,
    Cursor(Option<(f64, f64)>),
}

struct Model {
    ropes: Vec<Rope>,
    cursor: Option<Point>,
}

struct Demo;

const TICK: Duration = Duration::from_millis(16);

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · física sobre esqueletos"
    }
    fn initial_size() -> (u32, u32) {
        (700, 440)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let palette = [
            ((90, 180, 230), (40, 90, 140)),
            ((235, 130, 100), (150, 60, 50)),
            ((130, 220, 150), (50, 120, 70)),
            ((220, 180, 90), (140, 100, 40)),
            ((200, 130, 220), (110, 60, 140)),
        ];
        let ropes = palette
            .iter()
            .enumerate()
            .map(|(i, (a, b))| {
                let x = 110.0 + i as f64 * 120.0;
                make_rope(Point::new(x, 40.0), checker(*a, *b))
            })
            .collect();
        handle.spawn_periodic(TICK, || Msg::Tick);
        Model { ropes, cursor: None }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => {
                let dt = TICK.as_secs_f64();
                let cursor = model.cursor;
                for rope in &mut model.ropes {
                    rope.phys.step(dt, 8);
                    if let Some(c) = cursor {
                        rope.phys.repel(c, 90.0, 28.0);
                    }
                    let pts = rope.phys.positions();
                    pose_chain_from_points(&mut rope.skel, &rope.bones, &pts);
                }
            }
            Msg::Cursor(p) => model.cursor = p.map(|(x, y)| Point::new(x, y)),
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        // Deformar todas las mallas ahora; mover los datos al closure.
        let painted: Vec<(Mesh, Vec<Point>, ImageBrush)> = model
            .ropes
            .iter()
            .map(|r| (r.mesh.clone(), r.mesh.deform(&r.skel), r.image.clone()))
            .collect();

        View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(Color::from_rgba8(16, 18, 26, 255))
        .paint_with(move |scene, _ts, rect| {
            let to_screen = Affine::translate((rect.x as f64, rect.y as f64));
            // Piso.
            let floor = PaintRect {
                x: rect.x,
                y: rect.y + FLOOR_Y as f32,
                w: rect.w,
                h: (rect.h - FLOOR_Y as f32).max(0.0),
            };
            let mut fp = llimphi_ui::llimphi_raster::kurbo::BezPath::new();
            fp.move_to((floor.x as f64, floor.y as f64));
            fp.line_to((floor.x as f64 + floor.w as f64, floor.y as f64));
            fp.line_to((floor.x as f64 + floor.w as f64, floor.y as f64 + floor.h as f64));
            fp.line_to((floor.x as f64, floor.y as f64 + floor.h as f64));
            fp.close_path();
            scene.fill(
                llimphi_ui::llimphi_raster::peniko::Fill::NonZero,
                Affine::IDENTITY,
                &llimphi_ui::llimphi_raster::peniko::Brush::Solid(Color::from_rgba8(26, 28, 38, 255)),
                None,
                &fp,
            );
            for (mesh, positions, image) in &painted {
                paint_textured(scene, mesh, positions, to_screen, image);
            }
        })
        .on_pointer_move_at(|lx, ly, _w, _h| Some(Msg::Cursor(Some((lx as f64, ly as f64)))))
        .on_pointer_leave(Msg::Cursor(None))
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
