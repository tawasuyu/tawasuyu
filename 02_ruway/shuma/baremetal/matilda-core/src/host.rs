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
    /// Usuario SSH para administrar el host (default `root`). Opcional para
    /// que los inventarios viejos sigan parseando.
    #[serde(default)]
    pub user: Option<String>,
    /// Puerto SSH (default 22).
    #[serde(default)]
    pub port: Option<u16>,
}

impl Host {
    pub fn new(name: impl Into<String>, address: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            address: address.into(),
            tags: Vec::new(),
            user: None,
            port: None,
        }
    }

    /// Añade una etiqueta (encadenable). No duplica.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        let tag = tag.into();
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
        self
    }

    /// Fija el usuario SSH (encadenable).
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Fija el puerto SSH (encadenable).
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// `true` si el host lleva la etiqueta `tag`.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// Usuario SSH efectivo (default `root`).
    pub fn ssh_user(&self) -> &str {
        self.user.as_deref().unwrap_or("root")
    }

    /// Puerto SSH efectivo (default 22).
    pub fn ssh_port(&self) -> u16 {
        self.port.unwrap_or(22)
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
