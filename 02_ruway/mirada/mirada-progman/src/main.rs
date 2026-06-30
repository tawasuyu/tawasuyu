//! `mirada-progman` — el **Program Manager** de Windows 3.1, de vuelta como app
//! **cliente real**: una ventana Llimphi (toplevel movible, con su barra de
//! título) que muestra una grilla de programas y los lanza al click. Esto es lo
//! que pata —una barra/layer-shell— no podía ser; por eso el PM es una app.
//!
//! Pensado para correr como **autoexec efímero** de la vista `windows-3.1`:
//! aparece con esa vista y el compositor lo termina al cambiar de vista.
//!
//! El dato son las apps del registro (`app-bus`). Estética Win3.1: gris Motif,
//! barra de título azul marino, grilla de íconos.

use app_bus::{AppEntry, AppRegistry};
use llimphi_ui::llimphi_layout::taffy::prelude::*;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, View};

const NAVY: Color = Color::from_rgba8(0, 0, 128, 255);
const GRIS: Color = Color::from_rgba8(192, 192, 192, 255);
const GRIS_OSCURO: Color = Color::from_rgba8(128, 128, 128, 255);
const BLANCO: Color = Color::from_rgba8(255, 255, 255, 255);
const NEGRO: Color = Color::from_rgba8(0, 0, 0, 255);

#[derive(Clone)]
enum Msg {
    Launch(String),
}

struct Progman;

impl App for Progman {
    type Model = AppRegistry;
    type Msg = Msg;

    fn title() -> &'static str {
        "Administrador de programas"
    }
    fn app_id() -> Option<&'static str> {
        Some("com.tawasuyu.progman")
    }
    fn initial_size() -> (u32, u32) {
        (660, 480)
    }

    fn init(_: &Handle<Msg>) -> AppRegistry {
        AppRegistry::with_defaults()
    }

    fn update(model: AppRegistry, msg: Msg, _: &Handle<Msg>) -> AppRegistry {
        match msg {
            Msg::Launch(id) => {
                if let Some(e) = model.get(&id) {
                    if let Err(err) = e.spawn() {
                        eprintln!("progman: no pude lanzar {id}: {err}");
                    }
                }
            }
        }
        model
    }

    fn view(model: &AppRegistry) -> View<Msg> {
        // Barra de título azul marino — el sello de Win3.1.
        let titulo = View::new(Style {
            size: Size { width: percent(1.0), height: length(24.0) },
            align_items: Some(AlignItems::Center),
            padding: Rect {
                left: length(8.0),
                right: length(8.0),
                top: length(0.0),
                bottom: length(0.0),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .fill(NAVY)
        .text("Administrador de programas".to_string(), 13.0, BLANCO);

        // Grilla de programas (flex-wrap).
        let tiles: Vec<View<Msg>> = model.all().iter().map(tile).collect();
        let grilla = View::new(Style {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_content: Some(AlignContent::FlexStart),
            gap: Size { width: length(12.0), height: length(12.0) },
            padding: Rect {
                left: length(14.0),
                right: length(14.0),
                top: length(14.0),
                bottom: length(14.0),
            },
            size: Size { width: percent(1.0), height: auto() },
            flex_grow: 1.0,
            ..Default::default()
        })
        .fill(GRIS)
        .children(tiles);

        // La «ventana»: marco gris con barra azul arriba.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0), height: percent(1.0) },
            ..Default::default()
        })
        .fill(GRIS_OSCURO)
        .children(vec![titulo, grilla])
    }
}

/// Un ícono de programa: glifo grande + etiqueta, lanza al click.
fn tile(e: &AppEntry) -> View<Msg> {
    let glifo = e
        .icon
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| e.label.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default());

    let icono = View::new(Style {
        size: Size { width: length(48.0), height: length(48.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(glifo, 30.0, NEGRO);

    let etiqueta = View::new(Style {
        size: Size { width: percent(1.0), height: auto() },
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(e.label.clone(), 12.0, NEGRO);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(84.0), height: length(78.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(0.0), height: length(4.0) },
        ..Default::default()
    })
    .on_click(Msg::Launch(e.id.clone()))
    .children(vec![icono, etiqueta])
}

fn main() {
    llimphi_ui::run::<Progman>();
}
