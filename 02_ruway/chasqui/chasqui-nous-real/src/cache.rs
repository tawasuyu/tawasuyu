//! Cache de embeddings keyed por sha256 del contenido + model_id.
//!
//! Razón de existir: el modelo real (`fastembed-allMiniLML6V2`) es
//! caro (1-50 ms por archivo según tamaño y CPU). Cada vez que el
//! daemon de chasqui re-publica una Mónada o el watcher dispara un
//! re-cluster por cambio de FS, todos los archivos pasan otra vez
//! por embed. Para árboles de 1000 archivos, eso son segundos
//! desperdiciados re-embedidando contenido que no cambió.
//!
//! ## Diseño
//!
//! - **Cache key**: `sha256(bytes que el modelo realmente vio)` +
//!   `MODEL_ID` (string). Usar el sha de los bytes-vistos garantiza
//!   que la cache no devuelva un embedding de contenido viejo
//!   simplemente porque el path no cambió.
//! - **Cache value**: el `Vec<f32>` serializado como bytes
//!   little-endian (4 bytes por f32). Compacto, sin overhead de
//!   bincode/postcard para datos numéricos puros.
//! - **Backend**: sled, tree único `embed_cache_v1`. Path:
//!   `$XDG_CACHE_HOME/brahman/chasqui-nous-real-embed-cache.sled`.
//!
//! ## Versionado
//!
//! El nombre del tree (`embed_cache_v1`) es el "schema version" del
//! format value. Si bumpeamos a (p. ej.) almacenar también el
//! tiempo de cómputo o el ONNX session id, creamos `embed_cache_v2`
//! y el viejo queda como dato muerto que sled puede limpiar.
//!
//! El `MODEL_ID` viaja dentro del key, así que cambiar de modelo
//! invalida implícitamente las entradas viejas (no se accede más
//! a esos keys; sled las mantiene hasta GC manual).

use std::path::PathBuf;

/// Wrapper sobre sled::Db con la API justa que necesita `handle_file`.
#[derive(Clone)]
pub struct EmbedCache {
    tree: sled::Tree,
}

impl EmbedCache {
    /// Abre (o crea) la cache en su path canónico. El sled::Db queda
    /// referenciado por el Tree; mientras `EmbedCache` viva, el DB
    /// vive.
    pub fn open() -> Result<Self, sled::Error> {
        let path = default_path();
        if let Some(parent) = path.parent() {
            // best-effort: si no podemos crear el dir, sled falla con
            // mensaje específico abajo.
            let _ = std::fs::create_dir_all(parent);
        }
        let db = sled::open(&path)?;
        let tree = db.open_tree("embed_cache_v1")?;
        Ok(Self { tree })
    }

    /// Variante para tests: cache efímera bajo `dir`.
    #[cfg(test)]
    pub fn open_at(dir: &std::path::Path) -> Result<Self, sled::Error> {
        let db = sled::open(dir)?;
        let tree = db.open_tree("embed_cache_v1")?;
        Ok(Self { tree })
    }

    /// Lookup. `None` si miss; `Some(vec)` si hit.
    pub fn get(&self, file_sha: &[u8; 32], model_id: &str) -> Option<Vec<f32>> {
        let key = build_key(file_sha, model_id);
        let bytes = self.tree.get(&key).ok()??;
        decode_embedding(&bytes)
    }

    /// Almacena. Errores se loggean pero no propagan — cache miss es
    /// recuperable, no querés tirar el embed válido por fallo de I/O
    /// de cache.
    pub fn put(&self, file_sha: &[u8; 32], model_id: &str, embedding: &[f32]) {
        let key = build_key(file_sha, model_id);
        let bytes = encode_embedding(embedding);
        if let Err(e) = self.tree.insert(key, bytes) {
            tracing::warn!(error = %e, "embed-cache put falló (no-fatal)");
        }
    }

    /// Cantidad actual de entradas (best-effort para logs).
    pub fn len(&self) -> usize {
        self.tree.len()
    }
}

/// Path default. Honra `XDG_CACHE_HOME`, cae a `$HOME/.cache`, y de
/// último recurso a `/tmp` (sin persistencia, pero al menos no
/// crashea en entornos minimalistas como CI sin HOME).
fn default_path() -> PathBuf {
    if let Ok(p) = std::env::var("NOUSER_NOUS_REAL_CACHE") {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".cache"))
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("brahman").join("chasqui-nous-real-embed-cache.sled")
}

fn build_key(file_sha: &[u8; 32], model_id: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(32 + 1 + model_id.len());
    k.extend_from_slice(file_sha);
    // separator byte para que prefijos de model_id no choquen con
    // bytes del sha (improbable pero barato).
    k.push(0xff);
    k.extend_from_slice(model_id.as_bytes());
    k
}

fn encode_embedding(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

fn decode_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(s: &[u8]) -> [u8; 32] {
        arje_cas::blake3_of(s)
    }

    #[test]
    fn roundtrip_returns_same_vector() {
        let dir = tempfile::tempdir().unwrap();
        let cache = EmbedCache::open_at(dir.path()).unwrap();
        let key = sha(b"hello world");
        let v = vec![0.1f32, -0.5, 1.0, 3.14159];
        cache.put(&key, "real-fastembed-allMiniLML6V2-384d", &v);
        let got = cache
            .get(&key, "real-fastembed-allMiniLML6V2-384d")
            .expect("hit esperado");
        assert_eq!(got, v);
    }

    #[test]
    fn miss_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let cache = EmbedCache::open_at(dir.path()).unwrap();
        let key = sha(b"never stored");
        assert!(cache.get(&key, "real-fastembed-allMiniLML6V2-384d").is_none());
    }

    #[test]
    fn different_models_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let cache = EmbedCache::open_at(dir.path()).unwrap();
        let key = sha(b"same content");
        cache.put(&key, "model-a", &[1.0, 2.0]);
        cache.put(&key, "model-b", &[7.0, 8.0]);
        assert_eq!(cache.get(&key, "model-a").unwrap(), vec![1.0, 2.0]);
        assert_eq!(cache.get(&key, "model-b").unwrap(), vec![7.0, 8.0]);
    }

    #[test]
    fn different_content_different_keys() {
        let dir = tempfile::tempdir().unwrap();
        let cache = EmbedCache::open_at(dir.path()).unwrap();
        let k1 = sha(b"abc");
        let k2 = sha(b"abd");
        cache.put(&k1, "m", &[1.0]);
        assert!(cache.get(&k2, "m").is_none());
    }

    #[test]
    fn corrupted_value_returns_none() {
        // Si sled devuelve bytes con length no múltiplo de 4, decode
        // debe fallar limpio (None) en vez de panicar.
        let dir = tempfile::tempdir().unwrap();
        let cache = EmbedCache::open_at(dir.path()).unwrap();
        let key = sha(b"x");
        // Insertamos manualmente bytes inválidos.
        let raw_key = build_key(&key, "m");
        cache.tree.insert(raw_key, &[1u8, 2, 3][..]).unwrap();
        assert!(cache.get(&key, "m").is_none());
    }
}
