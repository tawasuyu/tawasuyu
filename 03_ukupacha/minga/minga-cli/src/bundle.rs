//! Bundle: empaquetado offline de una raíz para transferencia sin red.
//!
//! Un `BundleV1` agrupa todo lo necesario para que un peer que recibe
//! el archivo pueda reconstruir e integrar la raíz sin contactar al
//! emisor:
//! - el α-hash, struct_hash y dialect (binding RootDeclaration);
//! - todos los `StoredNode`s alcanzables desde el struct_hash;
//! - todas las atestaciones firmadas sobre el α-hash;
//! - todas las retracciones firmadas sobre el α-hash.
//!
//! Se serializa con postcard — el mismo codec del wire de sync — y se
//! escribe a un archivo. El receptor verifica criptográficamente cada
//! pieza antes de mergear:
//! - cada `StoredNode` entra por `put_chunked` (re-hashea y compara);
//! - el α-hash se re-deriva de la raíz reconstruida bajo el dialect
//!   declarado y se compara contra el claimado en el bundle;
//! - cada atestación / retracción re-verifica su firma Ed25519 en
//!   `add()` antes de persistirse.
//!
//! Es el equivalente "USB-stick" al wire libp2p: misma garantía de
//! integridad, distinto transporte.

use std::path::Path;

use minga_core::{
    alpha::hash_alpha_with, hash_stored, parse, Attestation, ContentHash, Retraction, StoredNode,
};
use minga_store::{keypair_file, PersistentRepo};

use crate::commands::{unix_now_secs, KEYPAIR_FILENAME, REPO_DIRNAME};
use crate::error::CliError;

/// Versión actual del formato. Se serializa explícitamente para que el
/// importador pueda rechazar (o adaptar) bundles de otra época.
pub const BUNDLE_VERSION: u32 = 1;

/// Magic prefix de un multi-bundle (varias raíces empacadas juntas).
/// Si los primeros 4 bytes del archivo coinciden, el cuerpo restante es
/// un `BundleMultiV1` postcard; si no, el archivo entero es un
/// `BundleV1` clásico — esto preserva compat con bundles existentes
/// sin tocar su layout.
pub const MULTI_MAGIC: &[u8; 4] = b"MNGM";
/// Variante zstd-comprimida del multi-bundle. El cuerpo tras el magic
/// es un stream zstd que, descomprimido, produce exactamente el postcard
/// de `BundleMultiV1`. El export nuevo siempre escribe esta variante; el
/// import detecta `MNGM` (legacy, sin compresión) y `MNGZ` (comprimido)
/// transparentemente — repos viejos siguen pudiendo importar y leer.
pub const MULTI_MAGIC_ZSTD: &[u8; 4] = b"MNGZ";
pub const MULTI_VERSION: u32 = 1;

/// Nivel de compresión zstd para el export. 3 es el default del crate
/// (rápido, ratio decente). Subir a 19+ exprime hasta 30 % extra pero
/// triplica el tiempo de export — no vale para un caso "dump completo
/// del repo a USB".
const ZSTD_LEVEL: i32 = 3;

/// El bundle serializable. El layout es estable: cualquier cambio de
/// campos sube `BUNDLE_VERSION` y agrega una rama en `import`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BundleV1 {
    pub version: u32,
    pub alpha: ContentHash,
    pub struct_hash: ContentHash,
    /// Dialect serializado como byte para no atar el formato a una
    /// variante específica de `Dialect`. Si el importador es más viejo
    /// y no reconoce este byte, falla con `UnknownDialect` sin tocar
    /// los stores.
    pub dialect_byte: u8,
    /// Todos los `StoredNode`s alcanzables desde `struct_hash` (DAG).
    /// El orden es BFS por el emisor — el importador no lo necesita
    /// (deduplica por hash) pero conservarlo facilita debug.
    pub nodes: Vec<StoredNode>,
    pub attestations: Vec<Attestation>,
    pub retractions: Vec<Retraction>,
}

/// Estadísticas devueltas por `cmd_bundle_export`.
#[derive(Debug, Clone, Copy)]
pub struct BundleExportStats {
    pub alpha: ContentHash,
    pub nodes: usize,
    pub attestations: usize,
    pub retractions: usize,
    pub bytes: usize,
}

/// Wrapper de múltiples bundles en un solo archivo. Se serializa con
/// postcard precedido del prefijo `MULTI_MAGIC` para que el importador
/// pueda distinguirlo de un `BundleV1` plano.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BundleMultiV1 {
    pub version: u32,
    pub items: Vec<BundleV1>,
}

