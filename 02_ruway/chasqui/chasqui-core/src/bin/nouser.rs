//! `chasqui` CLI — explorador de Mónadas.
//!
//! Subcomandos:
//!
//! - `scan <dir>`        recorre `dir` y muestra las Mónadas detectadas.
//! - `show <dir> <id?>`  scan + detalles de la Mónada con prefijo de ID.
//! - `json <dir>`        scan + dump JSON con los manifests.
//!
//! Phase A: in-memory, sin persistencia, sin brahman sidecar. La
//! sesión termina y todo se descarta. Phase B agrega persistencia y
//! presencia ante el Init.

use std::path::PathBuf;
use std::process::ExitCode;

use chasqui_core::{
    cluster, db, embed,
    scanner::{self, ScanConfig},
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let prog = args.first().cloned().unwrap_or_else(|| "chasqui".into());
    let sub = match args.get(1).map(String::as_str) {
        Some(s) => s,
        None => {
            print_usage(&prog);
            return ExitCode::from(2);
        }
    };
    let rest = &args[2..];

    let result = match sub {
        "scan" => cmd_scan(rest),
        "show" => cmd_show(rest),
        "json" => cmd_json(rest),
        "daemon" => cmd_daemon(rest),
        "attract" => cmd_attract(rest),
        "--help" | "-h" | "help" => {
            print_usage(&prog);
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("chasqui: comando desconocido '{other}'");
            print_usage(&prog);
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("chasqui: {e}");
            ExitCode::from(1)
        }
    }
}

fn print_usage(prog: &str) {
    eprintln!("uso: {prog} <comando> [args]");
    eprintln!();
    eprintln!("comandos:");
    eprintln!("  scan <dir>           recorre un directorio y lista las Mónadas detectadas");
    eprintln!("  show <dir> <prefix>  scan + detalle de la Mónada cuyo ID empieza con <prefix>");
    eprintln!("  json <dir>           scan + dump JSON de todos los manifests");
    eprintln!("  daemon <dir>         scan + sidecarea cada Mónada al Init brahman");
    eprintln!("  attract <dir> <file> dado un archivo, qué Mónada del scan lo atrae más");
    eprintln!();
    eprintln!("env:");
    eprintln!("  NOUSER_MIN_FILES         mínimo de archivos por Mónada (default: 3)");
    eprintln!("  NOUSER_DB_PATH           si está set, abre sled en esa ruta (persistencia)");
    eprintln!("  BRAHMAN_INIT_SOCKET      socket del Init (heredado de brahman-handshake)");
}

type Cmd = Result<(), Box<dyn std::error::Error>>;

fn min_files() -> usize {
    std::env::var("NOUSER_MIN_FILES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(cluster::DEFAULT_MIN_FILES_PER_MONAD)
}

fn require_dir(args: &[String]) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = args.first().ok_or("falta argumento <dir>")?;
    Ok(PathBuf::from(dir))
}

fn run_scan(dir: &PathBuf) -> Result<(db::MonadDb, usize), Box<dyn std::error::Error>> {
    let files = scanner::scan_directory(dir, &ScanConfig::default())?;
    let n_files = files.len();
    let monads = cluster::by_directory(&files, min_files());
    let mut db = open_db()?;
    db.ingest_files(files);
    db.replace_monads(monads);
    Ok((db, n_files))
}

/// Abre el `MonadDb`. Si `NOUSER_DB_PATH` está set, persistencia sled;
/// si no, store en memoria.
fn open_db() -> Result<db::MonadDb, Box<dyn std::error::Error>> {
    if let Ok(path) = std::env::var("NOUSER_DB_PATH") {
        Ok(db::MonadDb::open(&path)?)
    } else {
        Ok(db::MonadDb::new())
    }
}

