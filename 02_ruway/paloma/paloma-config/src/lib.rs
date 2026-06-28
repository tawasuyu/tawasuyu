//! paloma-config — la **configuración de cuentas** del correo.
//!
//! `paloma` arrancó con UNA sola cuenta en `~/.config/paloma/cuenta.json`. Este
//! crate la generaliza a **varias cuentas** (`cuentas.json`), cada una con su
//! método de autenticación —contraseña clásica u **OAuth2** (Gmail/Outlook)— y
//! un **preset por proveedor** que autocompleta los servidores. Es el modelo que
//! consumen el binario (`paloma-app`) y el panel de control del SO
//! (`wawa-panel-llimphi`), así editar las cuentas desde el panel y a mano apuntan
//! al mismo archivo.
//!
//! Como el resto de la suite, es **agnóstico**: sólo tipos + serde + IO de un
//! JSON editable. No habla red ni dibuja nada. Los secretos (contraseñas, tokens
//! OAuth) **no** viven acá: la contraseña la provee el entorno y el token OAuth
//! su propio archivo (ver [`oauth_token_path`]).

use std::path::{Path, PathBuf};

use paloma_core::{Account, Address, Security, ServerConfig};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod presets;
pub use presets::{preset, presets, Preset};

/// Método de autenticación de una cuenta.
pub mod auth {
    /// Contraseña / app-password clásica (IMAP `LOGIN`, SMTP `AUTH PLAIN`).
    pub const PASSWORD: &str = "password";
    /// OAuth2 (`XOAUTH2`): el secreto es un *access token* renovable.
    pub const OAUTH2: &str = "oauth2";
}

fn sec_tls() -> String {
    "tls".to_string()
}
fn auth_password() -> String {
    auth::PASSWORD.to_string()
}

/// Traduce el texto de seguridad del JSON al enum del núcleo.
pub fn parse_security(s: &str) -> Security {
    match s.to_ascii_lowercase().as_str() {
        "plain" | "none" => Security::Plain,
        "starttls" => Security::StartTls,
        _ => Security::Tls,
    }
}

/// Una cuenta de correo tal como se escribe en el JSON: plana y cómoda de editar
/// a mano. Lleva los servidores de entrada/salida y el método de autenticación.
/// **Sin** secretos (contraseña/token): esos van aparte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountEntry {
    /// Clave estable y opaca de la cuenta (clave en el store / la config). Suele
    /// derivarse del correo la primera vez, pero no cambia si el correo cambia.
    pub id: String,
    pub display_name: String,
    pub email: String,
    /// Usuario de login; vacío ⇒ se usa `email`.
    #[serde(default)]
    pub username: String,
    pub imap_host: String,
    pub imap_port: u16,
    #[serde(default = "sec_tls")]
    pub imap_security: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    #[serde(default = "sec_tls")]
    pub smtp_security: String,
    /// Método de autenticación: [`auth::PASSWORD`] (default) o [`auth::OAUTH2`].
    #[serde(default = "auth_password")]
    pub auth: String,
    /// Proveedor OAuth (`"google"` / `"microsoft"` / `"custom"`) — sólo si
    /// `auth == oauth2`. Da los endpoints de autorización (ver [`Preset`]).
    #[serde(default)]
    pub oauth_provider: String,
    /// `client_id` de la app OAuth registrada por el usuario (Gmail/Outlook
    /// exigen una). Sin esto no se puede pedir un token.
    #[serde(default)]
    pub oauth_client_id: String,
    /// `client_secret` de la app OAuth. Vacío para clientes públicos con PKCE
    /// (el flujo recomendado para apps de escritorio).
    #[serde(default)]
    pub oauth_client_secret: String,
}

