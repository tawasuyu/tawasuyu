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

use format::{ConcesionCapacidad, Permisos};
use llimphi_ui::{App, Handle, KeyEvent, View};
use llimphi_wasm_dist::{
    hash_from_hex, hash_to_hex, resolve_from_catalog, AppManifest, Catalog, CatalogEntry, DiskStore,
    RunnerMsg, TrustRing, VerifiedAppExt, WasmGuest,
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
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let tiene = |f: &str| argv.iter().any(|a| a == f);

    // Modo registro: conectar a un registro de apps (REST en RON o directorio
    // local) — listar/buscar y, con --ingest, descargar al CAS + catálogo. No
    // abre ventana.
    if tiene("--registry") {
        return match registro(argv.iter().cloned()) {
            Ok(msg) => {
                println!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("llimphi-wasm-open --registry: {e}");
                uso();
                ExitCode::FAILURE
            }
        };
    }

    // Modo catálogo (búsqueda): listar/buscar apps por texto. No abre ventana.
    if tiene("--list") || tiene("--search") {
        return match buscar(argv.iter().cloned()) {
            Ok(msg) => {
                println!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("llimphi-wasm-open: {e}");
                uso();
                ExitCode::FAILURE
            }
        };
    }

    // Modo productor: publicar una app (blob al CAS + manifiesto + entrada de
    // catálogo si se pide). No abre ventana.
    if tiene("--install") {
        return match instalar(argv.into_iter()) {
            Ok(msg) => {
                println!("{msg}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("llimphi-wasm-open --install: {e}");
                uso();
                ExitCode::FAILURE
            }
        };
    }

    // Modo consumidor: resolver la app (por id del catálogo o por hash/archivo)
    // y abrirla como ventana.
    let spec = if tiene("--run") {
        spec_desde_catalogo(argv.into_iter())
    } else {
        resolver_spec(argv.into_iter())
    };
    match spec {
        Ok(spec) => {
            let _ = SPEC.set(spec);
            llimphi_ui::run::<Host>();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("llimphi-wasm-open: {e}");
            uso();
            ExitCode::FAILURE
        }
    }
}

fn uso() {
    eprintln!();
    eprintln!("uso:");
    eprintln!("  llimphi-wasm-open <app.wasm>");
    eprintln!("  llimphi-wasm-open --hash <hex> [--store <dir>] \\");
    eprintln!("      [--grant <hex>] [--ring <archivo>] [--name <título>]");
    eprintln!("  llimphi-wasm-open --list [--catalog <archivo>]");
    eprintln!("  llimphi-wasm-open --search <texto> [--catalog <archivo>]");
    eprintln!("  llimphi-wasm-open --run <id> [--catalog <archivo>] \\");
    eprintln!("      [--store <dir>] [--ring <archivo>]");
    eprintln!("  llimphi-wasm-open --install <app.wasm> --id <id> \\");
    eprintln!("      [--name <label>] [--desc <texto>] [--grant <archivo>] \\");
    eprintln!("      [--icon <glifo>] [--category <cat>] [--store <dir>] \\");
    eprintln!("      [--apps-dir <dir>] [--catalog <archivo>]");
    eprintln!("  llimphi-wasm-open --registry <descriptor.ron> [--instance <base>] \\");
    eprintln!("      [--query <texto>] [--ingest] [--catalog <archivo>] [--store <dir>]");
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

/// Publica una app WASM en el escritorio: mete el bytecode (y, si se da, la
/// concesión) en el CAS local y escribe un manifiesto `<id>.toml` en el
/// directorio de apps, para que el dock/spotlight la descubran y la lancen por
/// la ruta de hash. Es el lado productor del lazo de distribución —
/// el inverso de [`resolver_spec`] modo hash. Devuelve un resumen legible.
fn instalar(args: impl Iterator<Item = String>) -> Result<String, String> {
    let mut args = args.peekable();
    let mut wasm_path: Option<String> = None;
    let mut id: Option<String> = None;
    let mut name: Option<String> = None;
    let mut grant_file: Option<String> = None;
    let mut icon: Option<String> = None;
    let mut category: Option<String> = None;
    let mut store_dir: Option<String> = None;
    let mut apps_dir: Option<String> = None;
    let mut desc: Option<String> = None;
    let mut catalog_path: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--install" => wasm_path = Some(next_val(&mut args, "--install")?),
            "--id" => id = Some(next_val(&mut args, "--id")?),
            "--name" => name = Some(next_val(&mut args, "--name")?),
            "--desc" => desc = Some(next_val(&mut args, "--desc")?),
            "--grant" => grant_file = Some(next_val(&mut args, "--grant")?),
            "--icon" => icon = Some(next_val(&mut args, "--icon")?),
            "--category" => category = Some(next_val(&mut args, "--category")?),
            "--store" => store_dir = Some(next_val(&mut args, "--store")?),
            "--apps-dir" => apps_dir = Some(next_val(&mut args, "--apps-dir")?),
            "--catalog" => catalog_path = Some(next_val(&mut args, "--catalog")?),
            otro if otro.starts_with("--") => {
                return Err(format!("opción desconocida: {otro}"))
            }
            otro => wasm_path = Some(otro.to_string()),
        }
    }

    let wasm_path = wasm_path.ok_or("--install requiere la ruta del .wasm")?;
    let id = id.ok_or("--install requiere --id <id> (nombre del manifiesto)")?;
    let wasm = std::fs::read(&wasm_path).map_err(|e| format!("leer {wasm_path}: {e}"))?;

    let store_dir = store_dir.unwrap_or_else(cas_por_defecto);
    let store = DiskStore::open(&store_dir).map_err(|e| format!("abrir CAS {store_dir}: {e}"))?;

    // El bytecode entra al CAS direccionado por su hash (idéntico al que el
    // modo hash pedirá para correrlo).
    let bytecode = store.put(&wasm).map_err(|e| format!("guardar bytecode: {e}"))?;
    let bytecode_hex = hash_to_hex(&bytecode);

    // Si hay concesión, su blob entra también; el manifiesto la referencia por
    // hash (descubrimiento de concesiones, no inline).
    let grant_obj = match &grant_file {
        Some(path) => {
            let blob = std::fs::read(path).map_err(|e| format!("leer concesión {path}: {e}"))?;
            let grant = ConcesionCapacidad::deserializar(&blob)
                .map_err(|_| format!("la concesión {path} no deserializa"))?;
            if grant.bytecode != bytecode {
                return Err("la concesión es para otro bytecode".into());
            }
            let h = store
                .put_grant(&grant)
                .map_err(|e| format!("guardar concesión: {e}"))?;
            Some(h)
        }
        None => None,
    };
    let grant_hex = grant_obj.as_ref().map(hash_to_hex);
    // Con concesión declaramos MAX (efectivos = MAX & concedidos = concedidos);
    // sin ella, app de sólo-UI.
    let declarados = if grant_obj.is_some() { Permisos::MAX } else { 0 };

    let label = name.unwrap_or_else(|| id.clone());
    let entry = app_bus::AppEntry {
        id: id.clone(),
        label: label.clone(),
        icon,
        category: category.clone(),
        launch: app_bus::Launch::Wasm {
            bytecode_hex: bytecode_hex.clone(),
            grant_hex: grant_hex.clone(),
        },
        handles: Vec::new(),
    };
    let toml = app_bus::entry_to_toml(&entry).map_err(|e| format!("serializar manifiesto: {e}"))?;

    let apps_dir = match apps_dir {
        Some(d) => std::path::PathBuf::from(d),
        None => app_bus::apps_dir()
            .ok_or("no se pudo ubicar el directorio de apps (~/.config/tawasuyu/apps)")?,
    };
    std::fs::create_dir_all(&apps_dir)
        .map_err(|e| format!("crear {}: {e}", apps_dir.display()))?;
    let manifest_path = apps_dir.join(format!("{id}.toml"));
    std::fs::write(&manifest_path, &toml)
        .map_err(|e| format!("escribir {}: {e}", manifest_path.display()))?;

    // Si se pidió, publicar también en el catálogo buscable: el índice
    // content-addressed que hace `--search`/`--run` posibles (y que viaja por la
    // malla como un blob más).
    let catalog_línea = match catalog_path {
        Some(path) => {
            let mut catalog = cargar_catalogo(&path)?;
            catalog.upsert(CatalogEntry {
                id: id.clone(),
                name: label,
                description: desc.unwrap_or_default(),
                category,
                bytecode,
                declarados,
                concesion: grant_obj,
            });
            if let Some(parent) = std::path::Path::new(&path).parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("crear {}: {e}", parent.display()))?;
            }
            std::fs::write(&path, catalog.serializar())
                .map_err(|e| format!("escribir catálogo {path}: {e}"))?;
            format!("\n  catálogo: {path} ({} apps)", catalog.entries.len())
        }
        None => String::new(),
    };

    let grant_línea = grant_hex
        .map(|h| format!("\n  concesión: {h}"))
        .unwrap_or_default();
    Ok(format!(
        "instalada «{id}» en {}\n  bytecode: {bytecode_hex}{grant_línea}\n  CAS: {store_dir}{catalog_línea}",
        manifest_path.display()
    ))
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

