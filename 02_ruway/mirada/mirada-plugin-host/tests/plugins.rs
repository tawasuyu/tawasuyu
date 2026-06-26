//! Tests de integración del host de plugins. Cargan los `.wasm` de ejemplo
//! commiteados (`include_bytes!`) — herméticos, sin asumir el toolchain wasm32
//! ni un servidor gráfico. El oráculo: la geometría que el plugin de layout
//! produce es la partición dwindle, y el gateo de capacidades rechaza en carga
//! lo no concedido.

use mirada_brain::Desktop;
use mirada_plugin_host::caps::{CAP_KEYS, CAP_LAYOUT, CAP_SPAWN};
use mirada_plugin_host::{Conductor, LoadedPlugin, PluginKind};
use mirada_protocol::{BodyEvent, BrainCommand, Rect, TileInput};

const LAYOUT_WASM: &[u8] = include_bytes!("../assets/example-layout.wasm");
const REACTOR_WASM: &[u8] = include_bytes!("../assets/example-reactor.wasm");

// --- Tier-0: el plugin de layout es una función pura sin importaciones. ------

#[test]
fn layout_plugin_carga_con_solo_cap_layout() {
    // No importa nada del host → CAP_LAYOUT basta y nada más hace falta.
    let p = LoadedPlugin::load_bytes(LAYOUT_WASM, PluginKind::Layout, CAP_LAYOUT, 0, "dwindle");
    assert!(p.is_ok(), "el plugin de layout debería cargar: {:?}", p.err());
}

#[test]
fn dwindle_particiona_el_area() {
    let mut p =
        LoadedPlugin::load_bytes(LAYOUT_WASM, PluginKind::Layout, CAP_LAYOUT, 0, "dwindle").unwrap();
    let rects = p
        .call_tile(&TileInput { ids: vec![1, 2, 3], work: Rect::new(0, 0, 1000, 1000) })
        .unwrap();
    // dwindle: 1 → mitad izquierda; 2 → arriba-derecha; 3 → abajo-derecha.
    assert_eq!(rects.len(), 3);
    assert_eq!(rects[0], (1, Rect::new(0, 0, 500, 1000)));
    assert_eq!(rects[1], (2, Rect::new(500, 0, 500, 500)));
    assert_eq!(rects[2], (3, Rect::new(500, 500, 500, 500)));
    // Cobertura total y sin solape (área = suma de áreas).
    let suma: i32 = rects.iter().map(|(_, r)| r.w * r.h).sum();
    assert_eq!(suma, 1000 * 1000);
}

// --- Gateo de capacidades: fail-closed en carga. ----------------------------

#[test]
fn reactor_sin_capacidades_es_rechazado() {
    let err = LoadedPlugin::load_bytes(REACTOR_WASM, PluginKind::Reactor, 0, 0, "term")
        .err()
        .expect("un reactor sin capacidades debería ser rechazado");
    assert!(
        err.contains("capacidad") || err.contains("import"),
        "el rechazo debería nombrar la capacidad faltante: {err}"
    );
}

#[test]
fn reactor_con_cap_parcial_es_rechazado() {
    // Concede SPAWN pero no KEYS: como importa host_emit_keys, ni instancia.
    let err = LoadedPlugin::load_bytes(REACTOR_WASM, PluginKind::Reactor, CAP_SPAWN, 0, "term")
        .err()
        .expect("un reactor sin CAP_KEYS debería ser rechazado");
    assert!(err.contains("keys") || err.contains("capacidad"), "esperaba mención a keys: {err}");
}

#[test]
fn reactor_con_caps_completas_carga() {
    let p = LoadedPlugin::load_bytes(
        REACTOR_WASM,
        PluginKind::Reactor,
        CAP_KEYS | CAP_SPAWN,
        0,
        "term",
    );
    assert!(p.is_ok(), "con KEYS+SPAWN debería cargar: {:?}", p.err());
}

// --- Reactor e2e: registra atajo y lanza al pulsarlo. -----------------------

