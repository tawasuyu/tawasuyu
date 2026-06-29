//! `nakui-server` — el host autoritativo headless de un workspace nakui.
//!
//! Abre el WAL, carga los executors por módulo, y sirve el escritor a
//! clientes remotos por card-net (vía [`nakui_net::serve`]). Es la máquina
//! "del rincón de la oficina": el único escritor del workspace; las UIs
//! (locales o remotas) se conectan como clientes ([`nakui_net::RemoteBackend`]).
//!
//! Config por entorno (mismas convenciones que la UI):
//!   - `NAKUI_EVENT_LOG`        log JSONL (default `nakui-server-state.jsonl`)
//!   - `NAKUI_MODULES_DIR`      directorio de módulos (default `nakui-modules`)
//!   - `NAKUI_BIND`             multiaddr de escucha (default `/ip4/0.0.0.0/tcp/0`)
//!   - `NAKUI_SNAPSHOT_THRESHOLD`  umbral de auto-compaction (default `50`)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cards::CardBody;
use nakui_core::executor::Executor;
use nakui_net::serve;
use nakui_sync::Writer;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Descubre los módulos con executor nakui en `modules_dir`, devolviendo
/// `(module_id, ruta_del_dir_nakui)`. Sólo datos `Send` — los `Executor`
/// (que son `!Send`) se construyen después, dentro del thread del escritor.
fn descubrir_modulos(modules_dir: &Path) -> Vec<(String, PathBuf)> {
    let cards = match cards::load_cards_from_dir(modules_dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nakui-server: no pude cargar módulos de {}: {e}", modules_dir.display());
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for c in cards {
        let CardBody::UiModule(m) = c.body else { continue };
        let Some(rel) = &m.nakui_module_dir else { continue };
        let module_root = modules_dir.join(&m.id);
        let nakui_dir = if Path::new(rel).is_absolute() {
            PathBuf::from(rel)
        } else {
            module_root.join(rel)
        };
        out.push((m.id.clone(), nakui_dir));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn main() {
    let modules_dir = PathBuf::from(env_or("NAKUI_MODULES_DIR", "nakui-modules"));
    let log_path = PathBuf::from(env_or("NAKUI_EVENT_LOG", "nakui-server-state.jsonl"));
    let bind = env_or("NAKUI_BIND", "/ip4/0.0.0.0/tcp/0");
    let threshold: usize = env_or("NAKUI_SNAPSHOT_THRESHOLD", "50").parse().unwrap_or(50);

    let mods = descubrir_modulos(&modules_dir);
    println!("nakui-server");
    println!("  log:      {}", log_path.display());
    println!("  módulos:  {} cargados desde {}", mods.len(), modules_dir.display());
    for (id, _) in &mods {
        println!("            · {id}");
    }

    let handle = match serve(
        move || {
            let mut execs: BTreeMap<String, Arc<Executor>> = BTreeMap::new();
            for (id, dir) in &mods {
                match Executor::load_module(dir) {
                    Ok(exec) => {
                        execs.insert(id.clone(), Arc::new(exec));
                    }
                    Err(e) => {
                        eprintln!("nakui-server: módulo {id}: no cargó executor en {}: {e}", dir.display());
                    }
                }
            }
            Writer::open(log_path, threshold, execs).0
        },
        &bind,
    ) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("nakui-server: no pude arrancar: {e}");
            std::process::exit(1);
        }
    };

    println!("\nescuchando — dirección dialable para los clientes:");
    println!("  {}", handle.dial_addr());
    println!("\n(Ctrl-C para parar)");

    // El servidor vive en su propio thread+runtime; el main sólo se aparca.
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
