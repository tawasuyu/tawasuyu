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
    // Anexamos al historial path → α para que `minga blame` pueda
    // atribuir cada línea actual al α que la introdujo. Errores de
    // canonicalize se tratan como "no hay historial" — la ingesta
    // funcional ya está completa.
    if let Some(path_key) = canonical_path_key(file) {
        let now_secs = unix_now_secs();
        let _ = repo.paths.append(&path_key, alpha, now_secs);
        let _ = repo.alpha_paths.record(alpha, &path_key, now_secs);
    }
    repo.flush()?;

    Ok(IngestResult {
        alpha,
        struct_hash,
        did: keypair.did(),
        dialect,
    })
}

/// Canonicaliza `path` y lo convierte a String para usar como clave
/// del historial. `None` si el archivo no existe o `canonicalize`
/// falla (filesystem inusual). El callsite debe degradar
/// silenciosamente — la ingesta principal no depende de esto.
fn canonical_path_key(path: &Path) -> Option<String> {
    let abs = fs::canonicalize(path).ok()?;
    Some(abs.to_string_lossy().into_owned())
}

pub(crate) fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
    let now_secs = unix_now_secs();
    repo.timestamps.put(&att.content, &att.author, now_secs)?;
    Ok((alpha, struct_hash))
}

/// Resultado de `cmd_sign`: confirmación con metadata sobre si la firma
/// es nueva (vouching genuino) o redundante (idempotencia).
#[derive(Debug, Clone)]
pub struct SignResult {
    pub alpha: ContentHash,
    pub author: Did,
    /// `false` si el repo ya tenía una atestación local de este author
    /// sobre `alpha` (re-firmar es idempotente, no se duplica entrada).
    /// `true` cuando esta es la primera vez que este DID firma esa raíz.
    pub is_new_attestation: bool,
    /// `true` si el α-hash está en el tree `roots`. Cuando es `false`,
    /// se firma igual — útil para vouching de fragmentos del CAS o de
    /// raíces que aún no llegaron por sync — pero el CLI lo avisa al
    /// usuario para que no firme algo desconocido por error de tipeo.
    pub is_known_root: bool,
}

/// `minga sign <α-hash>`: emite una atestación bajo el keypair local
/// sobre un α-hash existente. A diferencia de `ingest` (que firma como
/// efecto secundario de versionar contenido propio), `sign` es
/// **vouching explícito**: Alice ingiere, Bob sincroniza, Bob firma
/// con `sign` — la raíz queda con dos atestaciones independientes,
/// permitiendo "co-autoría" semántica o aval de revisores.
///
/// Re-firmar la misma raíz con el mismo keypair es idempotente:
/// `SledAttestationStore` indexa por `content || author`, así que la
/// segunda inserción reemplaza la primera con bytes idénticos.
pub fn cmd_sign(
    repo_path: &Path,
    passphrase: &str,
    hash_hex: &str,
) -> Result<SignResult, CliError> {
    let keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let alpha = parse_hash_hex(hash_hex)?;
    let did = keypair.did();

    // ¿Ya habíamos firmado este α con este DID? Sólo para reportar al
    // caller; la firma se emite igual y el store la deduplica sola.
    let already = repo
        .attestations
        .get(&alpha)?
        .iter()
        .any(|a| a.author == did);
    let is_known_root = repo.roots.contains(&alpha)?;

    let att = Attestation::create(&keypair, alpha);
    repo.attestations.add(att.clone())?;
    repo.timestamps
        .put(&att.content, &att.author, unix_now_secs())?;
    repo.flush()?;

    Ok(SignResult {
        alpha,
        author: did,
        is_new_attestation: !already,
        is_known_root,
    })
}

/// Una entrada de `cmd_signers`: quién firmó la raíz, cuándo, y si
/// también la retractó.
#[derive(Debug, Clone)]
pub struct SignerEntry {
    pub author: Did,
    /// Timestamp local de cuándo se observó la atestación. `0` si no
    /// hay timestamp (atestación vieja sin entry en
    /// `SledTimestampStore`).
    pub ts_secs: u64,
    /// `true` si el mismo `author` también firmó una `Retraction` sobre
    /// esta raíz — declara que avaló y luego revocó. La atestación
    /// original sigue presente como prueba histórica.
    pub retracted: bool,
}