/// Estadísticas devueltas por `cmd_bundle_export_all`.
#[derive(Debug, Clone)]
pub struct BundleExportAllStats {
    /// Raíces efectivamente empacadas.
    pub roots: usize,
    /// Raíces saltadas por falta de dialect persistido (typically
    /// raíces sincronizadas bajo el wire pre-`RootDeclaration`).
    pub skipped_missing_dialect: Vec<ContentHash>,
    pub total_nodes: usize,
    pub total_attestations: usize,
    pub total_retractions: usize,
    /// Tamaño final del archivo en disco (post-compresión).
    pub bytes: usize,
    /// Tamaño del postcard plano antes de comprimir — útil para ver el
    /// ratio cuando interesa.
    pub uncompressed_bytes: usize,
}

/// Estadísticas agregadas del import de un multi-bundle.
#[derive(Debug, Clone)]
pub struct BundleImportAllStats {
    /// Resultado individual de cada raíz en el orden en que vino dentro
    /// del multi-bundle.
    pub items: Vec<BundleImportStats>,
}

impl BundleImportAllStats {
    pub fn roots_new(&self) -> usize {
        self.items.iter().filter(|s| s.root_was_new).count()
    }
    pub fn total_nodes_inserted(&self) -> usize {
        self.items.iter().map(|s| s.nodes_inserted).sum()
    }
    pub fn total_attestations_added(&self) -> usize {
        self.items.iter().map(|s| s.attestations_added).sum()
    }
    pub fn total_retractions_added(&self) -> usize {
        self.items.iter().map(|s| s.retractions_added).sum()
    }
}

/// `minga bundle export <α-hash> <out>`: serializa la raíz, todos los
/// nodos alcanzables, atestaciones y retractions en un archivo
/// postcard. Errores:
/// - `HashNotFound` si el α-hash no es una raíz local;
/// - `BundleMissingDialect` si la raíz fue sincronizada bajo el wire
///   viejo (pre-`RootDeclaration`) y no tiene dialect persistido — sin
///   dialect el receptor no puede re-verificar el α-hash.
pub fn cmd_bundle_export(
    repo_path: &Path,
    passphrase: &str,
    hash_hex: &str,
    out: &Path,
) -> Result<BundleExportStats, CliError> {
    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let alpha = crate::commands::parse_hash_hex(hash_hex)?;
    let bundle = build_bundle_for_root(&repo, alpha)?;

    let bytes = postcard::to_allocvec(&bundle).map_err(|_| CliError::InvalidBundle)?;
    std::fs::write(out, &bytes)?;

    Ok(BundleExportStats {
        alpha: bundle.alpha,
        nodes: bundle.nodes.len(),
        attestations: bundle.attestations.len(),
        retractions: bundle.retractions.len(),
        bytes: bytes.len(),
    })
}

/// Empaqueta todas las raíces del repo en un solo archivo (multi-bundle).
/// Raíces sin dialect persistido (sync'd bajo el wire viejo) se saltan y
/// se reportan en `skipped_missing_dialect` — el caller decide si eso es
/// fatal o se acepta como degradación.
pub fn cmd_bundle_export_all(
    repo_path: &Path,
    passphrase: &str,
    out: &Path,
) -> Result<BundleExportAllStats, CliError> {
    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let mut items: Vec<BundleV1> = Vec::new();
    let mut skipped: Vec<ContentHash> = Vec::new();
    for entry in repo.roots.iter() {
        let (alpha, _struct_hash, dialect_opt) = entry?;
        if dialect_opt.is_none() {
            skipped.push(alpha);
            continue;
        }
        items.push(build_bundle_for_root(&repo, alpha)?);
    }

    let total_nodes: usize = items.iter().map(|b| b.nodes.len()).sum();
    let total_attestations: usize = items.iter().map(|b| b.attestations.len()).sum();
    let total_retractions: usize = items.iter().map(|b| b.retractions.len()).sum();
    let roots = items.len();

    let multi = BundleMultiV1 {
        version: MULTI_VERSION,
        items,
    };
    let body = postcard::to_allocvec(&multi).map_err(|_| CliError::InvalidBundle)?;
    let uncompressed_bytes = body.len();
    let compressed = zstd::encode_all(body.as_slice(), ZSTD_LEVEL).map_err(CliError::Io)?;

    let mut bytes = Vec::with_capacity(MULTI_MAGIC_ZSTD.len() + compressed.len());
    bytes.extend_from_slice(MULTI_MAGIC_ZSTD);
    bytes.extend_from_slice(&compressed);
    std::fs::write(out, &bytes)?;

    Ok(BundleExportAllStats {
        roots,
        skipped_missing_dialect: skipped,
        total_nodes,
        total_attestations,
        total_retractions,
        bytes: bytes.len(),
        uncompressed_bytes,
    })
}