fn cmd_scan(args: &[String]) -> Cmd {
    let dir = require_dir(args)?;
    let (db, n_files) = run_scan(&dir)?;

    println!(
        "scan: {} archivos en {}, {} mónadas (min_files={})",
        n_files,
        dir.display(),
        db.monad_count(),
        min_files()
    );
    if db.monad_count() == 0 {
        println!("  (ninguna Mónada — bajá NOUSER_MIN_FILES o apuntá a un dir con más archivos)");
        return Ok(());
    }
    println!();
    for m in db.monads() {
        let id_short = format!("{}", m.id);
        let id_short = &id_short[..8];
        println!(
            "  [{}]  {:30}  card={}  ent={:.2}  lens={:?}",
            id_short, m.label, m.cardinality, m.entropy, m.dominant_lens,
        );
        if !m.keywords.is_empty() {
            println!("           keywords: {}", m.keywords.join(", "));
        }
    }
    Ok(())
}

fn cmd_show(args: &[String]) -> Cmd {
    let dir = require_dir(args)?;
    let prefix = args.get(1).ok_or("falta argumento <prefix>")?;
    let (db, _) = run_scan(&dir)?;

    let m = db
        .monads()
        .find(|m| m.id.to_string().starts_with(prefix))
        .ok_or_else(|| format!("ninguna Mónada con prefijo '{prefix}'"))?;

    println!("Monad {}", m.id);
    println!("  label:       {}", m.label);
    println!("  summary:     {}", m.summary);
    println!("  cardinality: {}", m.cardinality);
    println!("  entropy:     {:.4}", m.entropy);
    println!("  lens:        {:?}", m.dominant_lens);
    println!("  keywords:    {}", m.keywords.join(", "));
    println!("  members ({}):", m.members.len());
    for f in db.resolve_members(m.id) {
        println!(
            "    {:>10} bytes  {}",
            f.size,
            f.path.display()
        );
    }
    Ok(())
}

fn cmd_json(args: &[String]) -> Cmd {
    let dir = require_dir(args)?;
    let (db, _) = run_scan(&dir)?;
    let manifests: Vec<_> = db.monads().cloned().collect();
    println!("{}", serde_json::to_string_pretty(&manifests)?);
    Ok(())
}

