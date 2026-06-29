//! Prueba de `RemoteBackend`: la UI corriendo contra un servidor remoto.
//!
//! Verifica el contrato `MetaBackend` sobre la red (catch-up del estado al
//! conectar, escritura, sincronización bidireccional y morfismos ejecutados
//! por el servidor) — el end-to-end real de la fase 3.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nahual_meta_runtime::MetaBackend;
use nakui_core::executor::Executor;
use nakui_net::{serve, RemoteBackend};
use nakui_sync::Writer;
use serde_json::{json, Map, Value};

fn map_of(items: &[(&str, Value)]) -> Map<String, Value> {
    items.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// Servidor vacío (sin módulos) con log en tempdir.
fn servidor_vacio() -> (nakui_net::ServerHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("log.jsonl");
    let handle = serve(
        move || Writer::open(log_path, 0, BTreeMap::new()).0,
        "/ip4/127.0.0.1/tcp/0",
    )
    .expect("servidor arranca");
    (handle, dir)
}

/// Escribe un módulo nakui mínimo y correcto (`demo`, morfismo `crear`) en
/// `dir/mod/` y devuelve esa ruta. Schema con `id` explícito (a diferencia
/// del demo `tesoro`, cuyo contrato estricto rechaza el `id` inyectado).
fn escribir_modulo_demo(dir: &std::path::Path) -> std::path::PathBuf {
    let mod_dir = dir.join("mod");
    std::fs::create_dir_all(mod_dir.join("scripts")).unwrap();
    std::fs::write(
        mod_dir.join("nsmc.json"),
        r#"{ "module": "demo", "schemas": ["schema.ncl"],
            "morphisms": [
              { "name": "crear", "inputs": [], "reads": [], "writes": ["Cosa"],
                "script": "scripts/crear.rhai" } ] }"#,
    )
    .unwrap();
    // El extractor de entities es heurístico: exige el nombre con EXACTAMENTE
    // 2 espacios de indent + CapitalCase + `=`. Por eso el formato multilínea.
    std::fs::write(
        mod_dir.join("schema.ncl"),
        "{\n  Cosa = {\n    id | String,\n    nombre | String,\n  },\n}\n",
    )
    .unwrap();
    std::fs::write(
        mod_dir.join("scripts/crear.rhai"),
        r#"[ #{ op: "create", entity: "Cosa", id: input.params.cosa_id,
              data: #{ id: input.params.cosa_id, nombre: input.params.nombre } } ]"#,
    )
    .unwrap();
    mod_dir
}

/// Servidor con un módulo `demo` cargado (para probar morfismos sobre red).
fn servidor_con_modulo() -> (nakui_net::ServerHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("log.jsonl");
    let module_dir = escribir_modulo_demo(dir.path());
    let handle = serve(
        move || {
            let exec = Executor::load_module(&module_dir).expect("módulo demo carga");
            let mut execs: BTreeMap<String, Arc<Executor>> = BTreeMap::new();
            execs.insert("demo".into(), Arc::new(exec));
            Writer::open(log_path, 0, execs).0
        },
        "/ip4/127.0.0.1/tcp/0",
    )
    .expect("servidor con módulo arranca");
    (handle, dir)
}

/// Reintenta hasta que la entity tenga `objetivo` records (los broadcasts son
/// asíncronos; `list_records` drena pendientes en cada llamada).
fn esperar_count(b: &RemoteBackend, entity: &str, objetivo: usize, segundos: u64) -> bool {
    let limite = Instant::now() + Duration::from_secs(segundos);
    loop {
        if b.list_records(entity).len() >= objetivo {
            return true;
        }
        if Instant::now() > limite {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn catch_up_trae_el_estado_existente_al_conectar() {
    let (handle, _dir) = servidor_vacio();

    // Cliente A escribe dos clientes del ERP.
    let mut a = RemoteBackend::connect(handle.dial_addr()).expect("A conecta");
    a.seed("Cliente", map_of(&[("nombre", json!("Acme"))])).unwrap();
    a.seed("Cliente", map_of(&[("nombre", json!("Globex"))])).unwrap();

    // Cliente B conecta DESPUÉS: el catch-up debe traer los dos altas que ya
    // existían, sin haberlos visto difundir.
    let b = RemoteBackend::connect(handle.dial_addr()).expect("B conecta");
    assert_eq!(
        b.list_records("Cliente").len(),
        2,
        "B recibe el estado existente por catch-up al conectar"
    );
}

#[test]
fn sincronizacion_bidireccional_en_vivo() {
    let (handle, _dir) = servidor_vacio();
    let mut a = RemoteBackend::connect(handle.dial_addr()).expect("A conecta");
    let mut b = RemoteBackend::connect(handle.dial_addr()).expect("B conecta");

    // A da de alta; B lo ve en vivo por difusión.
    let id = a
        .seed("Cliente", map_of(&[("nombre", json!("Acme"))]))
        .unwrap()
        .id
        .unwrap();
    assert!(esperar_count(&b, "Cliente", 1, 15), "B ve el alta de A");

    // B edita ese mismo record; A ve el cambio.
    b.update("Cliente", id, map_of(&[("nombre", json!("Acme S.A."))]), vec![])
        .unwrap();
    let limite = Instant::now() + Duration::from_secs(15);
    loop {
        if a.load_record("Cliente", id).and_then(|r| r.get("nombre").cloned())
            == Some(json!("Acme S.A."))
        {
            break;
        }
        assert!(Instant::now() < limite, "A debe ver la edición de B");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn morfismo_remoto_lo_ejecuta_el_servidor() {
    let (handle, _dir) = servidor_con_modulo();
    let mut cliente = RemoteBackend::connect(handle.dial_addr()).expect("conecta");

    // El cliente NO tiene executors: el morfismo `crear` corre en el servidor
    // (que sí los tiene). El id es un UUID (lo exige el op Create del kernel).
    let cosa_id = uuid::Uuid::new_v4().to_string();
    let out = cliente
        .morphism(
            "demo",
            "crear",
            BTreeMap::new(),
            json!({ "cosa_id": cosa_id, "nombre": "primera" }),
        )
        .expect("morfismo remoto");
    assert_eq!(out.changed, 1, "crear produce 1 op");

    assert!(esperar_count(&cliente, "Cosa", 1, 15), "la Cosa aparece en la proyección");
    let cosa = cliente.list_records("Cosa").into_iter().next().unwrap().1;
    assert_eq!(cosa.get("nombre"), Some(&json!("primera")));
}