impl AccountEntry {
    /// Una cuenta nueva mínima (servidores en blanco), con `id` derivado del
    /// correo. El panel/usuario completa el resto (o aplica un preset).
    pub fn new(id: impl Into<String>, display_name: impl Into<String>, email: impl Into<String>) -> Self {
        let email = email.into();
        Self {
            id: id.into(),
            display_name: display_name.into(),
            username: String::new(),
            imap_host: String::new(),
            imap_port: 993,
            imap_security: sec_tls(),
            smtp_host: String::new(),
            smtp_port: 465,
            smtp_security: sec_tls(),
            auth: auth_password(),
            oauth_provider: String::new(),
            oauth_client_id: String::new(),
            oauth_client_secret: String::new(),
            email,
        }
    }

    /// El usuario de login efectivo (campo `username`, o el `email` si está vacío).
    pub fn login_user(&self) -> &str {
        if self.username.trim().is_empty() {
            &self.email
        } else {
            &self.username
        }
    }

    /// `true` si la cuenta usa OAuth2 (`XOAUTH2`) en vez de contraseña.
    pub fn is_oauth(&self) -> bool {
        self.auth == auth::OAUTH2
    }

    /// Traduce a la [`Account`] del núcleo (sin secreto): direcciones + servidores.
    pub fn to_account(&self) -> Account {
        let user = self.login_user().to_string();
        let imap = ServerConfig::new(
            self.imap_host.clone(),
            self.imap_port,
            parse_security(&self.imap_security),
            user.clone(),
        );
        let smtp = ServerConfig::new(
            self.smtp_host.clone(),
            self.smtp_port,
            parse_security(&self.smtp_security),
            user,
        );
        Account::new(
            self.id.clone(),
            self.display_name.clone(),
            Address::named(self.display_name.clone(), self.email.clone()),
            imap,
            smtp,
        )
    }

    /// Aplica un preset de proveedor: autocompleta servidores/puertos/seguridad y,
    /// si el preset es OAuth, fija el método y el proveedor. No toca `email`/
    /// `display_name`/`username` ni el `client_id` (lo pone el usuario).
    pub fn apply_preset(&mut self, p: &Preset) {
        self.imap_host = p.imap_host.to_string();
        self.imap_port = p.imap_port;
        self.imap_security = p.imap_security.to_string();
        self.smtp_host = p.smtp_host.to_string();
        self.smtp_port = p.smtp_port;
        self.smtp_security = p.smtp_security.to_string();
        if p.oauth_provider.is_empty() {
            self.auth = auth::PASSWORD.to_string();
            self.oauth_provider.clear();
        } else {
            self.auth = auth::OAUTH2.to_string();
            self.oauth_provider = p.oauth_provider.to_string();
        }
    }

    /// El preset OAuth de esta cuenta (por `oauth_provider`), si lo usa.
    pub fn oauth_preset(&self) -> Option<&'static Preset> {
        if self.is_oauth() {
            preset(&self.oauth_provider).filter(|p| !p.oauth_provider.is_empty())
        } else {
            None
        }
    }
}

/// La configuración completa del correo: la lista de cuentas + cuál es la activa.
/// La cuenta activa es la que abre `paloma-app` por defecto.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PalomaConfig {
    /// `id` de la cuenta activa (la que arranca por defecto). Vacío ⇒ la primera.
    #[serde(default)]
    pub active: String,
    #[serde(default)]
    pub accounts: Vec<AccountEntry>,
}

