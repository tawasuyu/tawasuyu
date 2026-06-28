//! llimphi-wasm-open — abre una app WASM Tier 3 como una ventana de escritorio.
//!
//! Cierra el lazo de la distribución: el stack Tier 3 ya sabía resolver una app
//! por hash, verificar su integridad y su concesión Ed25519, y materializar su
//! `View<Msg>` en una ventana Llimphi. Lo que faltaba era el **ejecutable** que
//! ata todo desde la línea de comandos del escritorio — el gemelo host del
//! ejecutor de apps de wawa, y el consumidor real de `app_bus::Launch::Wasm`
//! (que hasta ahora devolvía `Unsupported` porque "lo resuelve el chasis": este
//! binario *es* ese chasis).
//!
//! Dos modos:
//!
//! ```text
//! # 1. Un .wasm local — app de sólo-UI, sin permisos (camino rápido).
//! llimphi-wasm-open app.wasm
//!
//! # 2. Por hash desde un CAS local — se verifica integridad y, si hay
//! #    concesión, se valida contra el anillo de confianza y se corre con
//! #    los permisos efectivos (los bits gatean qué host imports se enlazan).
//! llimphi-wasm-open --hash <blake3-hex> --store ~/.cache/llimphi/blobs \
//!     [--grant <blake3-hex>] [--ring claves.txt] [--name "Mi App"]
//! ```
//!
//! El modo hash es exactamente lo que un launcher de la UI (dock, spotlight,
//! mirada) invocaría para un `Launch::Wasm{bytecode_hex, grant_hex}`: spawnear
//! este binario con `--hash`/`--grant`/`--store`, manteniendo a `app-bus`
//! liviano (sólo transporta hex, no toca el runner ni la GPU).

use std::process::ExitCode;
use std::sync::OnceLock;

use format::Permisos;
use llimphi_ui::{App, Handle, KeyEvent, View};
use llimphi_wasm_dist::{
    hash_from_hex, AppManifest, DiskStore, RunnerMsg, TrustRing, VerifiedAppExt, WasmGuest,
};

/// Lo resuelto en `main` y consumido por `Host::init` en el hilo de la UI.
/// Cargamos el `WasmGuest` recién en `init` (no antes) porque el `Store` de
/// wasmi vive atado a su hilo; acá viajan sólo datos (`Send`).
#[cfg_attr(test, derive(Debug))]
struct LaunchSpec {
    wasm: Vec<u8>,
    permisos: Permisos,
    title: String,
}

static SPEC: OnceLock<LaunchSpec> = OnceLock::new();

/// El modelo del host: el guest vivo + el título que mostramos en la barra.
struct HostModel {
    guest: WasmGuest,
    title: String,
}

struct Host;

impl App for Host {
    type Model = HostModel;
    type Msg = RunnerMsg;

    fn title() -> &'static str {
        "llimphi · wasm"
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        let spec = SPEC.get().expect("LaunchSpec no inicializado");
        let guest = WasmGuest::load(&spec.wasm, spec.permisos)
            .unwrap_or_else(|e| panic!("cargar app WASM: {e}"));
        HostModel {
            guest,
            title: spec.title.clone(),
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        if let Err(e) = model.guest.apply(&msg) {
            eprintln!("wasm dispatch: {e}");
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        model.guest.render()
    }

    fn window_title(model: &Self::Model) -> Option<String> {
        Some(format!("llimphi · {}", model.title))
    }

    fn on_key(model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        model.guest.key_to_msg(event)
    }

    fn on_focus(_model: &Self::Model, id: Option<u64>) -> Option<Self::Msg> {
        Some(WasmGuest::focus_msg(id))
    }
}

fn main() -> ExitCode {
    match resolver_spec(std::env::args().skip(1)) {
        Ok(spec) => {
            let _ = SPEC.set(spec);
            llimphi_ui::run::<Host>();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("llimphi-wasm-open: {e}");
            eprintln!();
            eprintln!("uso:");
            eprintln!("  llimphi-wasm-open <app.wasm>");
            eprintln!("  llimphi-wasm-open --hash <hex> --store <dir> \\");
            eprintln!("      [--grant <hex>] [--ring <archivo>] [--name <título>]");
            ExitCode::FAILURE
        }
    }
}

/// Traduce los argumentos a un [`LaunchSpec`] resuelto y verificado. El modo se
/// decide por la presencia de `--hash`: con él, resolución completa contra el
/// CAS; sin él, el primer argumento es un `.wasm` local de sólo-UI.
fn resolver_spec(args: impl Iterator<Item = String>) -> Result<LaunchSpec, String> {
    let mut args = args.peekable();

    let mut hash_hex: Option<String> = None;
    let mut grant_hex: Option<String> = None;
    let mut store_dir: Option<String> = None;
    let mut ring_path: Option<String> = None;
    let mut name: Option<String> = None;
    let mut path: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--hash" => hash_hex = Some(next_val(&mut args, "--hash")?),
            "--grant" => grant_hex = Some(next_val(&mut args, "--grant")?),
            "--store" => store_dir = Some(next_val(&mut args, "--store")?),
            "--ring" => ring_path = Some(next_val(&mut args, "--ring")?),
            "--name" => name = Some(next_val(&mut args, "--name")?),
            "-h" | "--help" => return Err("ayuda".into()),
            otro if otro.starts_with("--") => {
                return Err(format!("opción desconocida: {otro}"))
            }
            otro => path = Some(otro.to_string()),
        }
    }

    match hash_hex {
        // Modo distribución: hash → CAS → verificar → permisos efectivos.
        Some(hex) => {
            // Sin `--store` cae al CAS de blobs por defecto del escritorio
            // (XDG cache), para que el chasis pueda spawnearnos con sólo el hash.
            let store_dir = store_dir.unwrap_or_else(cas_por_defecto);
            let store = DiskStore::open(&store_dir)
                .map_err(|e| format!("abrir CAS {store_dir}: {e}"))?;

            let trust = match &ring_path {
                Some(p) => TrustRing::load(p).map_err(|e| format!("anillo {p}: {e}"))?,
                None => TrustRing::empty(),
            };

            let bytecode = hash_from_hex(&hex).map_err(|e| format!("--hash inválido: {e}"))?;
            let concesion = match &grant_hex {
                Some(g) => Some(hash_from_hex(g).map_err(|e| format!("--grant inválido: {e}"))?),
                None => None,
            };
            // Con concesión declaramos MAX y dejamos que la intersección con lo
            // concedido fije los efectivos (honrar el grant completo); sin ella,
            // app de sólo-UI (fail-closed, permisos = 0).
            let declarados = if concesion.is_some() {
                Permisos::MAX
            } else {
                0
            };
            let manifest = AppManifest {
                bytecode,
                declarados,
                concesion,
            };

            let app = llimphi_wasm_dist::resolve_manifest(&store, &trust, &manifest)
                .map_err(|e| format!("resolver app: {e}"))?;
            // Instanciamos una vez acá para fallar temprano con un mensaje claro
            // (un guest que importe una capacidad no concedida trap-ea al
            // instanciar); el `WasmGuest` no cruza de hilo, así que init lo
            // recarga desde los bytes ya verificados.
            app.load().map_err(|e| format!("instanciar app: {e}"))?;
            Ok(LaunchSpec {
                wasm: app.wasm,
                permisos: app.permisos,
                title: name.unwrap_or_else(|| short_hex(&hex)),
            })
        }
        // Modo local: un .wasm de sólo-UI, sin verificación ni permisos.
        None => {
            let path = path.ok_or("falta el archivo .wasm o --hash")?;
            let wasm = std::fs::read(&path).map_err(|e| format!("leer {path}: {e}"))?;
            let title = name.unwrap_or_else(|| nombre_archivo(&path));
            Ok(LaunchSpec {
                wasm,
                permisos: 0,
                title,
            })
        }
    }
}

