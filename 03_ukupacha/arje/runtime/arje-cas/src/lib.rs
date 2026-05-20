//! Content-addressable store. Resuelve `Payload::Wasm.module_sha256` (y en
//! el futuro otros payloads firmados) desde el sistema de archivos con
//! verificación de hash. Path por defecto: `$XDG_DATA_HOME/ente/cas/<hex>`.
//!
//! Override por env: `ENTE_CAS_ROOT`.

use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tracing::debug;

pub fn cas_root() -> PathBuf {
    if let Ok(p) = std::env::var("ENTE_CAS_ROOT") {
        return p.into();
    }
    let base = if let Ok(d) = std::env::var("XDG_DATA_HOME") {
        d
    } else if let Ok(h) = std::env::var("HOME") {
        format!("{h}/.local/share")
    } else {
        "/var/lib".into()
    };
    PathBuf::from(base).join("ente").join("cas")
}

pub fn sha256_of(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

pub fn hex(sha: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in sha {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Lista todos los SHAs presentes en el CAS. Cada entrada del directorio
/// con nombre de 64 chars hex se considera un blob válido.
pub fn list_all_shas() -> anyhow::Result<Vec<[u8; 32]>> {
    let root = cas_root();
    if !root.exists() { return Ok(Vec::new()); }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let e = entry?;
        let name = e.file_name();
        let s = match name.to_str() {
            Some(s) if s.len() == 64 => s,
            _ => continue,
        };
        let mut sha = [0u8; 32];
        let mut ok = true;
        for i in 0..32 {
            match u8::from_str_radix(&s[i*2..i*2+2], 16) {
                Ok(b) => sha[i] = b,
                Err(_) => { ok = false; break; }
            }
        }
        if ok { out.push(sha); }
    }
    Ok(out)
}

/// Garbage collector. Borra todos los blobs que no están en `reachable`.
/// Devuelve (deleted_count, freed_bytes). El caller construye `reachable`
/// caminando todas las raíces (audit chain head, Wasm SHAs en Cards, etc).
///
/// Idempotente: re-correr no hace nada si el set no cambió.
pub fn gc(reachable: &std::collections::HashSet<[u8; 32]>) -> anyhow::Result<(usize, u64)> {
    let root = cas_root();
    let mut deleted = 0usize;
    let mut freed = 0u64;
    for sha in list_all_shas()? {
        if reachable.contains(&sha) { continue; }
        let path = root.join(hex(&sha));
        if let Ok(meta) = std::fs::metadata(&path) {
            freed += meta.len();
        }
        if std::fs::remove_file(&path).is_ok() {
            deleted += 1;
            tracing::debug!(sha = %hex(&sha), "CAS gc removed");
        }
    }
    Ok((deleted, freed))
}

pub fn resolve(sha: &[u8; 32]) -> anyhow::Result<Vec<u8>> {
    let path = cas_root().join(hex(sha));
    let bytes = std::fs::read(&path)
        .map_err(|e| anyhow::anyhow!("CAS read {}: {e}", path.display()))?;
    let actual = sha256_of(&bytes);
    if &actual != sha {
        anyhow::bail!(
            "CAS hash mismatch en {}: declarado={} real={}",
            path.display(), hex(sha), hex(&actual)
        );
    }
    Ok(bytes)
}

/// Almacena bytes en el CAS, devuelve su SHA. Idempotente: si el archivo ya
/// existe con el mismo hash, no reescribe.
pub fn store(bytes: &[u8]) -> anyhow::Result<[u8; 32]> {
    let sha = sha256_of(bytes);
    let root = cas_root();
    std::fs::create_dir_all(&root)
        .map_err(|e| anyhow::anyhow!("CAS mkdir {}: {e}", root.display()))?;
    let path = root.join(hex(&sha));
    if !path.exists() {
        // Escritura atómica: crear .tmp y rename.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)
            .map_err(|e| anyhow::anyhow!("CAS write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| anyhow::anyhow!("CAS rename {}: {e}", path.display()))?;
        debug!(hex = %hex(&sha), len = bytes.len(), path = %path.display(), "CAS store");
    }
    Ok(sha)
}
