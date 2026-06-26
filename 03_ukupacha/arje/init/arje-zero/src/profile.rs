//! Perfiles de arranque por composición de sesión.
//!
//! Un **perfil** no es una estructura nueva: es la Tarjeta Semilla base
//! (`arje-host` / `arje-qemu`) — que ya trae los inits básicos y el
//! greeter (el DM) — con, opcionalmente, los entes de una **sesión**
//! anexados a su `genesis`.
//!
//! - `arje.session=mirada` (o ausente): la base sola. Mirada es nativo;
//!   no necesita servicios de sistema extra. Es el default.
//! - `arje.session=gnome`: la base + el fragmento `session-gnome`, que
//!   aporta los shims D-Bus de `arje-compat` (logind, hostnamed,
//!   timedated, …) para que una sesión GNOME lanzada desde el greeter
//!   encuentre los `org.freedesktop.*` que consulta al arrancar.
//!
//! El punto de selección es el cmdline del kernel (`/proc/cmdline`), o la
//! env `ARJE_SESSION` en dev. La elección *fina* de qué sesión iniciar
//! tras autenticar es del greeter (que ya es el DM); este overlay sólo
//! decide qué backends de sistema están presentes para esa sesión.
//!
//! **v0 = eager**: los shims se declaran como entes del `genesis` y se
//! encarnan al boot. El upgrade natural es **activación perezosa** (al
//! estilo D-Bus): registrar los nombres como activables y spawnear el
//! shim al primer request. Cuando exista esa capa, el fragmento pasa a
//! declarar *disponibilidad* en vez de *spawn*.

use std::collections::HashSet;
use std::path::PathBuf;

use arje_card::EntityCard;
use tracing::{info, warn};

/// Anexa los entes de `session` al `genesis` de `base`, dedup por label.
///
/// Pura y determinista: no toca disco ni entorno. Idempotente — si un
/// ente de la sesión ya existe en la base (mismo `label`), se omite en
/// vez de duplicarse. Es la operación que materializa "perfil = base (+
/// sesión)".
pub fn overlay_session(mut base: EntityCard, session: EntityCard) -> EntityCard {
    let present: HashSet<String> = base.genesis.iter().map(|c| c.label.clone()).collect();
    for ente in session.genesis {
        if present.contains(&ente.label) {
            warn!(label = %ente.label, "sesión: ente ya presente en la base, no se duplica");
            continue;
        }
        base.genesis.push(ente);
    }
    base
}

/// Extrae el nombre de sesión de un cmdline estilo `/proc/cmdline`.
///
/// Busca el token `arje.session=<name>`. Devuelve `None` si no aparece o
/// si el valor está vacío.
pub fn parse_session(cmdline: &str) -> Option<String> {
    for tok in cmdline.split_whitespace() {
        if let Some(v) = tok.strip_prefix("arje.session=") {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Sesión seleccionada: `ARJE_SESSION` (dev) tiene prioridad sobre el
/// `arje.session=` del cmdline del kernel. `None` = sin selección.
fn selected_session() -> Option<String> {
    if let Ok(v) = std::env::var("ARJE_SESSION") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    let cmdline = std::fs::read_to_string("/proc/cmdline").ok()?;
    parse_session(&cmdline)
}

/// Directorio de fragmentos de sesión, primer candidato existente.
fn fragments_dir(dev_mode: bool) -> Option<PathBuf> {
    let cands: &[&str] = if dev_mode {
        &["seeds/fragments", "fragments"]
    } else {
        &["/ente/fragments"]
    };
    cands.iter().map(PathBuf::from).find(|p| p.is_dir())
}

/// Aplica la sesión seleccionada sobre la base, best-effort.
///
/// Devuelve la base intacta cuando no hay sesión, cuando es la nativa
/// (`mirada`/`default`/`base`), o cuando el fragmento no se encuentra /
/// no valida — un perfil mal nombrado **nunca** debe dejar sin arranque.
pub fn apply_selected_session(base: EntityCard, dev_mode: bool) -> EntityCard {
    let Some(name) = selected_session() else {
        return base;
    };
    if matches!(name.as_str(), "mirada" | "default" | "base") {
        info!(session = %name, "sesión nativa — sin overlay de fragmento");
        return base;
    }
    let Some(dir) = fragments_dir(dev_mode) else {
        warn!(session = %name, "sesión solicitada pero no hay directorio de fragmentos — arranco base");
        return base;
    };
    let path = dir.join(format!("session-{name}.card.json"));
    if !path.exists() {
        warn!(session = %name, path = %path.display(), "fragmento de sesión inexistente — arranco base");
        return base;
    }
    match EntityCard::from_path(&path) {
        Ok(session) => {
            let aportados = session.genesis.len();
            let out = overlay_session(base, session);
            info!(
                session = %name,
                aportados,
                total = out.genesis.len(),
                "sesión aplicada sobre la base"
            );
            out
        }
        Err(e) => {
            warn!(session = %name, error = %e, "fragmento de sesión inválido — arranco base");
            base
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arje_card::Payload;

    fn ente(label: &str) -> EntityCard {
        let mut c = EntityCard::new(label);
        c.payload = Payload::Virtual;
        c
    }

    fn con_genesis(label: &str, hijos: &[&str]) -> EntityCard {
        let mut c = ente(label);
        c.genesis = hijos.iter().map(|l| ente(l)).collect();
        c
    }

    #[test]
    fn parse_session_extrae_el_valor() {
        let cmd = "BOOT_IMAGE=/vmlinuz arje.session=gnome console=tty1 quiet";
        assert_eq!(parse_session(cmd).as_deref(), Some("gnome"));
    }

    #[test]
    fn parse_session_ausente_es_none() {
        assert_eq!(parse_session("console=ttyS0 ro quiet"), None);
        assert_eq!(parse_session("arje.session="), None);
    }

    #[test]
    fn overlay_anexa_los_entes_de_la_sesion() {
        let base = con_genesis("arje-host", &["agetty", "greeter"]);
        let session = con_genesis("session-gnome", &["compat-logind", "compat-hostnamed"]);
        let out = overlay_session(base, session);
        assert_eq!(out.genesis.len(), 4);
        let labels: Vec<&str> = out.genesis.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"greeter"));
        assert!(labels.contains(&"compat-logind"));
    }

    #[test]
    fn overlay_es_idempotente_por_label() {
        let base = con_genesis("arje-host", &["agetty", "compat-logind"]);
        let session = con_genesis("session-gnome", &["compat-logind", "compat-hostnamed"]);
        let out = overlay_session(base, session);
        // compat-logind ya estaba: no se duplica.
        assert_eq!(out.genesis.len(), 3);
        let n_logind = out
            .genesis
            .iter()
            .filter(|c| c.label == "compat-logind")
            .count();
        assert_eq!(n_logind, 1);
    }
}
