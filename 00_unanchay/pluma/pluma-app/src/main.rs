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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_theme::Theme;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_text_editor::{
    Clipboard, EditorMetrics, EditorPalette as TEPalette, Language, PointerEvent,
};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_cuerpo::CambioAtom;
use pluma_editor_llimphi::cuerpo_ide::{cuerpo_ide_view, CuerpoIde};
use pluma_editor_llimphi::multilienzo::{
    multilienzo_view, IndiceAtoms, MultilienzoConfig, PaletaHebras,
};
use pluma_editor_llimphi::Palette as MultPalette;
use pluma_llm::{build_client, BackendKind, LlmConfig};
use pluma_llm_core::ChatClient;
use pluma_store::PlumaStore;
use pluma_transform::{TipoTransformacion, Transformacion};
use pluma_transform_llm::{
    EjecutorResumirLlm, EjecutorTonoLlm, EjecutorTraducirLlm,
};
use uuid::Uuid;

const METRICS: EditorMetrics = EditorMetrics::for_font_size(13.0);
const VISIBLE_LINES: usize = 200;

const BACKENDS: [BackendKind; 6] = [
    BackendKind::Mock,
    BackendKind::Gemini,
    BackendKind::Anthropic,
    BackendKind::DeepSeek,
    BackendKind::Cohere,
    BackendKind::Ollama,
];

fn main() {
    llimphi_ui::run::<Pluma>();
}

// ---------------------------------------------------------------------
// Mensajes
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Msg {
    EditorKey(KeyEvent),
    EditorPointer(PointerEvent),
    AbrirDoc(Uuid),
    NuevoDoc,
    Guardar,
    PathInputKey(KeyEvent),
    FocusPath,
    DefocusPath,
    AbrirArchivo,
    ExportarMd,
    FindToggle,
    FindKey(KeyEvent),
    FindSiguiente,
    FindAnterior,
    FindClose,
    DiffToggle,
    MoverAtomArriba,
    MoverAtomAbajo,
    ToglearFusion,
    ZonaSiguiente,
    ZonaAnterior,
    CicloBackend,
    PedirTraducir(String),
    PedirTono(String),
    PedirResumir(Option<u32>),
    LlmListo {
        hija: Cuerpo,
        atoms_nuevos: Vec<NarrativeAtom>,
        carta: CartaHebras,
        transformacion: Transformacion,
    },
    LlmError(String),
    ResizeIzq(f32),
    ResizeDer(f32),
}

// ---------------------------------------------------------------------
// Modelo
// ---------------------------------------------------------------------

struct Model {
    store: Arc<PlumaStore>,
    cuerpos: Vec<Cuerpo>,
    atoms: HashMap<Uuid, NarrativeAtom>,
    cartas: Vec<CartaHebras>,
    transformaciones: Vec<Transformacion>,
    /// `id` del `Cuerpo` activo (el que se ve en `ide`). `None` sólo si
    /// la lista de cuerpos está vacía — el init siembra uno para evitarlo.
    activo: Option<Uuid>,
    ide: CuerpoIde,
    clipboard: ArboardClipboard,
    drag_accum: (f32, f32),

    chat: Arc<dyn ChatClient>,
    backend_idx: usize,
    en_curso: bool,
    ultimo_error: Option<String>,
    ultimo_status: String,

    /// Ruta del archivo a abrir/exportar — input compartido.
    /// Se interpreta según qué botón clickea el usuario.
    path_input: TextInputState,
    /// Cuando es `true`, las teclas del usuario van al `path_input` en
    /// vez del editor. Click sobre el input lo enciende; Esc, o un
    /// click fuera (en realidad, sólo Esc) lo apaga.
    path_focused: bool,

    /// Find-in-page sobre el cuerpo activo. `Ctrl+F` muestra el overlay
    /// y lo enfoca; Esc lo cierra; Enter/Shift+Enter cyclan matches.
    find_input: TextInputState,
    find_visible: bool,
    find_matches: Vec<(usize, usize)>,
    find_idx: usize,

    /// Cuando es `true` y el cuerpo activo es una hija, el centro
    /// muestra la madre y la hija lado a lado con las hebras pintadas
    /// (read-only). Cuando el activo es Original o no se encuentra la
    /// madre, el flag igual existe pero la vista cae al cuerpo_ide
    /// normal con un cartel.
    diff_visible: bool,

    side_izq_w: f32,
    side_der_w: f32,
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
}

// ---------------------------------------------------------------------
// Inicialización
// ---------------------------------------------------------------------

fn init_modelo() -> Model {
    let path = ruta_sled();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let store = match PlumaStore::open(&path) {
        Ok(s) => Arc::new(s),
        Err(e) => panic!("pluma-app :: PlumaStore::open({path:?}) falló: {e:?}"),
    };

    let mut atoms: HashMap<Uuid, NarrativeAtom> = store
        .iter_atoms()
        .filter_map(|r| r.ok())
        .map(|a| (a.id, a))
        .collect();
    let mut cuerpos: Vec<Cuerpo> = store.iter_cuerpos().filter_map(|r| r.ok()).collect();
    let transformaciones: Vec<Transformacion> = store
        .iter_transformaciones()
        .filter_map(|r| r.ok())
        .collect();
    let cartas: Vec<CartaHebras> = store.iter_cartas().filter_map(|r| r.ok()).collect();

    // Si el sled está vacío, sembrar un documento Original para que la
    // ventana no esté muerta al primer arranque.
    if cuerpos.is_empty() {
        let ahora = ahora_unix();
        let atom = NarrativeAtom::new("Empieza a escribir aquí…", "es");
        let mut cuerpo = Cuerpo::nuevo("es", "documento sin título", Intencion::Original, ahora);
        cuerpo.agregar(atom.id, ahora);
        let _ = store.put_atom(&atom);
        let _ = store.put_cuerpo(&cuerpo);
        let _ = store.flush();
        atoms.insert(atom.id, atom);
        cuerpos.push(cuerpo);
    }

    // Cuerpo activo = primer Original; si no hay, el primero a secas.
    let activo = cuerpos
        .iter()
        .find(|c| !c.metadatos.intencion.es_derivada())
        .map(|c| c.id)
        .or_else(|| cuerpos.first().map(|c| c.id));

    let ide = match activo {
        Some(id) => {
            let cuerpo = cuerpos.iter().find(|c| c.id == id).cloned().unwrap();
            let idx: HashMap<Uuid, &NarrativeAtom> =
                atoms.iter().map(|(k, v)| (*k, v)).collect();
            CuerpoIde::from_cuerpo(&cuerpo, &idx)
        }
        None => CuerpoIde::nuevo_vacio(),
    };

    let (chat, backend_idx) = inicializar_backend();

    Model {
        store,
        cuerpos,
        atoms,
        cartas,
        transformaciones,
        activo,
        ide,
        clipboard: ArboardClipboard::new(),
        drag_accum: (0.0, 0.0),
        chat,
        backend_idx,
        en_curso: false,
        ultimo_error: None,
        ultimo_status: "listo".to_string(),
        path_input: TextInputState::new(),
        path_focused: false,
        find_input: TextInputState::new(),
        find_visible: false,
        find_matches: Vec::new(),
        find_idx: 0,
        diff_visible: false,
        side_izq_w: 280.0,
        side_der_w: 340.0,
    }
}