/// Construye un `BundleV1` para una raíz registrada en `roots`. Es el
/// core compartido entre el export single y el export-all: BFS por el
/// DAG estructural + agrega atestaciones y retracciones de esa α.
fn build_bundle_for_root(
    repo: &PersistentRepo,
    alpha: ContentHash,
) -> Result<BundleV1, CliError> {
    use std::collections::HashSet;

    let (struct_hash, dialect_opt) = repo
        .roots
        .get(&alpha)?
        .ok_or(CliError::HashNotFound(alpha))?;
    let dialect = dialect_opt.ok_or(CliError::BundleMissingDialect(alpha))?;

    // BFS por el DAG de StoredNodes desde la raíz estructural. Los
    // hashes se dedupean en `visited`; el orden en `nodes` es estable
    // por iteración pero no esencial — el importador no lo asume.
    let mut visited: HashSet<ContentHash> = HashSet::new();
    let mut nodes: Vec<StoredNode> = Vec::new();
    let mut frontier: Vec<ContentHash> = vec![struct_hash];
    while let Some(h) = frontier.pop() {
        if !visited.insert(h) {
            continue;
        }
        let stored = repo
            .nodes
            .get(&h)?
            .ok_or(CliError::HashNotFound(h))?;
        for c in &stored.children {
            if !visited.contains(c) {
                frontier.push(*c);
            }
        }
        nodes.push(stored);
    }

    let attestations = repo.attestations.get(&alpha)?;
    let retractions = repo.retractions.get(&alpha)?;

    Ok(BundleV1 {
        version: BUNDLE_VERSION,
        alpha,
        struct_hash,
        dialect_byte: dialect.as_byte(),
        nodes,
        attestations,
        retractions,
    })
}

/// Estadísticas devueltas por `cmd_bundle_import`.
#[derive(Debug, Clone, Copy)]
pub struct BundleImportStats {
    pub alpha: ContentHash,
    /// `StoredNode`s recién agregados al `nodes` tree (los que ya
    /// estaban se omiten silenciosamente).
    pub nodes_inserted: usize,
    /// Atestaciones nuevas (no había una para `(content, author)`).
    pub attestations_added: usize,
    /// Atestaciones cuya firma falló — descartadas sin tocar el store.
    pub attestations_rejected: usize,
    pub retractions_added: usize,
    pub retractions_rejected: usize,
    /// `true` cuando la raíz quedó registrada en `roots`/`mst` (i.e.
    /// no estaba ya). `false` para imports idempotentes.
    pub root_was_new: bool,
}

/// `minga bundle import <archivo>`: deserializa un bundle, verifica
/// criptográficamente cada pieza, y mergea idempotentemente en los
/// stores locales. Si algo falla (versión incompatible, dialect
/// desconocido, α-hash inconsistente, postcard malformado), aborta
/// sin haber tocado nada — los `StoredNode`s pasan por `put_chunked`
/// que verifica el hash, pero el tree es append-only así que reintentos
/// son seguros.
pub fn cmd_bundle_import(
    repo_path: &Path,
    passphrase: &str,
    in_path: &Path,
) -> Result<BundleImportStats, CliError> {
    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let bytes = std::fs::read(in_path)?;
    if is_multi_bundle_magic(&bytes) {
        return Err(CliError::ExpectedSingleBundle);
    }
    let bundle: BundleV1 = postcard::from_bytes(&bytes).map_err(|_| CliError::InvalidBundle)?;
    let stats = import_one(&repo, bundle)?;
    repo.flush()?;
    Ok(stats)
}

/// `true` si los primeros bytes son `MNGM` (multi-bundle legacy) o
/// `MNGZ` (multi-bundle comprimido). Helper compartido entre
/// `cmd_bundle_import` (que lo usa para rechazar multi) y la detección
/// previa al strip del prefijo en `import_all`.
fn is_multi_bundle_magic(bytes: &[u8]) -> bool {
    bytes.len() >= MULTI_MAGIC.len()
        && (&bytes[..MULTI_MAGIC.len()] == MULTI_MAGIC
            || &bytes[..MULTI_MAGIC_ZSTD.len()] == MULTI_MAGIC_ZSTD)
}

