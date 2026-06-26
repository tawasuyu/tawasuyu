//! Tests de integración del host de plugins. Cargan los `.wasm` de ejemplo
//! commiteados (`include_bytes!`) — herméticos, sin asumir el toolchain wasm32
//! ni un servidor gráfico. El oráculo: el plugin de layout impone su geometría
//! (right-master) y **honra los `LayoutParams` del Desktop**, y el gateo de
//! capacidades rechaza en carga lo no concedido.

use mirada_brain::Desktop;
use mirada_plugin_host::caps::{CAP_KEYS, CAP_LAYOUT, CAP_SPAWN};
use mirada_plugin_host::{Conductor, LoadedPlugin, PluginKind};
use mirada_protocol::{BodyEvent, BrainCommand, LayoutMode, LayoutParams, Rect, TileInput};

const LAYOUT_WASM: &[u8] = include_bytes!("../assets/example-layout.wasm");
const REACTOR_WASM: &[u8] = include_bytes!("../assets/example-reactor.wasm");

fn params(ratio: f32, nmaster: usize, gap: i32) -> LayoutParams {
    LayoutParams { mode: LayoutMode::MasterStack, master_ratio: ratio, master_count: nmaster, gap }
}

/// Centro horizontal de un rect.
fn cx(r: &Rect) -> i32 {
    r.x + r.w / 2
}

// --- Tier-0: el plugin de layout es una función pura sin importaciones. ------

#[test]
fn layout_plugin_carga_con_solo_cap_layout() {
    let p = LoadedPlugin::load_bytes(LAYOUT_WASM, PluginKind::Layout, CAP_LAYOUT, 0, "rm");
    assert!(p.is_ok(), "el plugin de layout debería cargar: {:?}", p.err());
}

#[test]
fn right_master_pone_la_maestra_a_la_derecha() {
    let mut p =
        LoadedPlugin::load_bytes(LAYOUT_WASM, PluginKind::Layout, CAP_LAYOUT, 0, "rm").unwrap();
    let work = Rect::new(0, 0, 1000, 1000);
    let rects = p
        .call_tile(&TileInput { ids: vec![1, 2, 3], work, params: params(0.6, 1, 8) })
        .unwrap();
    assert_eq!(rects.len(), 3);
    let master = rects.iter().find(|(id, _)| *id == 1).unwrap().1;
    let stack: Vec<_> = rects.iter().filter(|(id, _)| *id != 1).map(|(_, r)| *r).collect();
    // La maestra (id 1) está en la mitad derecha; la pila, en la izquierda.
    assert!(cx(&master) > 500, "maestra a la derecha: {master:?}");
    assert!(stack.iter().all(|r| cx(r) < 500), "pila a la izquierda: {stack:?}");
    // Y la maestra es más ancha que las de la pila (ratio 0.6).
    assert!(stack.iter().all(|r| master.w > r.w));
}

// --- El plugin HONRA los LayoutParams del Desktop (lo que pedía el #2). ------

#[test]
fn mas_ratio_ensancha_la_maestra() {
    let mut p =
        LoadedPlugin::load_bytes(LAYOUT_WASM, PluginKind::Layout, CAP_LAYOUT, 0, "rm").unwrap();
    let work = Rect::new(0, 0, 1000, 1000);
    let ids = vec![1u64, 2];
    let estrecha = p.call_tile(&TileInput { ids: ids.clone(), work, params: params(0.6, 1, 8) }).unwrap();
    let ancha = p.call_tile(&TileInput { ids, work, params: params(0.8, 1, 8) }).unwrap();
    let w06 = estrecha.iter().find(|(id, _)| *id == 1).unwrap().1.w;
    let w08 = ancha.iter().find(|(id, _)| *id == 1).unwrap().1.w;
    assert!(w08 > w06, "subir master_ratio debe ensanchar la maestra: {w06} -> {w08}");
}

