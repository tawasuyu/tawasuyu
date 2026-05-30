//! `pluma-deck` — app de **presentaciones espaciales** (modo Recorrido, tipo Prezi).
//!
//! Unifica en un solo binario lo que antes vivía repartido en demos: abrir un
//! documento, **presentarlo** (la cámara vuela por la ruta) y **autorearlo**
//! (mover/crear/borrar/rotar marcos), con guardar/cargar en el formato nativo.
//! Toda la lógica vive en `pluma-deck-core` (cámara, ruta, autoría, persistencia,
//! adaptador pluma); aquí sólo se cablean eventos y se elige la vista por modo.
//!
//! Uso:
//!   `cargo run -p pluma-deck-app -- [archivo.deck | archivo.md]`
//!   - `*.deck`  → carga el recorrido binario (postcard) y guarda sobre él.
//!   - `*.md`    → importa markdown como recorrido (guarda en `recorrido.deck`).
//!   - sin arg   → recorrido de bienvenida (guarda en `recorrido.deck`).
//!
//! Controles comunes: **flechas / Espacio / Enter** vuela por la ruta ·
//! **Home/Esc** vista general · **p** modo presentador (autoplay) · **rueda**
//! zoom-a-cursor · **Tab** alterna presentar/editar · **Ctrl+S / Ctrl+O**
//! guarda / carga · **Ctrl+Z / Ctrl+Shift+Z** deshace / rehace autoría. En
//! **editar**: arrastrar mueve/selecciona un marco (o panea el vacío), **n**
//! crea, **Supr** elimina, **[ ]** rota. En **presentar**: arrastrar panea libre.

use std::path::PathBuf;
use std::time::Duration;

use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use pluma_deck_core::adaptador::recorrido_desde_atomos;
use pluma_deck_core::{Autoplay, ContenidoMarco, Marco, Recorrido, RecorridoState, Rect, RejillaOpts};
use pluma_deck_recorrido_llimphi::{dentro, panel_actual, recorrido_view, recorrido_view_editor, ZOOM_BASE};

const PANEL_INICIAL: Rect = Rect { x: 0.0, y: 0.0, w: 1200.0, h: 760.0 };

#[derive(Clone, Copy, PartialEq)]
enum Modo {
    Presentar,
    Editar,
}

#[derive(Clone)]
enum Msg {
    Zoom { mult: f64, cursor: (f32, f32) },
    /// Pan libre (modo presentar): delta de pantalla.
    Pan { dx: f32, dy: f32 },
    /// Arrastre de autoría (modo editar): delta + posición del press.
    Arrastre { dx: f32, dy: f32, lx: f32, ly: f32 },
    FinArrastre,
    NuevoMarco,
    Eliminar,
    Rotar(f64),
    Deshacer,
    Rehacer,
    Guardar,
    Cargar,
    ToggleModo,
    VistaGeneral,
    ToggleAutoplay,
    Siguiente,
    Anterior,
    Tick,
}

struct Model {
    rec: Recorrido,
    state: RecorridoState,
    autoplay: Autoplay,
    modo: Modo,
    seleccionado: Option<u64>,
    /// `None` = sin arrastre. `Some(None)` = paneando. `Some(Some(id))` = moviendo ese marco.
    arrastrando: Option<Option<u64>>,
    /// Destino de Ctrl+S (postcard).
    guardar_en: PathBuf,
    /// Undo/redo de autoría (snapshots del recorrido).
    historial: Historial<Recorrido>,
}

/// Recorrido de bienvenida cuando no se pasa archivo.
fn bienvenida() -> Recorrido {
    let slide = |t: &str, ps: &[&str]| ContenidoMarco::Texto {
        titulo: Some(t.into()),
        parrafos: ps.iter().map(|s| s.to_string()).collect(),
    };
    Recorrido::en_rejilla(
        vec![
            slide("pluma · deck", &["Presentaciones espaciales tipo Prezi.", "Pasá un .deck o un .md, o autoreá desde cero."]),
            slide("Presentar", &["Flechas / Espacio vuelan por la ruta.", "Home/Esc: vista general.   p: autoplay."]),
            slide("Editar (Tab)", &["Arrastrá para mover/seleccionar un marco.", "n: nuevo   Supr: borrar   [ ]: rotar."]),
            slide("Guardar", &["Ctrl+S guarda, Ctrl+O carga.", "Formato nativo postcard (.deck)."]),
        ],
        RejillaOpts { cols: 2, marco_w: 640.0, marco_h: 400.0, gap_x: 220.0, gap_y: 180.0 },
    )
}

