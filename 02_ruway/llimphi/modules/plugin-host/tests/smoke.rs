//! Smoke tests del runtime Tier 2 — verifican:
//!
//! 1. Carga desde disco (`manifest.toml` + `.wasm`) e invocación que
//!    devuelve `PluginAction::SetStatus` con el saludo concatenado.
//! 2. Sandbox por permisos: un plugin con `filesystem = "none"` que
//!    intenta llamar `plugin.open_at` trap-ea — el import no se
//!    enlazó, así que el módulo importa una función inexistente.
//! 3. Permiso concedido: el mismo plugin con `filesystem = "read-only"`
//!    sí enlaza, ejecuta, y emite `PluginAction::OpenAt`.

use std::path::PathBuf;

use card_core::{FsPolicy, Permissions};
use llimphi_plugin_host::{PluginAction, PluginError, PluginHost, PluginManifest};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello-status")
}

/// Compila el .wat del fixture a .wasm en el OUT_DIR efímero del test.
/// Lo hacemos por test (no en build.rs) para mantener el crate sin
/// build script — el costo es trivial y la lógica vive con el test.
fn compile_fixture_to(dir: &std::path::Path) {
    let wat = std::fs::read_to_string(dir.join("plugin.wat")).expect("leo plugin.wat");
    let wasm = wat::parse_str(&wat).expect("WAT del fixture compila a wasm");
    std::fs::write(dir.join("plugin.wasm"), wasm).expect("escribo plugin.wasm");
}

#[test]
fn carga_desde_directorio_y_devuelve_set_status() {
    let dir = fixture_dir();
    compile_fixture_to(&dir);

    let mut host = PluginHost::new();
    let id = host.load_from_dir(&dir).expect("plugin carga desde dir");

    let manifest = host.manifest(id).expect("manifest accesible");
    assert_eq!(manifest.name, "hello-status");
    assert_eq!(manifest.capabilities, vec!["status.greet".to_string()]);

    let action = host.invoke(id, "status.greet", b"mundo").expect("invoke ok");
    assert_eq!(action, PluginAction::SetStatus("hola, mundo".into()));

    // El host puede enumerar capabilities agregadas para construir su Card.
    assert_eq!(host.all_capabilities(), vec!["status.greet".to_string()]);
}

/// WAT que intenta importar `plugin.open_at`. Sirve como "plugin
/// malicioso" para verificar el sandbox: si el host no concede
/// `filesystem`, el linker no enlaza el import → wasmi rechaza la
/// instanciación con un error de import faltante.
fn wants_open_at_wat() -> &'static str {
    // El path va en offset 256 para no colisionar con el buffer
    // [cap | args] que el host escribe a partir del offset 0.
    r#"
(module
  (import "plugin" "open_at" (func $open_at (param i32 i32 i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 256) "/etc/passwd")
  (func (export "_invoke")
        (param i32) (param i32) (param i32) (param i32)
        (result i32)
    (call $open_at (i32.const 256) (i32.const 11) (i32.const 10) (i32.const 5))
    (i32.const 0)
  )
)
"#
}

#[test]
fn sin_permiso_filesystem_el_plugin_no_instancia() {
    let bytes = wat::parse_str(wants_open_at_wat()).unwrap();
    let manifest = PluginManifest {
        name: "wants-fs".into(),
        version: "0.1.0".into(),
        capabilities: vec!["fs.open".into()],
        permissions: Permissions::default(), // filesystem = none
    };

    let mut host = PluginHost::new();
    let id = host.load_bytes(manifest, &bytes).expect("carga ok — el sandbox actúa al invocar");

    let err = host.invoke(id, "fs.open", b"").expect_err("debe fallar sin permiso fs");
    // wasmi reporta el import faltante en la instanciación.
    assert!(
        matches!(err, PluginError::Instantiate(_)),
        "esperaba Instantiate, vi {err:?}"
    );
}

#[test]
fn con_permiso_filesystem_el_plugin_emite_open_at() {
    let bytes = wat::parse_str(wants_open_at_wat()).unwrap();
    let manifest = PluginManifest {
        name: "wants-fs".into(),
        version: "0.1.0".into(),
        capabilities: vec!["fs.open".into()],
        permissions: Permissions { filesystem: FsPolicy::ReadOnly, ..Permissions::default() },
    };

    let mut host = PluginHost::new();
    let id = host.load_bytes(manifest, &bytes).unwrap();
    let action = host.invoke(id, "fs.open", b"").expect("con permiso, debe correr");

    assert_eq!(
        action,
        PluginAction::OpenAt { path: PathBuf::from("/etc/passwd"), line: 10, col: 5 }
    );
}
