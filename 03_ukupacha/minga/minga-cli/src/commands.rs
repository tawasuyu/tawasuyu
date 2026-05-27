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
use minga_core::{
    alpha::hash_alpha_with, parse, Attestation, ContentHash, Did, Keypair, Retraction,
};
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

/// Resuelve un hash que puede ser α (raíz) o struct (nodo interno) al
/// struct-hash con el cual lookupar en `nodes`. Si es raíz, también
/// retorna el dialect persistido.
fn resolve_hash(
    repo: &PersistentRepo,
    hash: ContentHash,
) -> Result<(ContentHash, Option<parse::Dialect>, bool), CliError> {
    match repo.roots.get(&hash)? {
        Some((sh, dl)) => Ok((sh, dl, true)),
        None => Ok((hash, None, false)),
    }
}

/// Una línea del diff. `Same` se preserva tal cual; `Add`/`Remove` se
/// marcan con `+`/`-` en la salida unified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLine {
    Same(String),
    Add(String),
    Remove(String),
}

/// Resultado de `cmd_diff`: metadata de ambos lados + líneas resultantes.
#[derive(Debug, Clone)]
pub struct DiffResult {
    pub left_hash: ContentHash,
    pub right_hash: ContentHash,
    pub left_dialect: Option<parse::Dialect>,
    pub right_dialect: Option<parse::Dialect>,
    pub left_is_root: bool,
    pub right_is_root: bool,
    pub lines: Vec<DiffLine>,
    /// Cuántas líneas son `Add`.
    pub additions: usize,
    /// Cuántas líneas son `Remove`.
    pub deletions: usize,
}

/// `minga diff <left> <right>`: reconstruye ambos nodos y emite el
/// diff unified entre sus `render_source`. Acepta α-hashes (raíces) y
/// hashes estructurales.
pub fn cmd_diff(
    repo_path: &Path,
    passphrase: &str,
    left_hex: &str,
    right_hex: &str,
) -> Result<DiffResult, CliError> {
    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let left_hash = parse_hash_hex(left_hex)?;
    let right_hash = parse_hash_hex(right_hex)?;

    let (left_struct, left_dialect, left_is_root) = resolve_hash(&repo, left_hash)?;
    let (right_struct, right_dialect, right_is_root) = resolve_hash(&repo, right_hash)?;

    let left_node = repo
        .nodes
        .reconstruct(&left_struct)?
        .ok_or(CliError::HashNotFound(left_struct))?;
    let right_node = repo
        .nodes
        .reconstruct(&right_struct)?
        .ok_or(CliError::HashNotFound(right_struct))?;

    let left_text = minga_vfs::render_source(&left_node);
    let right_text = minga_vfs::render_source(&right_node);

    let diff = similar::TextDiff::from_lines(&left_text, &right_text);
    let mut lines = Vec::new();
    let mut additions = 0;
    let mut deletions = 0;
    for change in diff.iter_all_changes() {
        let text = change.to_string();
        // `similar` incluye el `\n` final en cada línea — lo conservamos
        // para que el caller pueda imprimir sin reflowing.
        match change.tag() {
            similar::ChangeTag::Equal => lines.push(DiffLine::Same(text)),
            similar::ChangeTag::Insert => {
                additions += 1;
                lines.push(DiffLine::Add(text));
            }
            similar::ChangeTag::Delete => {
                deletions += 1;
                lines.push(DiffLine::Remove(text));
            }
        }
    }

    Ok(DiffResult {
        left_hash,
        right_hash,
        left_dialect,
        right_dialect,
        left_is_root,
        right_is_root,
        lines,
        additions,
        deletions,
    })
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

/// Resultado de `cmd_verify_root`: si la raíz es consistente y bajo
/// qué dialecto (independiente del que figure en `roots`).
#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub alpha: ContentHash,
    pub struct_hash: ContentHash,
    /// Dialecto registrado en el tree `roots` al momento de ingerir.
    /// `None` si la raíz no estaba registrada (caso "huérfano":
    /// puede pasar tras sync si el wire no transmite el binding).
    pub stored_dialect: Option<parse::Dialect>,
    /// Dialecto bajo el cual `hash_alpha_with(d, &node) == alpha`.
    /// `None` significa **inconsistente** — el α-hash claimado no
    /// se corresponde con el contenido del nodo bajo ningún
    /// dialecto soportado.
    pub verified_dialect: Option<parse::Dialect>,
}

