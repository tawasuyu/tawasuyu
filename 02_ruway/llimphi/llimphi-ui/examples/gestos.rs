//! Demo de la **arena de gestos** de Llimphi (Tier 4 de PARIDAD-FLUTTER).
//!
//! Un canvas pannable + zoomable que ejercita los tres gestos nuevos:
//!
//! - **Ctrl + rueda** → `on_scale`: zoom hacia el cursor (camino universal de
//!   desktop; en macOS también responde al pinch del trackpad).
//! - **Arrastrar** (botón izquierdo) → `draggable`: paneo. Mover cancela un
//!   long-press en curso — esa desambiguación es la "arena".
//! - **Doble-click** → `on_double_tap`: resetea zoom y paneo.
//! - **Mantener apretado ~500 ms quieto** → `on_long_press_at`: deja una marca
//!   en el punto (coordenadas de mundo, así sigue al zoom/paneo).
//!
//! La barra inferior muestra el zoom, la cantidad de marcas y el último gesto.
//!
//! Corre con: `cargo run -p llimphi-ui --example gestos --release`.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle, Line, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::{App, DragPhase, GesturePhase, Handle, View};

#[derive(Clone)]
enum Msg {
    /// Zoom incremental con factor multiplicativo + punto focal (local al canvas).
    Zoom { factor: f32, fx: f32, fy: f32 },
    /// Paneo por delta de arrastre.
    Pan { dx: f32, dy: f32 },
    /// Doble-tap: resetear la vista.
    Reset,
    /// Long-press: dejar una marca en el punto (local al canvas).
    Mark { lx: f32, ly: f32 },
}

struct Model {
    zoom: f32,
    pan: (f32, f32),
    /// Marcas en coordenadas de **mundo** (independientes del zoom/paneo).
    marks: Vec<(f32, f32)>,
    last: String,
}

struct Gestos;

impl App for Gestos {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · gestos (pinch-zoom · double-tap · long-press)"
    }

    fn initial_size() -> (u32, u32) {
        (900, 640)
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        Model {
            zoom: 1.0,
            pan: (0.0, 0.0),
            marks: Vec::new(),
            last: "probá: Ctrl+rueda (zoom) · arrastrar (paneo) · doble-click (reset) · mantener (marca)".into(),
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Zoom { factor, fx, fy } => {
                // Zoom hacia el cursor: mantené fijo el punto de mundo bajo
                // (fx, fy) reajustando el paneo. new_pan = focal - rf·(focal - pan).
                let new_zoom = (model.zoom * factor).clamp(0.15, 12.0);
                let rf = new_zoom / model.zoom; // factor real tras el clamp
                model.pan.0 = fx - rf * (fx - model.pan.0);
                model.pan.1 = fy - rf * (fy - model.pan.1);
                model.zoom = new_zoom;
                model.last = format!("zoom ×{:.2}", model.zoom);
            }
            Msg::Pan { dx, dy } => {
                model.pan.0 += dx;
                model.pan.1 += dy;
                model.last = "paneo".into();
            }
            Msg::Reset => {
                model.zoom = 1.0;
                model.pan = (0.0, 0.0);
                model.last = "doble-tap → reset".into();
            }
            Msg::Mark { lx, ly } => {
                // Local del canvas → mundo: (local - pan) / zoom.
                let wx = (lx - model.pan.0) / model.zoom;
                let wy = (ly - model.pan.1) / model.zoom;
                model.marks.push((wx, wy));
                model.last = format!("long-press → marca #{} @ ({wx:.0}, {wy:.0})", model.marks.len());
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let zoom = model.zoom;
        let pan = model.pan;
        let marks = model.marks.clone();

        let canvas = View::new(Style {
            size: Size { width: percent(1.0_f32), height: Dimension::auto() },
            flex_grow: 1.0,
            ..Default::default()
        })
        .fill(Color::from_rgba8(16, 18, 26, 255))
        .clip(true)
        .paint_with(move |scene, _ts, rect| {
            // Grilla de mundo paso 40px, escalada por zoom y desplazada por pan.
            let step = 40.0 * zoom as f64;
            if step >= 4.0 {
                let thin = Stroke::new(1.0);
                let grid = Color::from_rgba8(40, 46, 60, 255);
                // Offset del primer línea visible (pan módulo step).
                let ox = (rect.x as f64) + (pan.0 as f64).rem_euclid(step);
                let mut x = ox;
                while x < (rect.x + rect.w) as f64 {
                    scene.stroke(&thin, Affine::IDENTITY, grid, None,
                        &Line::new((x, rect.y as f64), (x, (rect.y + rect.h) as f64)));
                    x += step;
                }
                let oy = (rect.y as f64) + (pan.1 as f64).rem_euclid(step);
                let mut y = oy;
                while y < (rect.y + rect.h) as f64 {
                    scene.stroke(&thin, Affine::IDENTITY, grid, None,
                        &Line::new((rect.x as f64, y), ((rect.x + rect.w) as f64, y)));
                    y += step;
                }
            }
            // Marcas (coords de mundo → pantalla): pan + world·zoom.
            let dot = Color::from_rgba8(90, 220, 150, 255);
            let r = (6.0 * zoom as f64).clamp(3.0, 24.0);
            for (wx, wy) in &marks {
                let sx = rect.x as f64 + pan.0 as f64 + (*wx as f64) * zoom as f64;
                let sy = rect.y as f64 + pan.1 as f64 + (*wy as f64) * zoom as f64;
                scene.fill(Fill::NonZero, Affine::IDENTITY, dot, None, &Circle::new((sx, sy), r));
            }
        })
        // Pinch-to-zoom (Ctrl+rueda / trackpad). El focal viene local al canvas.
        .on_scale(|phase, factor, fx, fy| match phase {
            GesturePhase::Update => Some(Msg::Zoom { factor, fx, fy }),
            _ => None,
        })
        // Paneo por arrastre. El movimiento cancela un long-press en curso.
        .draggable(|phase, dx, dy| match phase {
            DragPhase::Move => Some(Msg::Pan { dx, dy }),
            DragPhase::End => None,
        })
        // Doble-tap: reset. Long-press: marca en el punto.
        .on_double_tap(Msg::Reset)
        .on_long_press_at(|lx, ly, _w, _h| Some(Msg::Mark { lx, ly }));

        let status = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(Color::from_rgba8(28, 32, 42, 255))
        .text(
            format!("{}   ·   ×{:.2}   ·   {} marcas", model.last, model.zoom, model.marks.len()),
            18.0,
            Color::from_rgba8(210, 220, 235, 255),
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(Color::from_rgba8(16, 18, 26, 255))
        .children(vec![canvas, status])
    }
}

fn main() {
    llimphi_ui::run::<Gestos>();
}
