//! `raymi` — el binario lanzable de calendario y contactos.
//!
//! Arma el frontend Llimphi (`raymi-llimphi`) sobre un backend real
//! (`NetBackend`: CalDAV/CardDAV) **autodescubierto** desde una URL base. La
//! identidad y las credenciales se comparten con `paloma`: lee el mismo
//! `cuenta.json` (campo extra `dav_url`) y acepta `PALOMA_PASSWORD`. Es
//! **offline-first** vía `raymi-store`: al arrancar pinta lo último cacheado y
//! recién después refresca contra la red. Si no hay config —o falla la
//! conexión— cae a los datos de demostración, así la app siempre arranca.
//!
//! ## Configuración
//!
//! Cuenta en JSON, compartida con paloma en `~/.config/paloma/cuenta.json` (o el
//! dir de config del SO). raymi sólo necesita `dav_url` además de la identidad;
//! paloma ignora ese campo. Las contraseñas **no** van en el archivo: del entorno.
//!
//! ```json
//! {
//!   "display_name": "Sergio",
//!   "email": "sergio@jlsoltech.com",
//!   "username": "sergio@jlsoltech.com",
//!   "dav_url": "https://nube.jlsoltech.com/remote.php/dav/"
//! }
//! ```
//!
//! Entorno:
//! - `RAYMI_PASSWORD` (o, compartida con correo, `PALOMA_PASSWORD`) — contraseña.
//! - `RAYMI_DAV_URL` — pisa el `dav_url` del JSON.
//! - `RAYMI_CONFIG` — ruta alternativa al JSON de la cuenta.
//!
//! Sin `dav_url`/contraseña, arranca en modo demo (sin red).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;
use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, KeyEvent, Modifiers, View, WheelDelta};
use serde::Deserialize;

use raymi_llimphi::{Model, Msg};

/// La cuenta tal como se escribe en el JSON compartido con paloma. Sólo nos
/// importan la identidad y `dav_url`; el resto de campos (imap/smtp) se ignoran.
#[derive(Debug, Deserialize)]
struct CuentaFile {
    email: String,
    /// Usuario de login; si falta, se usa `email`.
    #[serde(default)]
    username: Option<String>,
    /// URL base CalDAV/CardDAV para autodescubrir. Sin esto → modo demo.
    #[serde(default)]
    dav_url: Option<String>,
}

/// Ruta del JSON de la cuenta: `RAYMI_CONFIG` si está, si no el `cuenta.json`
/// compartido con paloma (`~/.config/paloma/cuenta.json` en Linux).
fn config_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RAYMI_CONFIG") {
        return Some(PathBuf::from(p));
    }
    ProjectDirs::from("org", "tawasuyu", "paloma").map(|d| d.config_dir().join("cuenta.json"))
}

/// Contraseña desde entorno: `RAYMI_PASSWORD` o, compartida con el correo,
/// `PALOMA_PASSWORD`. `None` si no hay ninguna (→ modo demo).
fn password() -> Option<String> {
    std::env::var("RAYMI_PASSWORD").ok().or_else(|| std::env::var("PALOMA_PASSWORD").ok())
}

/// Directorio de caché en disco (`~/.cache/raymi` en Linux). Propio de raymi
/// (no comparte caché con paloma; sí la cuenta).
fn cache_dir() -> Option<PathBuf> {
    ProjectDirs::from("org", "tawasuyu", "raymi").map(|d| d.cache_dir().to_path_buf())
}

/// Lo que `try_net` entrega cuando hay backend real.
struct NetSession {
    backend: Box<dyn raymi_core::DavBackend>,
    /// Identificador de la cuenta (su correo) — clave en la caché en disco.
    account_id: String,
    label: String,
}

/// Intenta armar el `NetBackend` autodescubierto. `Err(motivo)` legible si falta
/// config/credenciales o falla el descubrimiento — el caller cae a demo.
fn try_net() -> Result<NetSession, String> {
    let path = config_path().ok_or_else(|| "no se pudo resolver el dir de config".to_string())?;
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("sin cuenta en {}: {e}", path.display()))?;
    let cuenta: CuentaFile =
        serde_json::from_str(&raw).map_err(|e| format!("cuenta.json inválido: {e}"))?;

    let dav_url = std::env::var("RAYMI_DAV_URL")
        .ok()
        .or(cuenta.dav_url)
        .ok_or_else(|| "falta dav_url (en cuenta.json o RAYMI_DAV_URL)".to_string())?;
    let password =
        password().ok_or_else(|| "falta contraseña (RAYMI_PASSWORD o PALOMA_PASSWORD)".to_string())?;
    let user = cuenta.username.unwrap_or_else(|| cuenta.email.clone());
    let account_id = cuenta.email.clone();

    let backend = raymi_net::NetBackend::discover(&user, &password, &dav_url)
        .map_err(|e| format!("no se pudo autodescubrir DAV: {e}"))?;
    let label = format!("conectado · {account_id}");
    Ok(NetSession { backend: Box::new(backend), account_id, label })
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

struct Raymi;

impl App for Raymi {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "raymi"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 720)
    }

    fn init(_handle: &Handle<Msg>) -> Model {
        match try_net() {
            Ok(s) => {
                // Caché en disco si la plataforma la permite; si no, sin persistencia.
                let mut model = match cache_dir().and_then(|d| raymi_store::CalDb::open(d).ok()) {
                    Some(db) => Model::with_persistence(s.backend, Theme::dark(), db, s.account_id),
                    None => Model::new(s.backend, Theme::dark()),
                };
                model.status = s.label;
                model
            }
            Err(why) => {
                eprintln!("raymi · modo demo: {why}");
                let mut model =
                    Model::new(Box::new(raymi_llimphi::demo::backend(now_unix())), Theme::dark());
                model.status = format!("modo demo (sin red) · {why}");
                model
            }
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        raymi_llimphi::update(model, msg, handle)
    }

    fn view(model: &Model) -> View<Msg> {
        raymi_llimphi::view(model)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        raymi_llimphi::view_overlay(model)
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        raymi_llimphi::on_key(model, event)
    }

    fn on_wheel(model: &Model, delta: WheelDelta, cursor: (f32, f32), mods: Modifiers) -> Option<Msg> {
        raymi_llimphi::on_wheel(model, delta, cursor, mods)
    }
}

fn main() {
    llimphi_ui::run::<Raymi>();
}
