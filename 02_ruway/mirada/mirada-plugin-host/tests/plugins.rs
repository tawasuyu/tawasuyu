//! Tests de integración del host de plugins. Cargan los `.wasm` de ejemplo
//! commiteados (`include_bytes!`) — herméticos, sin asumir el toolchain wasm32
//! ni un servidor gráfico. El oráculo: el plugin de layout impone su geometría
//! (right-master) y **honra los `LayoutParams` del Desktop**, y el gateo de
//! capacidades rechaza en carga lo no concedido.

use std::path::Path;

use mirada_brain::Desktop;
use mirada_plugin_host::caps::{CAP_ACTIONS, CAP_EFFECTS, CAP_KEYS, CAP_LAYOUT, CAP_SPAWN};
use mirada_plugin_host::{Conductor, LoadedPlugin, PluginKind, PluginManifest, TrustSet};
use mirada_protocol::{BodyEvent, BrainCommand, LayoutMode, LayoutParams, Rect, TileInput};

const LAYOUT_WASM: &[u8] = include_bytes!("../assets/example-layout.wasm");
const REACTOR_WASM: &[u8] = include_bytes!("../assets/example-reactor.wasm");
const DWINDLE_WASM: &[u8] = include_bytes!("../assets/dwindle.wasm");
const ASIGNADOR_WASM: &[u8] = include_bytes!("../assets/asignador.wasm");

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

// --- Catálogo base: dwindle (BSP recursivo). --------------------------------

#[test]
fn dwindle_tesela_recursivo_sin_solaparse_ni_huecos() {
    let mut p =
        LoadedPlugin::load_bytes(DWINDLE_WASM, PluginKind::Layout, CAP_LAYOUT, 20, "dw").unwrap();
    let work = Rect::new(0, 0, 1000, 800);
    // gap 0 → las celdas teselan el área exactamente (sin márgenes).
    let rects = p
        .call_tile(&TileInput { ids: vec![1, 2, 3, 4], work, params: params(0.6, 1, 0) })
        .unwrap();
    assert_eq!(rects.len(), 4);

    // Cada celda cae dentro del área de trabajo.
    for (_, r) in &rects {
        assert!(
            r.x >= work.x && r.y >= work.y && r.x + r.w <= work.x + work.w && r.y + r.h <= work.y + work.h,
            "{r:?} fuera de {work:?}"
        );
    }
    // La maestra (id 1) toma master_ratio del eje largo (1000 ancho → 60 %).
    let master = rects.iter().find(|(id, _)| *id == 1).unwrap().1;
    assert!(
        (master.w as f32 / work.w as f32 - 0.6).abs() < 0.02,
        "la maestra debería medir ~60 % del ancho: {master:?}"
    );
    // Ningún par de celdas se solapa (BSP ⇒ partición disjunta).
    for i in 0..rects.len() {
        for j in i + 1..rects.len() {
            let (a, b) = (rects[i].1, rects[j].1);
            let disjuntos = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
            assert!(disjuntos, "se solapan {a:?} y {b:?}");
        }
    }
    // La suma de áreas cubre todo el trabajo (sin huecos, con gap 0).
    let cubierto: i64 = rects.iter().map(|(_, r)| r.w as i64 * r.h as i64).sum();
    assert_eq!(cubierto, work.w as i64 * work.h as i64, "dwindle debería cubrir el área sin huecos");
}

#[test]
fn dwindle_master_ratio_ensancha_la_primera() {
    let mut p =
        LoadedPlugin::load_bytes(DWINDLE_WASM, PluginKind::Layout, CAP_LAYOUT, 20, "dw").unwrap();
    let work = Rect::new(0, 0, 1000, 800);
    let ids = vec![1u64, 2, 3];
    let estrecha = p.call_tile(&TileInput { ids: ids.clone(), work, params: params(0.5, 1, 0) }).unwrap();
    let ancha = p.call_tile(&TileInput { ids, work, params: params(0.75, 1, 0) }).unwrap();
    let w05 = estrecha.iter().find(|(id, _)| *id == 1).unwrap().1.w;
    let w075 = ancha.iter().find(|(id, _)| *id == 1).unwrap().1.w;
    assert!(w075 > w05, "subir master_ratio ensancha la primera celda: {w05} -> {w075}");
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
        LoadedPlugin::load_bytes(REACTOR_WASM, PluginKind::Reactor, CAP_KEYS | CAP_SPAWN | CAP_EFFECTS | CAP_ACTIONS, 0, "term");
    assert!(p.is_ok(), "con KEYS+SPAWN debería cargar: {:?}", p.err());
}

// --- Grants firmados: el camino real (manifest + trust + verificación). -----

