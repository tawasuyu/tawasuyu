//! `VHost` — un host virtual de proxy inverso.

use serde::{Deserialize, Serialize};

/// El destino al que un `VHost` reenvía el tráfico.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Upstream {
    /// Una dirección `host:puerto` literal.
    Address(String),
    /// Un contenedor del inventario, por nombre y puerto interno.
    Container { name: String, port: u16 },
}

/// Un host virtual: un dominio que se reenvía a un upstream. Clave
/// única: `domain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VHost {
    pub domain: String,
    pub upstream: Upstream,
    /// Si se sirve sobre HTTPS.
    pub tls: bool,
    /// Dominios alternativos que resuelven al mismo upstream.
    pub aliases: Vec<String>,
}

impl VHost {
    /// VHost que apunta a una dirección literal.
    pub fn to_address(domain: impl Into<String>, address: impl Into<String>) -> Self {
        Self {
            domain: domain.into(),
            upstream: Upstream::Address(address.into()),
            tls: false,
            aliases: Vec::new(),
        }
    }

    /// VHost que apunta a un contenedor del inventario.
    pub fn to_container(
        domain: impl Into<String>,
        container: impl Into<String>,
        port: u16,
    ) -> Self {
        Self {
            domain: domain.into(),
            upstream: Upstream::Container { name: container.into(), port },
            tls: false,
            aliases: Vec::new(),
        }
    }

    /// Activa TLS (encadenable).
    pub fn with_tls(mut self) -> Self {
        self.tls = true;
        self
    }

    /// Añade un alias de dominio (encadenable).
    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    /// Nombre del contenedor del que depende, si el upstream es uno.
    pub fn depends_on_container(&self) -> Option<&str> {
        match &self.upstream {
            Upstream::Container { name, .. } => Some(name),
            Upstream::Address(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_upstream_reports_its_dependency() {
        let v = VHost::to_container("app.example.com", "web", 8080).with_tls();
        assert_eq!(v.depends_on_container(), Some("web"));
        assert!(v.tls);
    }

    #[test]
    fn address_upstream_has_no_container_dependency() {
        let v = VHost::to_address("static.example.com", "10.0.0.9:80");
        assert_eq!(v.depends_on_container(), None);
    }
}
