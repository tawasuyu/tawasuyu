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
//! Persistencia automática en `~/.cache/tawasuyu/pluma-app/pluma.sled`
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
mod dump;
mod init;
mod model;
mod update;
mod util;
mod view;

use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, WheelDelta};
use llimphi_ui::View;

use crate::init::init_modelo;
use crate::model::{Modo, Model, Msg};
use crate::update::actualizar;
use crate::view::{vista, vista_overlay};

fn main() {
    // Subcomando oculto de evidencia: `pluma-app --dump <out.png> [diente]`.
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--dump") {
        let out = args.get(pos + 1).cloned().unwrap_or_else(|| "pluma.png".into());
        let diente = args.get(pos + 2).and_then(|s| s.parse().ok()).unwrap_or(1);
        dump::run(&out, diente);
        return;
    }
    llimphi_ui::run::<Pluma>();
}

struct Pluma;

impl App for Pluma {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · editor multilienzo"
    }

    /// `app_id` Wayland: pata lo usa para correlacionar foco ↔ dientes hospedados.
    fn app_id() -> Option<&'static str> {
        Some("tawasuyu.pluma")
    }

    fn initial_size() -> (u32, u32) {
        (1600, 900)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let mut m = init_modelo();
        // Rail hospedado: si delega, publica sus secciones como dientes en pata.
        if m.delegated {
            let teeth = vec![
                pata_host::HostedTooth::new(0, "files", "Archivo"),
                pata_host::HostedTooth::new(1, "folder", "Lienzos"),
                pata_host::HostedTooth::new(2, "tools", "Derivar"),
                pata_host::HostedTooth::new(3, "tools", "Modelo"),
                pata_host::HostedTooth::new(4, "tools", "Grafo"),
            ];
            let h = handle.clone();
            m._host = pata_host::HostClient::connect("tawasuyu.pluma", "Pluma", teeth, move |id| {
                h.dispatch(Msg::HostActivate(id))
            });
        }
        m
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        actualizar(model, msg, handle)
    }

    fn on_key(model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // Menús abiertos: las flechas navegan y tienen prioridad sobre todo.
        if let Some(mi) = model.menu_open {
            let n = crate::update::menu_principal(model).menus.len().max(1);
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        if model.edit_menu.is_some() {
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::EditNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::EditNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::EditActivate),
                _ => None,
            };
        }
        // Si el input de ruta tiene foco, las teclas van ahí — incluso
        // Ctrl/Shift combos. Esc lo apaga; cualquier otra cosa edita.
        if model.path_focused {
            if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::DefocusPath);
            }
            return Some(Msg::PathInputKey(event.clone()));
        }
        // Ídem para el input de prompt del diente Derivar.
        if model.preset_focused {
            if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::DefocusPreset);
            }
            if matches!(&event.key, Key::Named(NamedKey::Enter)) {
                return Some(Msg::CrearAlterno);
            }
            return Some(Msg::PresetInputKey(event.clone()));
        }
        // Input del término del filtro Concepto (diente Grafo).
        if model.grafo_input_focused {
            if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::DefocusGrafo);
            }
            return Some(Msg::GrafoInputKey(event.clone()));
        }
        // Ctrl+M cicla el modo del centro (Lienzos → Presentar → Plano), en
        // cualquier contexto que no sea un input de texto.
        if event.modifiers.ctrl || event.modifiers.meta {
            if let Key::Character(s) = &event.key {
                if s.eq_ignore_ascii_case("m") {
                    return Some(Msg::CicloModo);
                }
            }
        }
        // Edición in-situ de un lienzo (modo Lienzos): las teclas van a ese
        // editor; Esc guarda y cierra.
        if model.editando.is_some() {
            if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::LienzoCommit);
            }
            return Some(Msg::LienzoEditKey(event.clone()));
        }
        // Modo Presentar: navegación por teclado (flechas vuelan; Home/Esc =
        // vista general). No edita texto.
        if model.modo == Modo::Presentar {
            return match &event.key {
                Key::Named(NamedKey::ArrowRight) | Key::Named(NamedKey::ArrowDown)
                | Key::Named(NamedKey::Enter) => Some(Msg::PresSiguiente),
                Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::ArrowUp) => {
                    Some(Msg::PresAnterior)
                }
                Key::Named(NamedKey::Home) | Key::Named(NamedKey::Escape) => {
                    Some(Msg::PresVistaGeneral)
                }
                _ => None,
            };
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
            // Ctrl+Tab / Ctrl+Shift+Tab: cicla el foco entre lienzos.
            if matches!(&event.key, Key::Named(NamedKey::Tab)) {
                return Some(if shift {
                    Msg::FocoAnterior
                } else {
                    Msg::FocoSiguiente
                });
            }
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

    /// Rueda: el eje X del touchpad o `Shift`+rueda desplazan el multilienzo en
    /// HORIZONTAL; la rueda vertical normal scrollea el lienzo con foco en
    /// VERTICAL (sin confundirse con el horizontal cuando hay texto que correr).
    fn on_wheel(
        _model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        modifiers: Modifiers,
    ) -> Option<Self::Msg> {
        const PX_POR_LINEA: f32 = 48.0;
        if delta.x.abs() > 0.0 {
            return Some(Msg::ScrollHoriz(-delta.x * PX_POR_LINEA));
        }
        if modifiers.shift {
            return Some(Msg::ScrollHoriz(-delta.y * PX_POR_LINEA));
        }
        if delta.y != 0.0 {
            return Some(Msg::ScrollVert(delta.y));
        }
        None
    }

    fn on_resize(_model: &Self::Model, width: u32, height: u32) -> Option<Self::Msg> {
        Some(Msg::Resized(width as f32, height as f32))
    }

    fn view(model: &Model) -> View<Msg> {
        vista(model)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        vista_overlay(model)
    }
}
