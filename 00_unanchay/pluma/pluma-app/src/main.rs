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
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_text_editor::{
    EditorMetrics, EditorPalette as TEPalette, Language, MemClipboard, PointerEvent,
};
use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_cuerpo::CambioAtom;
use pluma_editor_llimphi::cuerpo_ide::{cuerpo_ide_view, CuerpoIde};
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
    clipboard: MemClipboard,
    drag_accum: (f32, f32),

    chat: Arc<dyn ChatClient>,
    backend_idx: usize,
    en_curso: bool,
    ultimo_error: Option<String>,
    ultimo_status: String,

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
                if s.eq_ignore_ascii_case("n") {
                    return Some(Msg::NuevoDoc);
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
        clipboard: MemClipboard::default(),
        drag_accum: (0.0, 0.0),
        chat,
        backend_idx,
        en_curso: false,
        ultimo_error: None,
        ultimo_status: "listo".to_string(),
        side_izq_w: 240.0,
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
    .children(vec![header, boton_nuevo, boton_guardar, lista])
}

fn panel_editor(model: &Model, palette_editor: &TEPalette) -> View<Msg> {
    let editor = cuerpo_ide_view::<Msg>(
        &model.ide,
        palette_editor,
        METRICS,
        VISIBLE_LINES,
        Language::Plain,
        |ev| Some(Msg::EditorPointer(ev)),
    );

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
        ..Default::default()
    })
    .fill(palette_editor.bg)
    .clip(true)
    .children(vec![editor])
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

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![lista])
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