/// Intenta el primer backend con env key configurada; si ninguno
/// matchea, cae a Mock. Devuelve también el índice en `BACKENDS`.
fn inicializar_backend() -> (Arc<dyn ChatClient>, usize) {
    let preferencias: &[(BackendKind, &[&str])] = &[
        (BackendKind::Anthropic, &["ANTHROPIC_API_KEY"]),
        (BackendKind::Gemini, &["GEMINI_API_KEY", "GOOGLE_API_KEY"]),
        (BackendKind::DeepSeek, &["DEEPSEEK_API_KEY"]),
        (BackendKind::Cohere, &["COHERE_API_KEY"]),
    ];
    for (kind, envs) in preferencias {
        if envs.iter().any(|e| std::env::var(e).is_ok()) {
            if let Ok(c) = build_client(&LlmConfig {
                kind: *kind,
                ..Default::default()
            }) {
                let idx = BACKENDS.iter().position(|k| k == kind).unwrap_or(0);
                return (c, idx);
            }
        }
    }
    // Fallback: Mock (siempre construye).
    let c = build_client(&LlmConfig {
        kind: BackendKind::Mock,
        ..Default::default()
    })
    .expect("Mock backend no debería fallar");
    let idx = BACKENDS
        .iter()
        .position(|k| *k == BackendKind::Mock)
        .unwrap_or(0);
    (c, idx)
}

// ---------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------

fn actualizar(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
    match msg {
        Msg::EditorKey(ev) => {
            let _ = model.ide.apply_key_with_clipboard(&ev, &mut model.clipboard);
        }
        Msg::EditorPointer(ev) => {
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
        }
        Msg::AbrirDoc(id) => {
            cambiar_activo(&mut model, id);
        }
        Msg::NuevoDoc => {
            crear_doc_nuevo(&mut model);
        }
        Msg::Guardar => {
            guardar_activo(&mut model);
        }
        Msg::PathInputKey(ev) => {
            model.path_input.apply_key(&ev);
        }
        Msg::FocusPath => {
            model.path_focused = true;
        }
        Msg::DefocusPath => {
            model.path_focused = false;
        }
        Msg::AbrirArchivo => {
            model.path_focused = false;
            abrir_archivo(&mut model);
        }
        Msg::ExportarMd => {
            model.path_focused = false;
            exportar_md(&mut model);
        }
        Msg::FindToggle => {
            model.find_visible = !model.find_visible;
            if model.find_visible {
                recomputar_matches(&mut model);
                if !model.find_matches.is_empty() {
                    saltar_a_match(&mut model);
                }
            }
        }
        Msg::FindKey(ev) => {
            model.find_input.apply_key(&ev);
            recomputar_matches(&mut model);
            if !model.find_matches.is_empty() {
                saltar_a_match(&mut model);
            }
        }
        Msg::FindSiguiente => {
            if model.find_matches.is_empty() {
                return model;
            }
            model.find_idx = (model.find_idx + 1) % model.find_matches.len();
            saltar_a_match(&mut model);
        }
        Msg::FindAnterior => {
            if model.find_matches.is_empty() {
                return model;
            }
            let n = model.find_matches.len();
            model.find_idx = (model.find_idx + n - 1) % n;
            saltar_a_match(&mut model);
        }
        Msg::FindClose => {
            model.find_visible = false;
        }
        Msg::DiffToggle => {
            model.diff_visible = !model.diff_visible;
        }
        Msg::MoverAtomArriba => {
            mover_atom_caret(&mut model, -1);
        }
        Msg::MoverAtomAbajo => {
            mover_atom_caret(&mut model, 1);
        }
        Msg::ToglearFusion => {
            if let Some(idx) = model.ide.junction_antes_del_caret() {
                model.ide.togglear_junction(idx);
            }
        }
        Msg::ZonaSiguiente => {
            model.ide.ir_a_zona_siguiente();
            model.ide.state.ensure_caret_visible(VISIBLE_LINES);
        }
        Msg::ZonaAnterior => {
            model.ide.ir_a_zona_anterior();
            model.ide.state.ensure_caret_visible(VISIBLE_LINES);
        }
        Msg::CicloBackend => {
            cycle_backend(&mut model);
        }
        Msg::PedirTraducir(lengua) => {
            lanzar(&mut model, handle, TrabajoLlm::Traducir(lengua));
        }
        Msg::PedirTono(etiqueta) => {
            lanzar(&mut model, handle, TrabajoLlm::Tono(etiqueta));
        }
        Msg::PedirResumir(palabras) => {
            lanzar(&mut model, handle, TrabajoLlm::Resumir(palabras));
        }
        Msg::LlmListo {
            hija,
            atoms_nuevos,
            carta,
            transformacion,
        } => {
            recibir_hija(&mut model, hija, atoms_nuevos, carta, transformacion);
        }
        Msg::LlmError(s) => {
            eprintln!("pluma-app :: error LLM: {s}");
            model.ultimo_error = Some(s);
            model.en_curso = false;
        }
        Msg::ResizeIzq(dx) => {
            model.side_izq_w = (model.side_izq_w + dx).clamp(160.0, 420.0);
        }
        Msg::ResizeDer(dx) => {
            // El divisor está del lado izquierdo de la columna derecha:
            // dx>0 = divisor a la derecha = panel der encoge.
            model.side_der_w = (model.side_der_w - dx).clamp(220.0, 520.0);
        }
    }
    model
}