#[test]
fn reactor_registra_atajo_y_lanza() {
    let mut p =
        LoadedPlugin::load_bytes(REACTOR_WASM, PluginKind::Reactor, CAP_KEYS | CAP_SPAWN, 0, "term")
            .unwrap();

    // Cualquier evento dispara el registro idempotente del atajo.
    let cmds = p.call_on_event(&BodyEvent::OutputAdded { id: 0, width: 800, height: 600 }).unwrap();
    assert!(
        cmds.iter().any(|c| matches!(c, BrainCommand::GrabKeys(k) if k.iter().any(|s| s == "Super+a"))),
        "el reactor debería registrar Super+a: {cmds:?}"
    );

    // Pulsar el atajo lanza la terminal.
    let cmds = p.call_on_event(&BodyEvent::Keybind("Super+a".into())).unwrap();
    assert!(
        cmds.iter().any(|c| matches!(c, BrainCommand::Spawn(s) if s == "foot")),
        "Super+a debería lanzar `foot`: {cmds:?}"
    );
}

// --- Conductor: Desktop autoritativo + layout que refina + reactor. ---------

#[test]
fn conductor_startup_no_olvida_capacidades_ni_decoracion() {
    let mut c = Conductor::new(Desktop::new(), Vec::new());
    let cmds = c.startup();
    assert!(
        cmds.iter().any(|c| matches!(c, BrainCommand::SetCapabilities(_))),
        "startup debe empujar SetCapabilities (control de seguridad)"
    );
    assert!(cmds.iter().any(|c| matches!(c, BrainCommand::SetDecorations(_))));
    assert!(cmds.iter().any(|c| matches!(c, BrainCommand::GrabKeys(_))));
}

#[test]
fn conductor_aplica_dwindle_a_las_ventanas() {
    let layout =
        LoadedPlugin::load_bytes(LAYOUT_WASM, PluginKind::Layout, CAP_LAYOUT, 10, "dwindle").unwrap();
    let mut c = Conductor::new(Desktop::new(), vec![layout]);
    let _ = c.startup();

    c.on_body_event(BodyEvent::OutputAdded { id: 0, width: 1000, height: 1000 });
    let mut last_place: Vec<Rect> = Vec::new();
    for id in 1..=3u64 {
        let cmds = c.on_body_event(BodyEvent::WindowOpened {
            id,
            app_id: "test".into(),
            title: "w".into(),
        });
        for cmd in cmds {
            if let BrainCommand::Place(ps) = cmd {
                last_place = ps
                    .iter()
                    .filter(|p| p.visible && !p.floating && !p.fullscreen)
                    .map(|p| p.rect)
                    .collect();
            }
        }
    }

    // Las tres ventanas teseladas deben caer en la partición dwindle del área.
    last_place.sort_by_key(|r| (r.x, r.y));
    let mut esperado = vec![
        Rect::new(0, 0, 500, 1000),
        Rect::new(500, 0, 500, 500),
        Rect::new(500, 500, 500, 500),
    ];
    esperado.sort_by_key(|r| (r.x, r.y));
    assert_eq!(last_place, esperado, "el layout plugin debería haber impuesto dwindle");
}

#[test]
fn conductor_propaga_spawn_del_reactor() {
    let reactor = LoadedPlugin::load_bytes(
        REACTOR_WASM,
        PluginKind::Reactor,
        CAP_KEYS | CAP_SPAWN,
        0,
        "term",
    )
    .unwrap();
    let mut c = Conductor::new(Desktop::new(), vec![reactor]);
    let _ = c.startup();
    c.on_body_event(BodyEvent::OutputAdded { id: 0, width: 800, height: 600 });

    let cmds = c.on_body_event(BodyEvent::Keybind("Super+a".into()));
    assert!(
        cmds.iter().any(|c| matches!(c, BrainCommand::Spawn(s) if s == "foot")),
        "el conductor debe propagar el Spawn del reactor: {cmds:?}"
    );
}
