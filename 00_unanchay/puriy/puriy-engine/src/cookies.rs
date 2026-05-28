//! Cookie jar in-memory por proceso.
//!
//! Subset minimal de RFC 6265 — alcanza para sites que usan cookies
//! para tracking de sesión simple. Sin persistencia entre sesiones, sin
//! `Domain=`/`SameSite=`/`Expires=`/`Max-Age=` reales: cada cookie
//! queda atada al host del response y vive hasta el reset del jar.

use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Cookie {
    name: String,
    value: String,
}

static JAR: Mutex<Option<HashMap<String, Vec<Cookie>>>> = Mutex::new(None);

/// Devuelve el header `Cookie: …` para el host dado, o `None` si no
/// hay cookies para mandar.
pub fn cookie_header(host: &str) -> Option<String> {
    let guard = JAR.lock().ok()?;
    let map = guard.as_ref()?;
    let cookies = map.get(host)?;
    if cookies.is_empty() {
        return None;
    }
    let s = cookies
        .iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ");
    Some(s)
}

/// Parsea un `Set-Cookie` y lo guarda en el jar bajo `host`. Soporta
/// la forma básica `name=value; Path=/; ...`. Ignora attributes.
pub fn put_set_cookie(host: &str, set_cookie: &str) {
    let first = set_cookie.split(';').next().unwrap_or("");
    let Some(eq) = first.find('=') else { return };
    let name = first[..eq].trim().to_string();
    let value = first[eq + 1..].trim().to_string();
    if name.is_empty() {
        return;
    }
    let mut guard = match JAR.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let map = guard.get_or_insert_with(HashMap::new);
    let entries = map.entry(host.to_string()).or_default();
    // Reemplaza si el nombre ya existía; sino agrega.
    if let Some(existing) = entries.iter_mut().find(|c| c.name == name) {
        existing.value = value;
    } else {
        entries.push(Cookie { name, value });
    }
}

/// Limpia todas las cookies — útil para tests.
#[allow(dead_code)]
pub fn clear() {
    if let Ok(mut g) = JAR.lock() {
        *g = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Lock shared entre los tests del módulo — todos comparten el jar
    /// global, así que `cargo test` con --test-threads>1 los serializa
    /// vía esta mutex.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn put_y_get_round_trip() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        put_set_cookie("foo.test", "sid=abc123; Path=/; HttpOnly");
        put_set_cookie("foo.test", "theme=dark");
        let h = cookie_header("foo.test").unwrap();
        assert!(h.contains("sid=abc123"));
        assert!(h.contains("theme=dark"));
    }

    #[test]
    fn put_reemplaza_si_existe() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        put_set_cookie("bar.test", "sid=old");
        put_set_cookie("bar.test", "sid=new");
        let h = cookie_header("bar.test").unwrap();
        assert_eq!(h, "sid=new");
    }

    #[test]
    fn host_aislado() {
        let _g = TEST_LOCK.lock().unwrap();
        clear();
        put_set_cookie("a.test", "x=1");
        put_set_cookie("b.test", "y=2");
        assert_eq!(cookie_header("a.test").as_deref(), Some("x=1"));
        assert_eq!(cookie_header("b.test").as_deref(), Some("y=2"));
        assert_eq!(cookie_header("c.test"), None);
    }
}