fn cambiar_activo(model: &mut Model, id: Uuid) {
    if model.activo == Some(id) {
        return;
    }
    let cuerpo = match model.cuerpos.iter().find(|c| c.id == id) {
        Some(c) => c.clone(),
        None => return,
    };
    model.activo = Some(id);
    let idx: HashMap<Uuid, &NarrativeAtom> =
        model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    model.ide.recargar(&cuerpo, &idx);
    model.ultimo_status = format!("doc: {}", cuerpo.metadatos.nombre_legible);
}

fn crear_doc_nuevo(model: &mut Model) {
    let ahora = ahora_unix();
    let n = model
        .cuerpos
        .iter()
        .filter(|c| !c.metadatos.intencion.es_derivada())
        .count()
        + 1;
    let atom = NarrativeAtom::new("Empieza a escribir aquí…", "es");
    let mut cuerpo = Cuerpo::nuevo(
        format!("es-{n}"),
        format!("doc #{n} sin título"),
        Intencion::Original,
        ahora,
    );
    cuerpo.agregar(atom.id, ahora);
    let _ = model.store.put_atom(&atom);
    let _ = model.store.put_cuerpo(&cuerpo);
    let _ = model.store.flush();
    let id = cuerpo.id;
    model.atoms.insert(atom.id, atom);
    model.cuerpos.push(cuerpo);
    cambiar_activo(model, id);
    model.ultimo_status = format!("doc #{n} creado");
}

fn guardar_activo(model: &mut Model) {
    let Some(activo_id) = model.activo else {
        model.ultimo_status = "sin doc activo".into();
        return;
    };
    let idx: HashMap<Uuid, &NarrativeAtom> =
        model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    let cambios = model.ide.diff(&idx);
    drop(idx);

    if cambios.is_empty() {
        model.ultimo_status = "sin cambios".into();
        return;
    }

    let mut creados: Vec<Uuid> = Vec::new();
    let branch_id = model
        .cuerpos
        .iter()
        .find(|c| c.id == activo_id)
        .map(|c| c.branch_id.clone())
        .unwrap_or_else(|| "es".to_string());

    for c in &cambios {
        match c {
            CambioAtom::Mutar { id, texto_nuevo } => {
                if let Some(a) = model.atoms.get_mut(id) {
                    a.set_content(texto_nuevo.as_str());
                    let _ = model.store.put_atom(a);
                }
            }
            CambioAtom::Crear { texto, posicion: _ } => {
                let atom = NarrativeAtom::new(texto.as_str(), &branch_id);
                let id = atom.id;
                let _ = model.store.put_atom(&atom);
                model.atoms.insert(id, atom);
                creados.push(id);
            }
            CambioAtom::Eliminar { id } => {
                model.atoms.remove(id);
                // El sled mantiene el atom histórico — no lo borramos
                // del backend porque hijas/cartas pueden seguir apuntando
                // a él. La memoria local sí lo descarta.
            }
        }
    }

    model.ide.aplicar_cambios(&cambios, &creados);

    // Reconstruir `cuerpo.orden` con el orden nuevo del IDE.
    let nuevo_orden: Vec<Uuid> = model.ide.editor_cuerpo.atom_ids.clone();
    if let Some(c) = model.cuerpos.iter_mut().find(|c| c.id == activo_id) {
        let ahora = c.metadatos.modificado_en.saturating_add(1);
        let viejo: Vec<Uuid> = c.orden.clone();
        for id in &viejo {
            let _ = c.remover(*id, ahora);
        }
        for id in &nuevo_orden {
            c.agregar(*id, ahora);
        }
        let _ = model.store.put_cuerpo(c);
    }
    let _ = model.store.flush();

    let n_mut = cambios
        .iter()
        .filter(|c| matches!(c, CambioAtom::Mutar { .. }))
        .count();
    let n_new = creados.len();
    let n_del = cambios
        .iter()
        .filter(|c| matches!(c, CambioAtom::Eliminar { .. }))
        .count();
    model.ultimo_status = format!("guardado: {n_mut} mut · {n_new} crear · {n_del} del");
}

/// Recalcula las posiciones (línea, col) donde aparece el query en el
/// buffer actual. Búsqueda case-insensitive, substring. Llamarlo cada
/// vez que el query o el texto cambian. Reset de `find_idx` al primer
/// match cuando hay alguno; lo deja en 0 si no hay (consistente con
/// "0 de 0"), pero la UI no salta si está vacío.
fn recomputar_matches(model: &mut Model) {
    let query = model.find_input.text();
    if query.is_empty() {
        model.find_matches.clear();
        model.find_idx = 0;
        return;
    }
    let q_lower = query.to_lowercase();
    let mut matches: Vec<(usize, usize)> = Vec::new();
    let texto = model.ide.texto_buffer();
    for (line_idx, linea) in texto.lines().enumerate() {
        let l_lower = linea.to_lowercase();
        let mut start = 0;
        while let Some(pos) = l_lower[start..].find(&q_lower) {
            let col = start + pos;
            matches.push((line_idx, col));
            start = col + q_lower.len().max(1);
            if start >= l_lower.len() {
                break;
            }
        }
    }
    model.find_matches = matches;
    if model.find_idx >= model.find_matches.len() {
        model.find_idx = 0;
    }
}

