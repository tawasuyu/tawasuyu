//! Demo del modo Recorrido (presentación espacial tipo Prezi).
//!
//! Un lienzo infinito con 5 marcos esparcidos a distintas escalas y giros;
//! la cámara vuela entre ellos siguiendo la ruta. Controles:
//!   - **→ / ↓ / Espacio / Enter**: paso siguiente (la cámara vuela al marco).
//!   - **← / ↑**: paso anterior.
//!   - **Home / Esc**: vista general (aleja para ver todo el lienzo).
//!   - **rueda**: zoom-a-cursor.
//!   - **arrastrar**: paneo libre por el lienzo.
//!
//! Corre con:
//!   `cargo run -p pluma-deck-recorrido-llimphi --example recorrido_demo --release`

use std::time::Duration;

use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use pluma_deck_core::{ContenidoMarco, Marco, Recorrido, RecorridoState, Rect, RejillaOpts};
use pluma_deck_recorrido_llimphi::{dentro, panel_actual, recorrido_view, ZOOM_BASE};

/// Panel inicial supuesto antes del primer paint (= `initial_size`), para
/// encuadrar el primer marco de entrada. Tras el primer frame se usa el real.
const PANEL_INICIAL: Rect = Rect { x: 0.0, y: 0.0, w: 1100.0, h: 720.0 };

#[derive(Clone)]
enum Msg {
    Zoom { mult: f64, cursor: (f32, f32) },
    Pan { dx: f32, dy: f32 },
    Siguiente,
    Anterior,
    VistaGeneral,
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
        "pluma · recorrido (presentación espacial tipo Prezi)"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 720)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        // Seis "slides" con contenido real (título + párrafos), auto-colocados
        // en rejilla por en_rejilla; la ruta los recorre en orden de lectura.
        let slide = |t: &str, ps: &[&str]| ContenidoMarco::Texto {
            titulo: Some(t.into()),
            parrafos: ps.iter().map(|s| s.to_string()).collect(),
        };
        let contenidos = vec![
            slide(
                "Presentaciones espaciales",
                &[
                    "Tipo Prezi: un lienzo infinito en vez de una pila de diapositivas.",
                    "La cámara vuela entre marcos haciendo zoom y paneo — el recorrido ES la narrativa.",
                ],
            ),
            slide(
                "Un solo material",
                &[
                    "Cada marco vive en coordenadas de mundo; el orden de los pasos define el guion.",
                    "El strip lineal de pluma-deck es el caso degenerado de esto.",
                ],
            ),
            slide(
                "Zoom narrativo",
                &["Alejarse muestra el mapa completo; acercarse, el detalle.", "El zoom se interpola en espacio logarítmico para un vuelo natural."],
            ),
            slide(
                "Contenido nativo",
                &["El contenido es agnóstico (título + párrafos).", "Un adaptador mapea un cuerpo o subgrafo de pluma a estos marcos."],
            ),
            slide(
                "Controles",
                &["Flechas / Espacio / Enter: volar al paso.", "Rueda: zoom-a-cursor.  Arrastrar: paneo libre."],
            ),
            slide("Fin", &["Esto es la Fase 3a del §6.sexies en marcha."]),
        ];
        let mut rec = Recorrido::en_rejilla(
            contenidos,
            RejillaOpts { cols: 3, marco_w: 660.0, marco_h: 420.0, gap_x: 240.0, gap_y: 200.0 },
        );
        // Un par de marcos sueltos con giro para lucir la libertad espacial.
        let id_a = (rec.marcos.len() + 1) as u64;
        rec.agregar_marco(
            Marco::new(id_a, Rect::new(-520.0, 760.0, 300.0, 200.0), ContenidoMarco::Etiqueta("← lienzo infinito →".into()))
                .con_giro(-0.12),
        );

        let mut state = RecorridoState::new();
        state.saltar_a_paso(&rec, 0, PANEL_INICIAL);

        // Tick de animación a ~60 Hz; avanzar() es no-op cuando no hay vuelo.
        handle.spawn_periodic(Duration::from_millis(16), || Msg::Tick);

        Model { rec, state }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        let panel = panel_actual().unwrap_or(PANEL_INICIAL);
        match msg {
            Msg::Zoom { mult, cursor } => {
                model.state.wheel(mult, (cursor.0 as f64, cursor.1 as f64), panel);
            }
            Msg::Pan { dx, dy } => {
                model.state.arrastrar_delta(dx as f64, dy as f64);
            }
            Msg::Siguiente => {
                model.state.siguiente(&model.rec, panel);
            }
            Msg::Anterior => {
                model.state.anterior(&model.rec, panel);
            }
            Msg::VistaGeneral => {
                model.state.vista_general(&model.rec, panel);
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

    fn on_wheel(
        _model: &Self::Model,
        delta: WheelDelta,
        cursor: (f32, f32),
        _modifiers: Modifiers,
    ) -> Option<Self::Msg> {
        let panel = panel_actual()?;
        if !dentro(panel, cursor.0, cursor.1) {
            return None;
        }
        // delta.y > 0 ⇒ scroll abajo ⇒ alejar (convención CSS, igual que tullpu).
        let mult = ZOOM_BASE.powf(-delta.y as f64);
        Some(Msg::Zoom { mult, cursor })
    }

    fn on_key(_model: &Self::Model, ev: &KeyEvent) -> Option<Self::Msg> {
        if ev.state != KeyState::Pressed {
            return None;
        }
        match &ev.key {
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown | NamedKey::Enter | NamedKey::Space) => {
                Some(Msg::Siguiente)
            }
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => Some(Msg::Anterior),
            Key::Named(NamedKey::Home | NamedKey::Escape) => Some(Msg::VistaGeneral),
            _ => None,
        }
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
