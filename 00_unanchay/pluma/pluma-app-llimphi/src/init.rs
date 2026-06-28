//! Inicialización del modelo: reapertura de los proyectos `.pluma` (la única
//! persistencia), sembrado del primer documento si se arranca en blanco, y
//! autodetección del backend LLM.

use std::collections::HashMap;
use std::sync::Arc;

use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use pluma_editor_llimphi::cuerpo_ide::CuerpoIde;
use pluma_llm::{build_client, BackendKind, LlmConfig};
use pluma_llm_core::ChatClient;
use pluma_transform::Transformacion;
use llimphi_widget_text_input::TextInputState;
use uuid::Uuid;

use crate::clipboard::ArboardClipboard;
use crate::model::{Model, BACKENDS};
use crate::util::ahora_unix;

pub fn init_modelo() -> Model {
    // El estado vive en los proyectos `.pluma`; el editor arranca en blanco y
    // se llena con el documento activo del proyecto reabierto (más abajo).
    let mut atoms: HashMap<Uuid, NarrativeAtom> = HashMap::new();
    let mut cuerpos: Vec<Cuerpo> = Vec::new();
    let transformaciones: Vec<Transformacion> = Vec::new();
    let cartas: Vec<CartaHebras> = Vec::new();
    let estilos: HashMap<Uuid, pluma_estilo::EstiloLienzo> = HashMap::new();

    // Si no hay nada (ningún proyecto con contenido), sembrar un documento
    // Original para que la ventana no esté muerta al primer arranque.
    if cuerpos.is_empty() {
        let ahora = ahora_unix();
        let atom = NarrativeAtom::new("Empieza a escribir aquí…", "es");
        let mut cuerpo = Cuerpo::nuevo("es", "documento sin título", Intencion::Original, ahora);
        cuerpo.agregar(atom.id, ahora);
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

    // Reabre los proyectos que estaban abiertos (rutas .pluma persistidas). Si
    // ninguno abre, arranca con un proyecto vacío "Proyecto 1".
    let mut proyectos: Vec<crate::model::ProyectoAbierto> = Vec::new();
    for ruta in crate::util::cargar_abiertos() {
        if let Ok(p) = pluma_proyecto::Proyecto::abrir(&ruta) {
            let doc_activo = p
                .documentos()
                .first()
                .map(|(id, _)| *id)
                .unwrap_or_else(Uuid::new_v4);
            proyectos.push(crate::model::ProyectoAbierto {
                proyecto: p,
                ruta: Some(ruta),
                doc_activo,
            });
        }
    }
    if proyectos.is_empty() {
        proyectos.push(crate::model::ProyectoAbierto::vacio("Proyecto 1"));
    }

    let mut m = Model {
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
        estilo_expand: None,
        wizard: None,
        proyectos,
        proyecto_activo: 0,
        proyecto_tab: crate::model::ProyectoTab::Historia,
        commit_preview: None,
        push_abierto: false,
        renombrar: None,
        proyectos_recientes: crate::util::cargar_recientes(),
        toasts: Vec::new(),
        next_toast: 0,
        cotejo: None,
        cotejo_dialog: None,
    };

    // Si reabrimos un proyecto con contenido, su documento activo manda sobre el
    // estado sembrado desde el sled.
    crate::update::cargar_doc_activo_inicial(&mut m);
    m
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
