//! Diente **Correo (paloma)** del panel de control: configura las cuentas del
//! cliente de correo nativo de la suite.
//!
//! Es un *diente-de-app*: edita la config de `paloma` (`~/.config/paloma/
//! cuentas.json`) sin que el panel sepa nada de IMAP/SMTP — sólo monta el schema
//! de [`paloma_config`] y rutea los cambios. Soporta **varias cuentas** (lista +
//! alta/duplicado/baja) y, por cuenta, el método de autenticación: contraseña
//! clásica u **OAuth2** (Gmail/Outlook), con un preset por proveedor que
//! autocompleta los servidores.
//!
//! El secreto (contraseña/token) **no** se edita acá: la contraseña la toma
//! paloma del entorno y el token OAuth lo consigue el helper `paloma-oauth`, que
//! este diente dispara con el botón «Autorizar».

use std::path::PathBuf;

use allichay::{EnumOption, Field, FieldPath, FieldValue, Schema, Section};
use directories::ProjectDirs;
use paloma_config::{auth, preset, presets, AccountEntry, PalomaConfig};

/// Opciones de seguridad de transporte (IMAP/SMTP) para los dropdowns.
const SECURITIES: &[(&str, &str)] =
    &[("tls", "TLS (implícito)"), ("starttls", "STARTTLS"), ("plain", "Sin cifrado")];

/// Métodos de autenticación ofrecidos.
const AUTH_METHODS: &[(&str, &str)] =
    &[(auth::PASSWORD, "Contraseña / app-password"), (auth::OAUTH2, "OAuth2 (Google/Microsoft)")];

/// Estado del diente: la config de cuentas + dónde persiste.
pub struct PalomaState {
    pub cfg: PalomaConfig,
    /// Directorio de config de paloma (hogar de `cuentas.json` y los tokens).
    pub dir: Option<PathBuf>,
}

/// Lo que `route` le devuelve al `update` del panel: si hay que persistir y un
/// texto de estado para la barra/toast.
pub struct PalomaAction {
    pub dirty: bool,
    pub status: String,
}

impl PalomaAction {
    fn dirty(status: impl Into<String>) -> Self {
        Self { dirty: true, status: status.into() }
    }
    fn clean(status: impl Into<String>) -> Self {
        Self { dirty: false, status: status.into() }
    }
}

/// Resuelve el dir de config de paloma (`~/.config/paloma`).
fn paloma_dir() -> Option<PathBuf> {
    ProjectDirs::from("org", "tawasuyu", "paloma").map(|d| d.config_dir().to_path_buf())
}

impl PalomaState {
    /// Carga la config de cuentas de paloma (migra el viejo `cuenta.json` si
    /// hace falta). Sin dir de config resuelve a vacío (el panel igual arranca).
    pub fn load() -> Self {
        let dir = paloma_dir();
        let cfg = dir
            .as_ref()
            .map(|d| PalomaConfig::load(&paloma_config::config_path(d)).unwrap_or_default())
            .unwrap_or_default();
        Self { cfg, dir }
    }

    /// Persiste la config a `cuentas.json`. `Err` legible si no hay dir/falla IO.
    pub fn save(&self) -> Result<(), String> {
        let dir = self.dir.as_ref().ok_or_else(|| "sin dir de config de paloma".to_string())?;
        self.cfg
            .save(&paloma_config::config_path(dir))
            .map_err(|e| format!("paloma cuentas: {e}"))
    }

    /// El id de la cuenta activa (la que editan las secciones de servidores).
    fn active_id(&self) -> String {
        self.cfg.active_id()
    }
}

// =====================================================================
// Schema (lo que se pinta)
// =====================================================================

/// Arma el schema del diente: lista de cuentas + cuenta activa + (si OAuth) la
/// sección de autorización.
pub fn schema(state: &PalomaState) -> Schema {
    let mut schema = Schema::new().section(cuentas_section(state));
    let active = state.active_id();
    if let Some(entry) = state.cfg.get(&active) {
        schema = schema.section(cuenta_section(entry));
        if entry.is_oauth() {
            schema = schema.section(oauth_section(state, entry));
        }
    }
    schema
}

