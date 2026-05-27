//! Cache de bytes por URL — global y compartida entre todas las cargas
//! del proceso. Acelera:
//! - **Recargas** (F5): HTML y assets se sirven de RAM.
//! - **Back/forward**: idem.
//! - **Múltiples pestañas** del mismo origen.
//! - **Re-arranque del proceso**: cargamos lo que persistió en disco
//!   (ver [`load_from_disk`] / [`flush`]).
//!
//! Política: LRU implícito por orden de inserción + cap por suma de
//! bytes (64 MB). No es estricto: cuando se inserta una entrada que
//! pone el total por encima, eyectamos hasta volver bajo el cap. No
//! hay TTL — los `Cache-Control: max-age=…` son una promesa que la
//! Fase 4 podrá honrar. Por ahora, asumimos que mientras el proceso
//! vive el contenido no cambia.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const CAP_BYTES: usize = 64 * 1024 * 1024;
const PERSIST_MAGIC: &[u8; 4] = b"PUYC";
const PERSIST_VERSION: u8 = 1;

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

/// Path canónico para persistir la cache: `$XDG_CACHE_HOME/puriy/cache.bin`
/// con fallback a `$HOME/.cache/puriy/cache.bin`. `None` si no se puede
/// resolver ningún base path.
fn persist_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("puriy").join("cache.bin"))
}

/// Carga la cache desde disco si existe el archivo. Silencioso en caso
/// de error (cache corrupta, archivo ausente) — la cache simplemente
/// arranca vacía. Conviene llamar una vez al startup del proceso.
pub fn load_from_disk() {
    let Some(path) = persist_path() else { return };
    let Ok(mut f) = std::fs::File::open(&path) else { return };
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return;
    }
    let entries = match decode(&buf) {
        Some(e) => e,
        None => return,
    };
    let Ok(mut c) = cache().lock() else { return };
    for (url, data) in entries {
        let n = data.len();
        c.entries.insert(url.clone(), data);
        c.order.push_back(url);
        c.bytes += n;
    }
    // Evict si lo cargado supera el cap (puede pasar si el cap bajó
    // entre versiones).
    while c.bytes > CAP_BYTES {
        let Some(victim) = c.order.pop_front() else { break };
        if let Some(v) = c.entries.remove(&victim) {
            c.bytes = c.bytes.saturating_sub(v.len());
        }
    }
}

/// Vuelca la cache al disco. Escritura atómica: archivo `.tmp` + rename.
/// Silencioso ante errores I/O — perder el flush no rompe la sesión, sólo
/// implica que la próxima arranca con cache fría. Llamar después de cada
/// navegación exitosa, no después de cada `put` (sería write-amplification).
pub fn flush() {
    let Some(path) = persist_path() else { return };
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let bytes = {
        let Ok(c) = cache().lock() else { return };
        encode(&c)
    };
    let tmp = parent.join("cache.bin.tmp");
    {
        let Ok(mut f) = std::fs::File::create(&tmp) else { return };
        if f.write_all(&bytes).is_err() {
            return;
        }
    }
    let _ = std::fs::rename(&tmp, &path);
}

fn encode(c: &CacheInner) -> Vec<u8> {
    let mut out = Vec::with_capacity(c.bytes + 64);
    out.extend_from_slice(PERSIST_MAGIC);
    out.push(PERSIST_VERSION);
    let count = c.order.len() as u32;
    out.extend_from_slice(&count.to_le_bytes());
    for url in c.order.iter() {
        let Some(data) = c.entries.get(url) else { continue };
        let url_b = url.as_bytes();
        out.extend_from_slice(&(url_b.len() as u32).to_le_bytes());
        out.extend_from_slice(url_b);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

fn decode(buf: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    if buf.len() < 4 + 1 + 4 || &buf[..4] != PERSIST_MAGIC {
        return None;
    }
    if buf[4] != PERSIST_VERSION {
        return None;
    }
    let mut i = 5;
    let count = u32::from_le_bytes(buf[i..i + 4].try_into().ok()?) as usize;
    i += 4;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if i + 4 > buf.len() {
            return None;
        }
        let ul = u32::from_le_bytes(buf[i..i + 4].try_into().ok()?) as usize;
        i += 4;
        if i + ul + 4 > buf.len() {
            return None;
        }
        let url = std::str::from_utf8(&buf[i..i + ul]).ok()?.to_string();
        i += ul;
        let dl = u32::from_le_bytes(buf[i..i + 4].try_into().ok()?) as usize;
        i += 4;
        if i + dl > buf.len() {
            return None;
        }
        let data = buf[i..i + dl].to_vec();
        i += dl;
        out.push((url, data));
    }
    Some(out)
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
    fn codec_round_trip() {
        // Construir un CacheInner manual y codificar/decodificar.
        let mut c = CacheInner {
            entries: std::collections::HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
        };
        for (url, data) in [
            ("https://a.test/", &b"hola"[..]),
            ("https://b.test/img.png", &[0xffu8, 0xd8, 0xff, 0xe0, 0x00, 0x10][..]),
        ] {
            c.entries.insert(url.into(), data.to_vec());
            c.order.push_back(url.into());
            c.bytes += data.len();
        }
        let bytes = encode(&c);
        let decoded = decode(&bytes).expect("decode ok");
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].0, "https://a.test/");
        assert_eq!(decoded[0].1, b"hola");
        assert_eq!(decoded[1].0, "https://b.test/img.png");
    }

    #[test]
    fn decode_rechaza_magic_invalida() {
        assert!(decode(b"NOPE\x01\x00\x00\x00\x00").is_none());
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