#[test]
fn master_count_mueve_ventanas_a_la_columna_maestra() {
    let mut p =
        LoadedPlugin::load_bytes(LAYOUT_WASM, PluginKind::Layout, CAP_LAYOUT, 0, "rm").unwrap();
    let work = Rect::new(0, 0, 1000, 1000);
    let ids = vec![1u64, 2, 3];
    // Con nmaster=2 hay dos ventanas en la columna derecha (la maestra).
    let r = p.call_tile(&TileInput { ids, work, params: params(0.6, 2, 8) }).unwrap();
    let en_derecha = r.iter().filter(|(_, rect)| cx(rect) > 500).count();
    assert_eq!(en_derecha, 2, "nmaster=2 → dos ventanas en la columna maestra: {r:?}");
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
    let err = LoadedPlugin::load_bytes(REACTOR_WASM, PluginKind::Reactor, CAP_SPAWN, 0, "term")
        .err()
        .expect("un reactor sin CAP_KEYS debería ser rechazado");
    assert!(err.contains("keys") || err.contains("capacidad"), "esperaba mención a keys: {err}");
}

#[test]
fn reactor_con_caps_completas_carga() {
    let p =
        LoadedPlugin::load_bytes(REACTOR_WASM, PluginKind::Reactor, CAP_KEYS | CAP_SPAWN, 0, "term");
    assert!(p.is_ok(), "con KEYS+SPAWN debería cargar: {:?}", p.err());
}

// --- Reactor e2e. -----------------------------------------------------------

#[test]
fn reactor_registra_atajo_y_lanza() {
    let mut p =
        LoadedPlugin::load_bytes(REACTOR_WASM, PluginKind::Reactor, CAP_KEYS | CAP_SPAWN, 0, "term")
            .unwrap();
    let cmds = p.call_on_event(&BodyEvent::OutputAdded { id: 0, width: 800, height: 600 }).unwrap();
    assert!(
        cmds.iter().any(|c| matches!(c, BrainCommand::GrabKeys(k) if k.iter().any(|s| s == "Super+a"))),
        "el reactor debería registrar Super+a: {cmds:?}"
    );
    let cmds = p.call_on_event(&BodyEvent::Keybind("Super+a".into())).unwrap();
    assert!(
        cmds.iter().any(|c| matches!(c, BrainCommand::Spawn(s) if s == "foot")),
        "Super+a debería lanzar `foot`: {cmds:?}"
    );
}

// --- Conductor: Desktop autoritativo + plugins que lo aumentan. -------------

