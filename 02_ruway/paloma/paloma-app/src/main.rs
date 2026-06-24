//! `paloma` — el binario lanzable del correo.
//!
//! Arma el frontend Llimphi (`paloma-llimphi`) sobre un backend real
//! (`NetBackend`: IMAP+SMTP) construido desde la configuración de la cuenta.
//! Si no hay config —o falla la conexión— cae a los datos de demostración
//! (`MockBackend`), así la app siempre arranca y muestra algo.
//!
//! ## Configuración
//!
//! Cuenta en JSON, en `~/.config/paloma/cuenta.json` (o el dir de config del
//! SO). Las contraseñas **no** van en el archivo: se leen de entorno.
//!
//! ```json
//! {
//!   "display_name": "Sergio",
//!   "email": "sergio@jlsoltech.com",
//!   "username": "sergio@jlsoltech.com",
//!   "imap_host": "imap.jlsoltech.com", "imap_port": 993, "imap_security": "tls",
//!   "smtp_host": "smtp.jlsoltech.com", "smtp_port": 465, "smtp_security": "tls"
//! }
//! ```
//!
//! Entorno:
//! - `PALOMA_PASSWORD` — contraseña única (IMAP y SMTP), o bien
//! - `PALOMA_IMAP_PASSWORD` / `PALOMA_SMTP_PASSWORD` por separado.
//! - `PALOMA_CONFIG` — ruta alternativa al JSON de la cuenta.
//!
//! Sin `cuenta.json` o sin contraseña, arranca en modo demo (sin red).

use std::path::PathBuf;

use directories::ProjectDirs;
use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, KeyEvent, Modifiers, View, WheelDelta};
use serde::Deserialize;

use paloma_core::{Account, Address, MailBackend, Security, ServerConfig};
use paloma_llimphi::{Model, Msg};

mod identity;
mod llm;
mod rail;
mod semantic;

/// La cuenta tal como se escribe en el JSON: plana y cómoda de editar a mano.
/// Se traduce a [`Account`] al arrancar.
#[derive(Debug, Deserialize)]
struct CuentaFile {
    display_name: String,
    email: String,
    /// Usuario de login; si falta, se usa `email`.
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

fn sec_tls() -> String {
    "tls".to_string()
}

fn parse_security(s: &str) -> Security {
    match s.to_ascii_lowercase().as_str() {
        "plain" | "none" => Security::Plain,
        "starttls" => Security::StartTls,
        _ => Security::Tls,
    }
}

impl CuentaFile {
    fn into_account(self) -> Account {
        let user = self.username.unwrap_or_else(|| self.email.clone());
        let imap = ServerConfig::new(self.imap_host, self.imap_port, parse_security(&self.imap_security), user.clone());
        let smtp = ServerConfig::new(self.smtp_host, self.smtp_port, parse_security(&self.smtp_security), user);
        Account::new(
            "default",
            self.display_name.clone(),
            Address::named(self.display_name, self.email),
            imap,
            smtp,
        )
    }
}

/// Ruta del JSON de la cuenta: `PALOMA_CONFIG` si está, si no el dir de config
/// del SO (`~/.config/paloma/cuenta.json` en Linux).
fn config_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PALOMA_CONFIG") {
        return Some(PathBuf::from(p));
    }
    ProjectDirs::from("org", "tawasuyu", "paloma").map(|d| d.config_dir().join("cuenta.json"))
}

/// Contraseñas IMAP/SMTP desde entorno. `PALOMA_PASSWORD` cubre ambas; las
/// específicas la pisan. `None` si no hay ninguna (→ modo demo).
fn passwords() -> Option<(String, String)> {
    let both = std::env::var("PALOMA_PASSWORD").ok();
    let imap = std::env::var("PALOMA_IMAP_PASSWORD").ok().or_else(|| both.clone());
    let smtp = std::env::var("PALOMA_SMTP_PASSWORD").ok().or(both);
    match (imap, smtp) {
        (Some(i), Some(s)) => Some((i, s)),
        _ => None,
    }
}

/// Lo que `try_net` entrega cuando hay una conexión real.
struct NetSession {
    backend: Box<dyn MailBackend>,
    me: Address,
    /// Identificador de la cuenta (su correo) — clave en la caché en disco.
    account_id: String,
    label: String,
}

/// Directorio de caché en disco (`~/.cache/paloma` en Linux). `None` si la
/// plataforma no expone ProjectDirs.
fn cache_dir() -> Option<PathBuf> {
    ProjectDirs::from("org", "tawasuyu", "paloma").map(|d| d.cache_dir().to_path_buf())
}

/// Directorio de config (`~/.config/paloma` en Linux). Hogar de `cuenta.json` y
/// de la seed de identidad (`identity.seed`).
fn config_dir() -> Option<PathBuf> {
    ProjectDirs::from("org", "tawasuyu", "paloma").map(|d| d.config_dir().to_path_buf())
}

