//! Reconciliación de backends de sistema por sesión, vía el bus de arje.
//!
//! Cuando el usuario elige en el login una sesión que necesita servicios
//! que no son nativos de mirada (p. ej. GNOME, que consulta los
//! `org.freedesktop.*` al arrancar), el greeter —que es el DM, un Ente del
//! fractal— reconcilia con su init (`arje-zero`):
//!
//! - **Levanta** el bundle de la sesión elegida: un `SpawnCardFromDisk
//!   { name: "session-<perfil>" }` que encarna los shims de `arje-compat`
//!   instalados en `/etc/arje/cards.d/`.
//! - **Baja** los otros perfiles opcionales que hayan quedado vivos de una
//!   sesión anterior: un `StopCardFromDisk { name: "session-<otro>" }` que
//!   los detiene sin que su supervisor `Restart` los revive (teardown, p.
//!   ej. al volver de gnome a mirada).
//!
//! Es la vía "login-time" del perfil de arranque (la otra es el overlay a
//! boot por `arje.session=` en el cmdline). Best-effort y acotado: sin bus
//! (greeter fuera de arje, dev), perfil inexistente, o bus lento → el login
//! continúa igual. Nunca bloquea ni tumba el traspaso. Loguea a **stderr**:
//! stdout es el canal de protocolo con el compositor (`emit_action`).

use std::time::Duration;

use arje_bus::{BusRequest, BusResponse};

use crate::sessions::Session;

/// Tope de espera de cada llamada al bus: el login no puede colgarse por un
/// init lento o muerto.
const BUS_TIMEOUT: Duration = Duration::from_secs(2);

/// Descriptor de un perfil de sesión con backends de sistema gestionados por
/// arje. Es la **fuente única**: agregar un perfil = una fila acá + el
/// fragmento `session-<name>.card.json` + sus shims instalados. El matcher se
/// aplica sobre el `.desktop` elegido en el greeter.
struct Profile {
    /// Nombre del perfil = sufijo de la card (`session-<name>`).
    name: &'static str,
    /// Prefijo del basename del `Exec` que delata el perfil (p. ej.
    /// `gnome-session`).
    exec_prefix: &'static str,
    /// Substring (lowercase) del `Name` del `.desktop` que también lo delata.
    name_substr: &'static str,
}

/// Tabla de perfiles conocidos. Hoy sólo GNOME.
const PROFILES: &[Profile] = &[Profile {
    name: "gnome",
    exec_prefix: "gnome-session",
    name_substr: "gnome",
}];

/// Perfil de arje que una sesión necesita, si alguno. `None` = sesión
/// autosuficiente (mirada nativo, o un compositor ajeno que no depende de
/// los shims systemd de arje). Casa por `exec`/`name` contra [`PROFILES`].
pub fn profile_for(session: &Session) -> Option<&'static str> {
    let exec = session.exec.to_lowercase();
    let first = exec.split_whitespace().next().unwrap_or("");
    let base = first.rsplit('/').next().unwrap_or(first);
    let name = session.name.to_lowercase();
    PROFILES
        .iter()
        .find(|p| base.starts_with(p.exec_prefix) || name.contains(p.name_substr))
        .map(|p| p.name)
}

/// Perfiles a bajar dado el seleccionado: todos los conocidos menos el
/// elegido. Pura y testeable.
pub fn profiles_to_deactivate(selected: Option<&str>) -> Vec<&'static str> {
    PROFILES
        .iter()
        .map(|p| p.name)
        .filter(|p| Some(*p) != selected)
        .collect()
}