/// `minga signers <α-hash>`: lista los DIDs que han atestado una raíz.
/// Complementa `cmd_sign` ofreciendo la vista "quién avaló esto" sin
/// pasar por `cmd_log` (que mezcla todas las raíces).
///
/// Salida ordenada por timestamp local descendente (más reciente
/// primero). Marca con `retracted = true` a los DIDs que también
/// emitieron una retracción — útil para visualizar cambios de postura
/// en la cadena de aval.
pub fn cmd_signers(
    repo_path: &Path,
    passphrase: &str,
    hash_hex: &str,
) -> Result<Vec<SignerEntry>, CliError> {
    use std::collections::HashSet;

    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let alpha = parse_hash_hex(hash_hex)?;

    let atts = repo.attestations.get(&alpha)?;
    let retractions = repo.retractions.get(&alpha)?;
    let retract_authors: HashSet<Did> = retractions.into_iter().map(|r| r.author).collect();

    let mut entries: Vec<SignerEntry> = atts
        .into_iter()
        .map(|a| {
            let ts = repo
                .timestamps
                .get(&a.content, &a.author)
                .ok()
                .flatten()
                .unwrap_or(0);
            SignerEntry {
                author: a.author,
                ts_secs: ts,
                retracted: retract_authors.contains(&a.author),
            }
        })
        .collect();

    entries.sort_by(|a, b| b.ts_secs.cmp(&a.ts_secs).then(a.author.0.cmp(&b.author.0)));
    Ok(entries)
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

pub(crate) fn parse_hash_hex(s: &str) -> Result<ContentHash, CliError> {
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

/// Una línea de `cmd_blame`: el texto tal como aparece en el archivo
/// actual, más el α-hash que la introdujo (la primera versión del
/// archivo que la contenía) con su timestamp y autor.
#[derive(Debug, Clone)]
pub struct BlameLine {
    pub text: String,
    pub alpha: ContentHash,
    pub ts_secs: u64,
    pub author: Did,
}

/// `minga blame <path>`: para cada línea del archivo actual, devuelve
/// el α-hash que la introdujo. Reconstruye la cadena de versiones del
/// path desde su historial, ejecuta diffs línea-a-línea entre versiones
/// consecutivas, y propaga la atribución hacia adelante: las líneas
/// nuevas en una versión se atribuyen a ella; las preservadas heredan
/// la atribución de la versión anterior.
///
/// Necesita que el path haya sido ingerido al menos una vez (vía
/// `minga ingest` o el `cmd_watch`). El archivo en disco se ignora —
/// la blame es contra la **última** versión registrada en el historial,
/// no contra la copia actual no-ingerida.
pub fn cmd_blame(
    repo_path: &Path,
    passphrase: &str,
    file: &Path,
) -> Result<Vec<BlameLine>, CliError> {
    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let path_key = canonical_path_key(file)
        .ok_or_else(|| CliError::PathNotIngested(file.to_path_buf()))?;
    let history = repo.paths.history(&path_key)?;
    if history.is_empty() {
        return Err(CliError::PathNotIngested(file.to_path_buf()));
    }

    // Atribución por línea para la versión "current" del walk. La
    // representamos como Vec<(line_text, attribution_index)>, donde
    // attribution_index apunta a una entrada de `history` (el α que
    // introdujo esa línea). Empezamos con la versión más vieja: todas
    // sus líneas se atribuyen a su propio α.
    let oldest_source = source_for_alpha(&repo, &history[0].0)?;
    let mut current_lines: Vec<(String, usize)> = oldest_source
        .lines()
        .map(|l| (l.to_string(), 0_usize))
        .collect();

    // Avanzamos por la historia: en cada paso, computamos diff entre
    // current_lines y la fuente de la siguiente versión, y construimos
    // la nueva lista de líneas atribuidas preservando attributions de
    // las que no cambiaron y asignando el nuevo α a las insertadas.
    for (idx, (alpha, _ts)) in history.iter().enumerate().skip(1) {
        let new_source = source_for_alpha(&repo, alpha)?;
        let current_text: String = current_lines
            .iter()
            .map(|(t, _)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let diff = similar::TextDiff::from_lines(&current_text, &new_source);
        let mut next: Vec<(String, usize)> = Vec::new();
        let mut old_idx = 0usize;
        for change in diff.iter_all_changes() {
            // `similar` agrega `\n` al final de cada línea — lo
            // quitamos para uniformidad con la entrada original
            // (que vino de .lines()).
            let mut text = change.to_string();
            if text.ends_with('\n') {
                text.pop();
            }
            match change.tag() {
                similar::ChangeTag::Equal => {
                    let attr = current_lines
                        .get(old_idx)
                        .map(|(_, a)| *a)
                        .unwrap_or(idx);
                    next.push((text, attr));
                    old_idx += 1;
                }
                similar::ChangeTag::Delete => {
                    old_idx += 1;
                }
                similar::ChangeTag::Insert => {
                    next.push((text, idx));
                }
            }
        }
        current_lines = next;
    }

    // Resolvemos cada attribution_index a su BlameLine completa.
    // Para `author`/`ts` consultamos la atestación local sobre el α.
    let mut out = Vec::with_capacity(current_lines.len());
    for (text, attr_idx) in current_lines {
        let (alpha, ts) = history[attr_idx];
        let author = first_author_for(&repo, &alpha).unwrap_or(Did([0u8; 32]));
        out.push(BlameLine {
            text,
            alpha,
            ts_secs: ts,
            author,
        });
    }
    Ok(out)
}

/// Reconstruye la fuente canónica del α-hash de una raíz. Resuelve
/// vía `roots` (α → struct), reconstruye y renderea.
fn source_for_alpha(
    repo: &PersistentRepo,
    alpha: &ContentHash,
) -> Result<String, CliError> {
    let (struct_hash, _, _) = resolve_hash(repo, *alpha)?;
    let node = repo
        .nodes
        .reconstruct(&struct_hash)?
        .ok_or(CliError::HashNotFound(struct_hash))?;
    Ok(minga_vfs::render_source(&node))
}

/// Devuelve el primer DID que firmó una atestación sobre `alpha`.
/// `None` si la raíz no tiene atestaciones registradas localmente.
fn first_author_for(repo: &PersistentRepo, alpha: &ContentHash) -> Option<Did> {
    let atts = repo.attestations.get(alpha).ok()?;
    atts.first().map(|a| a.author)
}

/// Una fila de `cmd_roots`: una raíz registrada en el repo con metadata
/// agregada de paths e historial de atestaciones.
#[derive(Debug, Clone)]
pub struct RootRow {
    pub alpha: ContentHash,
    pub struct_hash: ContentHash,
    pub dialect: Option<parse::Dialect>,
    /// Path local donde se ingirió este α por última vez. `None` si la
    /// raíz vino por sync sin pasar nunca por un path local, o si el
    /// historial path→α no la cubre.
    pub path: Option<String>,
    /// Timestamp Unix de la atestación más reciente sobre esta raíz
    /// (cualquier autor). `0` si no hay timestamps locales registrados.
    pub last_seen_secs: u64,
    /// Cuántas atestaciones distintas hay almacenadas localmente.
    pub attestations: usize,
}

/// `minga roots`: lista todas las raíces registradas en el repo con
/// path conocido, dialect, fecha de última atestación y cantidad de
/// firmas. Ordenado por `last_seen_secs` descendente — empate por
/// α-hash para estabilidad.
///
/// Cierra la asimetría histórica entre `status` (sólo da counts) y
/// `show <hash>` (que exige conocer el hash de antemano): permite
/// descubrir las raíces de un repo sin levantar el explorer Llimphi
/// ni el módulo shuma.
pub fn cmd_roots(repo_path: &Path, passphrase: &str) -> Result<Vec<RootRow>, CliError> {
    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let mut rows = Vec::new();
    for r in repo.roots.iter() {
        let (alpha, struct_hash, dialect) = r?;
        let atts = repo.attestations.get(&alpha)?;
        let attestations = atts.len();
        let last_seen_secs = atts
            .iter()
            .filter_map(|a| repo.timestamps.get(&a.content, &a.author).ok().flatten())
            .max()
            .unwrap_or(0);
        // Reverse-index persistente: lookup directo por prefijo α en
        // lugar de reconstruir el mapa en RAM. Ver
        // `minga_store::SledAlphaPathsStore`.
        let path = repo.alpha_paths.most_recent_path(&alpha)?;
        rows.push(RootRow {
            alpha,
            struct_hash,
            dialect,
            path,
            last_seen_secs,
            attestations,
        });
    }
    rows.sort_by(|a, b| {
        b.last_seen_secs
            .cmp(&a.last_seen_secs)
            .then(a.alpha.0.cmp(&b.alpha.0))
    });
    Ok(rows)
}

/// Una fila de `cmd_history`: una versión histórica de un path.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub alpha: ContentHash,
    pub ts_secs: u64,
    pub dialect: Option<parse::Dialect>,
    /// `true` si el α-hash actual del archivo en disco coincide con esta
    /// entrada. Si el archivo no existe o no se puede parsear hoy, todas
    /// las filas vienen con `current = false`.
    pub current: bool,
}

/// `minga history <path>`: dumpea el historial path→α (poblado por
/// `ingest`/`watch`) para `path`. Útil para ver cuándo cambió un
/// archivo sin reconstruir el blame completo (que ya requiere correr
/// diff línea-a-línea entre cada par de versiones consecutivas).
///
/// Salida cronológica **descendente** (la versión más reciente arriba),
/// igual que `cmd_log`. Si el archivo todavía existe en disco y su
/// α-hash actual coincide con alguna entrada, esa entrada lleva
/// `current = true`.
pub fn cmd_history(
    repo_path: &Path,
    passphrase: &str,
    file: &Path,
) -> Result<Vec<HistoryEntry>, CliError> {
    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let path_key = canonical_path_key(file)
        .ok_or_else(|| CliError::PathNotIngested(file.to_path_buf()))?;
    let history = repo.paths.history(&path_key)?;
    if history.is_empty() {
        return Err(CliError::PathNotIngested(file.to_path_buf()));
    }

    // α-hash del contenido actual en disco, si parseamos sin error.
    // Best-effort: si el archivo se movió o ya no parsea, todas las
    // filas salen sin el marcador `current`.
    let current_alpha = file
        .exists()
        .then(|| try_current_alpha(file))
        .flatten();

    let mut out = Vec::with_capacity(history.len());
    for (alpha, ts) in history {
        let dialect = repo.roots.get(&alpha)?.and_then(|(_, d)| d);
        let current = current_alpha.as_ref().map(|a| a == &alpha).unwrap_or(false);
        out.push(HistoryEntry {
            alpha,
            ts_secs: ts,
            dialect,
            current,
        });
    }
    // Más reciente primero, idéntica convención que `cmd_log`.
    out.reverse();
    Ok(out)
}

/// Best-effort: lee `file`, detecta dialect y devuelve su α-hash actual.
/// Cualquier error (io, dialect, parse) → `None` — el caller degrada
/// silenciosamente al modo "sin marcador current".
fn try_current_alpha(file: &Path) -> Option<ContentHash> {
    let source = fs::read_to_string(file).ok()?;
    let dialect = detect_dialect(file).ok()?;
    let node = dialect.parse(&source).ok()?;
    Some(hash_alpha_with(dialect, &node))
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

/// Resultado de `cmd_prune`: cuántos nodos había antes, cuántos
/// quedaron vivos (alcanzables desde alguna raíz), cuántos se borraron.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PruneStats {
    pub before: usize,
    pub alive: usize,
    pub removed: usize,
    pub roots: usize,
}

/// `minga prune`: mark-sweep del grafo CAS. Marca todos los nodos
/// alcanzables desde alguna raíz del tree `roots` (siguiendo los
/// `children` recursivamente) y borra del tree `nodes` los que no
/// quedaron marcados. Idempotente: correr dos veces seguidas no
/// elimina nada en la segunda pasada.
///
/// Las atestaciones, retracciones y timestamps quedan intactos —
/// referencian α-hashes (no struct-hashes) y son históricos.
pub fn cmd_prune(repo_path: &Path, passphrase: &str) -> Result<PruneStats, CliError> {
    use std::collections::HashSet;

    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let before = repo.nodes.len();

    // 1. Recolectar los struct-hashes raíz a partir de `roots`.
    let mut roots_set: HashSet<ContentHash> = HashSet::new();
    let mut roots_count = 0usize;
    for r in repo.roots.iter() {
        let (_alpha, struct_hash, _dialect) = r?;
        roots_set.insert(struct_hash);
        roots_count += 1;
    }

    // 2. Mark: BFS desde cada raíz por `children_of` (lee sólo los
    //    hashes, sin reconstruir el SemanticNode completo).
    let mut alive_set: HashSet<ContentHash> = HashSet::new();
    let mut frontier: Vec<ContentHash> = roots_set.into_iter().collect();
    while let Some(h) = frontier.pop() {
        if !alive_set.insert(h) {
            continue; // ya visitado
        }
        if let Some(children) = repo.nodes.children_of(&h)? {
            for c in children {
                if !alive_set.contains(&c) {
                    frontier.push(c);
                }
            }
        }
        // Si `children_of` devolvió None, el hash es huérfano (en el
        // tree `roots` pero NO en `nodes` — debería ser imposible bajo
        // ingest sano; pasamos de él en silencio).
    }
    let alive = alive_set.len();

    // 3. Sweep: borrar todo lo que no quedó vivo.
    let all_hashes: Vec<ContentHash> = repo
        .nodes
        .iter_hashes()
        .collect::<Result<Vec<_>, _>>()?;
    let mut removed = 0usize;
    for h in &all_hashes {
        if !alive_set.contains(h) {
            if repo.nodes.remove(h)? {
                removed += 1;
            }
        }
    }
    repo.flush()?;

    Ok(PruneStats {
        before,
        alive,
        removed,
        roots: roots_count,
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

    // Bootstrap del DHT: anuncia todas las raíces locales como
    // proveedores en Kademlia. Los peers que busquen el α-hash de un
    // archivo que tengamos podrán descubrirnos sin conocer nuestro
    // multiaddr de antemano (siempre que compartan al menos un peer
    // bootstrap de la malla `brahman-net`).
    let announced = peer.announce_all_roots().await;

    // Bloqueamos para siempre mientras la task de accept procesa
    // sincronizaciones. El usuario cierra con Ctrl+C.
    println!("Escuchando en: {}", actual);
    println!("DID Minga: {}", did);
    println!("PeerID libp2p: {}", peer.peer_id());
    if announced > 0 {
        println!("Anunciadas {} raíces en el DHT", announced);
    }
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

/// Estadísticas devueltas por `cmd_ingest_dir`: cuántos archivos
/// soportados se vieron, cuántos se ingirieron sin error, y la lista
/// de fallos para que el CLI los reporte.
#[derive(Debug, Clone)]
pub struct BulkIngestStats {
    /// Archivos que pasaron `is_supported_source` (extensión o shebang).
    pub seen: usize,
    /// De esos, cuántos terminaron en el grafo.
    pub ingested: usize,
    /// Fallos individuales — la ingesta sigue tras un error.
    pub failed: Vec<(std::path::PathBuf, String)>,
}

/// `minga ingest-dir <dir> [--recursive]`: ingiere todos los archivos
/// soportados de un directorio en una sola pasada. Es básicamente
/// `initial_scan` (el bootstrap interno de `watch`) expuesto como
/// one-shot, para versionar un repo entero sin dejar el watcher
/// corriendo.
///
/// En modo recursivo, **omite directorios ocultos** (los que empiezan
/// con `.`): evita pisar `.git`, `.minga`, `.venv` y similares — son
/// fuentes de ruido (archivos generados, repos anidados). Si necesitás
/// versionar un dot-dir explícitamente, llamalo con `--recursive` desde
/// dentro o pasalo como `dir` raíz.
pub fn cmd_ingest_dir(
    repo_path: &Path,
    passphrase: &str,
    dir: &Path,
    recursive: bool,
) -> Result<BulkIngestStats, CliError> {
    let keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let mut stats = BulkIngestStats {
        seen: 0,
        ingested: 0,
        failed: Vec::new(),
    };
    walk_and_ingest(&repo, &keypair, dir, recursive, &mut stats);
    repo.flush()?;
    Ok(stats)
}

fn walk_and_ingest(
    repo: &PersistentRepo,
    keypair: &Keypair,
    dir: &Path,
    recursive: bool,
    stats: &mut BulkIngestStats,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        // Saltar dot-dirs sólo en descenso recursivo. El directorio raíz
        // pasado por el usuario puede ser oculto y se respeta.
        if p.is_dir() {
            if recursive && !is_hidden_dirname(&p) {
                walk_and_ingest(repo, keypair, &p, recursive, stats);
            }
            continue;
        }
        if is_supported_source(&p) {
            stats.seen += 1;
            match ingest_into_repo(repo, keypair, &p) {
                Ok(_) => stats.ingested += 1,
                Err(e) => stats.failed.push((p, e.to_string())),
            }
        }
    }
}

/// `true` si el último componente del path empieza con `.`. Usamos esto
/// para podar descenso en `ingest-dir --recursive` y evitar `.git`,
/// `.minga`, `.venv`, etc.
fn is_hidden_dirname(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
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
    if let Some(path_key) = canonical_path_key(file) {
        let ts = unix_now_secs();
        let _ = repo.paths.append(&path_key, alpha, ts);
        let _ = repo.alpha_paths.record(alpha, &path_key, ts);
    }
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
