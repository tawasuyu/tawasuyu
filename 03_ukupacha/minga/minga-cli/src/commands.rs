//! Implementaciones de los subcomandos. Funciones puras que retornan
//! datos estructurados — el binario las llama y formatea la salida.
//!
//! Layout en disco bajo `repo_path/`:
//! - `keypair`     — la `Keypair` del peer cifrada con passphrase.
//! - `repo/`       — directorio sled con `nodes`, `attestations`, `mst`.

use std::fs;
use std::path::Path;
use std::time::Duration;

use libp2p::{multiaddr::Protocol, Multiaddr, PeerId};
use minga_core::{alpha::hash_alpha_with, parse, Attestation, ContentHash, Did, Keypair};
use minga_p2p::MingaPeer;
use minga_store::{keypair_file, PersistentRepo};

use crate::error::CliError;

pub const KEYPAIR_FILENAME: &str = "keypair";
pub const REPO_DIRNAME: &str = "repo";

#[derive(Debug, Clone)]
pub struct RepoStatus {
    pub did: Did,
    pub mst_len: usize,
    pub nodes_len: usize,
    pub attestations_len: usize,
    pub roots_len: usize,
}

#[derive(Debug, Clone)]
pub struct IngestResult {
    /// α-hash de la raíz: identidad del archivo, estable bajo
    /// renombrado de variables ligadas.
    pub alpha: ContentHash,
    /// Hash estructural de la raíz dentro del grafo CAS.
    pub struct_hash: ContentHash,
    pub did: Did,
    pub dialect: parse::Dialect,
}

/// `minga init`: genera un keypair fresco, crea el repo persistente,
/// y guarda el keypair cifrado.
pub fn cmd_init(repo_path: &Path, passphrase: &str) -> Result<Did, CliError> {
    if repo_path.exists() {
        // Si el directorio existe pero está vacío, lo aceptamos.
        // Si tiene cualquier cosa, abortamos para no pisar un repo.
        let mut entries = fs::read_dir(repo_path)?;
        if entries.next().is_some() {
            return Err(CliError::AlreadyExists(repo_path.to_path_buf()));
        }
    } else {
        fs::create_dir_all(repo_path)?;
    }

    let keypair = Keypair::generate();
    keypair_file::save(&keypair, repo_path.join(KEYPAIR_FILENAME), passphrase)?;

    // Crear el repo sled vacío. Se cierra al final del scope; el
    // siguiente comando lo reabre.
    let _ = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    Ok(keypair.did())
}

/// `minga status`: descifra el keypair, abre el repo, devuelve
/// estadísticas básicas.
pub fn cmd_status(repo_path: &Path, passphrase: &str) -> Result<RepoStatus, CliError> {
    let keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    Ok(RepoStatus {
        did: keypair.did(),
        mst_len: repo.mst.len(),
        nodes_len: repo.nodes.len(),
        attestations_len: repo.attestations.len(),
        roots_len: repo.roots.len(),
    })
}

/// `minga ingest <file>`: parsea el archivo con tree-sitter, inserta
/// el AST en el grafo CAS, calcula su α-hash (estable bajo renombrado
/// de variables ligadas), lo registra como raíz del MST y crea una
/// atestación firmada por el dueño del keypair (auto-firma de autoría).
pub fn cmd_ingest(
    repo_path: &Path,
    passphrase: &str,
    file: &Path,
) -> Result<IngestResult, CliError> {
    let keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let source = fs::read_to_string(file)?;
    let dialect = detect_dialect(file)?;
    let node = dialect.parse(&source)?;
    let (alpha, struct_hash) = ingest_node_alpha(&repo, &keypair, dialect, &node)?;
    repo.flush()?;

    Ok(IngestResult {
        alpha,
        struct_hash,
        did: keypair.did(),
        dialect,
    })
}

/// Ingiere un nodo ya parseado: lo desempaqueta en el grafo CAS,
/// calcula su α-hash, lo registra como raíz y atesta autoría. Compartido
/// entre `cmd_ingest` y el bucle interno de `cmd_watch`.
fn ingest_node_alpha(
    repo: &PersistentRepo,
    keypair: &Keypair,
    dialect: parse::Dialect,
    node: &minga_core::SemanticNode,
) -> Result<(ContentHash, ContentHash), CliError> {
    let struct_hash = repo.nodes.put(node)?;
    let alpha = hash_alpha_with(dialect, node);
    repo.roots.put(alpha, struct_hash, dialect)?;
    repo.mst.insert(alpha)?;
    let att = Attestation::create(keypair, alpha);
    repo.attestations.add(att.clone())?;
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    repo.timestamps.put(&att.content, &att.author, now_secs)?;
    Ok((alpha, struct_hash))
}

