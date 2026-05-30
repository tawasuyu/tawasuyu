//! Demo de **autoría**: colocar/mover/editar marcos en el lienzo y guardar.
//!
//! A diferencia de `recorrido_demo` (presentar), aquí el arrastre edita:
//!   - **arrastrar sobre un marco**: lo mueve (y lo selecciona; borde ámbar).
//!   - **arrastrar sobre el vacío**: panea el lienzo.
//!   - **n**: crea un marco nuevo en el centro de la cámara (lo agrega a la ruta y lo selecciona).
//!   - **Supr / Retroceso**: elimina el marco seleccionado (y lo purga del guion).
//!   - **[ / ]**: rota el marco seleccionado.
//!   - **Ctrl+S / Ctrl+O**: guarda / carga el recorrido en `recorrido.deck` (postcard).
//!   - **rueda**: zoom-a-cursor.   **flechas / Espacio**: vuela por la ruta.
//!
//! El hit-test, el movimiento y las ops de autoría (`eliminar_marco`,
//! `rotar_marco`, …) y la persistencia (`serializar`/`deserializar`) viven en
//! `pluma-deck-core`; aquí sólo se cablean a eventos.
//!
//! Corre con:
//!   `cargo run -p pluma-deck-recorrido-llimphi --example recorrido_editor_demo --release`

use std::time::Duration;

use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use pluma_deck_core::{ContenidoMarco, Marco, Recorrido, RecorridoState, Rect, RejillaOpts};
use pluma_deck_recorrido_llimphi::{dentro, panel_actual, recorrido_view_editor, ZOOM_BASE};

const PANEL_INICIAL: Rect = Rect { x: 0.0, y: 0.0, w: 1100.0, h: 720.0 };
/// Archivo donde guarda/carga el demo (codec nativo postcard).
const ARCHIVO: &str = "recorrido.deck";

#[derive(Clone)]
enum Msg {
    Zoom { mult: f64, cursor: (f32, f32) },
    /// Move del arrastre: delta `(dx,dy)` + posición inicial del press `(lx,ly)`.
    Arrastre { dx: f32, dy: f32, lx: f32, ly: f32 },
    FinArrastre,
    NuevoMarco,
    Eliminar,
    Rotar(f64),
    Guardar,
    Cargar,
    Siguiente,
    Anterior,
    Tick,
}

struct Model {
    rec: Recorrido,
    state: RecorridoState,
    /// `None` = sin arrastre. `Some(None)` = paneando. `Some(Some(id))` = moviendo ese marco.
    arrastrando: Option<Option<u64>>,
    /// Marco seleccionado (objetivo de eliminar/rotar). Se realza en ámbar.
    seleccionado: Option<u64>,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · recorrido (autoría: arrastrar mueve marcos, n crea)"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 720)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let etiqueta = |s: &str| ContenidoMarco::Etiqueta(s.into());
        let rec = Recorrido::en_rejilla(
            vec![etiqueta("arrastrá un marco"), etiqueta("o el vacío para panear"), etiqueta("tecla n: marco nuevo")],
            RejillaOpts { cols: 3, marco_w: 460.0, marco_h: 300.0, gap_x: 200.0, gap_y: 160.0 },
        );
        let mut state = RecorridoState::new();
        state.saltar_a_paso(&rec, 0, PANEL_INICIAL);
        handle.spawn_periodic(Duration::from_millis(16), || Msg::Tick);
        Model { rec, state, arrastrando: None, seleccionado: None }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        let panel = panel_actual().unwrap_or(PANEL_INICIAL);
        match msg {
            Msg::Zoom { mult, cursor } => {
                model.state.wheel(mult, (cursor.0 as f64, cursor.1 as f64), panel);
            }
            Msg::Arrastre { dx, dy, lx, ly } => {
                // En el primer Move decidimos qué se agarró (marco o vacío) y
                // lo fijamos hasta soltar — así no se cambia de presa a mitad.
                let modo = match model.arrastrando {
                    Some(m) => m,
                    None => {
                        let world = model.state.camara.screen_to_world((lx as f64, ly as f64), panel);
                        let m = model.rec.marco_en_punto(world);
                        model.arrastrando = Some(m);
                        // Agarrar un marco lo selecciona (objetivo de editar).
                        if m.is_some() {
                            model.seleccionado = m;
                        }
                        m
                    }
                };
                match modo {
                    Some(id) => {
                        let (wdx, wdy) = model.state.camara.delta_pantalla_a_mundo(dx as f64, dy as f64);
                        model.rec.mover_marco(id, wdx, wdy);
                    }
                    None => model.state.arrastrar_delta(dx as f64, dy as f64),
                }
            }
            Msg::FinArrastre => model.arrastrando = None,
            Msg::NuevoMarco => {
                let id = model.rec.marcos.iter().map(|m| m.id).max().unwrap_or(0) + 1;
                let (cx, cy) = model.state.camara.centro;
                let (w, h) = (420.0, 260.0);
                model.rec.agregar_marco(Marco::new(
                    id,
                    Rect::new(cx - w * 0.5, cy - h * 0.5, w, h),
                    ContenidoMarco::Etiqueta(format!("marco {id}")),
                ));
                model.rec.pasos.push(id);
                model.seleccionado = Some(id);
            }
            Msg::Eliminar => {
                if let Some(id) = model.seleccionado.take() {
                    model.rec.eliminar_marco(id);
                    // Reencuadra el paso actual (el guion pudo encogerse).
                    let idx = model.state.paso.min(model.rec.n_pasos().saturating_sub(1));
                    model.state.saltar_a_paso(&model.rec, idx, panel);
                }
            }
            Msg::Rotar(d) => {
                if let Some(id) = model.seleccionado {
                    model.rec.rotar_marco(id, d);
                }
            }
            Msg::Guardar => match model.rec.serializar() {
                Ok(bytes) => {
                    let _ = std::fs::write(ARCHIVO, &bytes);
                    eprintln!("guardado {ARCHIVO} ({} bytes)", bytes.len());
                }
                Err(e) => eprintln!("error al guardar: {e}"),
            },
            Msg::Cargar => match std::fs::read(ARCHIVO).map_err(|_| "no se pudo leer").and_then(|b| Recorrido::deserializar(&b)) {
                Ok(rec) => {
                    model.rec = rec;
                    model.seleccionado = None;
                    model.state.saltar_a_paso(&model.rec, 0, panel);
                    eprintln!("cargado {ARCHIVO}");
                }
                Err(e) => eprintln!("error al cargar: {e}"),
            },
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
        recorrido_view_editor(&model.rec, &model.state, model.seleccionado).draggable_at(
            |phase, dx, dy, lx, ly| match phase {
                DragPhase::Move => Some(Msg::Arrastre { dx, dy, lx, ly }),
                DragPhase::End => Some(Msg::FinArrastre),
            },
        )
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
        // Atajos con Ctrl: guardar / cargar.
        if ev.modifiers.ctrl {
            return match &ev.key {
                Key::Character(c) if c.eq_ignore_ascii_case("s") => Some(Msg::Guardar),
                Key::Character(c) if c.eq_ignore_ascii_case("o") => Some(Msg::Cargar),
                _ => None,
            };
        }
        match &ev.key {
            Key::Character(c) if c.as_str() == "n" => Some(Msg::NuevoMarco),
            Key::Character(c) if c.as_str() == "[" => Some(Msg::Rotar(-0.08)),
            Key::Character(c) if c.as_str() == "]" => Some(Msg::Rotar(0.08)),
            Key::Named(NamedKey::Delete | NamedKey::Backspace) => Some(Msg::Eliminar),
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
