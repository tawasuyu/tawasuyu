//! Demo **lottie + rive** en una sola app: los dos paradigmas de animación del
//! motor nativo trabajando juntos.
//!
//! - **rive (esqueletal + IK)**: un brazo de 2 huesos con una malla texturizada
//!   skinneada, que **persigue el cursor** resolviendo IK de 2 huesos cada frame
//!   (`llimphi_anim::constraint::solve_two_bone_ik`).
//! - **lottie**: un pulso vectorial (`llimphi-lottie` sobre el fork de velato)
//!   reproducido en la **punta del brazo** y otro marcando el **objetivo** bajo
//!   el cursor.
//!
//! Mové el mouse: el brazo alcanza el cursor (IK), el Lottie late en la punta y
//! en el blanco. **F** alterna el codo (flip de la solución IK). Todo Rust
//! nativo sobre vello 0.7, cero C++.
//!
//! Corre con:
//!   `cargo run -p llimphi-mesh --example lottie_rive_demo --release`

use std::sync::Arc;
use std::time::Duration;

use llimphi_anim::constraint::solve_two_bone_ik;
use llimphi_anim::skel::{BoneId, Mesh, Pose, Skeleton, Vertex, Weight};
use llimphi_lottie::LottieAsset;
use llimphi_mesh::{paint_textured, paint_wireframe};
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, Vec2};
use llimphi_ui::llimphi_raster::peniko::{
    Blob, Color, ImageAlphaType, ImageBrush, ImageData, ImageFormat,
};
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, PaintRect, View};

// Brazo en coordenadas locales del lienzo (px). Ancla a la izquierda-centro.
const ANCHOR: (f64, f64) = (110.0, 200.0);
const L1: f64 = 95.0; // hueso superior
const L2: f64 = 95.0; // hueso inferior
const ARM_H: f64 = 40.0; // grosor del brazo
const COLS: usize = 6; // columnas de la malla a lo largo del brazo

/// Pulso azul: círculo cuya opacidad late en 1.5 s.
const PULSE_LOTTIE: &str = r#"{
  "v":"5.5.2","fr":30,"ip":0,"op":45,"w":100,"h":100,
  "layers":[{"ty":4,"ip":0,"op":45,"st":0,"sr":1,
    "ks":{"o":{"a":1,"k":[
        {"i":{"x":[0.5],"y":[0.5]},"o":{"x":[0.5],"y":[0.5]},"t":0,"s":[100]},
        {"i":{"x":[0.5],"y":[0.5]},"o":{"x":[0.5],"y":[0.5]},"t":22,"s":[25]},
        {"t":45,"s":[100]}]},
      "r":{"a":0,"k":0},"p":{"a":0,"k":[50,50]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]}},
    "shapes":[{"ty":"gr","it":[
      {"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[60,60]}},
      {"ty":"fl","c":{"a":0,"k":[0.45,0.75,1.0]},"o":{"a":0,"k":100}},
      {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}]}]}]}"#;

/// Brazo de 2 huesos + malla-tira skinneada (peso suave alrededor del codo).
fn build_arm() -> (Skeleton, Mesh, BoneId, BoneId) {
    let mut s = Skeleton::new();
    let a = s.add_bone(None, Pose::translate(Vec2::new(ANCHOR.0, ANCHOR.1)));
    let b = s.add_bone(Some(a), Pose::translate(Vec2::new(L1, 0.0)));
    s.bind();
    s.update();

    let total = L1 + L2;
    let blend = total * 0.16; // ancho del blend de peso en el codo
    let mut m = Mesh::new();
    for i in 0..=COLS {
        let p = total * i as f64 / COLS as f64; // posición a lo largo del brazo
        // Peso suave: 1→A antes del codo, 0→A (todo B) después.
        let wa = (1.0 - (p - (L1 - blend)) / (2.0 * blend)).clamp(0.0, 1.0);
        let weights = vec![
            Weight { bone: a, weight: wa },
            Weight { bone: b, weight: 1.0 - wa },
        ];
        let u = i as f64 / COLS as f64;
        let x = ANCHOR.0 + p;
        m.vertices.push(Vertex {
            rest: Point::new(x, ANCHOR.1 - ARM_H / 2.0),
            uv: (u, 0.0),
            weights: weights.clone(),
        });
        m.vertices.push(Vertex {
            rest: Point::new(x, ANCHOR.1 + ARM_H / 2.0),
            uv: (u, 1.0),
            weights,
        });
    }
    for i in 0..COLS {
        let (t0, t1) = ((2 * i) as u32, (2 * (i + 1)) as u32);
        let (b0, b1) = ((2 * i + 1) as u32, (2 * (i + 1) + 1) as u32);
        m.triangles.push([t0, t1, b1]);
        m.triangles.push([t0, b1, b0]);
    }
    (s, m, a, b)
}