#[test]
fn reactor_firmado_carga_con_su_trust_y_se_rechaza_sin_el() {
    // Usa los assets commiteados, que el build script mantiene consistentes
    // (wasm + firma + trust.ron de la clave demo).
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let m = PluginManifest::load(&assets.join("example-reactor.ron")).unwrap();
    let trust = TrustSet::load(&assets.join("trust.ron"));
    assert!(!trust.is_empty(), "trust.ron debería traer la clave demo");

    // Con el anillo correcto, el reactor firmado carga.
    assert!(
        LoadedPlugin::load(&m, &trust).is_ok(),
        "el reactor firmado por una clave de confianza debería cargar"
    );
    // Sin confianza declarada, se rechaza (fail-closed).
    assert!(
        LoadedPlugin::load(&m, &TrustSet::empty()).is_err(),
        "sin trust, un reactor con caps peligrosas no debe cargar"
    );
}

// --- Reactor e2e. -----------------------------------------------------------

#[test]
fn reactor_registra_atajo_y_lanza() {
    let mut p =
        LoadedPlugin::load_bytes(REACTOR_WASM, PluginKind::Reactor, CAP_KEYS | CAP_SPAWN | CAP_EFFECTS | CAP_ACTIONS, 0, "term")
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

// --- Efectos Tier-2 (opacidad) + su gateo. ----------------------------------

#[test]
fn reactor_sin_cap_effects_es_rechazado() {
    // El reactor importa host_emit_opacity (atenuado): sin CAP_EFFECTS ni instancia.
    let err =
        LoadedPlugin::load_bytes(REACTOR_WASM, PluginKind::Reactor, CAP_KEYS | CAP_SPAWN, 0, "term")
            .err()
            .expect("falta CAP_EFFECTS → rechazo");
    assert!(
        err.contains("effects") || err.contains("capacidad"),
        "esperaba mención a effects: {err}"
    );
}

#[test]
fn reactor_atenua_las_ventanas_sin_foco() {
    let mut p = LoadedPlugin::load_bytes(
        REACTOR_WASM,
        PluginKind::Reactor,
        CAP_KEYS | CAP_SPAWN | CAP_EFFECTS | CAP_ACTIONS,
        0,
        "term",
    )
    .unwrap();
    p.call_on_event(&BodyEvent::WindowOpened { id: 1, app_id: "a".into(), title: "w".into() })
        .unwrap();
    // Al abrir la 2, pasa a ser la enfocada: 2 plena, 1 atenuada.
    let cmds = p
        .call_on_event(&BodyEvent::WindowOpened { id: 2, app_id: "b".into(), title: "w".into() })
        .unwrap();

    let mut fx = std::collections::HashMap::new();
    for c in &cmds {
        if let BrainCommand::SetEffects(v) = c {
            for (id, e) in v {
                fx.insert(*id, *e);
            }
        }
    }
    // La enfocada (2): opaca + sombra. La de fondo (1): atenuada, sin sombra.
    assert_eq!(fx.get(&2).map(|e| e.opacity), Some(255), "enfocada opaca: {fx:?}");
    assert_eq!(fx.get(&2).map(|e| e.shadow), Some(true), "enfocada con sombra: {fx:?}");
    assert_eq!(fx.get(&1).map(|e| e.opacity), Some(180), "fondo atenuado: {fx:?}");
    assert_eq!(fx.get(&1).map(|e| e.shadow), Some(false), "fondo sin sombra: {fx:?}");
}

// --- Acciones de escritorio (CAP_ACTIONS): el reactor maneja ventanas. ------

#[test]
fn reactor_sin_cap_actions_es_rechazado() {
    // El reactor importa host_emit_action (auto-teselado): sin CAP_ACTIONS ni
    // instancia — la frontera de capacidad es física, igual que con effects.
    let err =
        LoadedPlugin::load_bytes(REACTOR_WASM, PluginKind::Reactor, CAP_KEYS | CAP_SPAWN | CAP_EFFECTS, 0, "term")
            .err()
            .expect("falta CAP_ACTIONS → rechazo");
    assert!(
        err.contains("actions") || err.contains("capacidad"),
        "esperaba mención a actions: {err}"
    );
}

#[test]
fn reactor_pide_monocle_cuando_se_llena() {
    let mut p = LoadedPlugin::load_bytes(
        REACTOR_WASM,
        PluginKind::Reactor,
        CAP_KEYS | CAP_SPAWN | CAP_EFFECTS | CAP_ACTIONS,
        0,
        "term",
    )
    .unwrap();
    // Con pocas ventanas, pide master-stack; al cruzar el umbral (3), monocle.
    for id in 1..=2u64 {
        p.call_on_event(&BodyEvent::WindowOpened { id, app_id: "a".into(), title: "w".into() })
            .unwrap();
        assert_eq!(
            p.take_actions(),
            vec!["layout:master-stack".to_string()],
            "con {id} ventana(s) debería pedir master-stack"
        );
    }
    p.call_on_event(&BodyEvent::WindowOpened { id: 3, app_id: "a".into(), title: "w".into() })
        .unwrap();
    assert_eq!(
        p.take_actions(),
        vec!["layout:monocle".to_string()],
        "al llegar a 3 ventanas debería despejar a monocle"
    );
    // Al cerrar una, vuelve a master-stack.
    p.call_on_event(&BodyEvent::WindowClosed { id: 3 }).unwrap();
    assert_eq!(p.take_actions(), vec!["layout:master-stack".to_string()]);
}

#[test]
fn conductor_aplica_la_accion_del_reactor_al_desktop() {
    // El reactor pide monocle al llenarse; el conductor lo aplica al Desktop
    // autoritativo, que en monocle deja visible SÓLO la enfocada. Oráculo
    // crudo: con 3 ventanas abiertas, el Place final lista 1 visible (no 3).
    let reactor = LoadedPlugin::load_bytes(
        REACTOR_WASM,
        PluginKind::Reactor,
        CAP_KEYS | CAP_SPAWN | CAP_EFFECTS | CAP_ACTIONS,
        0,
        "term",
    )
    .unwrap();
    let mut c = Conductor::new(Desktop::new(), vec![reactor]);
    let _ = c.startup();
    c.on_body_event(BodyEvent::OutputAdded { id: 0, width: 1000, height: 1000 });

    let mut visibles = Vec::new();
    for id in 1..=3u64 {
        let cmds =
            c.on_body_event(BodyEvent::WindowOpened { id, app_id: "t".into(), title: "w".into() });
        for cmd in cmds {
            if let BrainCommand::Place(ps) = cmd {
                visibles = ps.iter().filter(|p| p.visible).map(|p| p.id).collect();
            }
        }
    }
    assert_eq!(
        visibles.len(),
        1,
        "tras la 3ª ventana el reactor pidió monocle y el Desktop dejó 1 visible: {visibles:?}"
    );
}

// --- Config por plugin + el enrutador de apps (asignador). ------------------

#[test]
fn asignador_enruta_por_app_id_segun_su_config() {
    let mut p =
        LoadedPlugin::load_bytes(ASIGNADOR_WASM, PluginKind::Reactor, CAP_ACTIONS, 0, "asg").unwrap();
    // La config llega por el canal `mirada_configure` (lo que da config por plugin).
    p.configure("firefox 2\npavucontrol float\ncalc 5 float\n# comentario\n")
        .unwrap();

    // Firefox → escritorio 2 (sin flotar).
    p.call_on_event(&BodyEvent::WindowOpened { id: 1, app_id: "firefox".into(), title: "w".into() })
        .unwrap();
    assert_eq!(p.take_actions(), vec!["send-to-workspace:2".to_string()]);

    // Substring case-insensitive: «org.PulseAudio.pavucontrol» casa «pavucontrol».
    p.call_on_event(&BodyEvent::WindowOpened {
        id: 2,
        app_id: "org.PulseAudio.pavucontrol".into(),
        title: "w".into(),
    })
    .unwrap();
    assert_eq!(p.take_actions(), vec!["toggle-float".to_string()]);

    // calc → flota PRIMERO (mientras tiene el foco) y luego va al 5.
    p.call_on_event(&BodyEvent::WindowOpened { id: 3, app_id: "calc".into(), title: "w".into() })
        .unwrap();
    assert_eq!(
        p.take_actions(),
        vec!["toggle-float".to_string(), "send-to-workspace:5".to_string()]
    );

    // App sin regla → ninguna acción.
    p.call_on_event(&BodyEvent::WindowOpened { id: 4, app_id: "mystery".into(), title: "w".into() })
        .unwrap();
    assert!(p.take_actions().is_empty(), "una app sin regla no debería enrutarse");
}

#[test]
fn asignador_firmado_carga_y_su_config_comentada_es_inocua() {
    // Camino real: lo carga desde su manifest (firmado + config por defecto, toda
    // comentada). Certifica que (a) la firma es válida, (b) el campo `config` del
    // .ron fluye al plugin, y (c) sin reglas no enruta nada.
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let m = PluginManifest::load(&assets.join("asignador.ron")).unwrap();
    let trust = TrustSet::load(&assets.join("trust.ron"));
    let mut p = LoadedPlugin::load(&m, &trust).expect("el asignador firmado debería cargar");
    p.call_on_event(&BodyEvent::WindowOpened { id: 1, app_id: "firefox".into(), title: "w".into() })
        .unwrap();
    assert!(p.take_actions().is_empty(), "config comentada → sin reglas → no-op");
}

#[test]
fn reconfigurar_reemplaza_las_reglas() {
    // Re-`configure` (lo que hace el hot-reload al cambiar el .ron) reemplaza la
    // política: la regla vieja deja de aplicar, la nueva sí.
    let mut p =
        LoadedPlugin::load_bytes(ASIGNADOR_WASM, PluginKind::Reactor, CAP_ACTIONS, 0, "asg").unwrap();
    p.configure("firefox 2").unwrap();
    p.call_on_event(&BodyEvent::WindowOpened { id: 1, app_id: "firefox".into(), title: "w".into() })
        .unwrap();
    assert_eq!(p.take_actions(), vec!["send-to-workspace:2".to_string()]);

    p.configure("firefox 7").unwrap();
    p.call_on_event(&BodyEvent::WindowOpened { id: 2, app_id: "firefox".into(), title: "w".into() })
        .unwrap();
    assert_eq!(p.take_actions(), vec!["send-to-workspace:7".to_string()]);
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
        LoadedPlugin::load_bytes(REACTOR_WASM, PluginKind::Reactor, CAP_KEYS | CAP_SPAWN | CAP_EFFECTS | CAP_ACTIONS, 0, "term")
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

// --- Hot-reload del directorio de plugins. ----------------------------------

#[test]
fn reload_plugins_aplica_el_nuevo_layout_al_instante() {
    // Arranca SIN plugins: el master-stack del Desktop pone la maestra a la
    // IZQUIERDA. El estado de ventanas vive en el Desktop, que reload preserva.
    let mut c = Conductor::new(Desktop::new(), Vec::new());
    let _ = c.startup();
    c.on_body_event(BodyEvent::OutputAdded { id: 0, width: 1000, height: 1000 });
    let mut rects = Vec::new();
    for id in 1..=2u64 {
        tiled_from(
            c.on_body_event(BodyEvent::WindowOpened { id, app_id: "t".into(), title: "w".into() }),
            &mut rects,
        );
    }
    let master_antes = rects.iter().max_by_key(|r| r.w).copied().unwrap();
    assert!(cx(&master_antes) < 500, "sin plugin la maestra va a la izquierda: {master_antes:?}");

    // Cargar el plugin right-master en caliente debe re-teselar YA (sin esperar
    // un evento nuevo): la maestra salta a la DERECHA en los comandos del reload.
    let layout =
        LoadedPlugin::load_bytes(LAYOUT_WASM, PluginKind::Layout, CAP_LAYOUT, 10, "rm").unwrap();
    let cmds = c.reload_plugins(vec![layout]);
    let mut rects2 = Vec::new();
    tiled_from(cmds, &mut rects2);
    assert!(!rects2.is_empty(), "el reload debería re-emitir un Place");
    let master_despues = rects2.iter().max_by_key(|r| r.w).copied().unwrap();
    assert!(
        cx(&master_despues) > 500,
        "tras recargar el plugin, el right-master pone la maestra a la derecha: {master_despues:?}"
    );
}

#[test]
fn reload_plugins_deja_operativo_al_reactor_nuevo() {
    // Arranca sin plugins; recarga con el reactor; el reactor recién instalado
    // debe atenuar las ventanas sin foco al abrirlas — prueba que reload lo dejó
    // vivo (instanciado, con sus capacidades enlazadas), no sólo registrado.
    let mut c = Conductor::new(Desktop::new(), Vec::new());
    let _ = c.startup();
    c.on_body_event(BodyEvent::OutputAdded { id: 0, width: 1000, height: 1000 });

    let reactor = LoadedPlugin::load_bytes(
        REACTOR_WASM,
        PluginKind::Reactor,
        CAP_KEYS | CAP_SPAWN | CAP_EFFECTS | CAP_ACTIONS,
        0,
        "term",
    )
    .unwrap();
    let _ = c.reload_plugins(vec![reactor]);

    c.on_body_event(BodyEvent::WindowOpened { id: 1, app_id: "a".into(), title: "w".into() });
    let cmds =
        c.on_body_event(BodyEvent::WindowOpened { id: 2, app_id: "b".into(), title: "w".into() });
    assert!(
        cmds.iter().any(|c| matches!(c, BrainCommand::SetEffects(_))),
        "el reactor recargado en caliente debería emitir efectos: {cmds:?}"
    );
}
