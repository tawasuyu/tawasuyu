//! Showcase de `dominium-canvas-llimphi`: arma un mundo pequeño con
//! patrones manuales (vetas de oro, parches de materia, niebla de
//! psique), construye el `RenderPlan` con `build_plan` y lo pinta
//! centrado en la ventana.
//!
//! Sin loop de simulación — la app Llimphi completa con tick vivo
//! va en `dominium-app-llimphi` (próximo bloque).
//!
//! Corré con: `cargo run -p dominium-canvas-llimphi --example canvas_demo --release`.

use dominium_canvas_llimphi::canvas_view;
use dominium_core::World;
use dominium_iso::{IsoProjector, ZWeights};
use dominium_render_plan::{build_plan, PlanConfig};
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, View};

const GRID: usize = 32;

#[derive(Clone)]
enum Msg {}

struct Model {
    world: World,
}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "dominium · canvas showcase"
    }

    fn initial_size() -> (u32, u32) {
        (1000, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model { world: seed() }
    }

    fn update(model: Model, _: Msg, _: &Handle<Msg>) -> Model {
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let iso = IsoProjector::new(1.0, 4.0);
        let weights = ZWeights::default();
        let cfg = PlanConfig::default();
        let plan = build_plan(&model.world, &iso, &weights, &cfg);

        let canvas = canvas_view::<Msg>(plan, Some(Color::from_rgba8(14, 16, 22, 255)));

        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![canvas])
    }
}

/// Mundo sembrado a mano: continentes de materia en el centro, vetas
/// de oro en una diagonal y un parche de psique en una esquina. Sin
/// PRNG — siempre la misma escena entre runs.
fn seed() -> World {
    let mut w = World::new(GRID, GRID);
    for cy in 0..GRID {
        for cx in 0..GRID {
            let idx = w.grid.idx(cx, cy);
            // Continente: gauss centrado.
            let dx = cx as f32 - (GRID as f32 * 0.5);
            let dy = cy as f32 - (GRID as f32 * 0.5);
            let d2 = dx * dx + dy * dy;
            let materia = (40.0 - d2 * 0.15).max(0.0);
            w.grid.materia[idx] = materia;

            // Veta de oro en la diagonal cx == cy.
            if cx == cy && cx > 4 && cx < GRID - 4 {
                w.grid.oro[idx] = 35.0;
            }

            // Psique en el cuadrante inferior derecho.
            if cx > GRID * 2 / 3 && cy > GRID * 2 / 3 {
                w.grid.psique[idx] = 18.0;
            }

            // Borde de degradación.
            if cx == 0 || cy == 0 || cx == GRID - 1 || cy == GRID - 1 {
                w.grid.degradacion[idx] = 25.0;
            }
        }
    }
    w
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
