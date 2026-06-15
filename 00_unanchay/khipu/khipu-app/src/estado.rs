//! `estado` — persistencia, embeddings y helpers de modelo.
//!
//! Cubre:
//! - Formatos de serialización (`PersistedState*`) y su migración.
//! - `load_state` / `persist`: I/O de disco con postcard.
//! - `from_state` / `seeded_model`: inicialización del `Model`.
//! - `embed` / `embed_now` / `schedule_embedding`: vectores semánticos.
//! - Helpers de modelo: `select`, `deselect`, `commit_edits`,
//!   `reinforce_and_touch`, `first_visible`, `now_secs`, `current_mass`,
//!   `parse_tags`, `unlock_identity`, `start_unlock`, `khipu_dir`, `data_file_path`.

use std::path::PathBuf;

use agora_core::Keypair;
use directories::ProjectDirs;
use khipu_core::{Note, NoteId};
use khipu_gravity::Gravity;
use llimphi_widget_text_editor::EditorState;
use llimphi_widget_text_input::TextInputState;
use serde::{Deserialize, Serialize};

use crate::modelo::{Embedder, Focus, Model, Msg, Region};
use crate::map::place_note;
use llimphi_ui::Handle;

// =====================================================================
// Formatos de persistencia
// =====================================================================

#[derive(Serialize, Deserialize)]
pub(crate) struct PersistedState {
    pub(crate) store: khipu_core::NoteStore,
    pub(crate) embeddings: Vec<(NoteId, Vec<f32>)>,
    pub(crate) order: Vec<NoteId>,
    /// Etiqueta del espacio vectorial con que se guardaron los embeddings.
    pub(crate) model: String,
    /// Topónimos bautizados.
    #[serde(default)]
    pub(crate) regions: Vec<Region>,
}

/// Formato previo a las regiones (postcard no es self-describing, así que
/// un campo trailing rompe el parseo y hay que intentar la forma vieja).
#[derive(Deserialize)]
struct PersistedStateV2 {
    store: khipu_core::NoteStore,
    embeddings: Vec<(NoteId, Vec<f32>)>,
    order: Vec<NoteId>,
    model: String,
}

/// Formato histórico, sin `model`. Fallback cuando ni el actual ni el V2
/// parsean (archivos escritos antes de enchufar `verbo`).
#[derive(Deserialize)]
struct PersistedStateV1 {
    store: khipu_core::NoteStore,
    embeddings: Vec<(NoteId, Vec<f32>)>,
    order: Vec<NoteId>,
}

// =====================================================================
// I/O de disco
// =====================================================================

/// Directorio de datos de khipu (`$XDG_DATA_HOME/khipu/`), creándolo si
/// hace falta. Raíz de `notes.bin`, `identidad.seed` y `compartido.khipu`.
pub(crate) fn khipu_dir() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("org", "tawasuyu", "khipu")?;
    let dir = dirs.data_dir().to_path_buf();
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

pub(crate) fn data_file_path() -> Option<PathBuf> {
    Some(khipu_dir()?.join("notes.bin"))
}

pub(crate) fn load_state(path: &PathBuf) -> Option<PersistedState> {
    let bytes = std::fs::read(path).ok()?;
    // Formato actual primero; si no parsea (payload viejo sin `model`)
    // caemos al V1 y lo migramos con `model` vacío → fuerza recálculo.
    if let Ok(state) = postcard::from_bytes::<PersistedState>(&bytes) {
        return Some(state);
    }
    // Archivo previo a las regiones: misma forma sin el campo trailing.
    if let Ok(v2) = postcard::from_bytes::<PersistedStateV2>(&bytes) {
        return Some(PersistedState {
            store: v2.store,
            embeddings: v2.embeddings,
            order: v2.order,
            model: v2.model,
            regions: Vec::new(),
        });
    }
    let v1: PersistedStateV1 = postcard::from_bytes(&bytes).ok()?;
    Some(PersistedState {
        store: v1.store,
        embeddings: v1.embeddings,
        order: v1.order,
        model: String::new(),
        regions: Vec::new(),
    })
}

