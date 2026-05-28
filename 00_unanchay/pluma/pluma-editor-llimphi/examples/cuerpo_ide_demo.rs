//! Demo del **text-editor IDE de Llimphi sobre `EditorCuerpo`**.
//!
//! Pintamos un cuerpo `es` con 5 párrafos sintéticos. El widget
//! `text-editor` lo ve como UN buffer plano — cada `\n\n` separa un
//! átomo. Editás libremente como en cualquier IDE:
//!
//!   - Tipeo / borrado normal, multi-cursor con `Ctrl+Alt+↑/↓`.
//!   - `Ctrl+Z` undo / `Ctrl+Shift+Z` redo (del widget).
//!   - Click + drag = selección; `Ctrl+C/X/V` con un clipboard en memoria.
//!   - **`Ctrl+S`** = "guardar": calcula el diff contra los atoms
//!     originales, lo aplica al `HashMap<Uuid, NarrativeAtom>` del
//!     modelo (mutando contenido, creando atoms nuevos para los
//!     párrafos extra, eliminando los faltantes), sincroniza el
//!     `Cuerpo.orden` y resetea el editor sobre el cuerpo nuevo
//!     preservando la posición del caret y el scroll.
//!   - **`Ctrl+]`** salta al siguiente átomo (`posicion_de_atom` +
//!     `set_caret`). Demuestra el lookup átomo → línea.
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
use llimphi_widget_text_editor::{
    EditorMetrics, EditorPalette, Language, MemClipboard, PointerEvent,
};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_cuerpo::CambioAtom;
use pluma_editor_llimphi::cuerpo_ide::{cuerpo_ide_view, CuerpoIde};
use uuid::Uuid;

/// Métricas del editor — fijas durante toda la sesión. Definidas como
/// const para no recalcularlas en cada `update` / `view`.
const METRICS: EditorMetrics = EditorMetrics::for_font_size(13.0);

/// Cap muy amplio para `visible_lines`. El widget lo trunca a 200
/// internamente — pasamos 200 y dejamos que decida.
const VISIBLE_LINES: usize = 200;

#[derive(Clone, Debug)]
enum Msg {
    EditorKey(KeyEvent),
    EditorPointer(PointerEvent),
    /// `Ctrl+S` — el caller pidió persistir los cambios.
    Guardar,
    /// `Ctrl+]` — saltar al átomo siguiente del cuerpo activo.
    SaltarAtomoSiguiente,
    /// `Ctrl+J` — togglea la junction inmediatamente anterior al
    /// átomo bajo el caret: si era separador (línea-guarda), pasa a
    /// fundida (línea editable, parte de la misma zona); si era
    /// fundida, vuelve a separador.
    ToglearFusion,
    /// `Ctrl+Shift+]` — salta el caret a la siguiente zona (grupo de
    /// atoms unidos por junctions fundidas). Wrap circular al final.
    ZonaSiguiente,
    /// `Ctrl+Shift+[` — salta a la zona anterior. Wrap circular al inicio.
    ZonaAnterior,
    /// `Ctrl+Shift+A` — selecciona la zona donde está el caret.
    SeleccionarZona,
}

