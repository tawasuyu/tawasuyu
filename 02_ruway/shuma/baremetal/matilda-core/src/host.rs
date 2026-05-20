//! `Host` — un servidor administrado.

use serde::{Deserialize, Serialize};

/// Un servidor bajo administración. La clave única es `name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    /// Nombre lógico — clave de inventario, no necesariamente el hostname.
    pub name: String,
    /// IP o nombre DNS por el que se alcanza.
    pub address: String,
    /// Etiquetas libres — `"prod"`, `"db"`, `"edge"`.
    pub tags: Vec<String>,
}

impl Host {
    pub fn new(name: impl Into<String>, address: impl Into<String>) -> Self {
        Self { name: name.into(), address: address.into(), tags: Vec::new() }
    }

    /// Añade una etiqueta (encadenable). No duplica.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        let tag = tag.into();
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
        self
    }

    /// `true` si el host lleva la etiqueta `tag`.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_tag_dedups() {
        let h = Host::new("edge-1", "10.0.0.1").with_tag("prod").with_tag("prod");
        assert_eq!(h.tags.len(), 1);
        assert!(h.has_tag("prod"));
    }
}