fn checker(n: u32, sq: u32) -> ImageBrush {
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let on = ((x / sq + y / sq) % 2) == 0;
            let (r, g, b) = if on { (235, 110, 90) } else { (250, 200, 120) };
            px.extend_from_slice(&[r, g, b, 255]);
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

#[derive(Clone)]
enum Msg {
    Tick,
    Target(f64, f64),
    Flip,
}

struct Model {
    skel: Skeleton,
    mesh: Mesh,
    upper: BoneId,
    lower: BoneId,
    image: ImageBrush,
    pulse: LottieAsset,
    target: Point,
    t: f64,
    flip: bool,
}

struct Demo;

const TICK: Duration = Duration::from_millis(16);
const TIP_LOCAL: Vec2 = Vec2::new(L2, 0.0);

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · lottie + rive"
    }
    fn initial_size() -> (u32, u32) {
        (640, 420)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let (skel, mesh, upper, lower) = build_arm();
        handle.spawn_periodic(TICK, || Msg::Tick);
        Model {
            skel,
            mesh,
            upper,
            lower,
            image: checker(8, 1),
            pulse: LottieAsset::from_str(PULSE_LOTTIE).expect("pulse lottie"),
            target: Point::new(360.0, 150.0),
            t: 0.0,
            flip: false,
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => {
                model.t += TICK.as_secs_f64();
                // IK cada frame: el brazo alcanza el objetivo (cursor).
                solve_two_bone_ik(
                    &mut model.skel,
                    model.upper,
                    model.lower,
                    TIP_LOCAL,
                    model.target,
                    model.flip,
                );
            }
            Msg::Target(x, y) => model.target = Point::new(x, y),
            Msg::Flip => model.flip = !model.flip,
        }
        model
    }

    fn on_key(_: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state == KeyState::Pressed {
            if let Key::Character(c) = &e.key {
                if c.as_str() == "f" {
                    return Some(Msg::Flip);
                }
            }
        }
        None
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let positions = model.mesh.deform(&model.skel);
        // Punta del brazo (mundo local) = world(lower) · tip_local.
        let tip_local = model.skel.world(model.lower) * Point::new(TIP_LOCAL.x, TIP_LOCAL.y);
        let mesh = model.mesh.clone();
        let image = model.image.clone();
        let pulse = model.pulse.clone();
        let target = model.target;
        let t = model.t;

        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(18, 22, 30, 255))
        .paint_with(move |scene, _ts, rect| {
            // Coords locales → pantalla (el lienzo arranca en rect.x/rect.y).
            let to_screen = Affine::translate((rect.x as f64, rect.y as f64));
            // Brazo skinneado (rive).
            paint_textured(scene, &mesh, &positions, to_screen, &image);
            paint_wireframe(
                scene,
                &mesh,
                &positions,
                to_screen,
                Color::from_rgba8(255, 255, 255, 90),
                1.0,
            );
            // Pulso lottie en la punta del brazo y en el objetivo.
            let pulse_rect = |p: Point, s: f64| PaintRect {
                x: (rect.x as f64 + p.x - s) as f32,
                y: (rect.y as f64 + p.y - s) as f32,
                w: (s * 2.0) as f32,
                h: (s * 2.0) as f32,
            };
            pulse.paint_at_time(scene, pulse_rect(target, 30.0), t);
            pulse.paint_at_time(scene, pulse_rect(tip_local, 22.0), t);
        })
        .on_pointer_move_at(|lx, ly, _w, _h| Some(Msg::Target(lx as f64, ly as f64)))
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
