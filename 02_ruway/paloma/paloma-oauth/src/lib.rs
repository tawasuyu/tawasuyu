//! `paloma-oauth` (lib) — la **autoridad del token OAuth2** del correo.
//!
//! La parte **no interactiva** del helper: leer/guardar el token de una cuenta,
//! renovarlo con el `refresh_token` (sin navegador) y entregar un `access_token`
//! **vigente** a quien lo pida. El binario (`src/main.rs`) agrega encima el
//! flujo interactivo (PKCE + loopback + navegador) para la primera autorización.
//!
//! `paloma-app` usa [`valid_access_token`] al arrancar: si el token guardado
//! venció, lo renueva solo y sigue; sin esto el usuario tendría que correr
//! `paloma-oauth <id>` a mano cada hora.

use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use paloma_config::{oauth_token_path, AccountEntry, Preset};

/// Margen (segundos) con el que consideramos que un access token «está por
/// vencer» y conviene renovarlo antes de usarlo. Cubre el RTT del login.
pub const REFRESH_MARGIN_SECS: u64 = 60;

/// El token guardado en `oauth-<id>.json`. `paloma-app` lee `access_token`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct Token {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    /// Unix-secs en que vence el access token (best-effort, para la renovación).
    #[serde(default)]
    pub expires_at: u64,
    #[serde(default)]
    pub token_type: String,
}

impl Token {
    /// Si la renovación no devolvió `refresh_token` (Google no siempre lo
    /// repite), conservamos el que ya teníamos.
    pub fn with_refresh_fallback(mut self, prev: &str) -> Self {
        if self.refresh_token.is_empty() {
            self.refresh_token = prev.to_string();
        }
        self
    }

    /// `true` si el access token está presente y todavía no vence (con margen).
    /// `expires_at == 0` (desconocido, p. ej. token escrito a mano) cuenta como
    /// vigente: no podemos saber, y mejor intentar usarlo que romper.
    pub fn is_fresh(&self, now: u64) -> bool {
        !self.access_token.is_empty()
            && (self.expires_at == 0 || self.expires_at > now + REFRESH_MARGIN_SECS)
    }
}

/// La respuesta JSON del endpoint de token del proveedor.
#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    token_type: String,
}

impl TokenResponse {
    fn into_token(self) -> Token {
        Token {
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            expires_at: now_secs() + self.expires_in,
            token_type: self.token_type,
        }
    }
}

/// Unix-secs actuales (0 si el reloj está antes de epoch, imposible en práctica).
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Entrega un `access_token` **vigente** para la cuenta `entry` (OAuth2),
/// renovándolo si hace falta y reescribiendo el archivo de token. `dir` es el
/// directorio de config de paloma (hogar de `oauth-<id>.json`).
///
/// - Token vigente → lo devuelve tal cual.
/// - Vencido + hay `refresh_token` → lo renueva, lo guarda y devuelve el nuevo.
/// - Vencido sin `refresh_token` → devuelve el viejo (que el login intente; si
///   falla, el caller cae a demo) o `Err` si no hay ni siquiera access token.
/// - Sin archivo de token → `Err` (hay que correr `paloma-oauth <id>` primero).
pub fn valid_access_token(dir: &Path, entry: &AccountEntry) -> Result<String, String> {
    let path = oauth_token_path(dir, &entry.id);
    let tok = load_token(&path)
        .map_err(|e| format!("sin token OAuth de «{}» ({e}); corré: paloma-oauth {}", entry.id, entry.id))?;
    if tok.is_fresh(now_secs()) {
        return Ok(tok.access_token);
    }
    // Vencido: intentamos renovar.
    if tok.refresh_token.is_empty() {
        if !tok.access_token.is_empty() {
            return Ok(tok.access_token); // que el login lo intente igual
        }
        return Err(format!("token de «{}» vencido y sin refresh_token; corré paloma-oauth {}", entry.id, entry.id));
    }
    let preset = entry
        .oauth_preset()
        .ok_or_else(|| format!("proveedor OAuth desconocido: «{}»", entry.oauth_provider))?;
    let nuevo = refresh(entry, preset, &tok.refresh_token)?.with_refresh_fallback(&tok.refresh_token);
    save_token(&path, &nuevo)?;
    Ok(nuevo.access_token)
}

