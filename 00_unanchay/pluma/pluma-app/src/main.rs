//! `pluma-app` — editor de escritura multilienzo.
//!
//! Layout en tres columnas (splitters draggables):
//!
//! ```text
//!   ┌─────────────┬───────────────────────────┬───────────────┐
//!   │ documentos  │   cuerpo_ide editable     │ panel LLM     │
//!   │ (lista de   │   (cuerpo activo)         │ - backend ▼   │
//!   │  cuerpos    │                           │ - botones LLM │
//!   │  del sled)  │                           │ - lista hijas │
//!   └─────────────┴───────────────────────────┴───────────────┘
//! ```
//!
//! Persistencia automática en `~/.cache/gioser/pluma-app/pluma.sled`
//! vía [`PlumaStore`]. Al primer arranque siembra un documento vacío
//! para que la ventana no esté muerta. Tras ese punto, todo doc/atom/
//! transformación/carta vive en sled.
//!
//! Atajos:
//!   - `Ctrl+S` guarda el cuerpo activo (diff buffer → atoms → sled).
//!   - `Ctrl+N` crea un documento Original nuevo.
//!   - `Ctrl+J` togglea la junction anterior al caret (zonas).
//!   - `Ctrl+Shift+]/[` saltan entre zonas.
//!
//! Botones del panel derecho dispara una transformación LLM sobre el
//! cuerpo activo completo (Traducir → qu/en, Tono formal, Resumir 30p).
//! La hija aparece como un cuerpo nuevo en la lista izquierda — click
//! la activa.
//!
//! El crate está partido en módulos: `model` (Model+Msg+consts),
//! `clipboard` (arboard), `util` (paths/etiquetas/reloj), `init`
//! (apertura del sled + backend), `update` (lógica + LLM) y `view`
//! (las tres columnas). Acá queda el `impl App` y el ruteo de teclado.

mod clipboard;
mod init;
mod model;
mod update;
mod util;
mod view;

use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey};
use llimphi_ui::View;

use crate::init::init_modelo;
use crate::model::{Model, Msg};
use crate::update::actualizar;
use crate::view::{vista, vista_overlay};

fn main() {
    llimphi_ui::run::<Pluma>();
}

struct Pluma;

impl App for Pluma {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · editor multilienzo"
    }

    fn initial_size() -> (u32, u32) {
        (1600, 900)
    }

    fn init(_: &Handle<Msg>) -> Model {
        init_modelo()
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        actualizar(model, msg, handle)
    }

    fn on_key(model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // Si el input de ruta tiene foco, las teclas van ahí — incluso
        // Ctrl/Shift combos. Esc lo apaga; cualquier otra cosa edita.
        if model.path_focused {
            if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::DefocusPath);
            }
            return Some(Msg::PathInputKey(event.clone()));
        }
        let ctrl = event.modifiers.ctrl || event.modifiers.meta;
        let shift = event.modifiers.shift;
        let alt = event.modifiers.alt;
        // Alt+Flecha: mover el átomo bajo el caret. Lo capturamos antes
        // que el editor para que no procese el evento como navegación.
        if alt && !ctrl {
            if matches!(&event.key, Key::Named(NamedKey::ArrowUp)) {
                return Some(Msg::MoverAtomArriba);
            }
            if matches!(&event.key, Key::Named(NamedKey::ArrowDown)) {
                return Some(Msg::MoverAtomAbajo);
            }
        }
        // Find overlay capturado: Esc cierra, Enter/Shift+Enter ciclan
        // matches, todo lo demás edita el query.
        if model.find_visible {
            if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::FindClose);
            }
            if matches!(&event.key, Key::Named(NamedKey::Enter)) {
                return Some(if shift {
                    Msg::FindAnterior
                } else {
                    Msg::FindSiguiente
                });
            }
            // Ctrl+F otra vez cierra (atajo simétrico a abrir).
            if ctrl {
                if let Key::Character(s) = &event.key {
                    if s.eq_ignore_ascii_case("f") {
                        return Some(Msg::FindClose);
                    }
                }
            }
            return Some(Msg::FindKey(event.clone()));
        }
        if ctrl {
            if let Key::Character(s) = &event.key {
                if s.eq_ignore_ascii_case("s") {
                    return Some(Msg::Guardar);
                }
                if s.eq_ignore_ascii_case("n") {
                    return Some(Msg::NuevoDoc);
                }
                if s.eq_ignore_ascii_case("f") {
                    return Some(Msg::FindToggle);
                }
                if s.eq_ignore_ascii_case("d") {
                    return Some(Msg::DiffToggle);
                }
                if shift && (s == "}" || s == "]") {
                    return Some(Msg::ZonaSiguiente);
                }
                if shift && (s == "{" || s == "[") {
                    return Some(Msg::ZonaAnterior);
                }
                if s.eq_ignore_ascii_case("j") {
                    return Some(Msg::ToglearFusion);
                }
            }
        }
        Some(Msg::EditorKey(event.clone()))
    }

    fn view(model: &Model) -> View<Msg> {
        vista(model)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        vista_overlay(model)
    }
}
