use super::*;

// --- Auth SSH para `Source::Remote` -----------------------------------------
// Espejo MÍNIMO del schema de `~/.config/shuma/hosts.json` (lo escribe el
// chasis vía `hosts.rs`): sólo los campos que necesita el transporte SSH.

#[derive(serde::Deserialize)]
pub(crate) struct HostEntry {
    pub(crate) host: String,
    #[serde(default)]
    pub(crate) user: String,
    #[serde(default = "host_default_port")]
    pub(crate) port: u16,
    #[serde(default)]
    pub(crate) auth: HostAuthJson,
}

pub(crate) fn host_default_port() -> u16 {
    22
}

#[derive(serde::Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum HostAuthJson {
    #[default]
    Password,
    Key {
        path: String,
    },
}

pub(crate) fn hosts_json_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("shuma").join("hosts.json"))
}

/// Corre el binario askpass (`SHUMA_ASKPASS`/`SSH_ASKPASS`) con `prompt` y
/// devuelve lo que imprime en stdout (la contraseña/passphrase). `None` si no
/// hay askpass configurado o el usuario canceló.
pub(crate) fn run_askpass(prompt: &str) -> Option<String> {
    let bin = std::env::var_os("SHUMA_ASKPASS").or_else(|| std::env::var_os("SSH_ASKPASS"))?;
    let out = std::process::Command::new(bin).arg(prompt).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout)
        .trim_end_matches(['\n', '\r'])
        .to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Resuelve el método de auth para `host`/`user` leyendo `hosts.json`. Clave
/// (PEM) → `SshAuth::Key`; contraseña → askpass al conectar.
pub(crate) fn resolve_ssh_auth(
    host: &str,
    user: &str,
) -> Result<shuma_remote_exec::SshAuth, String> {
    let path = hosts_json_path().ok_or("no se pudo ubicar hosts.json")?;
    let txt = std::fs::read_to_string(&path)
        .map_err(|e| format!("no pude leer {}: {e}", path.display()))?;
    let entries: Vec<HostEntry> =
        serde_json::from_str(&txt).map_err(|e| format!("hosts.json inválido: {e}"))?;
    let entry = entries
        .iter()
        .find(|h| h.host == host && (h.user == user || h.user.is_empty()))
        .or_else(|| entries.iter().find(|h| h.host == host))
        .ok_or_else(|| format!("no hay host guardado para {host} — gestioná hosts"))?;
    let _ = entry.port;
    match &entry.auth {
        HostAuthJson::Key { path } => Ok(shuma_remote_exec::SshAuth::Key {
            path: PathBuf::from(path),
            passphrase: None,
        }),
        HostAuthJson::Password => {
            let pw = run_askpass(&format!("Contraseña SSH para {user}@{host}:")).ok_or(
                "auth por contraseña: configurá SHUMA_ASKPASS/SSH_ASKPASS o usá una clave (PEM)",
            )?;
            Ok(shuma_remote_exec::SshAuth::Password(pw))
        }
    }
}

/// Carga el `Keypair` del shell desde el archivo de identidad,
/// creando uno nuevo si no existe. Usa el path por defecto de
/// `shuma-link::Keypair::default_path()` (`~/.config/shuma/keys/identity`).
pub(crate) fn load_or_create_identity() -> Result<shuma_link::Keypair, String> {
    let path = shuma_link::Keypair::default_path()
        .ok_or_else(|| "no se pudo derivar el path de identidad".to_string())?;
    shuma_link::Keypair::load_or_generate(&path).map_err(|e| e.to_string())
}

pub(crate) fn parse_pub_hex(hex_str: &str) -> Result<shuma_link::PublicKey, String> {
    shuma_link::PublicKey::from_hex(hex_str).map_err(|e| e.to_string())
}
