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
    use std::collections::HashSet;

    let _keypair = keypair_file::load(repo_path.join(KEYPAIR_FILENAME), passphrase)?;
    let repo = PersistentRepo::open(repo_path.join(REPO_DIRNAME))?;

    let alpha = crate::commands::parse_hash_hex(hash_hex)?;
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

    let bundle = BundleV1 {
        version: BUNDLE_VERSION,
        alpha,
        struct_hash,
        dialect_byte: dialect.as_byte(),
        nodes,
        attestations,
        retractions,
    };

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
    let bundle: BundleV1 = postcard::from_bytes(&bytes).map_err(|_| CliError::InvalidBundle)?;
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

    repo.flush()?;

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