fn cmd_daemon(args: &[String]) -> Cmd {
    let dir = require_dir(args)?;

    let pool = std::sync::Arc::new(
        brahman_sidecar::SidecarPool::new().map_err(|e| format!("crear pool: {e}"))?,
    );

    // 1. Decidir el path del query socket ANTES de armar el engine
    //    Card (porque viaja como service_socket en la Card).
    let query_socket = chasqui_card::query::transport::default_socket_path();

    // 2. Engine como Ente. Declara service_socket + flow.output para
    //    que el broker pueda emitir MatchEvent::Available a consumers
    //    interesados en `flow.input = monad-list:json`.
    let engine_card = build_engine_card(query_socket.clone());
    let engine_id = engine_card.id;
    let engine_label = engine_card.label.clone();
    eprintln!(
        "chasqui daemon: publicando engine '{}' (kind=Ente, id={}, socket={})",
        engine_label,
        engine_id,
        query_socket.display()
    );
    pool.spawn(engine_card);

    // 2. Hidratación: si NOUSER_DB_PATH apunta a un sled poblado,
    //    publicar lo que ya tenemos ANTES del re-scan. brahman-status
    //    ve mónadas reales en milisegundos, no en segundos.
    let mut db = open_db()?;
    let prior_count = db.monad_count();
    if prior_count > 0 {
        let mut hydrated = 0usize;
        let mut skipped_model = 0usize;
        for monad in db.monads() {
            // Sólo publicamos centroides del modelo actual; los demás
            // son data muerta hasta que el re-scan los reemplace.
            let valid = monad
                .centroid_model
                .as_deref()
                .map(|id| id == embed::MODEL_ID)
                .unwrap_or(false);
            if !valid {
                skipped_model += 1;
                continue;
            }
            let mut card = monad.to_brahman_card();
            card.references.push(brahman_card::CardReference {
                kind: brahman_card::RelationshipKind::OwnedBy,
                target_id: engine_id,
                target_label: engine_label.clone(),
            });
            pool.spawn(card);
            hydrated += 1;
        }
        eprintln!(
            "chasqui daemon: hidratadas {} mónadas previas{} en O(1)",
            hydrated,
            if skipped_model > 0 {
                format!(" ({} dropeadas por centroid_model distinto)", skipped_model)
            } else {
                String::new()
            }
        );
    }

    // 3. Re-scan con hidratación: las Mónadas con mismo path_hint
    //    reusan id, así que NO generamos sesiones duplicadas para los
    //    mismos directorios — el sidecar previo ya tiene esa identidad.
    let files = scanner::scan_directory(&dir, &scanner::ScanConfig::default())?;
    let n_files = files.len();
    let monads = cluster::by_directory_hydrated(&files, min_files(), Some(&db));
    let scanned_count = monads.len();
    eprintln!(
        "chasqui daemon: re-scan {} archivos en {} → {} mónadas",
        n_files,
        dir.display(),
        scanned_count
    );

    // Publicamos sólo las Mónadas NUEVAS (las que no estaban en la
    // hidratación inicial). El criterio: si el id estaba en la DB
    // previa, el sidecar de la hidratación ya cubre esa identidad.
    let prior_ids: std::collections::BTreeSet<_> = db.monads().map(|m| m.id).collect();
    let mut newly_spawned = 0usize;
    for monad in &monads {
        if prior_ids.contains(&monad.id) {
            continue; // ya publicada en hidratación
        }
        let mut card = monad.to_brahman_card();
        card.references.push(brahman_card::CardReference {
            kind: brahman_card::RelationshipKind::OwnedBy,
            target_id: engine_id,
            target_label: engine_label.clone(),
        });
        pool.spawn(card);
        newly_spawned += 1;
    }

    // Reescribimos la DB con el set actual (idempotente para los
    // hidratados; reemplazo para los nuevos).
    db.ingest_files(files);
    db.replace_monads(monads);

    eprintln!(
        "chasqui daemon: 1 ente + {} mónadas vivas ({} nuevas vs hidratación)",
        scanned_count, newly_spawned
    );

    // Engine query socket: bind antes del watcher para que cualquier
    // consumer descubierto vía broker pueda consultarnos enseguida.
    // Si el bind falla, seguimos sin él — la UI degrada a "no
    // alcanzable" pero el daemon sigue procesando cambios.
    let db_shared = std::sync::Arc::new(std::sync::Mutex::new(db));
    let _query_listener = match chasqui_core::engine_socket::spawn_listener(
        chasqui_core::engine_socket::ListenerConfig {
            socket_path: query_socket.clone(),
            engine_id,
            engine_label: engine_label.clone(),
            watching: Some(dir.clone()),
        },
        db_shared.clone(),
    ) {
        Ok(h) => {
            eprintln!(
                "chasqui daemon: query socket activo en {} (proto: chasqui_card::query)",
                query_socket.display()
            );
            Some(h)
        }
        Err(e) => {
            eprintln!(
                "chasqui daemon: query socket NO disponible ({e}) — explorer no podrá consultar"
            );
            None
        }
    };

    // Watcher: cada cambio en el árbol — coalescido con debounce de
    // 150ms — dispara un re-scan + re-cluster del directorio y
    // re-publica al broker las Mónadas afectadas (drop + spawn por id,
    // gracias al replace en `SidecarPool::spawn`).
    let _watcher = match spawn_fs_watcher(
        dir.clone(),
        db_shared.clone(),
        pool.clone(),
        engine_id,
        engine_label.clone(),
    ) {
        Ok(w) => {
            eprintln!(
                "chasqui daemon: watcher activo en {} (debounce 150ms, re-publish on) — Ctrl-C para terminar.",
                dir.display()
            );
            Some(w)
        }
        Err(e) => {
            eprintln!(
                "chasqui daemon: watcher deshabilitado ({e}) — Ctrl-C para terminar."
            );
            None
        }
    };

    std::thread::park();
    drop(_watcher);
    drop(_query_listener);
    let _ = std::fs::remove_file(&query_socket); // best-effort cleanup
    drop(pool);
    Ok(())
}

