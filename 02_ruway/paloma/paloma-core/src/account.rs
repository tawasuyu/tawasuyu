use serde::{Deserialize, Serialize};

use crate::address::Address;

/// Modo de cifrado del transporte hacia un servidor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Security {
    /// Sin cifrado (sólo para servidores locales / pruebas).
    Plain,
    /// STARTTLS: arranca en claro y negocia TLS sobre el mismo puerto.
    StartTls,
    /// TLS implícito desde el byte cero (IMAPS:993 / SMTPS:465).
    Tls,
}

/// Configuración de un servidor (IMAP o SMTP). **No** lleva la contraseña: el
/// secreto lo provee aparte un proveedor de credenciales (a futuro, la
/// identidad de `agora` / `shared/auth`), para no serializarlo junto a la
/// cuenta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    /// Usuario de login (suele ser la dirección completa).
    pub username: String,
}

impl ServerConfig {
    pub fn new(host: impl Into<String>, port: u16, security: Security, username: impl Into<String>) -> Self {
        Self { host: host.into(), port, security, username: username.into() }
    }
}

/// Una cuenta de correo: identidad para mostrar + servidores de entrada
/// (IMAP) y salida (SMTP). El `id` es estable y opaco (clave de la cuenta en
/// el store / la config).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub display_name: String,
    pub address: Address,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
}

impl Account {
    pub fn new(
        id: impl Into<String>,
        display_name: impl Into<String>,
        address: Address,
        imap: ServerConfig,
        smtp: ServerConfig,
    ) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            address,
            imap,
            smtp,
        }
    }

    /// La dirección "From" formateada como va en el header: `Nombre <correo>`.
    pub fn from_header(&self) -> String {
        Address::named(&self.display_name, &self.address.email).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cuenta() -> Account {
        Account::new(
            "acc1",
            "Ana Pérez",
            Address::new("ana@ejemplo.com"),
            ServerConfig::new("imap.ejemplo.com", 993, Security::Tls, "ana@ejemplo.com"),
            ServerConfig::new("smtp.ejemplo.com", 465, Security::Tls, "ana@ejemplo.com"),
        )
    }

    #[test]
    fn from_header_formatea_nombre_y_correo() {
        assert_eq!(cuenta().from_header(), "Ana Pérez <ana@ejemplo.com>");
    }

    #[test]
    fn serializa_sin_perder_campos() {
        let a = cuenta();
        let json = serde_json::to_string(&a).unwrap();
        let back: Account = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }
}
