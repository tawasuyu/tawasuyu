//! Sección **Autologin** del diente «Inicio»: entrar sin tipear contraseña como
//! preferencia, y —si se activa— **elegir explícitamente** el tradeoff de
//! secretos (cifrado de dotfiles), con sus warnings.
//!
//! Escribe `auth_core::AutologinCfg` (archivo `mirada/autologin.conf`, leído por
//! el greeter). Para el modo «Sellada» además guarda la frase en un archivo 0600
//! y siembra el auto-desbloqueo en el autostart de mirada.

use std::io::Write;
use std::path::PathBuf;

use allichay::{EnumOption, Field, FieldPath, FieldValue, Schema, Section};
use auth_core::{AutologinCfg, SecretosPolitica};
use directories::ProjectDirs;

/// Estado del diente de autologin: la config + la frase tipeada (transitoria).
pub struct AutologinState {
    pub cfg: AutologinCfg,
    /// Frase para sellar (no se persiste en `AutologinState`; se escribe al sello).
    pass: String,
}

/// Acción que devuelve el ruteo al panel.
pub struct AutologinAction {
    pub dirty: bool,
    pub status: String,
}
impl AutologinAction {
    fn dirty(s: impl Into<String>) -> Self {
        Self { dirty: true, status: s.into() }
    }
    fn clean(s: impl Into<String>) -> Self {
        Self { dirty: false, status: s.into() }
    }
}

impl AutologinState {
    pub fn load() -> Self {
        Self { cfg: AutologinCfg::load(), pass: String::new() }
    }
    pub fn save(&self) -> Result<(), String> {
        self.cfg.save().map_err(|e| format!("autologin.conf: {e}"))
    }
}

const SECRETOS: &[(&str, &str)] = &[
    ("manual", "Manual — seguro (ninguna frase toca el disco)"),
    ("sellada", "Sellada — cómodo (la frase queda en disco 0600)"),
];

/// La sección que el diente «Inicio» agrega.
pub fn section(state: &AutologinState) -> Section {
    let mut sec = Section::new("autologin::cfg", "Autologin").icon("⏻").help(
        "Entrar sin tipear contraseña, como preferencia de este equipo. Pensado para \
         máquinas de un solo usuario o de confianza física: cualquiera que la encienda \
         entra como vos. Si lo activás, elegí abajo qué pasa con el cifrado de tus \
         dotfiles —ahí está el tradeoff real.",
    );
    sec = sec.field(Field::toggle("enabled", "Activar autologin", state.cfg.enabled));
    if !state.cfg.enabled {
        return sec;
    }
    sec = sec
        .field(Field::text("user", "Usuario", state.cfg.user.clone()))
        .field(Field::text(
            "session",
            "Sesión (nombre del .desktop; vacío = la por defecto)",
            state.cfg.session.clone(),
        ));

    // El corazón del pedido: las dos opciones de secretos con sus warnings.
    let opts: Vec<EnumOption> =
        SECRETOS.iter().map(|(v, l)| EnumOption::new(v.to_string(), l.to_string())).collect();
    sec = sec.field(Field::radio("secretos", "Secretos al entrar", state.cfg.secretos.tag(), opts));

    match state.cfg.secretos {
        SecretosPolitica::Manual => {
            sec = sec.field(Field::display(
                "warn",
                "⚠ Manual",
                "Tus dotfiles cifrados quedan BLOQUEADOS al entrar (el almacén no se \
                 descifra solo). Desbloqueá cuando quieras en Contextos → Identidad, o \
                 con `agora-cli desbloquear`. Es el modo seguro: el «secreto para acceder \
                 a los secretos» sigue sólo en tu cabeza."
                    .to_string(),
            ));
        }
        SecretosPolitica::Sellada => {
            sec = sec
                .field(Field::display(
                    "warn",
                    "⚠ Sellada",
                    "Para descifrar sin que tipees nada, la frase de tu identidad se guarda \
                     en un archivo privado (permisos 0600) y la sesión la usa al entrar. \
                     DEBILITA el modelo: quien lea ese archivo (root, un backup, un robo de \
                     disco sin cifrar) puede desbloquear tus secretos. Usalo sólo con disco \
                     cifrado (LUKS) y si entendés el riesgo."
                        .to_string(),
                ))
                .field(Field::text("passphrase", "Frase a sellar (no se muestra al volver)", ""))
                .field(Field::button("sellar", "Sellar la frase y activar auto-desbloqueo"))
                .field(Field::display("sello", "Sello", estado_sello()));
        }
    }
    sec
}

pub fn text_value(state: &AutologinState, rel: &FieldPath) -> Option<String> {
    if rel.segments().first().map(String::as_str)? != "cfg" {
        return None;
    }
    Some(match rel.leaf()? {
        "user" => state.cfg.user.clone(),
        "session" => state.cfg.session.clone(),
        _ => String::new(),
    })
}