/// Ventana de debounce: notify dispara Create+Modify(+) por cada
/// edición; sin coalescer veríamos N reacciones por un solo `:w`.
/// 150ms es generoso para editores típicos (vim/code) y mantiene el
/// feedback "vivo" para el usuario.
const WATCHER_DEBOUNCE_MS: u64 = 150;

/// Watcher de filesystem con debounce + re-publish al broker.
///
/// Pipeline:
///
/// 1. **notify** dispara eventos crudos a un canal interno.
/// 2. **dispatcher**: filtra a Create/Modify/Remove de paths bajo
///    `dir`, descarta el resto, reenvía al canal de debounce.
/// 3. **coordinator**: mantiene un `HashMap<PathBuf, Instant>`.
///    Cada vez que el canal queda en silencio durante
///    `WATCHER_DEBOUNCE_MS`, agrupa los paths cuya última actividad
///    superó la ventana y los procesa en **un solo batch**.
/// 4. **process_change_batch**: re-scan + re-cluster hidratado +
///    diff vs DB + `pool.drop_session` para Mónadas desaparecidas
///    + `pool.spawn` para Mónadas nuevas o con composición distinta.
///    `pool.spawn` reemplaza la sesión previa con el mismo `Card.id`,
///    así que el broker ve el manifest fresco sin sesiones huérfanas.
fn spawn_fs_watcher(
    dir: std::path::PathBuf,
    db: std::sync::Arc<std::sync::Mutex<db::MonadDb>>,
    pool: std::sync::Arc<brahman_sidecar::SidecarPool>,
    engine_id: brahman_card::ulid::Ulid,
    engine_label: String,
) -> Result<notify::RecommendedWatcher, Box<dyn std::error::Error>> {
    use notify::{Event, EventKind, RecursiveMode, Watcher};

    let (notify_tx, notify_rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = notify_tx.send(res);
    })?;
    watcher.watch(&dir, RecursiveMode::Recursive)?;

    let (path_tx, path_rx) = std::sync::mpsc::channel::<std::path::PathBuf>();

    // Dispatcher: notify → filtro → canal de paths.
    let dispatch_dir = dir.clone();
    std::thread::Builder::new()
        .name("chasqui-watcher-dispatch".into())
        .spawn(move || {
            for res in notify_rx {
                let event = match res {
                    Ok(e) => e,
                    Err(e) => {
                        eprintln!("[watcher] error: {e}");
                        continue;
                    }
                };
                // Create/Modify viven; Remove también nos importa
                // (puede colapsar Mónadas).
                let interesting = matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                );
                if !interesting {
                    continue;
                }
                for path in event.paths {
                    if !path.starts_with(&dispatch_dir) {
                        continue;
                    }
                    let _ = path_tx.send(path);
                }
            }
        })?;

    // Coordinator: debounce + batch dispatch.
    let coord_dir = dir;
    std::thread::Builder::new()
        .name("chasqui-watcher-coord".into())
        .spawn(move || {
            let debounce = std::time::Duration::from_millis(WATCHER_DEBOUNCE_MS);
            let mut pending: std::collections::HashMap<
                std::path::PathBuf,
                std::time::Instant,
            > = std::collections::HashMap::new();
            loop {
                match path_rx.recv_timeout(debounce) {
                    Ok(path) => {
                        pending.insert(path, std::time::Instant::now());
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
                let now = std::time::Instant::now();
                let due: Vec<std::path::PathBuf> = pending
                    .iter()
                    .filter(|(_, t)| now.duration_since(**t) >= debounce)
                    .map(|(p, _)| p.clone())
                    .collect();
                if due.is_empty() {
                    continue;
                }
                for p in &due {
                    pending.remove(p);
                }
                process_change_batch(
                    &due,
                    &coord_dir,
                    &db,
                    &pool,
                    engine_id,
                    &engine_label,
                );
            }
        })?;

    Ok(watcher)
}

/// Procesa un batch de paths cambiados: re-scanea el árbol, re-clusteriza
/// con hidratación, y propaga el delta de Mónadas al broker.
///
/// El re-scan global es deliberado: el clustering por directorio es global
/// por diseño, así que un cambio en `src/foo.rs` puede mover Mónadas en
/// `src/` sin tocar `tests/`. Coste O(N archivos), aceptable para
/// directorios típicos (<10k archivos). Optimizar a re-cluster parcial
/// cuando duela.
fn process_change_batch(
    paths: &[std::path::PathBuf],
    dir: &std::path::Path,
    db: &std::sync::Arc<std::sync::Mutex<db::MonadDb>>,
    pool: &std::sync::Arc<brahman_sidecar::SidecarPool>,
    engine_id: brahman_card::ulid::Ulid,
    engine_label: &str,
) {
    eprintln!(
        "[watcher] ⚙ batch: {} path(s) coalescidos → re-scan",
        paths.len()
    );

    let files = match scanner::scan_directory(dir, &scanner::ScanConfig::default()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[watcher] re-scan falló: {e}");
            return;
        }
    };

    let mut db_lock = match db.lock() {
        Ok(g) => g,
        Err(_) => {
            eprintln!("[watcher] mutex envenenado — abortando batch");
            return;
        }
    };

    let prior_monads: Vec<chasqui_card::MonadManifest> = db_lock.monads().cloned().collect();
    let prior_ref: &db::MonadDb = &db_lock;
    let monads = cluster::by_directory_hydrated(&files, min_files(), Some(prior_ref));

    let prior_ids: std::collections::BTreeSet<_> =
        prior_monads.iter().map(|m| m.id).collect();
    let new_ids: std::collections::BTreeSet<_> = monads.iter().map(|m| m.id).collect();

    // Mónadas que ya no existen (directorio quedó por debajo de
    // min_files o fue removido): cerramos su sesión en el broker.
    let mut removed = 0usize;
    for id in prior_ids.difference(&new_ids) {
        pool.drop_session(*id);
        removed += 1;
        if let Some(prev) = prior_monads.iter().find(|m| &m.id == id) {
            eprintln!(
                "[watcher] ✖ {} ({}) desapareció — sesión cerrada",
                &id.to_string()[..8],
                prev.label
            );
        }
    }

    // Mónadas nuevas o cuya composición cambió (members/centroid):
    // (re)spawn — el pool reemplaza la sesión previa con el mismo id.
    let mut respawned = 0usize;
    let mut fresh = 0usize;
    for monad in &monads {
        let prev = prior_monads.iter().find(|m| m.id == monad.id);
        let is_new = prev.is_none();
        let changed = match prev {
            Some(p) => p.members != monad.members || p.centroid != monad.centroid,
            None => true,
        };
        if !changed {
            continue;
        }
        let mut card = monad.to_brahman_card();
        card.references.push(brahman_card::CardReference {
            kind: brahman_card::RelationshipKind::OwnedBy,
            target_id: engine_id,
            target_label: engine_label.to_string(),
        });
        pool.spawn(card);
        if is_new {
            fresh += 1;
            eprintln!(
                "[watcher] ✦ {} nace ({} miembros, lens={:?})",
                monad.label, monad.cardinality, monad.dominant_lens
            );
        } else {
            respawned += 1;
            let prev = prev.unwrap();
            let delta_members = monad.members.len() as i64 - prev.members.len() as i64;
            eprintln!(
                "[watcher] ↻ {} refresh ({} miembros, Δ={:+})",
                monad.label, monad.cardinality, delta_members
            );
        }
    }

    if removed == 0 && fresh == 0 && respawned == 0 {
        eprintln!("[watcher] (sin cambios estructurales tras re-cluster)");
    } else {
        eprintln!(
            "[watcher] ⌃ delta: {} nuevas, {} refrescadas, {} cerradas — {} sesiones vivas",
            fresh,
            respawned,
            removed,
            pool.live_sessions()
        );
    }

    db_lock.ingest_files(files);
    db_lock.replace_monads(monads);
}