/// Sección «Cuentas»: selector de la activa (con ● en la activa) + alta / baja /
/// duplicado.
fn cuentas_section(state: &PalomaState) -> Section {
    let active = state.active_id();
    let opts: Vec<EnumOption> = state
        .cfg
        .accounts
        .iter()
        .map(|a| {
            let label = if a.id == active {
                format!("● {} <{}>", a.display_name, a.email)
            } else {
                format!("{} <{}>", a.display_name, a.email)
            };
            EnumOption::new(a.id.clone(), label)
        })
        .collect();
    let mut sec = Section::new("paloma::cuentas", "Cuentas")
        .icon("✉")
        .help(
            "Las cuentas de correo de paloma (IMAP/SMTP). La cuenta activa (●) es \
             la que abre paloma por defecto. Escribí un correo en «Nueva cuenta» \
             para agregar una; las demás secciones editan la cuenta activa. Los \
             secretos no se guardan acá: la contraseña la toma paloma del entorno \
             y el token OAuth lo consigue el botón «Autorizar».",
        );
    if !opts.is_empty() {
        sec = sec.field(Field::radio("usar", "Cuenta activa", active.clone(), opts));
    }
    sec.field(Field::text("crear", "Nueva cuenta (correo)…", ""))
        .field(Field::button("duplicar", "Duplicar la cuenta activa"))
        .field(Field::button("eliminar", "Eliminar la cuenta activa"))
}

/// Sección de la cuenta activa: identidad + proveedor + método + servidores.
fn cuenta_section(e: &AccountEntry) -> Section {
    let provider = if e.is_oauth() {
        e.oauth_provider.clone()
    } else {
        // Heurística: si los servidores casan un preset conocido, mostralo; si no,
        // «custom». (Sólo afecta el valor inicial del dropdown.)
        presets()
            .iter()
            .find(|p| !p.oauth_provider.is_empty() && p.imap_host == e.imap_host)
            .map(|p| p.id.to_string())
            .unwrap_or_else(|| "custom".to_string())
    };
    Section::new("paloma::cuenta", format!("Cuenta «{}»", e.display_name))
        .icon("👤")
        .help("Identidad y servidores de la cuenta activa. Elegí un proveedor para autocompletar los servidores.")
        .field(Field::text("display_name", "Nombre visible", e.display_name.clone()))
        .field(Field::text("email", "Correo", e.email.clone()))
        .field(Field::text("username", "Usuario de login (vacío = el correo)", e.username.clone()))
        .field(Field::dropdown(
            "proveedor",
            "Proveedor (autocompleta servidores)",
            provider,
            presets().iter().map(|p| EnumOption::new(p.id, p.label)).collect(),
        ))
        .field(Field::dropdown(
            "auth",
            "Autenticación",
            e.auth.clone(),
            AUTH_METHODS.iter().map(|(id, l)| EnumOption::new(*id, *l)).collect(),
        ))
        .field(Field::text("imap_host", "IMAP · servidor", e.imap_host.clone()))
        .field(Field::text("imap_port", "IMAP · puerto", e.imap_port.to_string()))
        .field(Field::dropdown(
            "imap_security",
            "IMAP · seguridad",
            e.imap_security.clone(),
            SECURITIES.iter().map(|(id, l)| EnumOption::new(*id, *l)).collect(),
        ))
        .field(Field::text("smtp_host", "SMTP · servidor", e.smtp_host.clone()))
        .field(Field::text("smtp_port", "SMTP · puerto", e.smtp_port.to_string()))
        .field(Field::dropdown(
            "smtp_security",
            "SMTP · seguridad",
            e.smtp_security.clone(),
            SECURITIES.iter().map(|(id, l)| EnumOption::new(*id, *l)).collect(),
        ))
}

/// Sección «OAuth2» (sólo si la cuenta usa OAuth): client_id/secret + autorizar.
fn oauth_section(state: &PalomaState, e: &AccountEntry) -> Section {
    let prov_label = preset(&e.oauth_provider).map(|p| p.label).unwrap_or("OAuth2");
    let has_token = state
        .dir
        .as_ref()
        .map(|d| paloma_config::oauth_token_path(d, &e.id).exists())
        .unwrap_or(false);
    let token_estado =
        if has_token { "token presente ✓".to_string() } else { "sin token — falta autorizar".to_string() };
    Section::new("paloma::oauth", "OAuth2")
        .icon("🔐")
        .help(format!(
            "Autenticación OAuth2 con {prov_label}. Necesitás registrar una app \
             OAuth en el proveedor (Google Cloud Console / Azure) y pegar acá su \
             «client_id» (las apps de escritorio usan PKCE; el «client_secret» \
             queda vacío salvo que el proveedor lo exija). «Autorizar» abre el \
             navegador, te loguea y guarda el token en oauth-{}.json.",
            e.id
        ))
        .field(Field::text("client_id", "client_id de la app OAuth", e.oauth_client_id.clone()))
        .field(Field::text("client_secret", "client_secret (vacío con PKCE)", e.oauth_client_secret.clone()))
        .field(Field::display("estado", "Estado del token", token_estado))
        .field(Field::button("autorizar", "Autorizar con el navegador…"))
}

// =====================================================================
// Sembrado de campos de texto en edición
// =====================================================================

