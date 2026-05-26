//! Demo del **text-editor IDE de Llimphi sobre `EditorCuerpo`**.
//!
//! Pintamos un cuerpo `es` con 5 párrafos sintéticos. El widget
//! `text-editor` (multi-cursor, undo, find/replace, scroll) lo ve como
//! UN buffer plano — cada `\n\n` separa un átomo. Editás libremente
//! como en cualquier IDE:
//!
//!   - Tipeo / borrado normal.
//!   - `Ctrl+Z` undo / `Ctrl+Shift+Z` redo (del widget).
//!   - Click + drag = selección; `Ctrl+C/X/V` con clipboard interno.
//!   - **`Ctrl+S`** = "guardar": calcula el diff contra los atoms
//!     originales, lo aplica al `HashMap<Uuid, NarrativeAtom>` del modelo
//!     (mutando contenido, creando atoms nuevos para los párrafos
//!     extra, eliminando los faltantes), sincroniza el `Cuerpo.orden` y
//!     resetea el editor sobre el cuerpo nuevo. El header muestra el
//!     resumen del diff.
//!
//! No hay `pluma-graph` ni `pluma-store` acá — el modelo guarda los
//! atoms y el `Cuerpo` directamente. Es el caso pelado del IDE: el
//! caller real reemplaza el `HashMap` por su `NarrativeGraph` y
//! persiste con `pluma-store`.
//!
//! ```bash
//! cargo run -p pluma-editor-llimphi --example cuerpo_ide_demo --release
//! ```

use std::collections::HashMap;

use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, FlexDirection, Rect, Size, Style,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, View};
use llimphi_widget_text_editor::{EditorMetrics, EditorPalette, Language, PointerEvent};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_cuerpo::CambioAtom;
use pluma_editor_llimphi::cuerpo_ide::{cuerpo_ide_view, CuerpoIde};
use uuid::Uuid;

#[derive(Clone, Debug)]
enum Msg {
    /// Tecla aplicada al editor.
    EditorKey(KeyEvent),
    /// Click / drag sobre el área de texto.
    EditorPointer(PointerEvent),
    /// `Ctrl+S` — el caller pidió persistir los cambios.
    Guardar,
}

struct Model {
    /// Cuerpo activo — su `orden` es lo que el editor reconstruye.
    cuerpo: Cuerpo,
    /// Atoms del "grafo" plano. Clave = `id`, valor = atom completo.
    atoms: HashMap<Uuid, NarrativeAtom>,
    /// El IDE: buffer + cursor + diff vs `editor_cuerpo`.
    ide: CuerpoIde,
    /// Texto a mostrar en el header tras el último `Guardar`.
    /// Vacío al arrancar.
    ultimo_save: String,
    /// Acumulador de drag para la selección por mouse.
    drag_accum: (f32, f32),
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · text-editor IDE sobre EditorCuerpo"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let textos = [
            "El cóndor cruzó el cielo del valle al amanecer.",
            "Las llamas pastaban entre los pastizales del altiplano.",
            "Una mujer joven tejía un telar bajo el alero.",
            "El río Apurímac descendía rugiente por las rocas.",
            "Al caer la tarde, las nubes cubrieron el sol.",
        ];
        let atoms_vec: Vec<NarrativeAtom> = textos
            .iter()
            .map(|t| NarrativeAtom::new(*t, "es"))
            .collect();
        let mut cuerpo = Cuerpo::nuevo("es", "español (original)", Intencion::Original, 0);
        for a in &atoms_vec {
            cuerpo.agregar(a.id, 0);
        }
        let atoms: HashMap<Uuid, NarrativeAtom> =
            atoms_vec.into_iter().map(|a| (a.id, a)).collect();

        let idx: HashMap<Uuid, &NarrativeAtom> = atoms.iter().map(|(k, v)| (*k, v)).collect();
        let ide = CuerpoIde::from_cuerpo(&cuerpo, &idx);

        Model {
            cuerpo,
            atoms,
            ide,
            ultimo_save: String::new(),
            drag_accum: (0.0, 0.0),
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::EditorKey(ev) => {
                let mut model = model;
                let _ = model.ide.apply_key(&ev);
                model
            }
            Msg::EditorPointer(ev) => {
                let mut model = model;
                let metrics = EditorMetrics::for_font_size(13.0);
                let scroll = model.ide.state.scroll_offset;
                match ev {
                    PointerEvent::Click { x, y } => {
                        model.drag_accum = (0.0, 0.0);
                        let (line, col) = metrics.screen_to_pos(x, y, scroll);
                        model.ide.state.set_caret_at(line, col);
                    }
                    PointerEvent::Drag {
                        initial_x,
                        initial_y,
                        dx,
                        dy,
                    } => {
                        model.drag_accum.0 += dx;
                        model.drag_accum.1 += dy;
                        let cx = initial_x + model.drag_accum.0;
                        let cy = initial_y + model.drag_accum.1;
                        let (line, col) = metrics.screen_to_pos(cx, cy, scroll);
                        model.ide.state.extend_selection_to(line, col);
                    }
                }
                model
            }
            Msg::Guardar => {
                let mut model = model;
                let idx: HashMap<Uuid, &NarrativeAtom> =
                    model.atoms.iter().map(|(k, v)| (*k, v)).collect();
                let cambios = model.ide.diff(&idx);
                drop(idx);
                let resumen = persistir(&mut model, &cambios);
                // Tras persistir, recargá el IDE con el cuerpo + atoms
                // ya actualizados — limpia el undo y deja el caret al
                // final. Para un editor real querrías preservar la pos
                // del caret; para el MVP, reset es suficiente.
                let idx2: HashMap<Uuid, &NarrativeAtom> =
                    model.atoms.iter().map(|(k, v)| (*k, v)).collect();
                model.ide.recargar(&model.cuerpo.clone(), &idx2);
                model.ultimo_save = resumen;
                model
            }
        }
    }

    fn on_key(_model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // Ctrl+S = guardar; el resto va al editor.
        let ctrl = event.modifiers.ctrl || event.modifiers.meta;
        if ctrl {
            if let Key::Character(s) = &event.key {
                if s.eq_ignore_ascii_case("s") {
                    return Some(Msg::Guardar);
                }
            }
        }
        Some(Msg::EditorKey(event.clone()))
    }

    fn view(model: &Model) -> View<Msg> {
        let palette_editor = EditorPalette::default();
        let metrics = EditorMetrics::for_font_size(13.0);

        let bg_app = palette_editor.bg;
        let fg_text = palette_editor.fg_text;
        let fg_muted = palette_editor.fg_line_number;

        let header_text = format!(
            "cuerpo «{}»  ·  {} átomos  ·  {}  ·  Ctrl+S guarda",
            model.cuerpo.metadatos.nombre_legible,
            model.cuerpo.orden.len(),
            if model.ide.pendiente_sync() {
                "buffer con cambios sin guardar"
            } else {
                "sincronizado"
            },
        );
        let header = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(6.0_f32),
                bottom: length(6.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(40, 44, 52, 255))
        .text_aligned(header_text, 12.0, fg_text, Alignment::Start);

        let footer_text = if model.ultimo_save.is_empty() {
            "(sin saves todavía)".to_string()
        } else {
            model.ultimo_save.clone()
        };
        let footer = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(4.0_f32),
                bottom: length(4.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(33, 36, 42, 255))
        .text_aligned(footer_text, 11.0, fg_muted, Alignment::Start);

        // Calculamos visible_lines en función del alto disponible
        // aproximado. Sin acceso al rect real, asumimos que el ejemplo
        // se abre con `initial_size().1 = 720` y le restamos el header,
        // footer y un margen. El widget cap a 200 internamente.
        let alto_disponible = 720.0_f32 - 28.0 - 24.0 - 16.0;
        let visible = (alto_disponible / metrics.line_height) as usize;
        let visible = visible.max(8);

        let editor = cuerpo_ide_view::<Msg>(
            &model.ide,
            &palette_editor,
            metrics,
            visible,
            Language::Plain,
            |ev| Some(Msg::EditorPointer(ev)),
        );

        // El editor en sí va dentro de un contenedor con padding lateral
        // para que el texto no quede pegado al borde.
        let contenedor_editor = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(8.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        })
        .fill(bg_app)
        .children(vec![editor]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(bg_app)
        .children(vec![header, contenedor_editor, footer])
    }
}