/// Importa un multi-bundle (formato `MULTI_MAGIC + BundleMultiV1`). Si
/// el archivo es un single-bundle clásico, lo reportamos como error
/// para que el caller elija entre `bundle import` y `bundle import-all`.
pub fn cmd_bundle_import_all(
    repo_path: &Path,
    passphrase: &str,
    in_path: &Path,
) -> Result<BundleImportAllStats, CliError> {
    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let bytes = std::fs::read(in_path)?;
    let body: Vec<u8> = if bytes.len() >= MULTI_MAGIC_ZSTD.len()
        && &bytes[..MULTI_MAGIC_ZSTD.len()] == MULTI_MAGIC_ZSTD
    {
        zstd::decode_all(&bytes[MULTI_MAGIC_ZSTD.len()..]).map_err(CliError::Io)?
    } else if bytes.len() >= MULTI_MAGIC.len() && &bytes[..MULTI_MAGIC.len()] == MULTI_MAGIC {
        bytes[MULTI_MAGIC.len()..].to_vec()
    } else {
        return Err(CliError::ExpectedMultiBundle);
    };
    let multi: BundleMultiV1 =
        postcard::from_bytes(&body).map_err(|_| CliError::InvalidBundle)?;
    if multi.version != MULTI_VERSION {
        return Err(CliError::UnsupportedBundleVersion(multi.version));
    }

    let mut items = Vec::with_capacity(multi.items.len());
    for bundle in multi.items {
        items.push(import_one(&repo, bundle)?);
    }
    repo.flush()?;
    Ok(BundleImportAllStats { items })
}

/// Core compartido entre import single y multi. No flushea (el caller
/// lo hace una sola vez al final para amortizar I/O en el multi).
fn import_one(repo: &PersistentRepo, bundle: BundleV1) -> Result<BundleImportStats, CliError> {
    if bundle.version != BUNDLE_VERSION {
        return Err(CliError::UnsupportedBundleVersion(bundle.version));
    }

    let dialect = parse::Dialect::from_byte(bundle.dialect_byte)
        .ok_or(CliError::UnknownDialect(bundle.dialect_byte))?;

    // 1) Insertar nodos con verificación de hash. put_chunked rechaza
    // si `hash_stored(stored) != hash`, así que un bundle adulterado
    // se detecta en la primera entrada inconsistente.
    let mut nodes_inserted = 0usize;
    for stored in &bundle.nodes {
        let h = hash_stored(stored);
        if !repo.nodes.contains(&h)? {
            repo.nodes.put_chunked(h, stored)?;
            nodes_inserted += 1;
        }
    }

    // 2) Reconstruir el SemanticNode de la raíz para re-derivar α.
    let root_node = repo
        .nodes
        .reconstruct(&bundle.struct_hash)?
        .ok_or(CliError::HashNotFound(bundle.struct_hash))?;
    let computed_alpha = hash_alpha_with(dialect, &root_node);
    if computed_alpha != bundle.alpha {
        return Err(CliError::BundleAlphaMismatch {
            struct_hash: bundle.struct_hash,
            claimed_alpha: bundle.alpha,
        });
    }

    // 3) Registrar la raíz si es nueva. `roots.put` y `mst.insert` son
    // idempotentes — el flag `root_was_new` lo computamos antes.
    let root_was_new = !repo.roots.contains(&bundle.alpha)?;
    repo.roots.put(bundle.alpha, bundle.struct_hash, dialect)?;
    repo.mst.insert(bundle.alpha)?;

    // 4) Atestaciones — `add()` re-verifica firma Ed25519. Las que
    // tengan content != bundle.alpha (no deberían existir, pero…) se
    // descartan: la atestación es sobre OTRA raíz, no nos sirve acá.
    let now_secs = unix_now_secs();
    let mut atts_added = 0usize;
    let mut atts_rejected = 0usize;
    for att in bundle.attestations {
        if att.content != bundle.alpha {
            atts_rejected += 1;
            continue;
        }
        let existed = repo
            .attestations
            .get(&att.content)?
            .iter()
            .any(|a| a.author == att.author);
        match repo.attestations.add(att.clone()) {
            Ok(()) => {
                if !existed {
                    atts_added += 1;
                }
                let _ = repo.timestamps.put(&att.content, &att.author, now_secs);
            }
            Err(_) => atts_rejected += 1,
        }
    }

    // 5) Retracciones — misma lógica, mismo filtro por content.
    let mut rets_added = 0usize;
    let mut rets_rejected = 0usize;
    for r in bundle.retractions {
        if r.content != bundle.alpha {
            rets_rejected += 1;
            continue;
        }
        let existed = repo.retractions.contains(&r.content, &r.author)?;
        match repo.retractions.add(r) {
            Ok(()) => {
                if !existed {
                    rets_added += 1;
                }
            }
            Err(_) => rets_rejected += 1,
        }
    }

    Ok(BundleImportStats {
        alpha: bundle.alpha,
        nodes_inserted,
        attestations_added: atts_added,
        attestations_rejected: atts_rejected,
        retractions_added: rets_added,
        retractions_rejected: rets_rejected,
        root_was_new,
    })
}