fn saltar_a_match(model: &mut Model) {
    let Some(&(line, col)) = model.find_matches.get(model.find_idx) else {
        return;
    };
    model.ide.set_caret(line, col);
    model.ide.state.ensure_caret_visible(VISIBLE_LINES);
}

/// Mueve el átomo donde está el caret una posición arriba (`delta=-1`)
/// o abajo (`delta=1`). Sincroniza el buffer al modelo antes de
/// reordenar (para no perder ediciones pendientes), muta `cuerpo.orden`,
/// persiste, y recarga el IDE — junctions resetean a separadores (es
/// el costo del reorder; el usuario las re-fusiona si las quería).
/// El caret queda en la primera línea del átomo movido.
fn mover_atom_caret(model: &mut Model, delta: i32) {
    let Some(activo_id) = model.activo else {
        return;
    };
    // Sincroniza pendientes para no perderlos al recargar.
    guardar_activo(model);

    let (caret_line, _) = model.ide.caret();
    let Some(atom_id) = model.ide.atom_id_en_linea(caret_line) else {
        return;
    };
    let cuerpo = match model.cuerpos.iter_mut().find(|c| c.id == activo_id) {
        Some(c) => c,
        None => return,
    };
    let n = cuerpo.orden.len();
    if n < 2 {
        return;
    }
    let i = match cuerpo.orden.iter().position(|x| *x == atom_id) {
        Some(i) => i,
        None => return,
    };
    let j = if delta < 0 {
        if i == 0 {
            return;
        }
        i - 1
    } else {
        if i + 1 >= n {
            return;
        }
        i + 1
    };
    cuerpo.orden.swap(i, j);
    cuerpo.metadatos.modificado_en = cuerpo.metadatos.modificado_en.saturating_add(1);
    let _ = model.store.put_cuerpo(cuerpo);
    let _ = model.store.flush();

    // Recargar el IDE con el orden nuevo. Snapshot la cuerpo data
    // primero para evitar el borrow simultáneo del index.
    let cuerpo_clon = cuerpo.clone();
    // Liberamos el préstamo mutable de `model.cuerpos` antes de
    // tomar uno inmutable de `model.atoms` para construir el índice.
    let _ = cuerpo;
    let idx: HashMap<Uuid, &NarrativeAtom> =
        model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    model.ide.recargar(&cuerpo_clon, &idx);
    drop(idx);

    // Posicionar el caret al inicio del átomo movido. Su nuevo idx es
    // `j`; sumamos lineas anteriores (cada atom = 1 + atoms_extra_lineas
    // + separador). Más simple: usar posicion_de_atom.
    if let Some((line, col)) = model.ide.posicion_de_atom(atom_id) {
        model.ide.set_caret(line, col);
        model.ide.state.ensure_caret_visible(VISIBLE_LINES);
    }

    model.ultimo_status = format!(
        "atom movido {}",
        if delta < 0 { "↑" } else { "↓" }
    );
    model.ultimo_error = None;
}

fn abrir_archivo(model: &mut Model) {
    let path_raw = model.path_input.text().trim().to_string();
    if path_raw.is_empty() {
        model.ultimo_error = Some("ruta vacía".into());
        return;
    }
    let path = expandir_ruta(&path_raw);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            model.ultimo_error = Some(format!("leyendo {path:?}: {e}"));
            return;
        }
    };
    let nombre = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archivo".to_string());
    let ahora = ahora_unix();

    let importado = if extension_lower(&path) == Some("docx".to_string()) {
        match foreign_docx::parse_docx(&bytes, "es", nombre.clone(), ahora) {
            Ok(imp) => (imp.cuerpo, imp.atoms),
            Err(e) => {
                model.ultimo_error = Some(format!("parse_docx {nombre}: {e:?}"));
                return;
            }
        }
    } else if extension_lower(&path) == Some("md".to_string())
        || extension_lower(&path) == Some("markdown".to_string())
        || extension_lower(&path) == Some("txt".to_string())
    {
        let texto = match std::str::from_utf8(&bytes) {
            Ok(s) => s.to_string(),
            Err(e) => {
                model.ultimo_error = Some(format!("{nombre} no es UTF-8: {e}"));
                return;
            }
        };
        let imp = pluma_md::parse_md(&texto, "es", nombre.clone(), ahora);
        (imp.cuerpo, imp.atoms)
    } else {
        model.ultimo_error = Some(format!(
            "extensión no soportada en {nombre} — usá .md o .docx"
        ));
        return;
    };

    let (cuerpo, atoms_nuevos) = importado;
    if atoms_nuevos.is_empty() {
        model.ultimo_error = Some(format!("{nombre} no produjo átomos"));
        return;
    }
    for a in &atoms_nuevos {
        let _ = model.store.put_atom(a);
        model.atoms.insert(a.id, a.clone());
    }
    let _ = model.store.put_cuerpo(&cuerpo);
    let _ = model.store.flush();
    let id = cuerpo.id;
    let n = atoms_nuevos.len();
    model.cuerpos.push(cuerpo);
    model.ultimo_status = format!("abierto «{nombre}»: {n} átomos");
    model.ultimo_error = None;
    cambiar_activo(model, id);
}