fn cmd_attract(args: &[String]) -> Cmd {
    let mut remote = false;
    let mut positional: Vec<&String> = Vec::new();
    for a in args {
        if a == "--remote" {
            remote = true;
        } else {
            positional.push(a);
        }
    }
    let dir = positional
        .first()
        .map(|s| std::path::PathBuf::from(s.as_str()))
        .ok_or("falta argumento <dir>")?;
    let file_path = positional.get(1).ok_or("falta argumento <file>")?;
    let file_path = std::path::PathBuf::from(file_path.as_str());
    if !file_path.exists() {
        return Err(format!("archivo no existe: {}", file_path.display()).into());
    }

    let (db, _) = run_scan(&dir)?;

    // Construimos un FileEntry para el archivo objetivo.
    let metadata = std::fs::metadata(&file_path)?;
    let mtime_ms = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let target = chasqui_card::FileEntry {
        id: chasqui_card::FileId::from(chasqui_card::ulid::Ulid::new()),
        path: file_path.clone(),
        content_hash: None,
        size: metadata.len(),
        mtime_ms,
        extension: file_path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase()),
    };

    // Embedding del target + identificación del modelo que lo produjo.
    // Local: pseudo-32d. Remote: lo que devuelva el provider electo
    // (mock=pseudo-32d, real=fastembed-384d).
    let (target_vec, target_model, source) = if remote {
        let (v, model) = remote_embed(&target)?;
        (v, model, "remote")
    } else {
        (
            embed::embed(&target).to_vec(),
            embed::MODEL_ID.to_string(),
            "local",
        )
    };

    // Filtramos Mónadas cuyo centroid_model NO matchee. Mezclar
    // 32-d con 384-d daría scores sin sentido (diferente semántica
    // y cosine no compara cross-modelo).
    let mut ranked: Vec<(&chasqui_card::MonadManifest, f32)> = db
        .monads()
        .filter(|m| !m.centroid.is_empty())
        .filter(|m| match &m.centroid_model {
            Some(id) => id == &target_model,
            None => true, // legacy sin tag — comparamos best-effort
        })
        .map(|m| (m, embed::attraction_score(&target_vec, m)))
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let total_monads = db.monads().filter(|m| !m.centroid.is_empty()).count();
    let skipped = total_monads - ranked.len();

    if ranked.is_empty() {
        println!("ninguna Mónada con centroide en {}", dir.display());
        return Ok(());
    }

    println!("archivo:   {}", file_path.display());
    println!("scan dir:  {}", dir.display());
    println!("embed:     {} ({})", source, target_model);
    if skipped > 0 {
        println!(
            "skipped:   {} mónada(s) con centroid_model distinto (no comparables)",
            skipped
        );
    }
    println!("ranking de atracción (cosine similarity):");
    println!();
    for (i, (m, score)) in ranked.iter().take(5).enumerate() {
        let marker = if *score >= embed::DEFAULT_ATTRACTION_THRESHOLD && i == 0 {
            "🧲"
        } else if i == 0 {
            "·"
        } else {
            " "
        };
        let id_short = format!("{}", m.id);
        let id_short = &id_short[..8];
        println!(
            "  {}  {:.4}  [{}]  {:30}  ({})",
            marker, score, id_short, m.label, m.summary
        );
    }
    if ranked[0].1 < embed::DEFAULT_ATTRACTION_THRESHOLD {
        println!();
        println!(
            "  (mejor score {:.4} < umbral {:.4} — el archivo no se 'pega' a ninguna)",
            ranked[0].1,
            embed::DEFAULT_ATTRACTION_THRESHOLD
        );
    }
    Ok(())
}