/// El catálogo de apps por defecto del escritorio:
/// `$XDG_CONFIG_HOME/tawasuyu/catalog.bin` (o `~/.config/tawasuyu/catalog.bin`).
fn catalogo_por_defecto() -> String {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.config")))
        .unwrap_or_else(|| ".".to_string());
    format!("{base}/tawasuyu/catalog.bin")
}

/// Carga el catálogo del archivo (o el de por defecto). Un archivo ausente es un
/// catálogo vacío (todavía no se publicó nada); uno corrupto es error.
fn cargar_catalogo(path: &str) -> Result<Catalog, String> {
    match std::fs::read(path) {
        Ok(bytes) => Catalog::deserializar(&bytes)
            .map_err(|_| format!("el catálogo {path} está corrupto")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Catalog::default()),
        Err(e) => Err(format!("leer catálogo {path}: {e}")),
    }
}

/// Modo búsqueda: lista o filtra las apps del catálogo por texto. Devuelve el
/// listado formateado (id, nombre, categoría, hash corto, marca de permisos).
fn buscar(args: impl Iterator<Item = String>) -> Result<String, String> {
    let mut args = args.peekable();
    let mut query: Option<String> = None;
    let mut catalog_path: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--search" => query = Some(next_val(&mut args, "--search")?),
            "--list" => {}
            "--catalog" => catalog_path = Some(next_val(&mut args, "--catalog")?),
            otro if otro.starts_with("--") => {
                return Err(format!("opción desconocida: {otro}"))
            }
            otro => query = Some(otro.to_string()),
        }
    }
    let path = catalog_path.unwrap_or_else(catalogo_por_defecto);
    let catalog = cargar_catalogo(&path)?;
    let q = query.unwrap_or_default();
    let hits = catalog.search(&q);
    if hits.is_empty() {
        return Ok(if catalog.entries.is_empty() {
            format!("catálogo vacío ({path}) — publicá apps con --install --catalog")
        } else {
            format!("sin coincidencias para «{q}» en {path}")
        });
    }
    let mut out = format!("{} app(s) en {path}:\n", hits.len());
    for e in hits {
        let cat = e.category.as_deref().unwrap_or("—");
        let permisos = if e.concesion.is_some() { " ⚷" } else { "" };
        out.push_str(&format!(
            "  {:<14} {:<22} [{}]  {}{}\n",
            e.id,
            e.name,
            cat,
            &hash_to_hex(&e.bytecode)[..12],
            permisos,
        ));
    }
    Ok(out.trim_end().to_string())
}