struct Model {
    /// Cuerpo activo — su `orden` es lo que el editor reconstruye.
    cuerpo: Cuerpo,
    /// Atoms del "grafo" plano. Clave = `id`, valor = atom completo.
    atoms: HashMap<Uuid, NarrativeAtom>,
    /// El IDE: buffer + cursor + diff vs `editor_cuerpo`.
    ide: CuerpoIde,
    /// Clipboard local (no toca el del sistema — los demos viven en
    /// sandbox). `MemClipboard` cubre Ctrl+C/X/V durante la sesión.
    clipboard: MemClipboard,
    /// Mensaje del último `Guardar` para mostrar en el footer.
    ultimo_save: String,
    /// Acumulador de drag para selección por mouse.
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
            clipboard: MemClipboard::default(),
            ultimo_save: String::new(),
            drag_accum: (0.0, 0.0),
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::EditorKey(ev) => {
                let mut model = model;
                let _ = model.ide.apply_key_with_clipboard(&ev, &mut model.clipboard);
                model
            }
            Msg::EditorPointer(ev) => {
                let mut model = model;
                let scroll = model.ide.state.scroll_offset;
                match ev {
                    PointerEvent::Click { x, y } => {
                        model.drag_accum = (0.0, 0.0);
                        let (line, col) = METRICS.screen_to_pos(x, y, scroll);
                        model.ide.set_caret(line, col);
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
                        let (line, col) = METRICS.screen_to_pos(cx, cy, scroll);
                        model.ide.state.extend_selection_to(line, col);
                    }
                }
                model
            }
            Msg::Guardar => guardar(model),
            Msg::SaltarAtomoSiguiente => {
                let mut model = model;
                if let Some(siguiente) = atom_siguiente(&model.ide) {
                    if let Some((line, col)) = model.ide.posicion_de_atom(siguiente) {
                        model.ide.set_caret(line, col);
                        // Asegurar visibilidad: el widget no recalcula
                        // scroll cuando movemos el caret programáticamente.
                        model.ide.state.ensure_caret_visible(VISIBLE_LINES);
                    }
                }
                model
            }
            Msg::ToglearFusion => {
                let mut model = model;
                if let Some(idx) = model.ide.junction_antes_del_caret() {
                    model.ide.togglear_junction(idx);
                }
                model
            }
            Msg::ZonaSiguiente => {
                let mut model = model;
                model.ide.ir_a_zona_siguiente();
                model.ide.state.ensure_caret_visible(VISIBLE_LINES);
                model
            }
            Msg::ZonaAnterior => {
                let mut model = model;
                model.ide.ir_a_zona_anterior();
                model.ide.state.ensure_caret_visible(VISIBLE_LINES);
                model
            }
            Msg::SeleccionarZona => {
                let mut model = model;
                let zona = model.ide.zona_del_caret();
                model.ide.seleccionar_zona(zona);
                model.ide.state.ensure_caret_visible(VISIBLE_LINES);
                model
            }
        }
    }

    fn on_key(_model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        let ctrl = event.modifiers.ctrl || event.modifiers.meta;
        let shift = event.modifiers.shift;
        if ctrl {
            if let Key::Character(s) = &event.key {
                if s.eq_ignore_ascii_case("s") {
                    return Some(Msg::Guardar);
                }
                // Shift+] / Shift+[ saltan por zonas; sin shift, por átomos.
                // Algunos backends emiten "}" / "{" cuando shift+]/[ está
                // activo; aceptamos ambas formas.
                if shift && (s == "}" || s == "]") {
                    return Some(Msg::ZonaSiguiente);
                }
                if shift && (s == "{" || s == "[") {
                    return Some(Msg::ZonaAnterior);
                }
                if !shift && s == "]" {
                    return Some(Msg::SaltarAtomoSiguiente);
                }
                if shift && s.eq_ignore_ascii_case("a") {
                    return Some(Msg::SeleccionarZona);
                }
                if s.eq_ignore_ascii_case("j") {
                    return Some(Msg::ToglearFusion);
                }
            }
        }
        Some(Msg::EditorKey(event.clone()))
    }

    fn view(model: &Model) -> View<Msg> {
        let palette_editor = EditorPalette::default();
        let bg_app = palette_editor.bg;
        let fg_text = palette_editor.fg_text;
        let fg_muted = palette_editor.fg_line_number;

        let fundidas = model.ide.fundido_junctions.iter().filter(|f| **f).count();
        let total_junctions = model.ide.fundido_junctions.len();
        let header_text = format!(
            "cuerpo «{}»  ·  {} átomos  ·  {} zonas  ·  {} párrafos  ·  {}/{} junctions fundidas  ·  {}  ·  Ctrl+S guarda · Ctrl+] siguiente · Ctrl+J fundir · Ctrl+Shift+]/[ zona ↔ · Ctrl+Shift+A seleccionar zona",
            model.cuerpo.metadatos.nombre_legible,
            model.cuerpo.orden.len(),
            model.ide.n_zonas(),
            model.ide.n_parrafos_buffer(),
            fundidas,
            total_junctions,
            if model.ide.pendiente_sync() {
                "● sin guardar"
            } else {
                "○ sincronizado"
            },
        );
        let header = chip(header_text, 28.0, 12.0, Color::from_rgba8(40, 44, 52, 255), fg_text);

        let footer_text = if model.ultimo_save.is_empty() {
            "(sin saves todavía — editá libremente y dale Ctrl+S)".to_string()
        } else {
            model.ultimo_save.clone()
        };
        let footer = chip(footer_text, 24.0, 11.0, Color::from_rgba8(33, 36, 42, 255), fg_muted);

        let editor = cuerpo_ide_view::<Msg>(
            &model.ide,
            &palette_editor,
            METRICS,
            VISIBLE_LINES,
            Language::Plain,
            |ev| Some(Msg::EditorPointer(ev)),
        );

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

/// Banda de texto uniforme — header/footer comparten estilo.
fn chip(texto: String, alto: f32, font_size: f32, fondo: Color, fg: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(alto),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(fondo)
    .text_aligned(texto, font_size, fg, Alignment::Start)
}

/// Ciclo de "Ctrl+]": devuelve el siguiente atom_id en el orden del
/// cuerpo después de la línea actual del caret. Si el caret está en el
/// último átomo (o no se puede mapear), envuelve al primero.
fn atom_siguiente(ide: &CuerpoIde) -> Option<Uuid> {
    if ide.editor_cuerpo.atom_ids.is_empty() {
        return None;
    }
    let (linea, _) = ide.caret();
    let actual = ide.atom_id_en_linea(linea);
    let pos_actual = actual
        .and_then(|id| ide.editor_cuerpo.atom_ids.iter().position(|x| *x == id))
        .unwrap_or(usize::MAX);
    let n = ide.editor_cuerpo.atom_ids.len();
    let siguiente = if pos_actual == usize::MAX {
        0
    } else {
        (pos_actual + 1) % n
    };
    ide.editor_cuerpo.atom_ids.get(siguiente).copied()
}

/// Aplica el diff al modelo conservando caret + scroll. Reconstruye
/// `cuerpo.orden` y refresca el IDE.
fn guardar(model: Model) -> Model {
    let mut model = model;
    let caret_antes = model.ide.caret();
    let scroll_antes = model.ide.state.scroll_offset;

    let idx: HashMap<Uuid, &NarrativeAtom> = model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    let cambios = model.ide.diff(&idx);
    drop(idx);

    let resumen = persistir(&mut model, &cambios);

    // Tras persistir, recargá el IDE con el cuerpo + atoms actualizados.
    let cuerpo_clon = model.cuerpo.clone();
    let idx2: HashMap<Uuid, &NarrativeAtom> = model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    model.ide.recargar(&cuerpo_clon, &idx2);
    drop(idx2);

    // Restaurar caret + scroll para no perder el lugar — clamp al rango
    // del cuerpo nuevo (set_caret_at lo hace por nosotros).
    model.ide.set_caret(caret_antes.0, caret_antes.1);
    model.ide.state.scroll_offset = scroll_antes;
    model.ide.state.ensure_caret_visible(VISIBLE_LINES);

    model.ultimo_save = resumen;
    model
}

/// Aplica los `CambioAtom` al modelo: muta `atoms` y reconstruye
/// `cuerpo.orden`. Devuelve un resumen humano de qué pasó.
fn persistir(model: &mut Model, cambios: &[CambioAtom]) -> String {
    if cambios.is_empty() {
        return "guardado: sin cambios".to_string();
    }
    let mut mutados = 0usize;
    let mut creados: Vec<Uuid> = Vec::new();
    let mut eliminados = 0usize;

    for c in cambios {
        match c {
            CambioAtom::Mutar { id, texto_nuevo } => {
                if let Some(a) = model.atoms.get_mut(id) {
                    a.set_content(texto_nuevo.as_str());
                    mutados += 1;
                }
            }
            CambioAtom::Crear { texto, posicion: _ } => {
                let atom = NarrativeAtom::new(texto.as_str(), &model.cuerpo.branch_id);
                let id = atom.id;
                model.atoms.insert(id, atom);
                creados.push(id);
            }
            CambioAtom::Eliminar { id } => {
                model.atoms.remove(id);
                eliminados += 1;
            }
        }
    }

    // Reconstruí `cuerpo.orden`. El editor garantiza que `Crear` apunta
    // a posiciones consecutivas al final y los `Eliminar` salen del
    // final también — así podemos rehacer el orden con
    // `EditorCuerpo::aplicar_cambios`, que ya implementa esa semántica.
    model.ide.aplicar_cambios(cambios, &creados);
    let nuevo_orden: Vec<Uuid> = model.ide.editor_cuerpo.atom_ids.clone();

    let ahora = model.cuerpo.metadatos.modificado_en.saturating_add(1);
    let viejo: Vec<Uuid> = model.cuerpo.orden.clone();
    for id in &viejo {
        let _ = model.cuerpo.remover(*id, ahora);
    }
    for id in &nuevo_orden {
        model.cuerpo.agregar(*id, ahora);
    }

    format!(
        "guardado: {mutados} mutar · {} crear · {eliminados} eliminar — orden con {} átomos",
        creados.len(),
        nuevo_orden.len(),
    )
}

fn main() {
    llimphi_ui::run::<Demo>();
}