impl VerifyResult {
    pub fn is_consistent(&self) -> bool {
        self.verified_dialect.is_some()
    }

    pub fn matches_stored(&self) -> bool {
        match (self.stored_dialect, self.verified_dialect) {
            (Some(a), Some(b)) => a == b,
            (None, Some(_)) => true, // sin info previa, lo verificado es lo bueno
            _ => false,
        }
    }
}

/// `minga verify <hash>`: reconstruye el nodo al que apunta `hash`
/// (asumido α-hash de una raíz) y verifica que algún dialect produce
/// ese hash sobre ese contenido. Útil para auditar repos sincronizados
/// donde el remitente no es 100 % confiable.
pub fn cmd_verify_root(
    repo_path: &Path,
    passphrase: &str,
    hash_hex: &str,
) -> Result<VerifyResult, CliError> {
    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let alpha = parse_hash_hex(hash_hex)?;
    let (struct_hash, stored_dialect, _is_root) = resolve_hash(&repo, alpha)?;

    let node = repo
        .nodes
        .reconstruct(&struct_hash)?
        .ok_or(CliError::HashNotFound(struct_hash))?;

    let verified_dialect = minga_core::alpha::verify_root_alpha(&node, &alpha);

    Ok(VerifyResult {
        alpha,
        struct_hash,
        stored_dialect,
        verified_dialect,
    })
}

/// Resultado de `cmd_retire`: confirmación con metadata.
#[derive(Debug, Clone)]
pub struct RetireResult {
    pub alpha: ContentHash,
    pub author: Did,
    /// `true` si la raíz existía en el MST antes de la retracción
    /// (caso esperado). `false` si el hash no era una raíz conocida
    /// (la retracción se firma igual — funciona como "declaración de
    /// no autoría" útil para sync, aunque el efecto local es nulo).
    pub was_root: bool,
}

/// `minga retire <hash>`: emite una atestación negativa firmada
/// declarando que el dueño del keypair ya no respalda `hash`. Quita la
/// entrada del MST y de `roots`, persiste la retracción en su tree
/// propio. Las atestaciones originales NO se borran (siguen como
/// prueba histórica de que en algún momento se firmaron).
pub fn cmd_retire(
    repo_path: &Path,
    passphrase: &str,
    hash_hex: &str,
) -> Result<RetireResult, CliError> {
    let keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let alpha = parse_hash_hex(hash_hex)?;
    let was_root = repo.roots.contains(&alpha)?;

    let retraction = Retraction::create(&keypair, alpha);
    repo.retractions.add(retraction)?;

    // Quitar del MST y de roots (los nodos del CAS quedan: pueden estar
    // referenciados por otras raíces o ser navegables vía cas/<hash>).
    repo.mst.remove(&alpha)?;
    repo.roots.remove(&alpha)?;
    repo.flush()?;

    Ok(RetireResult {
        alpha,
        author: keypair.did(),
        was_root,
    })
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

/// Detecta el dialecto del archivo en tres pasos cada vez más caros:
/// 1. **Extensión** (`.rs`, `.py`, …) — sin abrir el archivo.
/// 2. **Shebang** (primera línea) — un read_line.
/// 3. **Contenido** — parsea con cada gramática y elige la que produce
///    el AST con menos errores. Para esto sí leemos el archivo entero.
///
/// Error si los tres pasos fallan.
fn detect_dialect(file: &Path) -> Result<parse::Dialect, CliError> {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if let Some(d) = parse::detect_by_extension(ext) {
        return Ok(d);
    }
    // Shebang: leer sólo la primera línea.
    if let Ok(f) = fs::File::open(file) {
        use std::io::{BufRead, BufReader};
        let mut first = String::new();
        if BufReader::new(f).read_line(&mut first).is_ok() {
            if let Some(d) = parse::detect_by_shebang(&first) {
                return Ok(d);
            }
        }
    }
    // Fallback caro: leer el contenido y probar cada parser.
    if let Ok(source) = fs::read_to_string(file) {
        if let Some(d) = parse::detect_by_content(&source) {
            return Ok(d);
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