/// Modo registro: conecta a un registro de apps (REST en RON, o un directorio
/// local) y lista/busca; con `--ingest`, descarga los módulos que coincidan al
/// CAS y los suma al catálogo local (para luego `--run`). El transporte se elige
/// por la instancia: `http(s)://…` ⇒ red real; cualquier otra cosa ⇒ filesystem.
fn registro(args: impl Iterator<Item = String>) -> Result<String, String> {
    use llimphi_wasm_registry::{LocalFetch, RegistryDescriptor, RegistryProvider, UreqFetch};

    let mut args = args.peekable();
    let mut descriptor_path: Option<String> = None;
    let mut instance: Option<String> = None;
    let mut query: Option<String> = None;
    let mut ingest = false;
    let mut catalog_path: Option<String> = None;
    let mut store_dir: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--registry" => descriptor_path = Some(next_val(&mut args, "--registry")?),
            "--instance" => instance = Some(next_val(&mut args, "--instance")?),
            "--query" | "--search" => query = Some(next_val(&mut args, "--query")?),
            "--ingest" => ingest = true,
            "--catalog" => catalog_path = Some(next_val(&mut args, "--catalog")?),
            "--store" => store_dir = Some(next_val(&mut args, "--store")?),
            otro if otro.starts_with("--") => {
                return Err(format!("opción desconocida: {otro}"))
            }
            _ => {}
        }
    }

    let descriptor_path = descriptor_path.ok_or("--registry requiere el archivo .ron")?;
    let ron = std::fs::read_to_string(&descriptor_path)
        .map_err(|e| format!("leer descriptor {descriptor_path}: {e}"))?;
    let descriptor =
        RegistryDescriptor::from_ron(&ron).map_err(|e| format!("descriptor: {e}"))?;
    let base = instance.unwrap_or_default();
    let es_http = base.starts_with("http://") || base.starts_with("https://");
    let q = query.unwrap_or_default();

    // El listado y la ingesta son idénticos sea cual sea el transporte; sólo
    // cambia el fetcher. Una pequeña función genérica evita duplicar la lógica.
    fn operar<F: llimphi_wasm_registry::HttpFetch>(
        provider: RegistryProvider<F>,
        q: &str,
        ingest: bool,
        catalog_path: Option<String>,
        store_dir: Option<String>,
    ) -> Result<String, String> {
        let apps = provider.list(q).map_err(|e| format!("listar: {e}"))?;
        let hits: Vec<_> = apps.into_iter().filter(|a| a.matches(q)).collect();
        if hits.is_empty() {
            return Ok(format!("sin apps para «{q}» en el registro"));
        }
        if !ingest {
            let mut out = format!("{} app(s) en el registro:\n", hits.len());
            for a in &hits {
                let cat = a.category.as_deref().unwrap_or("—");
                out.push_str(&format!("  {:<16} {:<24} [{}]  {}\n", a.id, a.name, cat, a.wasm_url));
            }
            out.push_str("  (usá --ingest para bajarlas al catálogo local)");
            return Ok(out);
        }
        // Ingesta: descargar cada módulo al CAS y sumarlo al catálogo local.
        let store_dir = store_dir.unwrap_or_else(cas_por_defecto);
        let store = DiskStore::open(&store_dir).map_err(|e| format!("abrir CAS {store_dir}: {e}"))?;
        let cat_path = catalog_path.unwrap_or_else(catalogo_por_defecto);
        let mut catalog = cargar_catalogo(&cat_path)?;
        let mut n = 0;
        for a in &hits {
            let entry = provider
                .ingest(a, &store)
                .map_err(|e| format!("ingerir «{}»: {e}", a.id))?;
            catalog.upsert(entry);
            n += 1;
        }
        if let Some(parent) = std::path::Path::new(&cat_path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("crear {}: {e}", parent.display()))?;
        }
        std::fs::write(&cat_path, catalog.serializar())
            .map_err(|e| format!("escribir catálogo {cat_path}: {e}"))?;
        Ok(format!(
            "ingeridas {n} app(s) al catálogo {cat_path} ({} en total)\n  CAS: {store_dir}",
            catalog.entries.len()
        ))
    }

    if es_http {
        operar(RegistryProvider::<UreqFetch>::new(descriptor, base), &q, ingest, catalog_path, store_dir)
    } else {
        // Sin instancia http: tratamos `base` (o el dir del descriptor) como un
        // registro local en disco.
        let root = if base.is_empty() {
            std::path::Path::new(&descriptor_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".into())
        } else {
            base.clone()
        };
        let provider = RegistryProvider::with_fetch(descriptor, root.clone(), LocalFetch::new(&root));
        operar(provider, &q, ingest, catalog_path, store_dir)
    }
}