fn exportar_md(model: &mut Model) {
    let Some(activo_id) = model.activo else {
        model.ultimo_error = Some("sin doc activo".into());
        return;
    };
    let path_raw = model.path_input.text().trim().to_string();
    if path_raw.is_empty() {
        model.ultimo_error = Some("ruta vacía".into());
        return;
    }
    let path = expandir_ruta(&path_raw);
    let Some(cuerpo) = model.cuerpos.iter().find(|c| c.id == activo_id) else {
        model.ultimo_error = Some("doc activo desapareció".into());
        return;
    };

    let ext = extension_lower(&path).unwrap_or_default();
    let bytes: Vec<u8> = if ext == "docx" {
        match foreign_docx::write_docx(cuerpo, &model.atoms) {
            Ok(b) => b,
            Err(e) => {
                model.ultimo_error = Some(format!("write_docx: {e}"));
                return;
            }
        }
    } else if ext.is_empty() || ext == "md" || ext == "markdown" || ext == "txt" {
        let md = pluma_md::to_md(cuerpo, &model.atoms);
        if md.is_empty() {
            model.ultimo_error = Some("doc vacío — nada que exportar".into());
            return;
        }
        md.into_bytes()
    } else {
        model.ultimo_error = Some(format!(
            "extensión .{ext} no soportada — usá .md o .docx"
        ));
        return;
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, &bytes) {
        Ok(()) => {
            model.ultimo_status = format!(
                "exportado «{}» a {} ({} bytes)",
                cuerpo.metadatos.nombre_legible,
                path.display(),
                bytes.len(),
            );
            model.ultimo_error = None;
        }
        Err(e) => {
            model.ultimo_error = Some(format!("escribiendo {path:?}: {e}"));
        }
    }
}

fn expandir_ruta(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    } else if raw == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(raw)
}

fn extension_lower(p: &std::path::Path) -> Option<String> {
    p.extension().map(|e| e.to_string_lossy().to_lowercase())
}

fn cycle_backend(model: &mut Model) {
    let total = BACKENDS.len();
    for step in 1..=total {
        let try_idx = (model.backend_idx + step) % total;
        let kind = BACKENDS[try_idx];
        match build_client(&LlmConfig {
            kind,
            ..Default::default()
        }) {
            Ok(c) => {
                model.chat = c;
                model.backend_idx = try_idx;
                model.ultimo_status = format!("backend → {}", etiqueta_backend(kind));
                model.ultimo_error = None;
                return;
            }
            Err(e) => {
                model.ultimo_error = Some(format!("backend {kind:?}: {e}"));
            }
        }
    }
    // Si todos fallaron (no debería: Mock siempre funciona), no-op.
}

fn recibir_hija(
    model: &mut Model,
    hija: Cuerpo,
    atoms_nuevos: Vec<NarrativeAtom>,
    carta: CartaHebras,
    transformacion: Transformacion,
) {
    for a in &atoms_nuevos {
        let _ = model.store.put_atom(a);
        model.atoms.insert(a.id, a.clone());
    }
    let _ = model.store.put_cuerpo(&hija);
    let _ = model.store.put_carta(&carta);
    let _ = model.store.put_transformacion(&transformacion);
    let _ = model.store.flush();
    let hija_id = hija.id;
    let nombre = hija.metadatos.nombre_legible.clone();
    model.cuerpos.push(hija);
    model.cartas.push(carta);
    model.transformaciones.push(transformacion);
    model.en_curso = false;
    model.ultimo_status = format!("hija «{nombre}» derivada");
    cambiar_activo(model, hija_id);
}

// ---------------------------------------------------------------------
// Trabajo LLM
// ---------------------------------------------------------------------

enum TrabajoLlm {
    Traducir(String),
    Tono(String),
    Resumir(Option<u32>),
}

fn lanzar(model: &mut Model, handle: &Handle<Msg>, trabajo: TrabajoLlm) {
    if model.en_curso {
        return;
    }
    let Some(activo_id) = model.activo else {
        model.ultimo_status = "sin doc activo".into();
        return;
    };
    // Sincronizar antes de transformar — si el usuario tipeó sin Ctrl+S,
    // queremos que el LLM vea el texto editado.
    guardar_activo(model);

    let madre = match model.cuerpos.iter().find(|c| c.id == activo_id) {
        Some(c) => c.clone(),
        None => {
            model.ultimo_error = Some("doc activo desapareció".into());
            return;
        }
    };
    if madre.orden.is_empty() {
        model.ultimo_status = "madre vacía — nada que transformar".into();
        return;
    }

    let atoms_owned: Vec<NarrativeAtom> = model.atoms.values().cloned().collect();
    let chat = model.chat.clone();
    let h = handle.clone();
    let ahora = ahora_unix();

    model.en_curso = true;
    model.ultimo_error = None;
    model.ultimo_status = format!("LLM en curso ({} backend)", etiqueta_backend(BACKENDS[model.backend_idx]));

    handle.spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => return Msg::LlmError(format!("runtime tokio: {e}")),
        };
        let idx: HashMap<Uuid, &NarrativeAtom> =
            atoms_owned.iter().map(|a| (a.id, a)).collect();

        let resultado = rt.block_on(async {
            match trabajo {
                TrabajoLlm::Traducir(lengua) => {
                    let ej = EjecutorTraducirLlm::from_arc(chat, lengua.clone());
                    let t = Transformacion::nueva(
                        madre.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Traducir {
                            lengua_destino: lengua,
                        },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora)
                        .await
                        .map(|p| (p, t))
                }
                TrabajoLlm::Tono(etiq) => {
                    let ej = EjecutorTonoLlm::from_arc(chat, etiq.clone());
                    let t = Transformacion::nueva(
                        madre.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Tono { etiqueta: etiq },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora)
                        .await
                        .map(|p| (p, t))
                }
                TrabajoLlm::Resumir(palabras) => {
                    let ej = EjecutorResumirLlm::from_arc(chat, palabras);
                    let t = Transformacion::nueva(
                        madre.id,
                        Uuid::new_v4(),
                        TipoTransformacion::Resumir {
                            palabras_objetivo: palabras,
                        },
                        "ui",
                        ahora,
                    );
                    ej.aplicar_con_atoms(&t, &madre, &idx, ahora)
                        .await
                        .map(|p| (p, t))
                }
            }
        });

        let _ = h;
        match resultado {
            Ok((prod, transformacion)) => Msg::LlmListo {
                hija: prod.hija,
                atoms_nuevos: prod.atoms_nuevos,
                carta: prod.carta,
                transformacion,
            },
            Err(e) => Msg::LlmError(format!("{e:?}")),
        }
    });
}

// ---------------------------------------------------------------------
// View
// ---------------------------------------------------------------------