pub fn route(state: &mut AutologinState, rel: &FieldPath, value: FieldValue) -> AutologinAction {
    if rel.segments().first().map(String::as_str) != Some("cfg") {
        return AutologinAction::clean(String::new());
    }
    match rel.leaf() {
        Some("enabled") => {
            state.cfg.enabled = value.as_bool().unwrap_or(false);
            AutologinAction::dirty(if state.cfg.enabled {
                "autologin activado"
            } else {
                "autologin desactivado"
            })
        }
        Some("user") => {
            if let Some(v) = value.as_str() {
                state.cfg.user = v.to_string();
            }
            AutologinAction::dirty(String::new())
        }
        Some("session") => {
            if let Some(v) = value.as_str() {
                state.cfg.session = v.to_string();
            }
            AutologinAction::dirty(String::new())
        }
        Some("secretos") => {
            if let Some(v) = value.as_str().and_then(SecretosPolitica::parse) {
                state.cfg.secretos = v;
                // Pasar a Manual: limpiar el sello y el auto-desbloqueo.
                if v == SecretosPolitica::Manual {
                    quitar_sello();
                }
            }
            AutologinAction::dirty(String::new())
        }
        Some("passphrase") => {
            if let Some(v) = value.as_str() {
                state.pass = v.to_string();
            }
            AutologinAction::clean(String::new())
        }
        Some("sellar") if value.as_bool() == Some(true) => {
            let r = sellar(&state.pass);
            state.pass.clear();
            match r {
                Ok(()) => AutologinAction::clean("frase sellada — se auto-desbloqueará al entrar".to_string()),
                Err(e) => AutologinAction::clean(format!("no se pudo sellar: {e}")),
            }
        }
        _ => AutologinAction::clean(String::new()),
    }
}

// ── sello: archivo 0600 + línea de autostart ────────────────────────────────

fn config_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "mirada").map(|d| d.config_dir().to_path_buf())
}

fn sello_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join(".agora-seal"))
}

fn autostart_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("autostart"))
}

/// La línea de autostart que auto-desbloquea desde el sello.
fn linea_autostart(sello: &std::path::Path) -> String {
    format!("agora-cli desbloquear --passphrase-file {}", sello.display())
}

fn estado_sello() -> String {
    match sello_path() {
        Some(p) if p.exists() => format!("activo ({})", p.display()),
        _ => "sin sellar".to_string(),
    }
}

/// Escribe la frase al sello 0600 y siembra el auto-desbloqueo en el autostart.
fn sellar(pass: &str) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    if pass.is_empty() {
        return Err("frase vacía".into());
    }
    let sello = sello_path().ok_or("sin dir de config de mirada")?;
    if let Some(dir) = sello.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&sello)
        .map_err(|e| e.to_string())?;
    f.write_all(pass.as_bytes()).map_err(|e| e.to_string())?;
    // Asegurar 0600 aunque el archivo ya existiera con otros permisos.
    std::fs::set_permissions(&sello, std::os::unix::fs::PermissionsExt::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    sembrar_autostart(&linea_autostart(&sello))?;
    Ok(())
}

/// Agrega la línea al autostart si no está (idempotente).
fn sembrar_autostart(linea: &str) -> Result<(), String> {
    let auto = autostart_path().ok_or("sin dir de config de mirada")?;
    if let Some(dir) = auto.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let actual = std::fs::read_to_string(&auto).unwrap_or_default();
    if actual.lines().any(|l| l.trim() == linea) {
        return Ok(());
    }
    let mut nuevo = actual;
    if !nuevo.is_empty() && !nuevo.ends_with('\n') {
        nuevo.push('\n');
    }
    nuevo.push_str(linea);
    nuevo.push('\n');
    std::fs::write(&auto, nuevo).map_err(|e| e.to_string())
}

/// Borra el sello y quita la línea de autostart (al volver a Manual).
fn quitar_sello() {
    if let Some(p) = sello_path() {
        let _ = std::fs::remove_file(&p);
    }
    if let (Some(auto), Some(sello)) = (autostart_path(), sello_path()) {
        if let Ok(actual) = std::fs::read_to_string(&auto) {
            let objetivo = linea_autostart(&sello);
            let filtrado: String = actual
                .lines()
                .filter(|l| l.trim() != objetivo)
                .map(|l| format!("{l}\n"))
                .collect();
            let _ = std::fs::write(&auto, filtrado);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(leaf: &str) -> FieldPath {
        FieldPath(vec!["cfg".to_string(), leaf.to_string()])
    }

    #[test]
    fn toggle_y_secretos_rutean() {
        let mut st = AutologinState { cfg: AutologinCfg::default(), pass: String::new() };
        let a = route(&mut st, &rel("enabled"), FieldValue::Bool(true));
        assert!(a.dirty && st.cfg.enabled);
        route(&mut st, &rel("user"), FieldValue::Text("sergio".into()));
        route(&mut st, &rel("secretos"), FieldValue::Enum("sellada".into()));
        assert_eq!(st.cfg.user, "sergio");
        assert_eq!(st.cfg.secretos, SecretosPolitica::Sellada);
        // El schema se arma con las dos opciones de secretos + warning de Sellada.
        let sec = section(&st);
        assert!(sec.fields.iter().any(|f| f.id == "secretos"));
        assert!(sec.fields.iter().any(|f| f.id == "passphrase"));
    }

    #[test]
    fn deshabilitado_no_muestra_detalle() {
        let st = AutologinState { cfg: AutologinCfg::default(), pass: String::new() };
        let sec = section(&st);
        // Sólo el toggle.
        assert!(sec.fields.iter().any(|f| f.id == "enabled"));
        assert!(!sec.fields.iter().any(|f| f.id == "secretos"));
    }
}
