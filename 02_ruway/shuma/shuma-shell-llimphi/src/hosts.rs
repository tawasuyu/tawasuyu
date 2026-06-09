//! Persistencia de **hosts remotos** para sesiones SSH/daemon.
//!
//! Vive en `$XDG_CONFIG_HOME/shuma/hosts.json`. El usuario gestiona la
//! lista desde una ventana secundaria; cuando crea una sesión nueva con
//! aislamiento Remote, el form del panel ofrece un select con los hosts
//! guardados (o "Crear nuevo…" que abre el gestor).
//!
//! La auth no guarda passwords en plano — usa `Password` (askpass al
//! conectar) o `Key { path, ... }` con la PEM en un archivo del usuario.
//! La passphrase de la PEM (si tiene) también se lee con askpass.

use serde::{Deserialize, Serialize};

/// Un host remoto guardado.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteHost {
    /// Nombre amigable para identificar el host (libre).
    pub name: String,
    pub host: String,
    pub user: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub auth: HostAuth,
}

fn default_port() -> u16 {
    22
}

/// Método de autenticación.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostAuth {
    /// Lee la pass al conectar via `shuma-askpass` (SSH_ASKPASS).
    Password,
    /// Clave privada en `path` (archivo PEM). Si requiere passphrase,
    /// la lee via askpass al conectar.
    Key { path: String },
}

impl Default for HostAuth {
    fn default() -> Self {
        HostAuth::Password
    }
}

impl HostAuth {
    pub fn label(&self) -> &'static str {
        match self {
            HostAuth::Password => "Contraseña",
            HostAuth::Key { .. } => "Clave (PEM)",
        }
    }
}

impl RemoteHost {
    /// Etiqueta corta para mostrar en listas/dropdowns.
    pub fn display(&self) -> String {
        let p = if self.port == 22 {
            String::new()
        } else {
            format!(":{}", self.port)
        };
        format!("{} · {}@{}{}", self.name, self.user, self.host, p)
    }
}

/// Path canónico del archivo.
pub fn hosts_path() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|b| b.config_dir().join("shuma").join("hosts.json"))
}

/// Lee la lista persistida. Vacío si no hay archivo o no parsea.
pub fn load_hosts() -> Vec<RemoteHost> {
    hosts_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Vec<RemoteHost>>(&s).ok())
        .unwrap_or_default()
}

/// Persiste la lista a disco (silencioso ante errores de IO).
pub fn save_hosts(hosts: &[RemoteHost]) {
    let Some(path) = hosts_path() else {
        return;
    };
    if let Ok(json) = serde_json::to_string_pretty(hosts) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, json);
    }
}