/// Pipeline completo del modo `--remote`:
/// 1. Si `NOUSER_NOUS_SOCKET` está set, lo usa directo (override
///    explícito, atajo para tests).
/// 2. Si no, delega en `brahman_sidecar::await_provider_blocking` —
///    el sidecar se conecta al broker, registra un consumer Card con
///    `flow.input = embed-result:json`, espera el primer
///    `MatchEvent::Available` y devuelve el socket. Esto activa la
///    lógica de `priority_contexts`: bajo `BRAHMAN_BROKER_CONTEXT=test/prod`,
///    el proveedor electo cambia sin que este código toque nada.
/// 3. Con el socket resuelto, dispara la RPC `EmbedFile`.
///
/// Devuelve `(embedding, model_id)` — el caller necesita ambos para
/// comparar contra centroides taggeados con su mismo `centroid_model`.
fn remote_embed(
    file: &chasqui_card::FileEntry,
) -> Result<(Vec<f32>, String), Box<dyn std::error::Error>> {
    if let Ok(explicit) = std::env::var("NOUSER_NOUS_SOCKET") {
        let sock = std::path::PathBuf::from(explicit);
        return embed_via(&sock, file);
    }

    let consumer = brahman_sidecar::build_consumer_card(
        "chasqui.attract-cli",
        chasqui_nous::FLOW_EMBED_RESULT,
        chasqui_nous::FLOW_TYPE_NAME,
    );
    let producer_sock = brahman_sidecar::await_provider_blocking(
        consumer,
        std::time::Duration::from_secs(3),
    )?;
    embed_via(&producer_sock, file)
}

