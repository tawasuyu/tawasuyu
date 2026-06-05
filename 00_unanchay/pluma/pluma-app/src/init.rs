//! Inicialización del modelo: apertura del sled, carga de cuerpos/atoms/
//! cartas/transformaciones, sembrado del primer documento y autodetección
//! del backend LLM.

use std::collections::HashMap;
use std::sync::Arc;

use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::cuerpo_ide::CuerpoIde;
use pluma_llm::{build_client, BackendKind, LlmConfig};
use pluma_llm_core::ChatClient;
use pluma_store::PlumaStore;
use pluma_transform::Transformacion;
use llimphi_widget_text_input::TextInputState;
use uuid::Uuid;

use crate::clipboard::ArboardClipboard;
use crate::model::{Model, BACKENDS};
use crate::util::{ahora_unix, ruta_sled};

pub(crate) fn init_modelo() -> Model {
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

    // La selección arranca con el activo solo; el usuario suma lienzos
    // desde el diente Lienzos para armar el multilienzo.
    let seleccionados: Vec<Uuid> = activo.into_iter().collect();

    Model {
        store,
        cuerpos,
        atoms,
        cartas,
        transformaciones,
        activo,
        ide,
        seleccionados,
        ides_ro: HashMap::new(),
        solo_activo: false,
        scroll_x: 0.0,
        diente_activo: 1, // arranca en Lienzos (el tree)
        panel_w: 280.0,
        clipboard: ArboardClipboard::new(),
        drag_accum: (0.0, 0.0),
        preset_input: TextInputState::new(),
        preset_focused: false,
        presets: crate::util::cargar_presets(),
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
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: llimphi_motion::Tween::idle(1.0),
        edit_menu: None,
        edit_active: usize::MAX,
        edit_anim: llimphi_motion::Tween::idle(1.0),
        // Rail hospedado: opt-in por env; el HostClient se conecta en `init`.
        delegated: std::env::var_os("PLUMA_DELEGATE_SIDEBAR").is_some(),
        _host: None,
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
