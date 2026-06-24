//! paloma-contacts — la **libreta** del correo.
//!
//! Mapea un alias humano ("Ana") a una dirección: un correo (`ana@gmail.com`) o
//! una identidad del rail (`<hex>@rail.suyu`). Sirve para escribir "Ana" en el
//! campo *Para* y que paloma lo expanda a la dirección real antes de enrutar —
//! ni un email largo ni una clave de 64 hex a mano.
//!
//! Agnóstica a la UI y a la red: sólo nombres y direcciones. Se persiste en
//! JSON, **editable a mano** (`~/.config/paloma/contactos.json`).

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Un contacto: un nombre legible y su dirección (correo o rail). El nombre es
/// la clave de resolución (sin distinguir mayúsculas).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub name: String,
    /// Dirección canónica: `usuario@dominio` (SMTP) o `<hex>@rail.suyu` (rail).
    pub address: String,
}

/// La libreta: una lista de contactos. Barata de clonar; se ordena por nombre al
/// guardar para que el archivo quede prolijo.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Contactbook {
    #[serde(default)]
    contacts: Vec<Contact>,
}

/// Errores de la libreta.
#[derive(Debug, Error)]
pub enum ContactError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl Contactbook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Los contactos, ordenados por nombre.
    pub fn all(&self) -> &[Contact] {
        &self.contacts
    }

    pub fn len(&self) -> usize {
        self.contacts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.contacts.is_empty()
    }

    /// Resuelve un alias a su dirección (case-insensitive). `None` si no existe.
    pub fn resolve(&self, name: &str) -> Option<&str> {
        let n = name.trim();
        self.contacts
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(n))
            .map(|c| c.address.as_str())
    }

    /// Expande un campo *Para* (`"Ana, bob@x.com"`): cada token que sea un alias
    /// conocido se reemplaza por su dirección; el resto pasa igual. Así el campo
    /// queda listo para `parse_address_list`. Preserva el orden y los espacios
    /// se normalizan a `", "`.
    pub fn expand(&self, to_text: &str) -> String {
        to_text
            .split(',')
            .map(|tok| {
                let t = tok.trim();
                self.resolve(t).map(|a| a.to_string()).unwrap_or_else(|| t.to_string())
            })
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Agrega o actualiza un contacto (por nombre, case-insensitive). Devuelve
    /// `true` si era nuevo. Ignora entradas con nombre o dirección vacíos.
    pub fn upsert(&mut self, name: impl Into<String>, address: impl Into<String>) -> bool {
        let name = name.into().trim().to_string();
        let address = address.into().trim().to_string();
        if name.is_empty() || address.is_empty() {
            return false;
        }
        match self.contacts.iter_mut().find(|c| c.name.eq_ignore_ascii_case(&name)) {
            Some(c) => {
                c.address = address;
                false
            }
            None => {
                self.contacts.push(Contact { name, address });
                self.contacts.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                true
            }
        }
    }

    /// Quita un contacto por nombre. `true` si existía.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.contacts.len();
        self.contacts.retain(|c| !c.name.eq_ignore_ascii_case(name.trim()));
        self.contacts.len() != before
    }

    /// Carga la libreta de `path` (JSON). Archivo inexistente → libreta vacía
    /// (primer arranque).
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ContactError> {
        match std::fs::read(path.as_ref()) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// Guarda la libreta a `path` (JSON legible, escritura atómica).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ContactError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(self)?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_y_expandir() {
        let mut lib = Contactbook::new();
        assert!(lib.upsert("Ana", "abcd@rail.suyu"));
        assert!(lib.upsert("Bob", "bob@gmail.com"));
        assert!(!lib.upsert("ana", "ef01@rail.suyu")); // actualiza, no agrega

        assert_eq!(lib.resolve("ANA"), Some("ef01@rail.suyu")); // case-insensitive + actualizado
        assert_eq!(lib.resolve("nadie"), None);

        // Expande sólo los alias conocidos; deja pasar el resto.
        assert_eq!(
            lib.expand("Ana, bob, carla@x.com"),
            "ef01@rail.suyu, bob@gmail.com, carla@x.com"
        );
        assert_eq!(lib.len(), 2);
    }

    #[test]
    fn upsert_ignora_vacios_y_remove() {
        let mut lib = Contactbook::new();
        assert!(!lib.upsert("", "x@y.com"));
        assert!(!lib.upsert("Nadie", "   "));
        assert!(lib.is_empty());
        lib.upsert("Ana", "a@x.com");
        assert!(lib.remove("ANA"));
        assert!(!lib.remove("ana"));
    }

    #[test]
    fn roundtrip_a_disco() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("contactos.json");
        let mut lib = Contactbook::new();
        lib.upsert("Ana", "abcd@rail.suyu");
        lib.save(&path).unwrap();

        let back = Contactbook::load(&path).unwrap();
        assert_eq!(back.resolve("ana"), Some("abcd@rail.suyu"));
        // Ruta inexistente → vacía.
        assert!(Contactbook::load(dir.path().join("no.json")).unwrap().is_empty());
    }
}