/// Una entrada del log: atestación + timestamp de cuándo se observó.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub alpha: ContentHash,
    pub struct_hash: Option<ContentHash>,
    pub dialect: Option<parse::Dialect>,
    pub author: Did,
    pub ts_secs: u64,
    /// Si `true`, esta entrada coincide con el archivo señalado por el
    /// caller (vía `cmd_log` con `Some(path)`). Sólo se calcula cuando
    /// hay path: en `cmd_log(None)` siempre es `false`.
    pub current: bool,
}

/// `minga log [path]`: enumera las atestaciones del repo ordenadas por
/// timestamp descendente. Si `path` es `Some`, computa su α-hash actual
/// y marca la entrada coincidente con `current = true`.
pub fn cmd_log(
    repo_path: &Path,
    passphrase: &str,
    path: Option<&Path>,
) -> Result<Vec<LogEntry>, CliError> {
    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let current_alpha = match path {
        Some(p) => {
            let source = fs::read_to_string(p)?;
            let dialect = detect_dialect(p)?;
            let node = dialect.parse(&source)?;
            Some(hash_alpha_with(dialect, &node))
        }
        None => None,
    };

    let mut entries: Vec<LogEntry> = Vec::new();
    for att in repo.attestations.iter() {
        let att = att?;
        let ts = repo
            .timestamps
            .get(&att.content, &att.author)?
            .unwrap_or(0);
        let (struct_hash, dialect) = match repo.roots.get(&att.content)? {
            Some((sh, dl)) => (Some(sh), dl),
            None => (None, None),
        };
        let current = current_alpha
            .as_ref()
            .map(|a| a == &att.content)
            .unwrap_or(false);
        entries.push(LogEntry {
            alpha: att.content,
            struct_hash,
            dialect,
            author: att.author,
            ts_secs: ts,
            current,
        });
    }
    // Más recientes primero. Empate por hash para orden estable.
    entries.sort_by(|a, b| b.ts_secs.cmp(&a.ts_secs).then(a.alpha.0.cmp(&b.alpha.0)));
    Ok(entries)
}

/// Resultado de `cmd_show`: la fuente reconstruida (forma canónica) o
/// el árbol como S-expression, según `mode`.
#[derive(Debug, Clone)]
pub struct ShowResult {
    pub alpha: Option<ContentHash>,
    pub struct_hash: ContentHash,
    pub dialect: Option<parse::Dialect>,
    /// `true` si el `hash` recibido era un α-hash (raíz registrada).
    /// `false` si era un hash estructural directo.
    pub is_root: bool,
    pub rendered: String,
}

/// `minga show <hash>`: pinta el contenido del nodo identificado por
/// `hash`. Acepta α-hashes (raíces) y hashes estructurales del grafo
/// interno. `as_sexp = true` devuelve el árbol literal del store; `false`
/// (default) devuelve el código fuente reconstruido en forma canónica.
pub fn cmd_show(
    repo_path: &Path,
    passphrase: &str,
    hash_hex: &str,
    as_sexp: bool,
) -> Result<ShowResult, CliError> {
    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let hash = parse_hash_hex(hash_hex)?;

    // ¿Es α-hash de una raíz registrada?
    let (alpha, struct_hash, dialect, is_root) = match repo.roots.get(&hash)? {
        Some((sh, dl)) => (Some(hash), sh, dl, true),
        None => (None, hash, None, false),
    };

    let stored_node = repo
        .nodes
        .reconstruct(&struct_hash)?
        .ok_or(CliError::HashNotFound(struct_hash))?;

    let rendered = if as_sexp {
        minga_vfs::render_sexp(&stored_node)
    } else {
        minga_vfs::render_source(&stored_node)
    };

    Ok(ShowResult {
        alpha,
        struct_hash,
        dialect,
        is_root,
        rendered,
    })
}

fn parse_hash_hex(s: &str) -> Result<ContentHash, CliError> {
    let bytes = hex_decode_32(s).ok_or(CliError::InvalidHash(s.to_string()))?;
    Ok(ContentHash(bytes))
}

