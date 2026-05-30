//! Demo: un documento **markdown real** presentado como recorrido espacial.
//!
//! Cadena completa: `markdown → pluma_md::parse_md → átomos → Recorrido`.
//! Cada encabezado (`#`, `##`, …) abre un "slide" cuyo título es el del
//! encabezado y cuyos párrafos son los bloques siguientes hasta el próximo
//! encabezado. `en_rejilla` los coloca y rutea en orden de lectura.
//!
//! El adaptador `slides_desde_atoms` vive aquí (no en el lib ni en el core):
//! es glue de demostración entre el modelo de documento de pluma y el
//! `ContenidoMarco` agnóstico de `pluma-deck-core`. Si prueba valor, se
//! promueve a un crate adaptador propio.
//!
//! Corre con un .md propio o el de ejemplo embebido:
//!   `cargo run -p pluma-deck-recorrido-llimphi --example recorrido_md_demo --release [archivo.md]`

use std::time::Duration;

use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use pluma_core::NarrativeAtom;
use pluma_deck_core::{ContenidoMarco, Recorrido, RecorridoState, Rect, RejillaOpts};
use pluma_deck_recorrido_llimphi::{dentro, panel_actual, recorrido_view, ZOOM_BASE};

const PANEL_INICIAL: Rect = Rect { x: 0.0, y: 0.0, w: 1100.0, h: 720.0 };

const MD_EJEMPLO: &str = "\
# Presentaciones desde markdown

gioser convierte un documento real en un recorrido espacial.

Cada encabezado abre un marco; los párrafos que le siguen son su cuerpo.

## El pipeline

El markdown se parsea con `pluma-md` en átomos (un bloque por átomo).

El adaptador agrupa esos átomos en slides por encabezado.

## El render

`pluma-deck-core` coloca los slides en una rejilla y arma la ruta.

El frontend Llimphi pinta cada marco y la cámara vuela entre ellos.

## Cierre

Mismo material que ya escribís — presentado sin diapositivas.
";

/// Adaptador: átomos de pluma (en orden de documento) → slides. Un átomo cuyo
/// contenido arranca con `#`+espacio es un encabezado y abre un slide nuevo.
fn slides_desde_atoms(atoms: &[NarrativeAtom]) -> Vec<ContenidoMarco> {
    let mut slides = Vec::new();
    let mut titulo: Option<String> = None;
    let mut parrafos: Vec<String> = Vec::new();

    for a in atoms {
        let c = a.content.as_str();
        let hashes = c.chars().take_while(|&ch| ch == '#').count();
        let es_encabezado = hashes > 0 && c[hashes..].starts_with(' ');
        if es_encabezado {
            empujar(&mut slides, &mut titulo, &mut parrafos);
            titulo = Some(c[hashes..].trim().to_string());
        } else {
            parrafos.push(c.to_string());
        }
    }
    empujar(&mut slides, &mut titulo, &mut parrafos);
    slides
}

fn empujar(slides: &mut Vec<ContenidoMarco>, titulo: &mut Option<String>, parrafos: &mut Vec<String>) {
    if titulo.is_some() || !parrafos.is_empty() {
        slides.push(ContenidoMarco::Texto {
            titulo: titulo.take(),
            parrafos: std::mem::take(parrafos),
        });
    }
}

#[derive(Clone)]
enum Msg {
    Zoom { mult: f64, cursor: (f32, f32) },
    Pan { dx: f32, dy: f32 },
    Siguiente,
    Anterior,
    Tick,
}

struct Model {
    rec: Recorrido,
    state: RecorridoState,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · recorrido desde markdown"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 720)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        // Si pasan un .md por arg lo leemos; si no, el embebido.
        let md = std::env::args()
            .nth(1)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_else(|| MD_EJEMPLO.to_string());
        let doc = pluma_md::parse_md(&md, "es", "recorrido", 0);
        let slides = slides_desde_atoms(&doc.atoms);
        let rec = Recorrido::en_rejilla(
            slides,
            RejillaOpts { cols: 3, marco_w: 660.0, marco_h: 420.0, gap_x: 240.0, gap_y: 200.0 },
        );

        let mut state = RecorridoState::new();
        state.saltar_a_paso(&rec, 0, PANEL_INICIAL);
        handle.spawn_periodic(Duration::from_millis(16), || Msg::Tick);
        Model { rec, state }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        let panel = panel_actual().unwrap_or(PANEL_INICIAL);
        match msg {
            Msg::Zoom { mult, cursor } => {
                model.state.wheel(mult, (cursor.0 as f64, cursor.1 as f64), panel);
            }
            Msg::Pan { dx, dy } => model.state.arrastrar_delta(dx as f64, dy as f64),
            Msg::Siguiente => {
                model.state.siguiente(&model.rec, panel);
            }
            Msg::Anterior => {
                model.state.anterior(&model.rec, panel);
            }
            Msg::Tick => {
                model.state.avanzar(1.0 / 60.0);
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        recorrido_view(&model.rec, &model.state).draggable(|phase, dx, dy| match phase {
            DragPhase::Move => Some(Msg::Pan { dx, dy }),
            DragPhase::End => None,
        })
    }

    fn on_wheel(_m: &Self::Model, delta: WheelDelta, cursor: (f32, f32), _mods: Modifiers) -> Option<Self::Msg> {
        let panel = panel_actual()?;
        if !dentro(panel, cursor.0, cursor.1) {
            return None;
        }
        Some(Msg::Zoom { mult: ZOOM_BASE.powf(-delta.y as f64), cursor })
    }

    fn on_key(_m: &Self::Model, ev: &KeyEvent) -> Option<Self::Msg> {
        if ev.state != KeyState::Pressed {
            return None;
        }
        match &ev.key {
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown | NamedKey::Enter | NamedKey::Space) => {
                Some(Msg::Siguiente)
            }
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => Some(Msg::Anterior),
            _ => None,
        }
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