/// Errores de carga/guardado de la config.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl PalomaConfig {
    /// `id`s de todas las cuentas, en orden de archivo.
    pub fn ids(&self) -> Vec<String> {
        self.accounts.iter().map(|a| a.id.clone()).collect()
    }

    /// La cuenta activa: la de `active`, o la primera si `active` no resuelve.
    pub fn active_account(&self) -> Option<&AccountEntry> {
        self.get(&self.active).or_else(|| self.accounts.first())
    }

    /// El `id` de la cuenta activa efectiva (resuelto contra la lista).
    pub fn active_id(&self) -> String {
        self.active_account().map(|a| a.id.clone()).unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Option<&AccountEntry> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut AccountEntry> {
        self.accounts.iter_mut().find(|a| a.id == id)
    }

    /// Agrega una cuenta nueva con un `id` único derivado de `base` (un correo o
    /// "cuenta"). La deja activa y devuelve su `id`.
    pub fn add(&mut self, display_name: &str, email: &str) -> String {
        let base = if email.contains('@') {
            email.split('@').next().unwrap_or("cuenta")
        } else {
            "cuenta"
        };
        let id = self.unique_id(base);
        let dn = if display_name.is_empty() { email } else { display_name };
        self.accounts.push(AccountEntry::new(id.clone(), dn, email));
        self.active = id.clone();
        id
    }

    /// Elimina la cuenta `id`. Si era la activa, pasa a la primera que quede.
    pub fn remove(&mut self, id: &str) {
        self.accounts.retain(|a| a.id != id);
        if self.active == id {
            self.active = self.accounts.first().map(|a| a.id.clone()).unwrap_or_default();
        }
    }

    /// Duplica la cuenta `id` (servidores y método incluidos), con un `id` nuevo.
    /// Deja la copia activa y devuelve su `id`.
    pub fn duplicate(&mut self, id: &str) -> Option<String> {
        let mut copy = self.get(id)?.clone();
        let new_id = self.unique_id(&copy.id);
        copy.id = new_id.clone();
        copy.display_name = format!("{} (copia)", copy.display_name);
        self.accounts.push(copy);
        self.active = new_id.clone();
        Some(new_id)
    }

    /// Genera un `id` que no choque con ninguno existente, a partir de `base`.
    fn unique_id(&self, base: &str) -> String {
        let base: String = base
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect();
        let base = if base.is_empty() { "cuenta".to_string() } else { base };
        if !self.accounts.iter().any(|a| a.id == base) {
            return base;
        }
        for n in 2.. {
            let cand = format!("{base}-{n}");
            if !self.accounts.iter().any(|a| a.id == cand) {
                return cand;
            }
        }
        unreachable!()
    }

    /// Carga la config desde `path` (JSON). Si `path` no existe pero hay un
    /// `cuenta.json` heredado en el mismo directorio, lo **migra** a una cuenta
    /// "default". Si no hay nada, devuelve una config vacía.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(raw) => Ok(serde_json::from_str(&raw)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(cfg) = migrate_legacy(path.parent()) {
                    return Ok(cfg);
                }
                Ok(Self::default())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Escribe la config a `path` (JSON prolijo). Crea el directorio si falta.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

/// El nombre del archivo de cuentas dentro del dir de config de paloma.
pub const FILENAME: &str = "cuentas.json";
/// El archivo heredado de cuenta única.
pub const LEGACY_FILENAME: &str = "cuenta.json";

/// Ruta del JSON de cuentas dentro del directorio de config `dir`.
pub fn config_path(dir: &Path) -> PathBuf {
    dir.join(FILENAME)
}

/// Ruta del token OAuth de una cuenta: `oauth-<id>.json` en el dir de config.
/// Ahí guarda el helper de autorización el `access_token`/`refresh_token`; el
/// archivo se crea con permisos `0600` (ver `paloma-oauth`).
pub fn oauth_token_path(dir: &Path, account_id: &str) -> PathBuf {
    dir.join(format!("oauth-{account_id}.json"))
}

/// El viejo `cuenta.json` (cuenta única, plano). Se lee sólo para migrar.
#[derive(Debug, Deserialize)]
struct LegacyCuenta {
    display_name: String,
    email: String,
    #[serde(default)]
    username: Option<String>,
    imap_host: String,
    imap_port: u16,
    #[serde(default = "sec_tls")]
    imap_security: String,
    smtp_host: String,
    smtp_port: u16,
    #[serde(default = "sec_tls")]
    smtp_security: String,
}