/// Valor de texto actual de un campo (para sembrar el editor al focarlo). `rel`
/// ya viene sin el prefijo `paloma::`.
pub fn text_value(state: &PalomaState, rel: &FieldPath) -> Option<String> {
    let section = rel.segments().first().map(String::as_str)?;
    let leaf = rel.leaf()?;
    match section {
        // En la lista, «crear» arranca vacío (escribís el correo nuevo).
        "cuentas" => Some(String::new()),
        "cuenta" => {
            let e = state.cfg.get(&state.active_id())?;
            Some(match leaf {
                "display_name" => e.display_name.clone(),
                "email" => e.email.clone(),
                "username" => e.username.clone(),
                "imap_host" => e.imap_host.clone(),
                "imap_port" => e.imap_port.to_string(),
                "smtp_host" => e.smtp_host.clone(),
                "smtp_port" => e.smtp_port.to_string(),
                _ => String::new(),
            })
        }
        "oauth" => {
            let e = state.cfg.get(&state.active_id())?;
            Some(match leaf {
                "client_id" => e.oauth_client_id.clone(),
                "client_secret" => e.oauth_client_secret.clone(),
                _ => String::new(),
            })
        }
        _ => None,
    }
}

// =====================================================================
// Ruteo de cambios
// =====================================================================

/// Aplica un cambio del diente. `rel` ya viene sin el prefijo `paloma::`.
pub fn route(state: &mut PalomaState, rel: &FieldPath, value: FieldValue) -> PalomaAction {
    let section = rel.segments().first().cloned().unwrap_or_default();
    match section.as_str() {
        "cuentas" => route_cuentas(state, rel, value),
        "cuenta" => route_cuenta(state, rel, value),
        "oauth" => route_oauth(state, rel, value),
        _ => PalomaAction::clean(String::new()),
    }
}

fn route_cuentas(state: &mut PalomaState, rel: &FieldPath, value: FieldValue) -> PalomaAction {
    match rel.leaf() {
        Some("usar") => {
            if let Some(id) = value.as_str() {
                state.cfg.active = id.to_string();
                return PalomaAction::dirty(format!("cuenta activa: {id}"));
            }
        }
        Some("crear") => {
            if let Some(correo) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                let id = state.cfg.add("", correo);
                return PalomaAction::dirty(format!("cuenta «{correo}» creada (id {id})"));
            }
        }
        Some("duplicar") if value.as_bool() == Some(true) => {
            let active = state.active_id();
            if let Some(id) = state.cfg.duplicate(&active) {
                return PalomaAction::dirty(format!("cuenta duplicada (id {id})"));
            }
        }
        Some("eliminar") if value.as_bool() == Some(true) => {
            let active = state.active_id();
            if state.cfg.accounts.len() <= 1 {
                return PalomaAction::clean("no se puede eliminar la última cuenta".to_string());
            }
            state.cfg.remove(&active);
            return PalomaAction::dirty(format!("cuenta «{active}» eliminada"));
        }
        _ => {}
    }
    PalomaAction::clean(String::new())
}

fn route_cuenta(state: &mut PalomaState, rel: &FieldPath, value: FieldValue) -> PalomaAction {
    let id = state.active_id();
    // El proveedor aplica un preset entero (servidores + método); lo tratamos
    // antes de tomar el `&mut` del entry para poder leer el preset.
    if rel.leaf() == Some("proveedor") {
        if let Some(sel) = value.as_str() {
            if let Some(p) = preset(sel) {
                let p = *p;
                if let Some(e) = state.cfg.get_mut(&id) {
                    e.apply_preset(&p);
                    return PalomaAction::dirty(format!("proveedor: {}", p.label));
                }
            }
        }
        return PalomaAction::clean(String::new());
    }
    let Some(e) = state.cfg.get_mut(&id) else {
        return PalomaAction::clean(String::new());
    };
    let leaf = rel.leaf().unwrap_or("");
    let text = value.as_str().map(str::to_string);
    match leaf {
        "display_name" => set_text(&mut e.display_name, text),
        "email" => set_text(&mut e.email, text),
        "username" => set_text(&mut e.username, text),
        "auth" => set_text(&mut e.auth, text),
        "imap_host" => set_text(&mut e.imap_host, text),
        "imap_security" => set_text(&mut e.imap_security, text),
        "smtp_host" => set_text(&mut e.smtp_host, text),
        "smtp_security" => set_text(&mut e.smtp_security, text),
        "imap_port" => {
            if let Some(p) = text.and_then(|s| s.trim().parse::<u16>().ok()) {
                e.imap_port = p;
            }
        }
        "smtp_port" => {
            if let Some(p) = text.and_then(|s| s.trim().parse::<u16>().ok()) {
                e.smtp_port = p;
            }
        }
        _ => return PalomaAction::clean(String::new()),
    }
    PalomaAction::dirty(String::new())
}

