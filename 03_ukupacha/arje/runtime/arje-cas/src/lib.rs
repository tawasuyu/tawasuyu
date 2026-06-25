//! Content-addressable store. Resuelve `Payload::Wasm.module_sha256` (y en
//! el futuro otros payloads firmados) desde el sistema de archivos con
//! verificación de hash. Path por defecto: `$XDG_DATA_HOME/ente/cas/<hex>`.
//!
//! Override por env: `ENTE_CAS_ROOT`.
//!
//! **Hash: BLAKE3** (migrado desde SHA-256, plan A0). Es el mismo hash que usan
//! hammer (`b3:`), `shared/format` y el kernel wawa, así que el `<hex>` del CAS
//! casa con el `expected_hash` de un `.swm` y con `mensaje_capacidad` de la
//! atestación. El ancho no cambia (256 bits = 32 bytes = 64 hex), así que el
//! layout en disco y la API por hash son idénticos; sólo cambia el cómputo. El
//! campo `module_sha256` conserva su nombre histórico aunque hoy lleva un BLAKE3.

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

pub fn blake3_of(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
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
    let actual = blake3_of(&bytes);
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
    let sha = blake3_of(bytes);
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

/// Cosecha (almacena) varios blobs en el CAS de una pasada, devolviendo sus
/// hashes en orden. Idempotente (cada `store` deduplica por contenido). Lo usa
/// el instalador para meter al CAS los binarios de arje que está instalando —
/// así quedan direccionados por su BLAKE3 (el mismo que firma la atestación) y
/// `arje-cas-aoe::servir_cas` puede distribuirlos por la red.
pub fn cosechar<'a>(
    blobs: impl IntoIterator<Item = &'a [u8]>,
) -> anyhow::Result<Vec<[u8; 32]>> {
    blobs.into_iter().map(store).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// store → resolve roundtrip con BLAKE3 (migración A0). Aísla el CAS en un dir temporal
    /// propio del proceso vía `ENTE_CAS_ROOT` para no tocar el real.
    #[test]
    fn store_resolve_roundtrip_blake3() {
        let tmp = std::env::temp_dir().join(format!("arje-cas-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::set_var("ENTE_CAS_ROOT", &tmp);

        let data = b"contenido de prueba para el CAS";
        let h = store(data).unwrap();
        // El hash devuelto es el BLAKE3 del contenido (no SHA-256).
        assert_eq!(h, blake3_of(data), "store debe direccionar por BLAKE3");
        assert_eq!(h, *blake3::hash(data).as_bytes());
        // Resuelve a los mismos bytes y queda listado.
        assert_eq!(resolve(&h).unwrap(), data);
        assert!(list_all_shas().unwrap().contains(&h));
        // El hex es de 64 chars (256 bits), igual que antes: el layout no cambió.
        assert_eq!(hex(&h).len(), 64);

        // cosechar almacena varios blobs de una pasada y devuelve sus hashes en
        // orden; idempotente con uno ya presente (el tercero == `data`).
        let hashes = cosechar([&b"alfa"[..], &b"beta"[..], &data[..]]).unwrap();
        assert_eq!(hashes.len(), 3);
        assert_eq!(hashes[2], h, "el tercer blob es `data` → mismo hash, dedup");
        assert_eq!(resolve(&hashes[0]).unwrap(), b"alfa");
        assert!(list_all_shas().unwrap().contains(&hashes[1]));

        std::env::remove_var("ENTE_CAS_ROOT");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