/// Intenta migrar un `cuenta.json` heredado (en `dir`) a una [`PalomaConfig`] de
/// una sola cuenta "default". `None` si no hay archivo o no parsea.
fn migrate_legacy(dir: Option<&Path>) -> Option<PalomaConfig> {
    let path = dir?.join(LEGACY_FILENAME);
    let raw = std::fs::read_to_string(&path).ok()?;
    let c: LegacyCuenta = serde_json::from_str(&raw).ok()?;
    let entry = AccountEntry {
        id: "default".to_string(),
        display_name: c.display_name,
        email: c.email,
        username: c.username.unwrap_or_default(),
        imap_host: c.imap_host,
        imap_port: c.imap_port,
        imap_security: c.imap_security,
        smtp_host: c.smtp_host,
        smtp_port: c.smtp_port,
        smtp_security: c.smtp_security,
        auth: auth_password(),
        oauth_provider: String::new(),
        oauth_client_id: String::new(),
        oauth_client_secret: String::new(),
    };
    Some(PalomaConfig { active: "default".to_string(), accounts: vec![entry] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gmail_entry() -> AccountEntry {
        let mut a = AccountEntry::new("ana", "Ana", "ana@gmail.com");
        a.apply_preset(preset("google").unwrap());
        a
    }

    #[test]
    fn preset_gmail_es_oauth_y_autocompleta_servidores() {
        let a = gmail_entry();
        assert!(a.is_oauth());
        assert_eq!(a.imap_host, "imap.gmail.com");
        assert_eq!(a.smtp_host, "smtp.gmail.com");
        assert_eq!(a.oauth_provider, "google");
    }

    #[test]
    fn to_account_usa_email_si_no_hay_username() {
        let a = AccountEntry::new("x", "X", "x@dominio.com");
        let acc = a.to_account();
        assert_eq!(acc.imap.username, "x@dominio.com");
        assert_eq!(acc.address.email, "x@dominio.com");
    }

    #[test]
    fn add_genera_ids_unicos() {
        let mut cfg = PalomaConfig::default();
        let a = cfg.add("Ana", "ana@gmail.com");
        let b = cfg.add("Ana otra", "ana@outlook.com");
        let c = cfg.add("Sin arroba", "anita");
        assert_eq!(a, "ana");
        assert_eq!(b, "ana-2"); // mismo local-part «ana» → sufijo
        assert_eq!(c, "cuenta");
        assert_eq!(cfg.accounts.len(), 3);
        assert_eq!(cfg.active, c); // la última agregada queda activa
    }

    #[test]
    fn remove_reasigna_la_activa() {
        let mut cfg = PalomaConfig::default();
        cfg.add("A", "a@x.com");
        let b = cfg.add("B", "b@x.com");
        assert_eq!(cfg.active, b);
        cfg.remove(&b);
        assert_eq!(cfg.active, "a"); // vuelve a la que queda
    }

    #[test]
    fn duplicate_copia_servidores_con_id_nuevo() {
        let mut cfg = PalomaConfig { active: "ana".into(), accounts: vec![gmail_entry()] };
        let dup = cfg.duplicate("ana").unwrap();
        assert_eq!(dup, "ana-2");
        let d = cfg.get(&dup).unwrap();
        assert_eq!(d.imap_host, "imap.gmail.com");
        assert!(d.display_name.contains("copia"));
    }

    #[test]
    fn roundtrip_disco() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        let mut cfg = PalomaConfig::default();
        cfg.add("Ana", "ana@gmail.com");
        cfg.save(&path).unwrap();
        let back = PalomaConfig::load(&path).unwrap();
        assert_eq!(back.accounts.len(), 1);
        assert_eq!(back.active, "ana");
    }

    #[test]
    fn migra_cuenta_json_heredado() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_FILENAME);
        std::fs::write(
            &legacy,
            r#"{"display_name":"Sergio","email":"s@jls.com",
                "imap_host":"imap.jls.com","imap_port":993,
                "smtp_host":"smtp.jls.com","smtp_port":465}"#,
        )
        .unwrap();
        // cuentas.json NO existe → load() debe migrar el cuenta.json.
        let cfg = PalomaConfig::load(&config_path(dir.path())).unwrap();
        assert_eq!(cfg.accounts.len(), 1);
        assert_eq!(cfg.active, "default");
        assert_eq!(cfg.accounts[0].imap_host, "imap.jls.com");
        assert_eq!(cfg.accounts[0].email, "s@jls.com");
    }
}