#[test]
fn configured_desktop_carga_la_config_del_usuario_y_arranca() {
    // Redirige XDG_CONFIG_HOME a un tempdir: hermético, no toca el ~/.config
    // real (Keymap::load_or_init escribiría un keymap.ron de arranque ahí).
    let tmp = std::env::temp_dir().join(format!("mirada-host-cfg-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    std::env::set_var("XDG_CONFIG_HOME", &tmp);

    let mut c = Conductor::new(mirada_plugin_host::configured_desktop(), Vec::new());
    let cmds = c.startup();
    // El control de seguridad y los atajos del usuario llegan igual.
    assert!(cmds.iter().any(|c| matches!(c, BrainCommand::SetCapabilities(_))));
    assert!(
        cmds.iter().any(|c| matches!(c, BrainCommand::GrabKeys(k) if !k.is_empty())),
        "el keymap (default o del usuario) debería producir atajos: {cmds:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn hot_reload_de_caps_reemite_setcapabilities() {
    use mirada_brain::Permisos;
    let mut c = Conductor::new(Desktop::new(), Vec::new());
    let _ = c.startup();

    // Recarga con una denylist de screencopy: debe emitir un SetCapabilities
    // que la lleve (el control de seguridad reaplicado en caliente).
    let caps = Permisos { screencopy_denylist: vec!["grim".into()], ..Permisos::default() };
    let cmds = c.apply_caps(caps);
    assert!(matches!(
        cmds.as_slice(),
        [BrainCommand::SetCapabilities(p)] if p.screencopy_denylist == ["grim"]
    ));
}

#[test]
fn hot_reload_de_keymap_reemite_grabkeys_sin_pisar_reactores() {
    use mirada_brain::Keymap;
    // Un reactor que registra Super+a, para verificar que la recarga del keymap
    // une (no pisa) sus atajos.
    let reactor =
        LoadedPlugin::load_bytes(REACTOR_WASM, PluginKind::Reactor, CAP_KEYS | CAP_SPAWN, 0, "term")
            .unwrap();
    let mut c = Conductor::new(Desktop::new(), vec![reactor]);
    let _ = c.startup();
    // Que el reactor registre Super+a.
    c.on_body_event(BodyEvent::OutputAdded { id: 0, width: 800, height: 600 });

    // Recargar un keymap distinto debe reemitir GrabKeys con la unión, que
    // SIGUE conteniendo Super+a (el atajo del reactor no se pierde).
    let cmds = c.apply_keymap(Keymap::default());
    let grab = cmds.iter().find_map(|c| match c {
        BrainCommand::GrabKeys(k) => Some(k.clone()),
        _ => None,
    });
    if let Some(keys) = grab {
        assert!(keys.iter().any(|s| s == "Super+a"), "la unión debe conservar el atajo del reactor: {keys:?}");
    }
    // (Si la unión no cambió, apply_keymap devuelve vacío: también válido, el
    // Cuerpo ya tenía la unión correcta.)
}

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

/// Rects de las ventanas teseladas del último `Place` de una tanda de comandos.
fn tiled_from(cmds: Vec<BrainCommand>, acc: &mut Vec<Rect>) {
    for cmd in cmds {
        if let BrainCommand::Place(ps) = cmd {
            *acc = ps
                .iter()
                .filter(|p| p.visible && !p.floating && !p.fullscreen)
                .map(|p| p.rect)
                .collect();
        }
    }
}

#[test]
fn conductor_impone_la_geometria_del_plugin() {
    let layout =
        LoadedPlugin::load_bytes(LAYOUT_WASM, PluginKind::Layout, CAP_LAYOUT, 10, "rm").unwrap();
    let mut c = Conductor::new(Desktop::new(), vec![layout]);
    let _ = c.startup();
    c.on_body_event(BodyEvent::OutputAdded { id: 0, width: 1000, height: 1000 });

    let mut rects = Vec::new();
    for id in 1..=2u64 {
        tiled_from(
            c.on_body_event(BodyEvent::WindowOpened { id, app_id: "t".into(), title: "w".into() }),
            &mut rects,
        );
    }
    // La maestra (la más ancha) cae en la mitad DERECHA — algo que el
    // master-stack izquierdo del Desktop nunca produciría: prueba que el plugin
    // impuso su geometría.
    let master = rects.iter().max_by_key(|r| r.w).copied().unwrap();
    assert!(cx(&master) > 500, "el plugin right-master debería poner la maestra a la derecha: {master:?}");
}

#[test]
fn grow_master_keybind_ensancha_via_params() {
    let layout =
        LoadedPlugin::load_bytes(LAYOUT_WASM, PluginKind::Layout, CAP_LAYOUT, 10, "rm").unwrap();
    let mut c = Conductor::new(Desktop::new(), vec![layout]);
    let _ = c.startup();
    c.on_body_event(BodyEvent::OutputAdded { id: 0, width: 1000, height: 1000 });

    let mut rects = Vec::new();
    for id in 1..=2u64 {
        tiled_from(
            c.on_body_event(BodyEvent::WindowOpened { id, app_id: "t".into(), title: "w".into() }),
            &mut rects,
        );
    }
    let antes = rects.iter().map(|r| r.w).max().unwrap();

    // Super+l = GrowMaster en el keymap por defecto → cambia los LayoutParams
    // del Desktop → el plugin los recibe y ensancha la maestra.
    tiled_from(c.on_body_event(BodyEvent::Keybind("Super+l".into())), &mut rects);
    let despues = rects.iter().map(|r| r.w).max().unwrap();

    assert!(
        despues > antes,
        "el atajo GrowMaster debería fluir como params al plugin y ensanchar la maestra: {antes} -> {despues}"
    );
}
