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
use minga_core::{parse, Attestation, ContentHash, Did, Keypair};
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
}

#[derive(Debug, Clone)]
pub struct IngestResult {
    pub hash: ContentHash,
    pub did: Did,
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
    })
}

/// `minga ingest <file>`: parsea un archivo Rust con tree-sitter,
/// inserta el AST en el store, lo añade al MST, y crea una atestación
/// firmada por el dueño del keypair (auto-firma de autoría).
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
    let hash = repo.nodes.put(&node)?;
    repo.mst.insert(hash)?;
    repo.attestations
        .add(Attestation::create(&keypair, hash))?;
    repo.flush()?;

    Ok(IngestResult {
        hash,
        did: keypair.did(),
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

/// Detecta el dialecto desde la extensión del archivo. Error si la
/// extensión no corresponde a un lenguaje soportado.
fn detect_dialect(file: &Path) -> Result<parse::Dialect, CliError> {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    parse::detect_by_extension(ext).ok_or_else(|| {
        CliError::UnsupportedLanguage {
            path: file.to_path_buf(),
            extension: ext.to_string(),
        }
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

/// `minga sync <multiaddr>`: dializa al peer y ejecuta una
/// sincronización completa con él.
pub async fn cmd_sync(
    repo_path: &Path,
    passphrase: &str,
    target: &str,
) -> Result<(), CliError> {
    let keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let peer = MingaPeer::open(keypair, repo_path.join(REPO_DIRNAME))?;

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
/// cualquier archivo `.rs` que se cree o modifique. Convierte Minga en
/// un VCS de fondo: el usuario escribe en su editor habitual y el
/// código queda versionado y firmado en el repo automáticamente.
pub async fn cmd_watch(
    repo_path: &Path,
    passphrase: &str,
    watch_dir: &Path,
) -> Result<(), CliError> {
    let keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    // Pasada inicial: ingerimos todos los .rs ya presentes para que
    // el repo arranque sincronizado con el contenido actual del
    // directorio (no solo con cambios futuros).
    initial_scan(&repo, &keypair, watch_dir);

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
        if !is_relevant_event(&event) {
            continue;
        }
        for path in &event.paths {
            if is_supported_source(path) {
                match ingest_into_repo(&repo, &keypair, path) {
                    Ok(hash) => {
                        eprintln!("ingerido: {} → {}", path.display(), hash);
                    }
                    Err(e) => {
                        eprintln!("warning: {} no se pudo ingerir: {}", path.display(), e);
                    }
                }
            }
        }
    }

    Ok(())
}

fn initial_scan(repo: &PersistentRepo, keypair: &Keypair, dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if is_supported_source(&p) {
            let _ = ingest_into_repo(repo, keypair, &p);
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
    let hash = repo.nodes.put(&node)?;
    repo.mst.insert(hash)?;
    repo.attestations
        .add(Attestation::create(keypair, hash))?;
    repo.flush()?;
    Ok(hash)
}

/// Detecta si un archivo debe ingerirse: existe, es regular, y su
/// extensión corresponde a un dialecto soportado.
fn is_supported_source(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    parse::detect_by_extension(ext).is_some()
}

fn is_relevant_event(event: &notify::Event) -> bool {
    matches!(
        event.kind,
        notify::EventKind::Create(_) | notify::EventKind::Modify(_)
    )
}