/// RPC blocking contra un socket chasqui-nous concreto. Devuelve
/// `(embedding, model_id)` — el `model_id` viaja en la response y
/// permite al caller saber qué centroides son comparables.
fn embed_via(
    sock_path: &std::path::Path,
    file: &chasqui_card::FileEntry,
) -> Result<(Vec<f32>, String), Box<dyn std::error::Error>> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    if !sock_path.exists() {
        return Err(format!("socket no existe: {}", sock_path.display()).into());
    }

    let mut stream = UnixStream::connect(sock_path)?;
    let req = chasqui_nous::EmbedRequest {
        kind: chasqui_nous::RequestKind::EmbedFile,
        payload: serde_json::to_value(chasqui_nous::EmbedFilePayload {
            path: file.path.display().to_string(),
            extension: file.extension.clone(),
            size: file.size,
            mtime_ms: file.mtime_ms,
        })?,
    };
    let line = serde_json::to_string(&req)?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    if response.is_empty() {
        return Err("chasqui-nous cerró sin respuesta".into());
    }

    if let Ok(resp) = serde_json::from_str::<chasqui_nous::EmbedResponse>(&response) {
        return Ok((resp.embedding, resp.model));
    }
    let err: chasqui_nous::ErrorResponse = serde_json::from_str(&response)?;
    Err(format!("chasqui-nous: {}", err.error).into())
}

/// Card del propio engine (kind=Ente). Es el "ser" que produce y
/// administra Mónadas; aparece en brahman-status junto a sus Mónadas.
///
/// Declara `service_socket` y `flow.output = monad-list:json` para
/// que un consumer (UI, CLI) pueda descubrir al daemon vía broker
/// MatchEvent y consultarle por sus Mónadas sin pasar por
/// brahman-admin.
fn build_engine_card(service_socket: std::path::PathBuf) -> brahman_card::Card {
    use brahman_card::{Card, CardKind, Flow, Flows, Lifecycle, Payload, Priority, Supervision, TypeRef};
    use chasqui_card::query::{FLOW_MONAD_LIST, FLOW_TYPE_NAME};

    Card {
        payload: Payload::Virtual,
        supervision: Supervision::Delegate,
        lifecycle: Lifecycle::Daemon,
        priority: Priority::Normal,
        kind: CardKind::Ente,
        service_socket: Some(service_socket),
        flow: Flows {
            input: vec![],
            output: vec![Flow {
                name: FLOW_MONAD_LIST.into(),
                ty: TypeRef::Primitive {
                    name: FLOW_TYPE_NAME.into(),
                },
                pin_to: None,
            }],
        },
        ..Card::new("brahman.nouser_engine")
    }
}