/// Abre un archivo como Recorrido: `.md` se importa con el adaptador pluma;
/// cualquier otro se intenta como `.deck` binario. `None` si falla.
fn abrir(ruta: &str) -> Option<Recorrido> {
    let bytes = std::fs::read(ruta).ok()?;
    if ruta.ends_with(".md") {
        let md = String::from_utf8(bytes).ok()?;
        let doc = pluma_md::parse_md(&md, "es", "deck", 0);
        Some(recorrido_desde_atomos(&doc.atoms, RejillaOpts::default()))
    } else {
        Recorrido::deserializar(&bytes).ok()
    }
}

struct Deck;

impl App for Deck {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · deck — presentaciones espaciales (Tab: presentar/editar)"
    }

    fn initial_size() -> (u32, u32) {
        (1200, 760)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        // Resuelve el recorrido inicial y el destino de guardado según el arg.
        let (rec, guardar_en) = match std::env::args().nth(1) {
            Some(p) if p.ends_with(".deck") => {
                let r = abrir(&p).unwrap_or_else(bienvenida);
                (r, PathBuf::from(p))
            }
            // .md u otro: importa si puede, pero guarda en un .deck para no
            // sobrescribir el fuente con binario.
            Some(p) => (abrir(&p).unwrap_or_else(bienvenida), PathBuf::from("recorrido.deck")),
            None => (bienvenida(), PathBuf::from("recorrido.deck")),
        };
        let mut state = RecorridoState::new();
        state.saltar_a_paso(&rec, 0, PANEL_INICIAL);
        handle.spawn_periodic(Duration::from_millis(16), || Msg::Tick);
        Model {
            rec,
            state,
            autoplay: Autoplay::default(),
            modo: Modo::Presentar,
            seleccionado: None,
            arrastrando: None,
            guardar_en,
            historial: Historial::new(64),
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        let panel = panel_actual().unwrap_or(PANEL_INICIAL);
        match msg {
            Msg::Zoom { mult, cursor } => {
                model.state.wheel(mult, (cursor.0 as f64, cursor.1 as f64), panel);
            }
            Msg::Pan { dx, dy } => model.state.arrastrar_delta(dx as f64, dy as f64),
            Msg::Arrastre { dx, dy, lx, ly } => {
                let modo = match model.arrastrando {
                    Some(m) => m,
                    None => {
                        let world = model.state.camara.screen_to_world((lx as f64, ly as f64), panel);
                        let m = model.rec.marco_en_punto(world);
                        model.arrastrando = Some(m);
                        if m.is_some() {
                            model.seleccionado = m;
                            // Una instantánea por arrastre (al agarrar el marco),
                            // no por cada move — undo revierte el movimiento entero.
                            model.historial.registrar(&model.rec);
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
                model.historial.registrar(&model.rec);
                let id = model.rec.marcos.iter().map(|m| m.id).max().unwrap_or(0) + 1;
                let (cx, cy) = model.state.camara.centro;
                let (w, h) = (520.0, 320.0);
                model.rec.agregar_marco(Marco::new(
                    id,
                    Rect::new(cx - w * 0.5, cy - h * 0.5, w, h),
                    ContenidoMarco::Texto { titulo: Some(format!("marco {id}")), parrafos: vec![] },
                ));
                model.rec.pasos.push(id);
                model.seleccionado = Some(id);
            }
            Msg::Eliminar => {
                if let Some(id) = model.seleccionado.take() {
                    model.historial.registrar(&model.rec);
                    model.rec.eliminar_marco(id);
                    let idx = model.state.paso.min(model.rec.n_pasos().saturating_sub(1));
                    model.state.saltar_a_paso(&model.rec, idx, panel);
                }
            }
            Msg::Rotar(d) => {
                if let Some(id) = model.seleccionado {
                    model.historial.registrar(&model.rec);
                    model.rec.rotar_marco(id, d);
                }
            }
            Msg::Deshacer | Msg::Rehacer => {
                let nuevo = match msg {
                    Msg::Deshacer => model.historial.deshacer(&model.rec),
                    _ => model.historial.rehacer(&model.rec),
                };
                if let Some(rec) = nuevo {
                    model.rec = rec;
                    // La selección puede haber dejado de existir; el paso se clampa.
                    if model.seleccionado.map_or(false, |id| model.rec.marco(id).is_none()) {
                        model.seleccionado = None;
                    }
                    let idx = model.state.paso.min(model.rec.n_pasos().saturating_sub(1));
                    model.state.saltar_a_paso(&model.rec, idx, panel);
                }
            }
            Msg::Guardar => match model.rec.serializar() {
                Ok(bytes) => {
                    let _ = std::fs::write(&model.guardar_en, &bytes);
                    eprintln!("guardado {} ({} bytes)", model.guardar_en.display(), bytes.len());
                }
                Err(e) => eprintln!("error al guardar: {e}"),
            },
            Msg::Cargar => {
                let r = std::fs::read(&model.guardar_en)
                    .map_err(|_| "no se pudo leer")
                    .and_then(|b| Recorrido::deserializar(&b));
                match r {
                    Ok(rec) => {
                        model.rec = rec;
                        model.seleccionado = None;
                        model.state.saltar_a_paso(&model.rec, 0, panel);
                        eprintln!("cargado {}", model.guardar_en.display());
                    }
                    Err(e) => eprintln!("error al cargar: {e}"),
                }
            }
            Msg::ToggleModo => {
                model.modo = match model.modo {
                    Modo::Presentar => Modo::Editar,
                    Modo::Editar => Modo::Presentar,
                };
                model.autoplay.pausa();
                eprintln!("modo: {}", if model.modo == Modo::Editar { "editar" } else { "presentar" });
            }
            Msg::VistaGeneral => {
                model.state.vista_general(&model.rec, panel);
            }
            Msg::ToggleAutoplay => {
                model.autoplay.toggle();
            }
            Msg::Siguiente => {
                model.state.siguiente(&model.rec, panel);
            }
            Msg::Anterior => {
                model.state.anterior(&model.rec, panel);
            }
            Msg::Tick => {
                model.state.avanzar(1.0 / 60.0);
                model.autoplay.tick(1.0 / 60.0, &mut model.state, &model.rec, panel);
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        match model.modo {
            Modo::Editar => recorrido_view_editor(&model.rec, &model.state, model.seleccionado)
                .draggable_at(|phase, dx, dy, lx, ly| match phase {
                    DragPhase::Move => Some(Msg::Arrastre { dx, dy, lx, ly }),
                    DragPhase::End => Some(Msg::FinArrastre),
                }),
            Modo::Presentar => recorrido_view(&model.rec, &model.state).draggable(|phase, dx, dy| match phase {
                DragPhase::Move => Some(Msg::Pan { dx, dy }),
                DragPhase::End => None,
            }),
        }
    }

    fn on_wheel(_m: &Self::Model, delta: WheelDelta, cursor: (f32, f32), _mods: Modifiers) -> Option<Self::Msg> {
        let panel = panel_actual()?;
        if !dentro(panel, cursor.0, cursor.1) {
            return None;
        }
        Some(Msg::Zoom { mult: ZOOM_BASE.powf(-delta.y as f64), cursor })
    }

    fn on_key(model: &Self::Model, ev: &KeyEvent) -> Option<Self::Msg> {
        if ev.state != KeyState::Pressed {
            return None;
        }
        // Guardar/cargar en cualquier modo.
        if ev.modifiers.ctrl {
            return match &ev.key {
                Key::Character(c) if c.eq_ignore_ascii_case("s") => Some(Msg::Guardar),
                Key::Character(c) if c.eq_ignore_ascii_case("o") => Some(Msg::Cargar),
                // Ctrl+Z deshace; Ctrl+Shift+Z o Ctrl+Y rehacen.
                Key::Character(c) if c.eq_ignore_ascii_case("z") => {
                    Some(if ev.modifiers.shift { Msg::Rehacer } else { Msg::Deshacer })
                }
                Key::Character(c) if c.eq_ignore_ascii_case("y") => Some(Msg::Rehacer),
                _ => None,
            };
        }
        // Comunes a ambos modos.
        match &ev.key {
            Key::Named(NamedKey::Tab) => return Some(Msg::ToggleModo),
            Key::Named(NamedKey::Home | NamedKey::Escape) => return Some(Msg::VistaGeneral),
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown | NamedKey::Enter | NamedKey::Space) => {
                return Some(Msg::Siguiente)
            }
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => return Some(Msg::Anterior),
            Key::Character(c) if c.eq_ignore_ascii_case("p") => return Some(Msg::ToggleAutoplay),
            _ => {}
        }
        // Sólo en modo editar.
        if model.modo == Modo::Editar {
            return match &ev.key {
                Key::Character(c) if c.as_str() == "n" => Some(Msg::NuevoMarco),
                Key::Character(c) if c.as_str() == "[" => Some(Msg::Rotar(-0.08)),
                Key::Character(c) if c.as_str() == "]" => Some(Msg::Rotar(0.08)),
                Key::Named(NamedKey::Delete | NamedKey::Backspace) => Some(Msg::Eliminar),
                _ => None,
            };
        }
        None
    }
}

/// Pila de undo/redo genérica para autoría. Antes de cada cambio se `registrar`a
/// el estado previo; `deshacer`/`rehacer` mueven el estado entre pasado y futuro.
/// Acotada a `max` entradas (descarta las más viejas).
struct Historial<T> {
    pasado: Vec<T>,
    futuro: Vec<T>,
    max: usize,
}

impl<T: Clone> Historial<T> {
    fn new(max: usize) -> Self {
        Self { pasado: Vec::new(), futuro: Vec::new(), max: max.max(1) }
    }

    /// Registra `actual` (estado **previo** al cambio que está por aplicarse) y
    /// limpia el redo — una rama nueva invalida los rehacer pendientes.
    fn registrar(&mut self, actual: &T) {
        self.pasado.push(actual.clone());
        if self.pasado.len() > self.max {
            self.pasado.remove(0);
        }
        self.futuro.clear();
    }

    /// Deshace: devuelve el último estado registrado y manda `actual` al futuro.
    fn deshacer(&mut self, actual: &T) -> Option<T> {
        let prev = self.pasado.pop()?;
        self.futuro.push(actual.clone());
        Some(prev)
    }

    /// Rehace: devuelve el último estado deshecho y manda `actual` al pasado.
    fn rehacer(&mut self, actual: &T) -> Option<T> {
        let next = self.futuro.pop()?;
        self.pasado.push(actual.clone());
        Some(next)
    }
}

fn main() {
    llimphi_ui::run::<Deck>();
}

#[cfg(test)]
mod pruebas {
    use super::Historial;

    #[test]
    fn deshacer_rehacer_round_trip() {
        let mut h = Historial::new(10);
        // estado: 0 → (registrar 0) → 1 → (registrar 1) → 2
        h.registrar(&0);
        h.registrar(&1);
        let mut actual = 2;
        // deshacer dos veces: 2→1→0
        actual = h.deshacer(&actual).unwrap();
        assert_eq!(actual, 1);
        actual = h.deshacer(&actual).unwrap();
        assert_eq!(actual, 0);
        assert!(h.deshacer(&actual).is_none(), "sin más pasado");
        // rehacer dos veces: 0→1→2
        actual = h.rehacer(&actual).unwrap();
        assert_eq!(actual, 1);
        actual = h.rehacer(&actual).unwrap();
        assert_eq!(actual, 2);
        assert!(h.rehacer(&actual).is_none(), "sin más futuro");
    }

    #[test]
    fn registrar_tras_deshacer_limpia_el_futuro() {
        let mut h = Historial::new(10);
        h.registrar(&0);
        let actual = h.deshacer(&1).unwrap(); // actual ahora = 0, futuro = [1]
        assert_eq!(actual, 0);
        h.registrar(&actual); // rama nueva: el futuro se descarta
        assert!(h.rehacer(&actual).is_none());
    }

    #[test]
    fn respeta_el_tope_descartando_lo_mas_viejo() {
        let mut h = Historial::new(2);
        h.registrar(&1);
        h.registrar(&2);
        h.registrar(&3); // descarta el 1
        assert_eq!(h.deshacer(&99), Some(3));
        assert_eq!(h.deshacer(&3), Some(2));
        assert!(h.deshacer(&2).is_none());
    }
}
