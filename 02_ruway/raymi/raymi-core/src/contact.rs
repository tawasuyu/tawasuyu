use serde::{Deserialize, Serialize};

/// Una libreta de direcciones (una colección CardDAV).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressBook {
    pub id: String,
    pub name: String,
}

impl AddressBook {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self { id: id.into(), name: name.into() }
    }
}

/// Un contacto (un `VCARD`, ya parseado al modelo nativo). Plano a propósito:
/// nombre para mostrar + listas de correos y teléfonos + organización y nota.
/// Los emails se reutilizan para invitar a eventos y para cruzar con `paloma`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    /// `UID` vCard — estable.
    pub uid: String,
    /// Nombre para mostrar (`FN`).
    pub full_name: String,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub phones: Vec<String>,
    #[serde(default)]
    pub org: Option<String>,
    #[serde(default)]
    pub note: String,
    /// Libreta donde reside (clave en el store).
    pub address_book: String,
}

impl Contact {
    /// El correo principal (el primero), si tiene.
    pub fn primary_email(&self) -> Option<&str> {
        self.emails.first().map(String::as_str)
    }

    /// Iniciales (1–2) del nombre, para el avatar.
    pub fn initials(&self) -> String {
        let mut words = self.full_name.split_whitespace().filter(|w| !w.is_empty());
        let a = words.next().and_then(|w| w.chars().next());
        let b = words.next().and_then(|w| w.chars().next());
        match (a, b) {
            (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_uppercase()),
            (Some(a), None) => a.to_uppercase().to_string(),
            _ => "?".to_string(),
        }
    }

    /// `true` si el contacto matchea `query` (nombre, correo, org, teléfono),
    /// sin importar mayúsculas.
    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        self.full_name.to_lowercase().contains(&q)
            || self.emails.iter().any(|e| e.to_lowercase().contains(&q))
            || self.phones.iter().any(|p| p.to_lowercase().contains(&q))
            || self.org.as_deref().map(|o| o.to_lowercase().contains(&q)).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c() -> Contact {
        Contact {
            uid: "u1".into(),
            full_name: "Ana Pérez".into(),
            emails: vec!["ana@ejemplo.com".into()],
            phones: vec!["+58 412 555".into()],
            org: Some("Acme".into()),
            note: String::new(),
            address_book: "personal".into(),
        }
    }

    #[test]
    fn primario_e_iniciales() {
        let c = c();
        assert_eq!(c.primary_email(), Some("ana@ejemplo.com"));
        assert_eq!(c.initials(), "AP");
    }

    #[test]
    fn matchea_por_varios_campos() {
        let c = c();
        assert!(c.matches("ana"));
        assert!(c.matches("ACME"));
        assert!(c.matches("ejemplo.com"));
        assert!(c.matches("412"));
        assert!(!c.matches("zzz"));
        assert!(c.matches(""), "consulta vacía matchea todo");
    }
}