fn vista(model: &Model) -> View<Msg> {
    let theme = Theme::dark();
    let editor_palette = TEPalette::default();
    let splitter_palette = SplitterPalette::from_theme(&theme);

    let status = barra_status(model, &theme);
    let panel_izq = panel_documentos(model, &theme);
    let panel_centro = panel_editor(model, &editor_palette);
    let panel_der = panel_llm(model, &theme);

    // Splitter anidado: izq | (centro | der).
    let centro_der = splitter_two(
        Direction::Row,
        panel_centro,
        PaneSize::Flex,
        panel_der,
        PaneSize::Fixed(model.side_der_w),
        |phase, dx| match phase {
            DragPhase::Move => Some(Msg::ResizeDer(dx)),
            DragPhase::End => None,
        },
        &splitter_palette,
    );
    let body = splitter_two(
        Direction::Row,
        panel_izq,
        PaneSize::Fixed(model.side_izq_w),
        centro_der,
        PaneSize::Flex,
        |phase, dx| match phase {
            DragPhase::Move => Some(Msg::ResizeIzq(dx)),
            DragPhase::End => None,
        },
        &splitter_palette,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![status, body])
}

fn barra_status(model: &Model, theme: &Theme) -> View<Msg> {
    let nombre = model
        .activo
        .and_then(|id| model.cuerpos.iter().find(|c| c.id == id))
        .map(|c| c.metadatos.nombre_legible.clone())
        .unwrap_or_else(|| "(sin doc)".to_string());
    let zona = model.ide.zona_del_caret();
    let n_zonas = model.ide.n_zonas();
    let backend = etiqueta_backend(BACKENDS[model.backend_idx]);
    let estado = if model.en_curso {
        "⏳"
    } else if model.ultimo_error.is_some() {
        "⚠"
    } else {
        "○"
    };
    let texto = format!(
        "pluma · {nombre} · zona {zona}/{n_zonas} · backend {backend} · {estado} {}",
        model.ultimo_status
    );
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(texto, 12.0, theme.fg_text, Alignment::Start)
}

fn panel_documentos(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_btn = ButtonPalette::from_theme(theme);
    let palette_list = ListPalette::from_theme(theme);

    // Originales primero, luego derivadas — el orden en la lista es
    // estable porque clonamos `model.cuerpos` y particionamos.
    let mut originales: Vec<&Cuerpo> = Vec::new();
    let mut derivadas: Vec<&Cuerpo> = Vec::new();
    for c in &model.cuerpos {
        if c.metadatos.intencion.es_derivada() {
            derivadas.push(c);
        } else {
            originales.push(c);
        }
    }

    let mut rows: Vec<ListRow<Msg>> = Vec::new();
    for c in originales.iter().chain(derivadas.iter()) {
        let prefijo = if c.metadatos.intencion.es_derivada() {
            "  ↳ "
        } else {
            "■ "
        };
        let label = format!(
            "{prefijo}{} · {}",
            c.metadatos.nombre_legible, c.branch_id
        );
        rows.push(ListRow {
            label,
            selected: model.activo == Some(c.id),
            on_click: Msg::AbrirDoc(c.id),
        });
    }

    let n = rows.len();
    let lista = list_view(ListSpec {
        rows,
        total: n,
        caption: Some(format!("{n} documentos")),
        truncated_hint: None,
        row_height: 22.0,
        palette: palette_list,
    });

    let boton_nuevo = button_view::<Msg>("＋  nuevo doc  (Ctrl+N)", &palette_btn, Msg::NuevoDoc);
    let boton_guardar = button_view::<Msg>("💾  guardar  (Ctrl+S)", &palette_btn, Msg::Guardar);

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned("DOCUMENTOS".to_string(), 10.0, theme.fg_muted, Alignment::Start);

    let archivo = seccion_archivo(model, theme);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .clip(true)
    .children(vec![header, boton_nuevo, boton_guardar, archivo, lista])
}

fn seccion_archivo(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_btn = ButtonPalette::from_theme(theme);
    let palette_input = TextInputPalette::from_theme(theme);

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "ARCHIVO".to_string(),
        10.0,
        theme.fg_muted,
        Alignment::Start,
    );

    let input = text_input_view::<Msg>(
        &model.path_input,
        "ruta .md o .docx (Esc para salir)",
        model.path_focused,
        &palette_input,
        Msg::FocusPath,
    );

    let fila_botones = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        button_view::<Msg>("📂 abrir", &palette_btn, Msg::AbrirArchivo),
        button_view::<Msg>("⬆ exportar (md/docx)", &palette_btn, Msg::ExportarMd),
    ]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(82.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(vec![header, input, fila_botones])
}

fn panel_editor(model: &Model, palette_editor: &TEPalette) -> View<Msg> {
    let cuerpo_central: View<Msg> = if model.diff_visible {
        vista_diff(model, palette_editor)
    } else {
        cuerpo_ide_view::<Msg>(
            &model.ide,
            palette_editor,
            METRICS,
            VISIBLE_LINES,
            Language::Plain,
            |ev| Some(Msg::EditorPointer(ev)),
        )
    };

    let mut hijos: Vec<View<Msg>> = Vec::new();
    if model.find_visible {
        hijos.push(barra_find(model));
    }
    hijos.push(cuerpo_central);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(palette_editor.bg)
    .clip(true)
    .children(hijos)
}