/// Modo correr-por-id: resuelve una app del catálogo por su `id` (trae bytecode
/// + concesión del CAS, verifica) y produce el [`LaunchSpec`] para abrirla.
fn spec_desde_catalogo(args: impl Iterator<Item = String>) -> Result<LaunchSpec, String> {
    let mut args = args.peekable();
    let mut id: Option<String> = None;
    let mut catalog_path: Option<String> = None;
    let mut store_dir: Option<String> = None;
    let mut ring_path: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--run" => id = Some(next_val(&mut args, "--run")?),
            "--catalog" => catalog_path = Some(next_val(&mut args, "--catalog")?),
            "--store" => store_dir = Some(next_val(&mut args, "--store")?),
            "--ring" => ring_path = Some(next_val(&mut args, "--ring")?),
            otro if otro.starts_with("--") => {
                return Err(format!("opción desconocida: {otro}"))
            }
            otro => id = Some(otro.to_string()),
        }
    }
    let id = id.ok_or("--run requiere el id de la app")?;
    let path = catalog_path.unwrap_or_else(catalogo_por_defecto);
    let catalog = cargar_catalogo(&path)?;
    let entry = catalog
        .get(&id)
        .ok_or_else(|| format!("«{id}» no está en el catálogo {path}"))?;
    let title = entry.name.clone();

    let store_dir = store_dir.unwrap_or_else(cas_por_defecto);
    let store = DiskStore::open(&store_dir).map_err(|e| format!("abrir CAS {store_dir}: {e}"))?;
    let trust = match &ring_path {
        Some(p) => TrustRing::load(p).map_err(|e| format!("anillo {p}: {e}"))?,
        None => TrustRing::empty(),
    };
    let app = resolve_from_catalog(&store, &trust, &catalog, &id)
        .map_err(|e| format!("resolver «{id}»: {e}"))?;
    Ok(LaunchSpec {
        wasm: app.wasm,
        permisos: app.permisos,
        title,
    })
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
    fn cas_por_defecto_honra_xdg_cache_home() {
        // Camino que el chasis asume al spawnearnos con sólo el hash.
        let prev = std::env::var("XDG_CACHE_HOME").ok();
        std::env::set_var("XDG_CACHE_HOME", "/tmp/xdgtest");
        assert_eq!(cas_por_defecto(), "/tmp/xdgtest/llimphi/blobs");
        match prev {
            Some(v) => std::env::set_var("XDG_CACHE_HOME", v),
            None => std::env::remove_var("XDG_CACHE_HOME"),
        }
    }

    #[test]
    fn opcion_desconocida_es_error() {
        let err = resolver_spec(args(&["--vuela"])).unwrap_err();
        assert!(err.contains("desconocida"), "mensaje real: {err}");
    }

    #[test]
    fn install_publica_y_se_puede_lanzar() {
        let dir = cas_temporal("install");
        let cas = dir.join("blobs");
        let apps = dir.join("apps");
        let wasm_path = dir.join("counter.wasm");
        std::fs::write(&wasm_path, COUNTER_WASM).unwrap();

        // Lado productor: publica la app (blob al CAS + manifiesto).
        let resumen = instalar(args(&[
            "--install",
            wasm_path.to_str().unwrap(),
            "--id",
            "counter",
            "--name",
            "Counter",
            "--store",
            cas.to_str().unwrap(),
            "--apps-dir",
            apps.to_str().unwrap(),
        ]))
        .expect("instalar");
        assert!(resumen.contains("instalada «counter»"), "resumen: {resumen}");

        // El manifiesto existe y el registro lo descubre como un Launch::Wasm.
        let toml = std::fs::read_to_string(apps.join("counter.toml")).unwrap();
        let parsed = app_bus::parse_entry(&toml).expect("re-parsea el manifiesto");
        assert_eq!(parsed.id, "counter");
        assert_eq!(parsed.label, "Counter");
        let bytecode_hex = match &parsed.launch {
            app_bus::Launch::Wasm { bytecode_hex, grant_hex } => {
                assert!(grant_hex.is_none(), "sin concesión");
                bytecode_hex.clone()
            }
            otro => panic!("se esperaba Launch::Wasm, vino {otro:?}"),
        };

        // Lado consumidor: el mismo hash resuelve desde el CAS y carga.
        let spec = resolver_spec(args(&[
            "--hash",
            &bytecode_hex,
            "--store",
            cas.to_str().unwrap(),
        ]))
        .expect("lanzar lo instalado");
        assert_eq!(spec.wasm, COUNTER_WASM);
        WasmGuest::load(&spec.wasm, spec.permisos).expect("instanciar counter instalado");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn catalogo_publica_busca_y_corre_por_id() {
        let dir = cas_temporal("catalogo");
        let cas = dir.join("blobs");
        let apps = dir.join("apps");
        let catalogo = dir.join("catalog.bin");
        let wasm_path = dir.join("counter.wasm");
        std::fs::write(&wasm_path, COUNTER_WASM).unwrap();

        // Publicar al catálogo (--catalog).
        let resumen = instalar(args(&[
            "--install",
            wasm_path.to_str().unwrap(),
            "--id",
            "counter",
            "--name",
            "Contador",
            "--desc",
            "suma y resta un número",
            "--category",
            "demo",
            "--store",
            cas.to_str().unwrap(),
            "--apps-dir",
            apps.to_str().unwrap(),
            "--catalog",
            catalogo.to_str().unwrap(),
        ]))
        .expect("instalar+publicar");
        assert!(resumen.contains("catálogo"), "resumen: {resumen}");

        // Buscar por texto encuentra la app.
        let listado = buscar(args(&[
            "--search",
            "resta",
            "--catalog",
            catalogo.to_str().unwrap(),
        ]))
        .expect("buscar");
        assert!(listado.contains("counter"), "listado: {listado}");
        assert!(listado.contains("Contador"), "listado: {listado}");

        // Una búsqueda que no matchea.
        let vacio = buscar(args(&["--search", "zzz", "--catalog", catalogo.to_str().unwrap()]))
            .expect("buscar vacío");
        assert!(vacio.contains("sin coincidencias"), "vacío: {vacio}");

        // Correr por id resuelve desde el catálogo + CAS y produce el spec.
        let spec = spec_desde_catalogo(args(&[
            "--run",
            "counter",
            "--catalog",
            catalogo.to_str().unwrap(),
            "--store",
            cas.to_str().unwrap(),
        ]))
        .expect("correr por id");
        assert_eq!(spec.wasm, COUNTER_WASM);
        assert_eq!(spec.title, "Contador");
        WasmGuest::load(&spec.wasm, spec.permisos).expect("instanciar lo elegido");

        // Re-publicar (upsert) no duplica.
        instalar(args(&[
            "--install",
            wasm_path.to_str().unwrap(),
            "--id",
            "counter",
            "--name",
            "Contador v2",
            "--store",
            cas.to_str().unwrap(),
            "--apps-dir",
            apps.to_str().unwrap(),
            "--catalog",
            catalogo.to_str().unwrap(),
        ]))
        .expect("re-publicar");
        let cat = Catalog::deserializar(&std::fs::read(&catalogo).unwrap()).unwrap();
        assert_eq!(cat.entries.len(), 1, "upsert por id, no duplica");
        assert_eq!(cat.get("counter").unwrap().name, "Contador v2");

        // Un id ausente del catálogo es error.
        assert!(spec_desde_catalogo(args(&[
            "--run",
            "fantasma",
            "--catalog",
            catalogo.to_str().unwrap(),
        ]))
        .is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn registro_local_lista_ingiere_y_queda_corrible() {
        // Registro local offline (un dir con apps.json + .wasm) → listar →
        // ingerir al catálogo → correr por id. Todo sin red.
        let dir = cas_temporal("registro");
        std::fs::write(dir.join("counter.wasm"), COUNTER_WASM).unwrap();
        std::fs::write(
            dir.join("apps.json"),
            r#"{"results":[{"slug":"counter","title":"Contador","summary":"demo offline","tags":["demo"],"download":{"wasm":"counter.wasm"}}]}"#,
        )
        .unwrap();
        let ron = dir.join("reg.ron");
        std::fs::write(
            &ron,
            "#![enable(implicit_some)]\n(name:\"local\",list:(path:\"/apps.json\",list_path:\"results\",fields:(id:\"slug\",name:\"title\",wasm_url:\"download.wasm\",description:\"summary\",category:\"tags.0\")))",
        )
        .unwrap();
        let cas = dir.join("blobs");
        let catalogo = dir.join("catalog.bin");

        // Listar (sin --ingest) describe la app sin bajarla.
        let listado = registro(args(&[
            "--registry",
            ron.to_str().unwrap(),
            "--instance",
            dir.to_str().unwrap(),
        ]))
        .expect("listar registro");
        assert!(listado.contains("counter"), "listado: {listado}");
        assert!(!cas.exists(), "sin --ingest no baja nada");

        // Ingerir baja al CAS y suma al catálogo local.
        let res = registro(args(&[
            "--registry",
            ron.to_str().unwrap(),
            "--instance",
            dir.to_str().unwrap(),
            "--ingest",
            "--store",
            cas.to_str().unwrap(),
            "--catalog",
            catalogo.to_str().unwrap(),
        ]))
        .expect("ingerir");
        assert!(res.contains("ingeridas 1"), "res: {res}");

        // Y lo ingerido se corre por id desde el catálogo local.
        let spec = spec_desde_catalogo(args(&[
            "--run",
            "counter",
            "--catalog",
            catalogo.to_str().unwrap(),
            "--store",
            cas.to_str().unwrap(),
        ]))
        .expect("correr lo ingerido");
        assert_eq!(spec.wasm, COUNTER_WASM);
        WasmGuest::load(&spec.wasm, spec.permisos).expect("instanciar lo ingerido");

        std::fs::remove_dir_all(&dir).ok();
    }
}