/// Aplica los `CambioAtom` al modelo: muta `atoms` y reconstruye
/// `cuerpo.orden`. Devuelve un resumen humano de qué pasó.
fn persistir(model: &mut Model, cambios: &[CambioAtom]) -> String {
    if cambios.is_empty() {
        return "guardado: sin cambios".to_string();
    }
    let mut mutados = 0usize;
    let mut creados: Vec<(usize, Uuid)> = Vec::new();
    let mut eliminados = 0usize;

    // 1. Aplicá Mutar + Crear/Eliminar a `atoms`.
    for c in cambios {
        match c {
            CambioAtom::Mutar { id, texto_nuevo } => {
                if let Some(a) = model.atoms.get_mut(id) {
                    a.set_content(texto_nuevo.as_str());
                    mutados += 1;
                }
            }
            CambioAtom::Crear { texto, posicion } => {
                let atom = NarrativeAtom::new(texto.as_str(), &model.cuerpo.branch_id);
                let id = atom.id;
                model.atoms.insert(id, atom);
                creados.push((*posicion, id));
            }
            CambioAtom::Eliminar { id } => {
                model.atoms.remove(id);
                eliminados += 1;
            }
        }
    }

    // 2. Reconstruí `cuerpo.orden`. El editor garantiza que `Crear`
    //    apunta a posiciones consecutivas al final (lo vimos en el
    //    diff greedy), y los `Eliminar` salen del final también — así
    //    podemos rehacer el orden de manera simple:
    //
    //    nuevo_orden = atom_ids del editor con los Eliminar removidos
    //                  + los Uuids recién creados al final.
    let nuevos_ids: Vec<Uuid> = creados.iter().map(|(_, id)| *id).collect();
    model
        .ide
        .aplicar_cambios_locales(cambios, &nuevos_ids);
    let nuevo_orden: Vec<Uuid> = model.ide.editor_cuerpo.atom_ids.clone();

    // 3. Refrescá el `Cuerpo`: borrá el orden viejo y reagrego.
    let ahora = model.cuerpo.metadatos.modificado_en.saturating_add(1);
    let viejo: Vec<Uuid> = model.cuerpo.orden.clone();
    for id in &viejo {
        let _ = model.cuerpo.remover(*id, ahora);
    }
    for id in &nuevo_orden {
        model.cuerpo.agregar(*id, ahora);
    }

    format!(
        "guardado: {} mutar · {} crear · {} eliminar — orden ahora con {} átomos",
        mutados,
        nuevos_ids.len(),
        eliminados,
        nuevo_orden.len(),
    )
}

fn main() {
    llimphi_ui::run::<Demo>();
}
