//! Demo del Tier 4: una tira **texturizada deformada por una cadena de huesos**
//! que ondula como una bandera/tentáculo.
//!
//! La cadena de huesos (`llimphi_anim::skel`) se re-posa cada frame con una onda
//! senoidal de fase creciente; el skinning (LBS) deforma los vértices; y
//! `llimphi-mesh` pinta la malla texturizada (warp piecewise-affine) con un
//! wireframe encima para ver la deformación. **Espacio** alterna el wireframe.
//!
//! Todo Rust nativo, sobre vello 0.7 — cero C++.
//!
//! Corre con:
//!   `cargo run -p llimphi-mesh --example bones_demo --release`

use std::sync::Arc;
use std::time::Duration;

use llimphi_anim::skel::{Mesh, Pose, Skeleton, Vertex};
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

const SEG: usize = 8; // segmentos de la tira (= huesos de la cadena, +1 raíz)
const STEP: f64 = 30.0; // largo de cada hueso/segmento
const STRIP_H: f64 = 72.0; // alto de la tira

/// Construye la cadena de huesos (raíz + SEG eslabones) y la malla-tira atada a
/// ella (rigid por columna), con UVs de 0..1 a lo ancho/alto.
fn build() -> (Skeleton, Mesh) {
    let mut s = Skeleton::new();
    // Raíz anclada a la izquierda; cada eslabón se traslada STEP en x respecto
    // al padre. Bone i tiene id i (orden de inserción).
    s.add_bone(None, Pose::translate(Vec2::new(0.0, STRIP_H / 2.0)));
    for _ in 1..=SEG {
        s.add_bone(Some(s.len() - 1), Pose::translate(Vec2::new(STEP, 0.0)));
    }
    s.bind();

    let mut m = Mesh::new();
    for i in 0..=SEG {
        let x = i as f64 * STEP;
        let u = i as f64 / SEG as f64;
        m.vertices.push(Vertex::rigid(Point::new(x, 0.0), (u, 0.0), i)); // borde sup
        m.vertices.push(Vertex::rigid(Point::new(x, STRIP_H), (u, 1.0), i)); // borde inf
    }
    for i in 0..SEG {
        let (t0, t1) = ((2 * i) as u32, (2 * (i + 1)) as u32);
        let (b0, b1) = ((2 * i + 1) as u32, (2 * (i + 1) + 1) as u32);
        m.triangles.push([t0, t1, b1]);
        m.triangles.push([t0, b1, b0]);
    }
    (s, m)
}

/// Textura procedural: tablero de ajedrez (para ver el warp).
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
    ToggleWire,
}

struct Model {
    skel: Skeleton,
    mesh: Mesh,
    image: ImageBrush,
    t: f64,
    wireframe: bool,
}

struct Demo;

const TICK: Duration = Duration::from_millis(16);

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · malla deformada por huesos"
    }
    fn initial_size() -> (u32, u32) {
        (520, 420)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let (skel, mesh) = build();
        handle.spawn_periodic(TICK, || Msg::Tick);
        Model {
            skel,
            mesh,
            image: checker(8, 1),
            t: 0.0,
            wireframe: true,
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => {
                model.t += TICK.as_secs_f64();
                // Onda viajera: cada hueso oscila con fase creciente → la tira
                // ondula desde el ancla hacia la punta.
                for i in 1..=SEG {
                    let r = 0.38 * (model.t * 3.0 - i as f64 * 0.6).sin();
                    model
                        .skel
                        .set_pose(i, Pose::new(Vec2::new(STEP, 0.0), r, Vec2::new(1.0, 1.0)));
                }
                model.skel.update();
            }
            Msg::ToggleWire => model.wireframe = !model.wireframe,
        }
        model
    }

    fn on_key(_: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state == KeyState::Pressed && e.key == Key::Named(NamedKey::Space) {
            Some(Msg::ToggleWire)
        } else {
            None
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        // Deformar acá (lectura) y mover los datos al closure de pintura.
        let positions = model.mesh.deform(&model.skel);
        let mesh = model.mesh.clone();
        let image = model.image.clone();
        let wire = model.wireframe;

        let stage = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .paint_with(move |scene, _ts, rect| {
            // Margen para que la onda no se recorte contra el borde.
            let inner = PaintRect {
                x: rect.x + 50.0,
                y: rect.y + 50.0,
                w: (rect.w - 100.0).max(1.0),
                h: (rect.h - 100.0).max(1.0),
            };
            let xf = fit_transform(rest_bounds(&mesh), inner);
            paint_textured(scene, &mesh, &positions, xf, &image);
            if wire {
                paint_wireframe(
                    scene,
                    &mesh,
                    &positions,
                    xf,
                    Color::from_rgba8(255, 255, 255, 120),
                    1.2,
                );
            }
        });

        let hint = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(30.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(
            "tira texturizada deformada por una cadena de huesos · Espacio: wireframe".to_string(),
            13.0,
            Color::from_rgba8(140, 155, 180, 255),
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(8.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(18, 22, 30, 255))
        .children(vec![stage, hint])
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