fn vista_diff(model: &Model, palette_editor: &TEPalette) -> View<Msg> {
    // Resolver activo + madre. Si activo no es derivado o la madre no
    // se encuentra, mostramos un cartel y volvemos a `cuerpo_ide_view`.
    let theme = Theme::dark();
    let activo_id = match model.activo {
        Some(id) => id,
        None => return cartel_diff("sin doc activo", palette_editor),
    };
    let activo = match model.cuerpos.iter().find(|c| c.id == activo_id) {
        Some(c) => c,
        None => return cartel_diff("activo no encontrado", palette_editor),
    };
    let madre_id = match activo.metadatos.derivado_de {
        Some(id) => id,
        None => {
            // Activo es Original — fallback al editor normal con cartel.
            return View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                gap: Size {
                    width: length(0.0_f32),
                    height: length(4.0_f32),
                },
                ..Default::default()
            })
            .children(vec![
                cartel_diff(
                    "este cuerpo es Original — no tiene madre con que diffear (Ctrl+D para cerrar)",
                    palette_editor,
                ),
                cuerpo_ide_view::<Msg>(
                    &model.ide,
                    palette_editor,
                    METRICS,
                    VISIBLE_LINES,
                    Language::Plain,
                    |ev| Some(Msg::EditorPointer(ev)),
                ),
            ]);
        }
    };
    let madre = match model.cuerpos.iter().find(|c| c.id == madre_id) {
        Some(c) => c,
        None => return cartel_diff(
            "madre referenciada no está en el sled — ¿borrada?",
            palette_editor,
        ),
    };

    // Buscar la carta de hebras entre estos dos. `pluma_align::CartaHebras`
    // anota su par; consideramos cualquier orden.
    let carta = model.cartas.iter().find(|c| {
        (c.cuerpo_a == Some(madre.id) && c.cuerpo_b == Some(activo.id))
            || (c.cuerpo_a == Some(activo.id) && c.cuerpo_b == Some(madre.id))
    });

    let cuerpos_ref: Vec<&Cuerpo> = vec![madre, activo];
    let cartas_ref: Vec<Option<&CartaHebras>> = vec![carta];
    let atoms_idx: IndiceAtoms = model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    let cfg = MultilienzoConfig::default();
    let paleta_hebras = PaletaHebras::default();
    let palette_mult = MultPalette::from_theme(&theme);

    let mult = multilienzo_view::<Msg>(
        &cuerpos_ref,
        &atoms_idx,
        &cartas_ref,
        &cfg,
        &paleta_hebras,
        &palette_mult,
    );

    let header_text = format!(
        "DIFF · madre «{}» ↔ hija «{}» ({})",
        madre.metadatos.nombre_legible,
        activo.metadatos.nombre_legible,
        if carta.is_some() {
            "con hebras"
        } else {
            "sin carta — hebras no disponibles"
        },
    );
    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(header_text, 11.0, theme.fg_muted, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(vec![header, mult])
}

fn cartel_diff(texto: &str, palette_editor: &TEPalette) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        texto.to_string(),
        12.0,
        palette_editor.fg_line_number,
        Alignment::Start,
    )
}