/// Intenta armar el `NetBackend` real. Devuelve `Err(motivo)` legible si falta
/// config/credenciales o falla la conexión — el caller cae a demo y lo informa.
fn try_net() -> Result<NetSession, String> {
    let path = config_path().ok_or_else(|| "no se pudo resolver el dir de config".to_string())?;
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("sin cuenta en {}: {e}", path.display()))?;
    let cuenta: CuentaFile =
        serde_json::from_str(&raw).map_err(|e| format!("cuenta.json inválido: {e}"))?;
    let (imap_pw, smtp_pw) = passwords().ok_or_else(|| {
        "falta contraseña (PALOMA_PASSWORD o PALOMA_IMAP_PASSWORD/PALOMA_SMTP_PASSWORD)".to_string()
    })?;
    let account = cuenta.into_account();
    // `account.address` ya lleva el display-name (lo puso `into_account`).
    let me = account.address.clone();
    let account_id = account.address.email.clone();
    let label = format!("conectado · {account_id}");
    let backend = paloma_net::NetBackend::connect(account, &imap_pw, &smtp_pw)
        .map_err(|e| format!("no se pudo conectar IMAP: {e}"))?;
    // Límite de fetch opcional: `PALOMA_FETCH_LIMIT=0` (o "all") trae todo.
    if let Ok(raw) = std::env::var("PALOMA_FETCH_LIMIT") {
        let limit = match raw.trim() {
            "0" | "all" | "todos" => None,
            n => n.parse::<usize>().ok().or(Some(200)),
        };
        backend.set_fetch_limit(limit);
    }
    Ok(NetSession { backend: Box::new(backend), me, account_id, label })
}

struct Paloma;

impl App for Paloma {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "paloma"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 720)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let (mut model, account_id) = match try_net() {
            Ok(s) => {
                let account_id = s.account_id.clone();
                // Caché en disco si la plataforma la permite; si no, sin persistencia.
                let model = match cache_dir().and_then(|d| paloma_store::MailDb::open(d).ok()) {
                    Some(db) => {
                        let mut model =
                            Model::with_persistence(s.backend, s.me, Theme::dark(), db, s.account_id);
                        model.status = s.label;
                        model
                    }
                    None => {
                        let mut model = Model::new(s.backend, s.me, Theme::dark());
                        model.status = s.label;
                        model
                    }
                };
                (model, account_id)
            }
            Err(why) => {
                eprintln!("paloma · modo demo: {why}");
                let me = Address::named("Sergio", "sergio@jlsoltech.com");
                let mut model =
                    Model::new(Box::new(paloma_llimphi::demo::backend()), me, Theme::dark());
                model.status = format!("modo demo (sin red) · {why}");
                (model, "demo".to_string())
            }
        };

        // Búsqueda por significado: se engancha si hay daemon de embeddings (o
        // PALOMA_SEMANTIC=mock para dev). Sin motor, el modo semántico de la UI
        // cae a la búsqueda exacta — la app arranca igual.
        if let Some(engine) = semantic::DaemonSemantic::try_build(&account_id, cache_dir()) {
            model.attach_semantic(Box::new(engine));
        }

        // Asistente LLM (Eje 2): resumir hilo + borrador de respuesta. Se
        // engancha si hay un backend real (o PLUMA_LLM_BACKEND explícito);
        // local-first con Ollama. Sin backend, los botones ✨ no aparecen.
        if let Some(assistant) = llm::LlmHelper::try_build() {
            model.attach_llm(Box::new(assistant));
        }

        // Identidad Ed25519/agora (Eje 3): una sola seed para firmar el correo
        // SMTP y para el rail P2P. Se crea la primera vez.
        if let Some(seed) = identity::load_or_create_seed(config_dir()) {
            let signer = identity::AgoraSigner::from_seed(seed);
            let pk = signer.public_key();
            eprintln!(
                "paloma · identidad Ed25519: {:02x}{:02x}{:02x}{:02x}…",
                pk[0], pk[1], pk[2], pk[3]
            );
            model.attach_signer(Box::new(signer));

            // Rail soberano P2P (Eje 3.B): buzón "Suyu" + enrutado @rail.suyu.
            let rail = rail::RailHost::new(seed, handle.clone());
            eprintln!("paloma · rail P2P · tu dirección: {}", rail.address());
            model.attach_rail(Box::new(rail));

            // Red de avales (web-of-trust): almacén JSON + generador con la seed.
            if let Some(dir) = config_dir() {
                let path = dir.join("avales.json");
                let store = paloma_trust::TrustStore::load(&path).unwrap_or_default();
                model.set_trust(store, path, Box::new(identity::AgoraVoucher::from_seed(seed)));
            }
        }

        // Libreta de contactos (alias → dirección), JSON editable en config.
        if let Some(dir) = config_dir() {
            let path = dir.join("contactos.json");
            let book = paloma_contacts::Contactbook::load(&path).unwrap_or_default();
            model.set_contacts(book, path);
        }

        model
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        paloma_llimphi::update(model, msg, handle)
    }

    fn view(model: &Model) -> View<Msg> {
        paloma_llimphi::view(model)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        paloma_llimphi::view_overlay(model)
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        paloma_llimphi::on_key(model, event)
    }

    fn on_wheel(model: &Model, delta: WheelDelta, cursor: (f32, f32), mods: Modifiers) -> Option<Msg> {
        paloma_llimphi::on_wheel(model, delta, cursor, mods)
    }
}

fn main() {
    llimphi_ui::run::<Paloma>();
}