/// Renueva el access token con el `refresh_token`, sin navegador.
pub fn refresh(entry: &AccountEntry, preset: &Preset, refresh_token: &str) -> Result<Token, String> {
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", &entry.oauth_client_id),
    ];
    if !entry.oauth_client_secret.trim().is_empty() {
        form.push(("client_secret", &entry.oauth_client_secret));
    }
    token_request(preset, &form)
}

/// POST al endpoint de token del proveedor y parseo de la respuesta. Lo usan
/// tanto la renovación (acá) como el intercambio del código (en el binario).
pub fn token_request(preset: &Preset, form: &[(&str, &str)]) -> Result<Token, String> {
    let body = ureq::post(preset.token_url)
        .send_form(form)
        .map_err(|e| format!("token endpoint: {e}"))?
        .into_string()
        .map_err(|e| format!("leer respuesta: {e}"))?;
    let parsed: TokenResponse =
        serde_json::from_str(&body).map_err(|e| format!("token JSON: {e} — respuesta: {body}"))?;
    if parsed.access_token.is_empty() {
        return Err("el proveedor no devolvió access_token".to_string());
    }
    Ok(parsed.into_token())
}

/// Lee el token de `path`. `Err` legible si no existe o no parsea.
pub fn load_token(path: &Path) -> Result<Token, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| format!("token inválido: {e}"))
}

/// El `refresh_token` guardado en `path`, si lo hay (no vacío).
pub fn existing_refresh_token(path: &Path) -> Option<String> {
    load_token(path).ok().map(|t| t.refresh_token).filter(|s| !s.is_empty())
}

/// Escribe el token a disco con permisos `0600` (sólo el dueño lo lee).
pub fn save_token(path: &Path, tok: &Token) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(tok).map_err(|e| format!("json: {e}"))?;
    write_private(path, json.as_bytes()).map_err(|e| format!("escribir token: {e}"))
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use paloma_config::preset;

    #[test]
    fn token_response_calcula_expira() {
        let tr = TokenResponse {
            access_token: "x".into(),
            refresh_token: "r".into(),
            expires_in: 3600,
            token_type: "Bearer".into(),
        };
        let tok = tr.into_token();
        assert_eq!(tok.access_token, "x");
        assert!(tok.expires_at > 3600); // now + 3600
    }

    #[test]
    fn is_fresh_respeta_vencimiento_y_margen() {
        let now = now_secs();
        let vivo = Token { access_token: "a".into(), expires_at: now + 3600, ..Default::default() };
        assert!(vivo.is_fresh(now));
        let vencido = Token { access_token: "a".into(), expires_at: now + 10, ..Default::default() };
        assert!(!vencido.is_fresh(now)); // dentro del margen de 60 s → no está fresco
        let sin_expira = Token { access_token: "a".into(), expires_at: 0, ..Default::default() };
        assert!(sin_expira.is_fresh(now)); // desconocido → lo damos por vigente
        let vacio = Token { access_token: String::new(), expires_at: now + 3600, ..Default::default() };
        assert!(!vacio.is_fresh(now));
    }

    #[test]
    fn valid_access_token_devuelve_el_vigente_sin_red() {
        let dir = tempfile::tempdir().unwrap();
        let mut entry = AccountEntry::new("ana", "Ana", "ana@gmail.com");
        entry.apply_preset(preset("google").unwrap());
        let path = oauth_token_path(dir.path(), &entry.id);
        let tok = Token {
            access_token: "vigente".into(),
            refresh_token: "r".into(),
            expires_at: now_secs() + 3600,
            token_type: "Bearer".into(),
        };
        save_token(&path, &tok).unwrap();
        // No vencido → lo devuelve sin tocar la red.
        assert_eq!(valid_access_token(dir.path(), &entry).unwrap(), "vigente");
    }

    #[test]
    fn valid_access_token_sin_archivo_es_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut entry = AccountEntry::new("ana", "Ana", "ana@gmail.com");
        entry.apply_preset(preset("google").unwrap());
        assert!(valid_access_token(dir.path(), &entry).is_err());
    }

    #[test]
    fn token_0600_en_disco() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oauth-x.json");
        save_token(&path, &Token { access_token: "a".into(), ..Default::default() }).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