fn next_val(
    args: &mut std::iter::Peekable<impl Iterator<Item = String>>,
    flag: &str,
) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} requiere un valor"))
}

/// El CAS de blobs por defecto del escritorio: `$XDG_CACHE_HOME/llimphi/blobs`
/// (o `~/.cache/llimphi/blobs`). Es la convención que el chasis (dock,
/// spotlight, mirada) asume al spawnearnos con sólo `--hash`.
fn cas_por_defecto() -> String {
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.cache")))
        .unwrap_or_else(|| ".".to_string());
    format!("{base}/llimphi/blobs")
}

/// Los primeros 8 caracteres del hex, para un título legible.
fn short_hex(hex: &str) -> String {
    format!("wasm:{}", &hex[..hex.len().min(8)])
}

/// El nombre de archivo sin directorio ni extensión, para el título.
fn nombre_archivo(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("wasm")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_wasm_dist::{hash_to_hex, DiskStore};

    /// El counter Tier 3 ya compilado y versionado por build-wasm-demo.sh.
    const COUNTER_WASM: &[u8] =
        include_bytes!("../../llimphi-wasm-runner/assets/counter.wasm");

    fn args(v: &[&str]) -> impl Iterator<Item = String> {
        v.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter()
    }

    /// Un CAS temporal único para este proceso de test.
    fn cas_temporal(sufijo: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("llimphi-wasm-open-{}-{sufijo}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn modo_hash_resuelve_desde_el_cas() {
        let dir = cas_temporal("hash");
        let store = DiskStore::open(&dir).unwrap();
        let hash = store.put(COUNTER_WASM).unwrap();
        let hex = hash_to_hex(&hash);

        let spec = resolver_spec(args(&[
            "--hash",
            &hex,
            "--store",
            dir.to_str().unwrap(),
        ]))
        .expect("resolver por hash");

        // Sin concesión ⇒ sólo-UI (fail-closed) y los bytes son los del counter.
        assert_eq!(spec.permisos, 0);
        assert_eq!(spec.wasm, COUNTER_WASM);
        // El guest verificado realmente instancia.
        WasmGuest::load(&spec.wasm, spec.permisos).expect("instanciar counter");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hash_que_no_esta_en_el_cas_falla() {
        let dir = cas_temporal("ausente");
        // Un hash válido en forma pero no presente en el CAS.
        let hex = "00".repeat(32);
        let err = resolver_spec(args(&["--hash", &hex, "--store", dir.to_str().unwrap()]))
            .unwrap_err();
        assert!(err.contains("resolver app"), "mensaje real: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn modo_local_carga_un_wasm_de_archivo() {
        let dir = cas_temporal("local");
        let path = dir.join("counter.wasm");
        std::fs::write(&path, COUNTER_WASM).unwrap();

        let spec =
            resolver_spec(args(&[path.to_str().unwrap()])).expect("cargar .wasm local");
        assert_eq!(spec.permisos, 0);
        assert_eq!(spec.wasm, COUNTER_WASM);
        assert_eq!(spec.title, "counter");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hash_sin_store_es_error() {
        let err = resolver_spec(args(&["--hash", &"ab".repeat(32)])).unwrap_err();
        assert!(err.contains("--store"), "mensaje real: {err}");
    }

    #[test]
    fn opcion_desconocida_es_error() {
        let err = resolver_spec(args(&["--vuela"])).unwrap_err();
        assert!(err.contains("desconocida"), "mensaje real: {err}");
    }
}