fn hex_decode_32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = nibble(chunk[0])?;
        let lo = nibble(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// `minga mount <punto>`: monta el repositorio como un filesystem FUSE
/// de sólo lectura. Cada hash del store se vuelve un archivo
/// navegable con `ls`/`cat`. Bloquea hasta que se desmonte el punto
/// (`fusermount -u <punto>` o una señal al proceso).
pub fn cmd_mount(
    repo_path: &Path,
    passphrase: &str,
    mountpoint: &Path,
) -> Result<(), CliError> {
    // Cargar el keypair valida la passphrase: montar es navegar el
    // repo, así que pedimos la misma credencial que `status`/`watch`.
    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;
    minga_vfs::mount(minga_vfs::RepoSource::new(repo), mountpoint)?;
    Ok(())
}

/// Detecta el dialecto del archivo. Prueba primero por **extensión**
/// (rápido, sin abrir el archivo); si no matchea, lee la primera línea
/// e intenta por **shebang** (cubre scripts sin extensión como `bin/foo`).
/// Error si ninguno de los dos métodos identifica un dialecto soportado.
fn detect_dialect(file: &Path) -> Result<parse::Dialect, CliError> {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if let Some(d) = parse::detect_by_extension(ext) {
        return Ok(d);
    }
    // Fallback: leer sólo la primera línea para el shebang.
    if let Ok(f) = fs::File::open(file) {
        use std::io::{BufRead, BufReader};
        let mut first = String::new();
        if BufReader::new(f).read_line(&mut first).is_ok() {
            if let Some(d) = parse::detect_by_shebang(&first) {
                return Ok(d);
            }
        }
    }
    Err(CliError::UnsupportedLanguage {
        path: file.to_path_buf(),
        extension: ext.to_string(),
    })
}

/// `minga listen <addr>`: arranca el peer, escucha en `addr`, y
/// acepta sincronizaciones entrantes hasta que el proceso se cierre.
pub async fn cmd_listen(
    repo_path: &Path,
    passphrase: &str,
    addr: &str,
) -> Result<Multiaddr, CliError> {
    let keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let did = keypair.did();
    let peer = MingaPeer::open(keypair, repo_path.join(REPO_DIRNAME))?;
    let multi: Multiaddr = addr
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| CliError::Multiaddr(e.to_string()))?;
    let actual = peer.listen(multi).await;
    let _accept = peer.run_passive_accept();

    // Bloqueamos para siempre mientras la task de accept procesa
    // sincronizaciones. El usuario cierra con Ctrl+C.
    println!("Escuchando en: {}", actual);
    println!("DID Minga: {}", did);
    println!("PeerID libp2p: {}", peer.peer_id());
    futures::future::pending::<()>().await;

    Ok(actual)
}