pub(crate) fn persist(model: &Model) {
    let Some(path) = model.data_path.as_ref() else {
        return;
    };
    let state = PersistedState {
        store: model.store.clone(),
        embeddings: model
            .field
            .iter()
            .map(|(id, v)| (id, v.to_vec()))
            .collect(),
        order: model.order.clone(),
        model: model.embedder.label(),
        regions: model.regions.clone(),
    };
    if let Ok(bytes) = postcard::to_allocvec(&state) {
        let tmp = path.with_extension("bin.tmp");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

// =====================================================================
// Inicialización del Model
// =====================================================================

pub(crate) fn from_state(state: PersistedState, embedder: Embedder) -> Model {
    // ¿Los vectores guardados son del mismo espacio que el embebedor
    // activo? Si cambió el modelo o la dimensión, son incomparables:
    // se descartan y se recalcula todo el cuaderno.
    let same_space = !state.model.is_empty() && state.model == embedder.label();
    let regions = state.regions;
    let mut model = Model::blank(embedder);
    model.store = state.store;
    model.order = state.order;
    model.regions = regions;

    if same_space {
        let restored: std::collections::HashSet<NoteId> =
            state.embeddings.iter().map(|(id, _)| *id).collect();
        for (id, v) in &state.embeddings {
            if !v.is_empty() {
                model.field.insert(*id, v.clone());
            }
        }
        // Notas sin vector persistido: recalcular sólo esas.
        let missing: Vec<NoteId> = model
            .order
            .iter()
            .copied()
            .filter(|id| !restored.contains(id))
            .collect();
        for id in missing {
            embed_now(&mut model, id);
            place_note(&mut model, id);
        }
    } else {
        let ids: Vec<NoteId> = model.order.clone();
        for id in ids {
            embed_now(&mut model, id);
        }
    }
    // Notas cargadas de disco sin posición reciben domicilio ahora.
    let unplaced: Vec<NoteId> = model
        .order
        .iter()
        .copied()
        .filter(|id| model.store.get(*id).map(|n| n.pos.is_none()).unwrap_or(false))
        .collect();
    for id in unplaced {
        place_note(&mut model, id);
    }
    model
}

pub(crate) fn seeded_model(embedder: Embedder) -> Model {
    let mut model = Model::blank(embedder);
    let now = now_secs();
    let seed: [(&str, &str, &[&str]); 7] = [
        (
            "Índice",
            "mi cuaderno: [[Recetas de la abuela]], [[Jardín]] y [[Oficina]]",
            &["meta"],
        ),
        (
            "Recetas de la abuela",
            "sopa de auyama; ver también [[Lista del mercado]]",
            &["cocina"],
        ),
        (
            "Lista del mercado",
            "auyama, cilantro, pan; vuelve al [[Índice]]",
            &["cocina"],
        ),
        (
            "Jardín",
            "riego semanal; las [[Semillas de cilantro]] van en marzo",
            &["jardín"],
        ),
        (
            "Semillas de cilantro",
            "germinan en diez días",
            &["jardín"],
        ),
        (
            "Oficina",
            "[[Reunión del lunes]] y pendientes varios",
            &["trabajo"],
        ),
        (
            "Diario sin enlaces",
            "una nota suelta, no la enlaza nadie",
            &["personal"],
        ),
    ];
    for (title, body, tags) in seed {
        let tags: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
        let id = model.store.create(title, body, tags, now);
        model.order.push(id);
        embed_now(&mut model, id);
        place_note(&mut model, id);
    }
    model
}

// =====================================================================
// Embeddings
// =====================================================================

// El embedder local de fallback (hash trigram → vector) vive en el core
// `khipu-gravity` como `local_embed`, junto a los vectores que alimenta (Regla
// 2). `modelo::Embedder::Local` lo llama directamente desde ahí.

/// Versión síncrona para el arranque (seed y migración de formato):
/// calcula el vector en línea y lo inserta. En init todavía no hay nada
/// que repintar, así que bloquear un instante es lo correcto.
pub(crate) fn embed_now(model: &mut Model, id: NoteId) {
    let Some(note) = model.store.get(id) else {
        return;
    };
    let combined = format!("{} {}", note.title, note.body);
    let v = model.embedder.embed_blocking(&combined);
    model.field.insert(id, v);
}

/// Pide el embedding de `id` en segundo plano. Asigna una secuencia
/// nueva, la marca como vigente, y dispara un worker (`Handle::spawn`)
/// que al terminar reentra al `update` con [`Msg::EmbeddingReady`]. Así
/// el `block_on` del arm remoto nunca corre en el hilo de UI.
pub(crate) fn schedule_embedding(model: &mut Model, id: NoteId, h: &Handle<Msg>) {
    let Some(note) = model.store.get(id) else {
        return;
    };
    let combined = format!("{} {}", note.title, note.body);
    model.embed_seq += 1;
    let seq = model.embed_seq;
    model.embed_latest.insert(id, seq);
    let embedder = model.embedder.clone();
    h.spawn(move || {
        let v = embedder.embed_blocking(&combined);
        Msg::EmbeddingReady(id, seq, v)
    });
}

// =====================================================================
// Helpers de modelo
// =====================================================================

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// La masa "vivida" de una nota en `now`: la guardada decae contra el
/// tiempo transcurrido desde `last_access`. Notas con `last_access == 0`
/// (payloads viejos) toman su `mass` tal cual.
pub(crate) fn current_mass(gravity: &Gravity, n: &Note, now: u64) -> f32 {
    if n.last_access == 0 {
        return n.mass;
    }
    let dt = if now > n.last_access {
        (now - n.last_access) as f32
    } else {
        0.0
    };
    gravity.decay(n.mass, dt)
}

/// Refuerza la masa de `id` y marca `last_access`. El gesto canónico
/// cuando el usuario selecciona o abre una nota.
pub(crate) fn reinforce_and_touch(model: &mut Model, id: NoteId) {
    let now = now_secs();
    let Some(n) = model.store.get(id) else {
        return;
    };
    let lived = current_mass(&model.gravity, n, now);
    let reinforced = model.gravity.reinforce(lived);
    model.store.set_mass(id, reinforced);
    model.store.touch(id, now);
}

/// Primera nota sobre el horizonte, ordenada por masa "viva".
pub(crate) fn first_visible(model: &Model) -> Option<NoteId> {
    let now = now_secs();
    let mut visible: Vec<(NoteId, f32)> = model
        .order
        .iter()
        .filter_map(|id| {
            model.store.get(*id).and_then(|n| {
                let m = current_mass(&model.gravity, n, now);
                model.gravity.is_visible(m).then_some((*id, m))
            })
        })
        .collect();
    visible.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(core::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    visible.first().map(|(id, _)| *id)
}

pub(crate) fn parse_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Carga la nota `id` en los campos del editor y foca el cuerpo.
pub(crate) fn select(model: &mut Model, id: NoteId) {
    let Some(note) = model.store.get(id) else {
        return;
    };
    model.selected = Some(id);
    model.title.set_text(note.title.clone());
    model.body = EditorState::default();
    model.body.set_text(&note.body);
    model.tags.set_text(note.tags.join(", "));
    model.focus = Focus::Body;
}

/// Suelta la nota seleccionada y limpia los campos del editor.
pub(crate) fn deselect(model: &mut Model) {
    model.selected = None;
    model.title.set_text(String::new());
    model.body = EditorState::default();
    model.tags.set_text(String::new());
    model.focus = Focus::None;
}

/// Sincroniza inputs/editor → store/field + persiste si cambió algo.
pub(crate) fn commit_edits(model: &mut Model, h: &Handle<Msg>) {
    let Some(id) = model.selected else {
        return;
    };
    let mut changed = false;
    let new_title = model.title.text();
    let new_body = model.body.text();
    let new_tags = parse_tags(&model.tags.text());
    let now = now_secs();
    if let Some(note) = model.store.get_mut(id) {
        if note.title != new_title {
            note.title = new_title;
            note.updated_at = now;
            changed = true;
        }
        if note.body != new_body {
            note.body = new_body;
            note.updated_at = now;
            changed = true;
        }
        if note.tags != new_tags {
            note.tags = new_tags;
            note.updated_at = now;
            changed = true;
        }
    }
    if changed {
        // El texto ya está en el store: persistimos de inmediato para no
        // perderlo. El embedding viaja a un worker y persistirá de nuevo
        // cuando llegue (`Msg::EmbeddingReady`).
        persist(model);
        schedule_embedding(model, id, h);
    }
}

/// Desbloquea (o crea, o migra) la identidad del cuaderno con `passphrase`,
/// vía [`khipu_share::identity::unlock`]. La semilla vive cifrada en
/// `<datos>/keys/`; si existe un `identidad.seed` en claro de versiones
/// viejas, se migra al keystore y se borra el claro.
pub(crate) fn unlock_identity(passphrase: &str) -> Option<Keypair> {
    let dir = khipu_dir()?;
    let legacy = dir.join("identidad.seed");
    khipu_share::identity::unlock(&dir.join("keys"), Some(&legacy), passphrase).ok()
}

/// Arranca el prompt de passphrase y memoriza la acción a reanudar.
pub(crate) fn start_unlock(model: &mut Model, accion: Msg) {
    model.unlocking = true;
    model.pending = Some(Box::new(accion));
    model.focus = Focus::Passphrase;
    model.passphrase.clear();
    model.status = Some("ingresá tu passphrase para desbloquear la identidad".into());
}
