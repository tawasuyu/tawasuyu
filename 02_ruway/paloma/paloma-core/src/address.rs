use std::fmt;

use serde::{Deserialize, Serialize};

/// Una dirección de correo, con nombre opcional para mostrar.
///
/// Forma canónica: `Ana Pérez <ana@ejemplo.com>` o, sin nombre, `ana@ejemplo.com`.
/// El parseo es deliberadamente tolerante (subset de RFC 5322): cubre los dos
/// formatos cotidianos y entrecomillado simple del display-name; la
/// codificación MIME de nombres (`=?utf-8?…?=`) la resuelve el puente de red
/// antes de construir el `Address`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address {
    /// Nombre para mostrar; `None` si la dirección venía pelada.
    pub name: Option<String>,
    /// El `buzón@dominio`, ya normalizado sin espacios ni ángulos.
    pub email: String,
}

impl Address {
    /// Dirección sin nombre.
    pub fn new(email: impl Into<String>) -> Self {
        Self { name: None, email: email.into() }
    }

    /// Dirección con nombre para mostrar.
    pub fn named(name: impl Into<String>, email: impl Into<String>) -> Self {
        Self { name: Some(name.into()), email: email.into() }
    }

    /// Parsea una sola dirección. Acepta `Nombre <e@x>`, `<e@x>` o `e@x`.
    /// El nombre puede venir entrecomillado (`"Ana, Dra" <a@x>`). Devuelve
    /// `None` si no hay un `@` plausible.
    pub fn parse(s: &str) -> Option<Address> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if let Some(open) = s.rfind('<') {
            // `Nombre <correo>` — el correo va entre ángulos.
            let close = s[open..].find('>')? + open;
            let email = s[open + 1..close].trim();
            if !looks_like_email(email) {
                return None;
            }
            let raw_name = s[..open].trim().trim_matches('"').trim();
            let name = if raw_name.is_empty() { None } else { Some(raw_name.to_string()) };
            Some(Address { name, email: email.to_string() })
        } else if looks_like_email(s) {
            Some(Address::new(s.to_string()))
        } else {
            None
        }
    }

    /// El texto a mostrar: el nombre si lo hay, sino el correo.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.email)
    }

    /// El dominio (lo que sigue al último `@`), si la dirección lo tiene.
    pub fn domain(&self) -> Option<&str> {
        self.email.rsplit_once('@').map(|(_, d)| d)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(n) => write!(f, "{n} <{}>", self.email),
            None => write!(f, "{}", self.email),
        }
    }
}

/// Parsea una lista de direcciones separadas por comas (`To`, `Cc`…),
/// respetando comas dentro de un display-name entrecomillado
/// (`"Pérez, Ana" <a@x>, b@y`). Las entradas no parseables se descartan.
pub fn parse_address_list(s: &str) -> Vec<Address> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'"' => in_quotes = !in_quotes,
            b',' if !in_quotes => {
                if let Some(a) = Address::parse(&s[start..i]) {
                    out.push(a);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if let Some(a) = Address::parse(&s[start..]) {
        out.push(a);
    }
    out
}

/// Heurística mínima de "esto parece un correo": un solo `@`, con texto a
/// ambos lados y un punto en el dominio. No valida RFC completo — alcanza
/// para no aceptar basura evidente.
fn looks_like_email(s: &str) -> bool {
    let s = s.trim();
    match s.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && !s.contains(char::is_whitespace)
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pelado() {
        let a = Address::parse("ana@ejemplo.com").unwrap();
        assert_eq!(a, Address::new("ana@ejemplo.com"));
        assert_eq!(a.display_name(), "ana@ejemplo.com");
        assert_eq!(a.domain(), Some("ejemplo.com"));
    }

    #[test]
    fn parse_con_nombre_y_angulos() {
        let a = Address::parse("Ana Pérez <ana@ejemplo.com>").unwrap();
        assert_eq!(a, Address::named("Ana Pérez", "ana@ejemplo.com"));
        assert_eq!(a.display_name(), "Ana Pérez");
    }

    #[test]
    fn parse_nombre_entrecomillado() {
        let a = Address::parse("\"Pérez, Ana\" <ana@ejemplo.com>").unwrap();
        assert_eq!(a.name.as_deref(), Some("Pérez, Ana"));
    }

    #[test]
    fn parse_rechaza_basura() {
        assert!(Address::parse("no-es-correo").is_none());
        assert!(Address::parse("").is_none());
        assert!(Address::parse("a@b").is_none()); // sin punto en el dominio
    }

    #[test]
    fn display_roundtrip() {
        let a = Address::named("Ana", "ana@x.com");
        assert_eq!(a.to_string(), "Ana <ana@x.com>");
        assert_eq!(Address::new("b@y.com").to_string(), "b@y.com");
    }

    #[test]
    fn lista_respeta_comas_entrecomilladas() {
        let l = parse_address_list("\"Pérez, Ana\" <a@x.com>, Bob <b@y.com>, c@z.com");
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].name.as_deref(), Some("Pérez, Ana"));
        assert_eq!(l[2].email, "c@z.com");
    }
}