/// Reconcilia los backends de sistema con la sesión elegida: levanta el
/// bundle del perfil seleccionado (si tiene) y baja los otros perfiles
/// opcionales que pudieran haber quedado vivos de una sesión anterior.
///
/// Se llama **antes** de emitir el `SessionTicket`. Bloquea hasta
/// `BUS_TIMEOUT` por llamada; cualquier fallo se registra en stderr y se
/// ignora. En el caso de un solo perfil opcional hace a lo sumo una llamada.
pub fn reconcile(selected: Option<&str>) {
    let to_stop: Vec<String> = profiles_to_deactivate(selected)
        .iter()
        .map(|p| card_name(p))
        .collect();
    // En modo lazy (marcador presente), el dbus-daemon del host activa los
    // shims on-demand vía `arje-activate`: el greeter NO los levanta eager.
    // El teardown (to_stop) se mantiene igual: bajar al salir sigue siendo del
    // greeter, y `StopCardFromDisk` baja los miembros vivos por label sin
    // importar cómo se encarnaron.
    let to_start = selected
        .filter(|p| {
            if is_lazy(p) {
                eprintln!("arje_session: «{p}» es lazy — dbus lo activa on-demand, no levanto eager");
                false
            } else {
                true
            }
        })
        .map(card_name);

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("arje_session: no se pudo crear runtime: {e}");
            return;
        }
    };
    rt.block_on(async move {
        for card in to_stop {
            let what = format!("bajar «{card}»");
            do_call(BusRequest::StopCardFromDisk { name: card }, &what).await;
        }
        if let Some(card) = to_start {
            let what = format!("levantar «{card}»");
            do_call(BusRequest::SpawnCardFromDisk { name: card }, &what).await;
        }
    });
}

fn card_name(profile: &str) -> String {
    format!("session-{profile}")
}

/// Ruta del marcador que el instalador deja en modo lazy: si existe, ese
/// perfil lo activa el dbus-daemon del host on-demand (vía `arje-activate`),
/// así el greeter no lo levanta eager.
fn lazy_marker(profile: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/etc/arje/session-{profile}.lazy"))
}

fn is_lazy(profile: &str) -> bool {
    lazy_marker(profile).exists()
}

/// Una llamada al bus, con tope de espera, best-effort (loguea y sigue).
async fn do_call(req: BusRequest, what: &str) {
    match tokio::time::timeout(BUS_TIMEOUT, call_inner(req)).await {
        Ok(Ok(())) => eprintln!("arje_session: {what} OK"),
        Ok(Err(e)) => eprintln!("arje_session: {what} falló — sigo: {e}"),
        Err(_) => eprintln!("arje_session: {what} expiró — sigo"),
    }
}

async fn call_inner(req: BusRequest) -> anyhow::Result<()> {
    let mut client = arje_bus::BusClient::from_env().await?;
    match client.call(req).await? {
        BusResponse::Ok => Ok(()),
        other => anyhow::bail!("bus rechazó: {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::{Kind, Session};

    fn sess(name: &str, exec: &str) -> Session {
        Session {
            name: name.into(),
            exec: exec.into(),
            kind: Kind::Wayland,
            foreign: true,
        }
    }

    #[test]
    fn gnome_por_exec() {
        assert_eq!(profile_for(&sess("GNOME", "gnome-session")), Some("gnome"));
        assert_eq!(
            profile_for(&sess("Sesión", "/usr/bin/gnome-session --systemd")),
            Some("gnome")
        );
    }

    #[test]
    fn gnome_por_nombre() {
        assert_eq!(
            profile_for(&sess("GNOME on Wayland", "/usr/bin/gnome-shell")),
            Some("gnome")
        );
    }

    #[test]
    fn mirada_y_otros_sin_perfil() {
        assert_eq!(profile_for(&sess("mirada", "")), None);
        assert_eq!(profile_for(&sess("Sway", "sway")), None);
        assert_eq!(profile_for(&sess("Plasma", "startplasma-wayland")), None);
    }

    #[test]
    fn marcador_lazy_por_perfil() {
        assert_eq!(
            lazy_marker("gnome"),
            std::path::PathBuf::from("/etc/arje/session-gnome.lazy")
        );
    }

    #[test]
    fn reconcilia_baja_los_otros_opcionales() {
        // Elegir gnome: no se baja gnome (es el seleccionado).
        assert!(profiles_to_deactivate(Some("gnome")).is_empty());
        // Elegir mirada (None): se baja gnome.
        assert_eq!(profiles_to_deactivate(None), vec!["gnome"]);
        // Un perfil desconocido: igual baja los opcionales conocidos.
        assert_eq!(profiles_to_deactivate(Some("kde")), vec!["gnome"]);
    }
}
