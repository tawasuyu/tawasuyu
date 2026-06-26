//! Demo que **cierra el círculo** del motor: máquina de estados → animación de
//! huesos → skinning → malla deformada.
//!
//! Dos clips son `BoneAnimation`s (animaciones esqueletales keyframeadas):
//! `rest` (tira recta) y `wave` (onda viajera). Una máquina de estados
//! (`llimphi-anim`) transiciona entre ellos por el input `active`, y
//! `pose_from_render_frame` posa el esqueleto **blendeando poses** durante el
//! crossfade — así la tira se *ease-in* a la onda en vez de saltar. El skinning
//! deforma la malla texturizada (`llimphi-mesh`).
//!
//! **Espacio** alterna `active` (rest ⇄ wave). Todo Rust nativo sobre vello 0.7.
//!
//! Corre con:
//!   `cargo run -p llimphi-mesh --example clips_to_bones_demo --release`

use std::f64::consts::PI;
use std::sync::Arc;
use std::time::Duration;

use llimphi_anim::skel::{
    pose_from_render_frame, BoneAnimation, BoneTrack, Mesh, Pose, PoseKey, Skeleton, Vertex,
};
use llimphi_anim::{Condition, Instance, StateMachine};
use llimphi_mesh::{fit_transform, paint_textured, paint_wireframe, rest_bounds};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Point, Vec2};
use llimphi_ui::llimphi_raster::peniko::{
    Blob, Color, ImageAlphaType, ImageBrush, ImageData, ImageFormat,
};
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, PaintRect, View};

const SEG: usize = 8;
const STEP: f64 = 30.0;
const STRIP_H: f64 = 72.0;

/// Cadena de huesos (raíz anclada + SEG eslabones) + malla-tira atada rigid.
fn build_rig() -> (Skeleton, Mesh) {
    let mut s = Skeleton::new();
    s.add_bone(None, Pose::translate(Vec2::new(0.0, STRIP_H / 2.0)));
    for _ in 1..=SEG {
        s.add_bone(Some(s.len() - 1), Pose::translate(Vec2::new(STEP, 0.0)));
    }
    s.bind();

    let mut m = Mesh::new();
    for i in 0..=SEG {
        let x = i as f64 * STEP;
        let u = i as f64 / SEG as f64;
        m.vertices.push(Vertex::rigid(Point::new(x, 0.0), (u, 0.0), i));
        m.vertices.push(Vertex::rigid(Point::new(x, STRIP_H), (u, 1.0), i));
    }
    for i in 0..SEG {
        let (t0, t1) = ((2 * i) as u32, (2 * (i + 1)) as u32);
        let (b0, b1) = ((2 * i + 1) as u32, (2 * (i + 1) + 1) as u32);
        m.triangles.push([t0, t1, b1]);
        m.triangles.push([t0, b1, b0]);
    }
    (s, m)
}

/// Pose de un eslabón: mantiene la translación STEP (estructura de la cadena) y
/// anima sólo la rotación.
fn link_pose(rot: f64) -> Pose {
    Pose::new(Vec2::new(STEP, 0.0), rot, Vec2::new(1.0, 1.0))
}

/// Clip "recto": todos los eslabones a rotación 0 (un keyframe).
fn straight_anim() -> BoneAnimation {
    let tracks = (1..=SEG)
        .map(|i| BoneTrack {
            bone: i,
            keys: vec![PoseKey { t: 0.0, pose: link_pose(0.0) }],
        })
        .collect();
    BoneAnimation { duration: 1.0, looping: true, tracks }
}

/// Clip "onda": cada eslabón keyframeado con una senoidal de fase creciente.
fn wave_anim(amp: f64, period: f64, nkeys: usize) -> BoneAnimation {
    let tracks = (1..=SEG)
        .map(|i| {
            let keys = (0..=nkeys)
                .map(|k| {
                    let t = period * k as f64 / nkeys as f64;
                    let r = amp * (2.0 * PI * t / period - i as f64 * 0.6).sin();
                    PoseKey { t, pose: link_pose(r) }
                })
                .collect();
            BoneTrack { bone: i, keys }
        })
        .collect();
    BoneAnimation { duration: period, looping: true, tracks }
}

fn checker(n: u32, sq: u32) -> ImageBrush {
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let on = ((x / sq + y / sq) % 2) == 0;
            let (r, g, b) = if on { (70, 130, 205) } else { (240, 180, 70) };
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
    Toggle,
}

struct Model {
    sm: Instance,
    skel: Skeleton,
    mesh: Mesh,
    clips: Vec<BoneAnimation>,
    image: ImageBrush,
    active: bool,
}

struct Demo;

const TICK: Duration = Duration::from_millis(16);

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · clips → huesos → malla"
    }
    fn initial_size() -> (u32, u32) {
        (560, 420)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let (skel, mesh) = build_rig();
        let clips = vec![straight_anim(), wave_anim(0.42, 2.2, 16)];

        let mut sm = StateMachine::new();
        let rest = sm.add_state("rest", 0, 1.0, true);
        let wave = sm.add_state("wave", 1, 1.0, true);
        sm.set_entry(rest);
        sm.transition(rest, wave, vec![Condition::bool("active", true)], 0.5);
        sm.transition(wave, rest, vec![Condition::bool("active", false)], 0.5);

        handle.spawn_periodic(TICK, || Msg::Tick);

        Model {
            sm: sm.instance(),
            skel,
            mesh,
            clips,
            image: checker(8, 1),
            active: false,
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => {
                model.sm.advance(TICK.as_secs_f64());
                // El estado/transición de la máquina posa el esqueleto (blend de
                // poses durante el crossfade). pose_from_render_frame llama
                // skel.update() internamente.
                let frame = model.sm.render_frame();
                pose_from_render_frame(&mut model.skel, &frame, &model.clips);
            }
            Msg::Toggle => {
                model.active = !model.active;
                model.sm.set_bool("active", model.active);
            }
        }
        model
    }

    fn on_key(_: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state == KeyState::Pressed && e.key == Key::Named(NamedKey::Space) {
            Some(Msg::Toggle)
        } else {
            None
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let positions = model.mesh.deform(&model.skel);
        let mesh = model.mesh.clone();
        let image = model.image.clone();

        let stage = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .paint_with(move |scene, _ts, rect| {
            let inner = PaintRect {
                x: rect.x + 50.0,
                y: rect.y + 50.0,
                w: (rect.w - 100.0).max(1.0),
                h: (rect.h - 100.0).max(1.0),
            };
            let xf = fit_transform(rest_bounds(&mesh), inner);
            paint_textured(scene, &mesh, &positions, xf, &image);
            paint_wireframe(scene, &mesh, &positions, xf, Color::from_rgba8(255, 255, 255, 110), 1.2);
        });

        let label = if model.sm.is_transitioning() {
            "· · · blend de poses · · ·".to_string()
        } else {
            format!("estado: {}   (Espacio: rest ⇄ wave)", model.sm.current_state())
        };
        let status = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(label, 14.0, Color::from_rgba8(150, 165, 190, 255));

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(8.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(18, 22, 30, 255))
        .children(vec![stage, status])
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
