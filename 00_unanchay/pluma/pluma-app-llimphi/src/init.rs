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

pub fn init_modelo() -> Model {
    let path = ruta_sled();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let store = match PlumaStore::open(&path) {
        Ok(s) => Arc::new(s),
        Err(e) => panic!("pluma-app-llimphi :: PlumaStore::open({path:?}) falló: {e:?}"),
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
    let estilos: HashMap<Uuid, pluma_estilo::EstiloLienzo> =
        store.iter_estilos().filter_map(|r| r.ok()).collect();

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

    // Orden inicial del tree: cada original seguido de sus derivadas; las
    // huérfanas al final. El usuario lo reordena por drag.
    let mut orden_lienzos: Vec<Uuid> = Vec::with_capacity(cuerpos.len());
    for o in cuerpos.iter().filter(|c| !c.metadatos.intencion.es_derivada()) {
        orden_lienzos.push(o.id);
        for d in cuerpos
            .iter()
            .filter(|c| c.metadatos.intencion.es_derivada() && c.metadatos.derivado_de == Some(o.id))
        {
            orden_lienzos.push(d.id);
        }
    }
    for c in &cuerpos {
        if !orden_lienzos.contains(&c.id) {
            orden_lienzos.push(c.id);
        }
    }

    Model {
        store,
        cuerpos,
        atoms,
        cartas,
        transformaciones,
        activo,
        ide,
        // El editor ES el multilienzo: pluma abre directo en él (antes abría en
        // las cajas read-only de `Lienzos`, que parecían "otra app").
        modo: crate::model::Modo::Plano,
        editando: None,
        recorrido_state: pluma_deck_core::RecorridoState::new(),
        salidas: HashMap::new(),
        lienzos_scroll_y: 0.0,
        fase_flujo: 0.0,
        seleccionados,
        orden_lienzos,
        ides_ro: HashMap::new(),
        solo_activo: false,
        scroll_x: 0.0,
        viewport: (1600.0, 900.0),
        diente_activo: 1, // arranca en Lienzos (el tree)
        foco_por_hover: false,
        panel_w: 280.0,
        clipboard: ArboardClipboard::new(),
        drag_accum: (0.0, 0.0),
        preset_input: TextInputState::new(),
        preset_focused: false,
        presets: crate::util::cargar_presets(),
        grafo: Vec::new(),
        grafo_src: (20.0, 16.0),
        grafo_sink: (20.0, 86.0),
        grafo_input: TextInputState::new(),
        grafo_input_focused: false,
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
        // Rail hospedado: por default delega a pata cuando está corriendo
        // (opt-out con PLUMA_DELEGATE_SIDEBAR=0); el HostClient se conecta en `init`.
        delegated: pata_host::delegate_sidebar_default("PLUMA_DELEGATE_SIDEBAR"),
        _host: None,
        host_active_synced: None,
        estilos,
        diente_estilo_activo: None,
        panel_estilo_w: 280.0,
        objetivo_estilo: crate::model::ObjetivoEstilo::Lienzo,
        wizard: None,
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
