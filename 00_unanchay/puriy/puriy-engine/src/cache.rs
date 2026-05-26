//! Cache de bytes por URL — global y compartida entre todas las cargas
//! del proceso. Acelera:
//! - **Recargas** (F5): HTML y assets se sirven de RAM.
//! - **Back/forward**: idem.
//! - **Múltiples pestañas** del mismo origen.
//!
//! Política: LRU implícito por orden de inserción + cap por suma de
//! bytes (64 MB). No es estricto: cuando se inserta una entrada que
//! pone el total por encima, eyectamos hasta volver bajo el cap. No
//! hay TTL — los `Cache-Control: max-age=…` son una promesa que la
//! Fase 4 podrá honrar. Por ahora, asumimos que mientras el proceso
//! vive el contenido no cambia.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const CAP_BYTES: usize = 64 * 1024 * 1024;

struct CacheInner {
    entries: std::collections::HashMap<String, Vec<u8>>,
    order: VecDeque<String>,
    bytes: usize,
}

fn cache() -> &'static Mutex<CacheInner> {
    static CACHE: OnceLock<Mutex<CacheInner>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(CacheInner {
            entries: std::collections::HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
        })
    })
}

/// Recupera los bytes cacheados para `url`, si existen. Mueve el slot
/// al final de la cola LRU (most-recent).
pub fn get(url: &str) -> Option<Vec<u8>> {
    let mut c = cache().lock().ok()?;
    let bytes = c.entries.get(url).cloned()?;
    // Re-anclar al final de la cola (LRU touch).
    if let Some(pos) = c.order.iter().position(|u| u == url) {
        c.order.remove(pos);
        c.order.push_back(url.to_string());
    }
    Some(bytes)
}

/// Inserta o reemplaza los bytes para `url`. Si el total supera el cap,
/// eyecta entradas LRU hasta volver debajo.
pub fn put(url: &str, bytes: Vec<u8>) {
    let mut c = match cache().lock() {
        Ok(g) => g,
        Err(_) => return, // poison: el cache es opcional, no propagamos
    };
    if let Some(old) = c.entries.remove(url) {
        c.bytes = c.bytes.saturating_sub(old.len());
        if let Some(pos) = c.order.iter().position(|u| u == url) {
            c.order.remove(pos);
        }
    }
    let n = bytes.len();
    c.entries.insert(url.to_string(), bytes);
    c.order.push_back(url.to_string());
    c.bytes += n;
    while c.bytes > CAP_BYTES {
        let Some(victim) = c.order.pop_front() else { break };
        if let Some(v) = c.entries.remove(&victim) {
            c.bytes = c.bytes.saturating_sub(v.len());
        }
    }
}

/// Vacía completamente la cache. Útil en tests; no expuesto a la app
/// (el usuario no tiene un Ctrl+Shift+Delete todavía).
#[cfg(test)]
pub fn clear() {
    let mut c = cache().lock().unwrap();
    c.entries.clear();
    c.order.clear();
    c.bytes = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_miss_y_put_round_trip() {
        clear();
        assert!(get("https://x.test/a").is_none());
        put("https://x.test/a", b"hola".to_vec());
        assert_eq!(get("https://x.test/a").as_deref(), Some(b"hola".as_slice()));
    }

    #[test]
    fn eviccion_cuando_supera_cap() {
        clear();
        // Llenamos con 65 MB en 13 entradas de 5 MB.
        let big = vec![0u8; 5 * 1024 * 1024];
        for i in 0..13 {
            put(&format!("https://x.test/{i}"), big.clone());
        }
        // La primera fue eyectada (suma = 65 MB > 64 MB cap).
        assert!(get("https://x.test/0").is_none());
        // La última sigue.
        assert!(get("https://x.test/12").is_some());
    }
}