fn barra_find(model: &Model) -> View<Msg> {
    let theme = Theme::dark();
    let palette_input = TextInputPalette::from_theme(&theme);
    let palette_btn = ButtonPalette::from_theme(&theme);

    let input = text_input_view::<Msg>(
        &model.find_input,
        "buscar (Enter siguiente · Shift+Enter previo · Esc cerrar)",
        true, // find_visible implica que tiene foco
        &palette_input,
        Msg::FindToggle, // click en el input no cambia foco — siempre vivo
    );

    let total = model.find_matches.len();
    let pos = if total == 0 {
        0
    } else {
        model.find_idx + 1
    };
    let counter = View::new(Style {
        size: Size {
            width: length(80.0_f32),
            height: length(34.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        format!("{pos}/{total}"),
        12.0,
        theme.fg_muted,
        Alignment::Center,
    );

    let prev = button_view::<Msg>("◀", &palette_btn, Msg::FindAnterior);
    let next = button_view::<Msg>("▶", &palette_btn, Msg::FindSiguiente);
    let cerrar = button_view::<Msg>("✕", &palette_btn, Msg::FindClose);

    let input_wrap = View::new(Style {
        flex_grow: 1.0,
        flex_shrink: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        ..Default::default()
    })
    .children(vec![input]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![input_wrap, counter, prev, next, cerrar])
}

fn panel_llm(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_btn_activo = ButtonPalette::from_theme(theme);
    let palette_btn_off = ButtonPalette {
        bg: Color::from_rgba8(60, 60, 60, 255),
        bg_hover: Color::from_rgba8(60, 60, 60, 255),
        fg: Color::from_rgba8(140, 140, 140, 255),
        radius: palette_btn_activo.radius,
    };
    let pal = if model.en_curso {
        &palette_btn_off
    } else {
        &palette_btn_activo
    };
    let pal_backend = &palette_btn_activo;

    let etiqueta_back = format!(
        "🔀  backend: {}",
        etiqueta_backend(BACKENDS[model.backend_idx])
    );
    let cycler = button_view::<Msg>(&etiqueta_back, pal_backend, Msg::CicloBackend);

    let etiqueta_diff = if model.diff_visible {
        "↔  diff: ON  (Ctrl+D)"
    } else {
        "↔  diff: off  (Ctrl+D)"
    };
    let diff_btn = button_view::<Msg>(etiqueta_diff, pal_backend, Msg::DiffToggle);

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned("LLM".to_string(), 10.0, theme.fg_muted, Alignment::Start);

    let mk = |label: &str, m: Msg| button_view::<Msg>(label, pal, m);
    let botones: Vec<View<Msg>> = vec![
        mk("→  traducir qu", Msg::PedirTraducir("qu".into())),
        mk("→  traducir en", Msg::PedirTraducir("en".into())),
        mk("✎  tono formal", Msg::PedirTono("formal".into())),
        mk("✂  resumir 30p", Msg::PedirResumir(Some(30))),
    ];

    // Lista de hijas del cuerpo activo — para abrirlas con click.
    let hijas_seccion = seccion_hijas(model, theme);

    let mut hijos: Vec<View<Msg>> = Vec::new();
    hijos.push(header);
    hijos.push(cycler);
    hijos.push(diff_btn);
    hijos.extend(botones);
    hijos.push(divider(theme));
    hijos.push(hijas_seccion);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .clip(true)
    .children(hijos)
}

fn seccion_hijas(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_list = ListPalette::from_theme(theme);
    let activo = model.activo;

    let hijas: Vec<&Cuerpo> = model
        .cuerpos
        .iter()
        .filter(|c| {
            c.metadatos.intencion.es_derivada() && c.metadatos.derivado_de == activo
        })
        .collect();

    let mut rows: Vec<ListRow<Msg>> = Vec::new();
    for h in &hijas {
        let label = format!("• {} · {}", h.branch_id, etiqueta_intencion(&h.metadatos.intencion));
        rows.push(ListRow {
            label,
            selected: false,
            on_click: Msg::AbrirDoc(h.id),
        });
    }

    let n = rows.len();
    let lista = list_view(ListSpec {
        rows,
        total: n,
        caption: Some(format!("hijas: {n}")),
        truncated_hint: None,
        row_height: 20.0,
        palette: palette_list,
    });

    let historial = seccion_historial(model, theme);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .children(vec![lista, divider(theme), historial])
}

fn seccion_historial(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_list = ListPalette::from_theme(theme);

    // Index para resolver Uuid → Cuerpo, cuerpo.metadatos.nombre_legible.
    let cuerpo_de = |id: Uuid| model.cuerpos.iter().find(|c| c.id == id);

    // Transformaciones del cuerpo activo: ya sea como madre o como hija.
    // Lo más útil al usuario suele ser "todo lo que pasó alrededor de
    // este doc" — así una hija de cuya madre vengo, lo veo.
    let activo = model.activo;
    let mut filtradas: Vec<&Transformacion> = model
        .transformaciones
        .iter()
        .filter(|t| match activo {
            Some(id) => t.madre == id || t.hija == id,
            None => true,
        })
        .collect();
    // Más recientes arriba.
    filtradas.sort_by(|a, b| b.creada_en.cmp(&a.creada_en));

    let mut rows: Vec<ListRow<Msg>> = Vec::new();
    for t in &filtradas {
        let madre = cuerpo_de(t.madre)
            .map(|c| c.metadatos.nombre_legible.as_str())
            .unwrap_or("?");
        let hija = cuerpo_de(t.hija)
            .map(|c| c.metadatos.nombre_legible.as_str())
            .unwrap_or("?");
        let tipo = etiqueta_tipo(&t.tipo);
        // Truncar nombres largos para que la fila no se rompa visual.
        let label = format!(
            "{}  →  {}  ·  {}",
            recortar(madre, 18),
            recortar(hija, 18),
            tipo,
        );
        rows.push(ListRow {
            label,
            selected: false,
            on_click: Msg::AbrirDoc(t.hija),
        });
    }

    let n = rows.len();
    let lista = list_view(ListSpec {
        rows,
        total: n,
        caption: Some(if activo.is_some() {
            format!("historial activo: {n}")
        } else {
            format!("historial: {n}")
        }),
        truncated_hint: None,
        row_height: 20.0,
        palette: palette_list,
    });

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![lista])
}

fn etiqueta_tipo(t: &TipoTransformacion) -> String {
    match t {
        TipoTransformacion::Identidad => "identidad".into(),
        TipoTransformacion::Traducir { lengua_destino } => format!("traducir → {lengua_destino}"),
        TipoTransformacion::Tono { etiqueta } => format!("tono {etiqueta}"),
        TipoTransformacion::Resumir {
            palabras_objetivo: Some(n),
        } => format!("resumir ≈{n}p"),
        TipoTransformacion::Resumir {
            palabras_objetivo: None,
        } => "resumir".into(),
        TipoTransformacion::Reescribir { .. } => "reescribir".into(),
        TipoTransformacion::Custom { kind, .. } => kind.clone(),
    }
}

fn recortar(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut o: String = s.chars().take(max.saturating_sub(1)).collect();
        o.push('…');
        o
    }
}

fn divider(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.border)
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn etiqueta_backend(k: BackendKind) -> &'static str {
    match k {
        BackendKind::Mock => "mock",
        BackendKind::Gemini => "gemini",
        BackendKind::Anthropic => "anthropic",
        BackendKind::DeepSeek => "deepseek",
        BackendKind::Cohere => "cohere",
        BackendKind::Ollama => "ollama",
    }
}

fn etiqueta_intencion(i: &Intencion) -> String {
    match i {
        Intencion::Original => "original".into(),
        Intencion::Traduccion => "traducción".into(),
        Intencion::Tono { etiqueta } => format!("tono {etiqueta}"),
        Intencion::Resumen {
            palabras_objetivo: Some(n),
        } => format!("resumen ≈{n}p"),
        Intencion::Resumen {
            palabras_objetivo: None,
        } => "resumen".into(),
        Intencion::Reescritura { .. } => "reescritura".into(),
        Intencion::Anotacion => "anotación".into(),
        Intencion::Custom { kind } => kind.clone(),
    }
}

fn ruta_sled() -> PathBuf {
    if let Ok(p) = std::env::var("PLUMA_APP_SLED") {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".cache"))
                .unwrap_or_else(|_| PathBuf::from(".cache"))
        });
    base.join("gioser").join("pluma-app").join("pluma.sled")
}

fn ahora_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// Clipboard backend — arboard puente al portapapeles del sistema
// ---------------------------------------------------------------------

/// Wrapper sobre `arboard::Clipboard`. Si el sistema no expone uno
/// (headless CI, sin Wayland/X), `inner` queda en `None` y los métodos
/// son no-op silenciosos — exactamente la semántica documentada del
/// trait [`Clipboard`].
struct ArboardClipboard {
    inner: Option<arboard::Clipboard>,
}

impl ArboardClipboard {
    fn new() -> Self {
        Self {
            inner: arboard::Clipboard::new().ok(),
        }
    }
}

impl Clipboard for ArboardClipboard {
    fn get(&mut self) -> Option<String> {
        self.inner.as_mut()?.get_text().ok()
    }
    fn set(&mut self, s: &str) {
        if let Some(c) = self.inner.as_mut() {
            let _ = c.set_text(s.to_owned());
        }
    }
}