fn route_oauth(state: &mut PalomaState, rel: &FieldPath, value: FieldValue) -> PalomaAction {
    // El botón «Autorizar» dispara el helper externo (no toca la config).
    if rel.leaf() == Some("autorizar") && value.as_bool() == Some(true) {
        let id = state.active_id();
        return spawn_authorizer(&id);
    }
    let id = state.active_id();
    let Some(e) = state.cfg.get_mut(&id) else {
        return PalomaAction::clean(String::new());
    };
    let text = value.as_str().map(str::to_string);
    match rel.leaf() {
        Some("client_id") => set_text(&mut e.oauth_client_id, text),
        Some("client_secret") => set_text(&mut e.oauth_client_secret, text),
        _ => return PalomaAction::clean(String::new()),
    }
    PalomaAction::dirty(String::new())
}

/// Lanza `paloma-oauth <id>` en segundo plano (abre el navegador y guarda el
/// token). Best-effort: si el binario no está instalado, lo informa.
fn spawn_authorizer(id: &str) -> PalomaAction {
    match std::process::Command::new("paloma-oauth").arg(id).spawn() {
        Ok(_) => PalomaAction::clean(format!("autorizando «{id}» en el navegador…")),
        Err(e) => PalomaAction::clean(format!("no pude lanzar paloma-oauth: {e}")),
    }
}

/// Copia `text` (si vino) en `dst`.
fn set_text(dst: &mut String, text: Option<String>) {
    if let Some(t) = text {
        *dst = t;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> PalomaState {
        PalomaState { cfg: PalomaConfig::default(), dir: None }
    }

    fn rel(section: &str, leaf: &str) -> FieldPath {
        FieldPath(vec![section.to_string(), leaf.to_string()])
    }

    fn has_section(s: &Schema, id: &str) -> bool {
        s.sections.iter().any(|sec| sec.id == id)
    }

    #[test]
    fn crear_agrega_cuenta_y_la_activa() {
        let mut st = state();
        let act = route(&mut st, &rel("cuentas", "crear"), FieldValue::Text("ana@gmail.com".into()));
        assert!(act.dirty);
        assert_eq!(st.cfg.accounts.len(), 1);
        assert_eq!(st.cfg.active_account().unwrap().email, "ana@gmail.com");
    }

    #[test]
    fn proveedor_google_vuelve_la_cuenta_oauth_y_muestra_seccion() {
        let mut st = state();
        route(&mut st, &rel("cuentas", "crear"), FieldValue::Text("ana@gmail.com".into()));
        route(&mut st, &rel("cuenta", "proveedor"), FieldValue::Enum("google".into()));
        let e = st.cfg.active_account().unwrap();
        assert!(e.is_oauth());
        assert_eq!(e.imap_host, "imap.gmail.com");
        // La sección OAuth aparece en el schema sólo cuando la cuenta es OAuth.
        assert!(has_section(&schema(&st), "paloma::oauth"));
    }

    #[test]
    fn cuenta_password_no_muestra_seccion_oauth() {
        let mut st = state();
        route(&mut st, &rel("cuentas", "crear"), FieldValue::Text("yo@dominio.com".into()));
        let s = schema(&st);
        assert!(has_section(&s, "paloma::cuenta"));
        assert!(!has_section(&s, "paloma::oauth"));
    }

    #[test]
    fn editar_puerto_parsea_y_guarda() {
        let mut st = state();
        route(&mut st, &rel("cuentas", "crear"), FieldValue::Text("yo@dominio.com".into()));
        route(&mut st, &rel("cuenta", "imap_port"), FieldValue::Text("1993".into()));
        assert_eq!(st.cfg.active_account().unwrap().imap_port, 1993);
        // Un puerto inválido no rompe ni pisa el válido.
        route(&mut st, &rel("cuenta", "imap_port"), FieldValue::Text("xxx".into()));
        assert_eq!(st.cfg.active_account().unwrap().imap_port, 1993);
    }

    #[test]
    fn usar_conmuta_la_cuenta_activa() {
        let mut st = state();
        route(&mut st, &rel("cuentas", "crear"), FieldValue::Text("a@x.com".into()));
        let first = st.cfg.active_id();
        route(&mut st, &rel("cuentas", "crear"), FieldValue::Text("b@x.com".into()));
        assert_ne!(st.cfg.active_id(), first);
        route(&mut st, &rel("cuentas", "usar"), FieldValue::Text(first.clone()));
        assert_eq!(st.cfg.active_id(), first);
    }

    #[test]
    fn no_elimina_la_ultima_cuenta() {
        let mut st = state();
        route(&mut st, &rel("cuentas", "crear"), FieldValue::Text("solo@x.com".into()));
        let act = route(&mut st, &rel("cuentas", "eliminar"), FieldValue::Bool(true));
        assert!(!act.dirty);
        assert_eq!(st.cfg.accounts.len(), 1);
    }
}