/// `minga sync <target>`: dializa al peer y ejecuta una sincronización
/// completa con él. `target` puede ser:
/// - un **multiaddr libp2p** con `/p2p/<peer_id>` — se conecta directo;
/// - un **α-hash en hex** (64 caracteres) — busca proveedores en el DHT
///   (vía [`minga_p2p::MingaPeer::find_providers`]) y sincroniza con el
///   primero que responda. Hay que tener al menos un peer bootstrap
///   conocido (vía `add_dht_peer`) o el lookup no devuelve resultados.
pub async fn cmd_sync(
    repo_path: &Path,
    passphrase: &str,
    target: &str,
) -> Result<(), CliError> {
    let keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let peer = MingaPeer::open(keypair, repo_path.join(REPO_DIRNAME))?;

    // ¿Es un α-hash hex? Si lo es, vamos por la rama DHT.
    if target.len() == 64 && target.chars().all(|c| c.is_ascii_hexdigit()) {
        let hash = parse_hash_hex(target)?;
        let providers = peer.find_providers(hash).await;
        if providers.is_empty() {
            return Err(CliError::NoProvidersForHash(hash));
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        for pid in providers {
            if peer.sync_with(pid).await.is_ok() {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(CliError::SyncTimeout);
            }
        }
        return Err(CliError::SyncTimeout);
    }

    // Rama multiaddr clásica.
    let multi: Multiaddr = target
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| CliError::Multiaddr(e.to_string()))?;
    let peer_id = extract_peer_id(&multi).ok_or(CliError::NoPeerIdInMultiaddr)?;

    peer.dial(multi);

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if peer.sync_with(peer_id).await.is_ok() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(CliError::SyncTimeout);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn extract_peer_id(addr: &Multiaddr) -> Option<PeerId> {
    addr.iter().find_map(|p| match p {
        Protocol::P2p(peer_id) => Some(peer_id),
        _ => None,
    })
}

/// `minga watch <dir>`: vigila un directorio, re-parsea y re-ingesta
/// cualquier archivo soportado que se cree o modifique. Si un archivo
/// se borra, retira su última raíz del MST y de `roots`. Los nodos del
/// grafo CAS NO se eliminan (pueden estar compartidos con otras raíces).
pub async fn cmd_watch(
    repo_path: &Path,
    passphrase: &str,
    watch_dir: &Path,
) -> Result<(), CliError> {
    use std::collections::HashMap;

    let keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    // Tracker en memoria: path → α-hash más reciente ingerido en esta
    // sesión. Necesario para resolver el "remove": cuando notify nos
    // dice "este path desapareció" sabemos cuál hash retirar.
    let mut path_to_alpha: HashMap<std::path::PathBuf, ContentHash> = HashMap::new();

    // Pasada inicial: ingerimos todos los archivos soportados ya
    // presentes y registramos su α-hash en el tracker.
    initial_scan(&repo, &keypair, watch_dir, &mut path_to_alpha);

    // Canal entre el callback síncrono de notify y el bucle async.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        },
    )?;
    notify::Watcher::watch(&mut watcher, watch_dir, notify::RecursiveMode::Recursive)?;

    while let Some(event) = rx.recv().await {
        match event.kind {
            notify::EventKind::Create(_) | notify::EventKind::Modify(_) => {
                for path in &event.paths {
                    if is_supported_source(path) {
                        match ingest_into_repo(&repo, &keypair, path) {
                            Ok(hash) => {
                                path_to_alpha.insert(path.clone(), hash);
                                eprintln!("ingerido: {} → {}", path.display(), hash);
                            }
                            Err(e) => {
                                eprintln!(
                                    "warning: {} no se pudo ingerir: {}",
                                    path.display(),
                                    e
                                );
                            }
                        }
                    }
                }
            }
            notify::EventKind::Remove(_) => {
                for path in &event.paths {
                    if let Some(hash) = path_to_alpha.remove(path) {
                        match retire_root(&repo, &hash) {
                            Ok(()) => eprintln!("retirado: {} (era {})", path.display(), hash),
                            Err(e) => eprintln!(
                                "warning: no se pudo retirar {}: {}",
                                path.display(),
                                e
                            ),
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn initial_scan(
    repo: &PersistentRepo,
    keypair: &Keypair,
    dir: &Path,
    tracker: &mut std::collections::HashMap<std::path::PathBuf, ContentHash>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if is_supported_source(&p) {
            if let Ok(hash) = ingest_into_repo(repo, keypair, &p) {
                tracker.insert(p, hash);
            }
        }
    }
}

fn ingest_into_repo(
    repo: &PersistentRepo,
    keypair: &Keypair,
    file: &Path,
) -> Result<ContentHash, CliError> {
    let source = fs::read_to_string(file)?;
    let dialect = detect_dialect(file)?;
    let node = dialect.parse(&source)?;
    let (alpha, _struct_hash) = ingest_node_alpha(repo, keypair, dialect, &node)?;
    repo.flush()?;
    Ok(alpha)
}

/// Retira una raíz del MST y del tree `roots`. **No** borra los nodos
/// del grafo CAS — quedan disponibles para `cas/<hash>` y por si otra
/// raíz los referencia. Las atestaciones tampoco se borran: registran
/// que el contenido existió en algún momento.
fn retire_root(repo: &PersistentRepo, alpha: &ContentHash) -> Result<(), CliError> {
    repo.mst.remove(alpha)?;
    repo.roots.remove(alpha)?;
    repo.flush()?;
    Ok(())
}

/// Detecta si un archivo debe ingerirse: existe, es regular, y o bien
/// su extensión corresponde a un dialecto soportado, o bien su primera
/// línea tiene un shebang reconocible. `watch` se apoya en esto para
/// no llamar a `read_to_string` sobre archivos ajenos.
fn is_supported_source(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if parse::detect_by_extension(ext).is_some() {
        return true;
    }
    if let Ok(f) = fs::File::open(path) {
        use std::io::{BufRead, BufReader};
        let mut first = String::new();
        if BufReader::new(f).read_line(&mut first).is_ok() {
            return parse::detect_by_shebang(&first).is_some();
        }
    }
    false
}
