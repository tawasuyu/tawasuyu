//! Activación de backends de sistema por sesión, vía el bus de arje.
//!
//! Cuando el usuario elige en el login una sesión que necesita servicios
//! que no son nativos de mirada (p. ej. GNOME, que consulta los
//! `org.freedesktop.*` al arrancar), el greeter —que es el DM, un Ente del
//! fractal— le pide a su init (`arje-zero`) que encarne el *bundle* de esa
//! sesión: un solo `SpawnCardFromDisk { name: "session-<perfil>" }` que
//! levanta los shims de `arje-compat` instalados en `/etc/arje/cards.d/`.
//!
//! Es la vía "login-time" del perfil de arranque (la otra es el overlay a
//! boot por `arje.session=` en el cmdline). El acople boot↔login se cierra
//! aquí: los backends de GNOME se levantan **cuando se elige esa sesión**,
//! no eagermente al arranque.
//!
//! Best-effort y acotado: sin bus (greeter fuera de arje, dev), perfil
//! inexistente, o bus lento → el login continúa igual. Nunca bloquea ni
//! tumba el traspaso. Loguea a **stderr**: stdout es el canal de protocolo
//! con el compositor (`emit_action`).

use std::time::Duration;

use crate::sessions::Session;

/// Tope de espera de la llamada al bus: el login no puede colgarse por un
/// init lento o muerto.
const BUS_TIMEOUT: Duration = Duration::from_secs(2);

/// Perfil de arje que una sesión necesita, si alguno. `None` = sesión
/// autosuficiente (mirada nativo, o un compositor ajeno que no depende de
/// los shims systemd de arje).
///
/// Heurística por `exec`/`name`: hoy sólo GNOME. Ampliable a una tabla o a
/// un campo del `.desktop` cuando aparezca otra sesión con backends arje.
pub fn profile_for(session: &Session) -> Option<&'static str> {
    let exec = session.exec.to_lowercase();
    let first = exec.split_whitespace().next().unwrap_or("");
    let base = first.rsplit('/').next().unwrap_or(first);
    if base.starts_with("gnome-session") || session.name.to_lowercase().contains("gnome") {
        return Some("gnome");
    }
    None
}

/// Pide al bus de arje encarnar el bundle de la sesión `profile`. Bloquea
/// hasta `BUS_TIMEOUT`; cualquier fallo se registra en stderr y se ignora.
///
/// Se llama **antes** de emitir el `SessionTicket`: deja el request
/// entregado a `arje-zero` (que encarna los shims en paralelo) justo antes
/// de soltar el DRM hacia la sesión ajena.
pub fn activate(profile: &str) {
    let card = format!("session-{profile}");
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("arje_session: no se pudo crear runtime para «{card}»: {e}");
            return;
        }
    };
    rt.block_on(async {
        match tokio::time::timeout(BUS_TIMEOUT, activate_inner(&card)).await {
            Ok(Ok(())) => eprintln!("arje_session: bundle «{card}» activado en arje"),
            Ok(Err(e)) => eprintln!("arje_session: activación de «{card}» falló — sigo: {e}"),
            Err(_) => eprintln!("arje_session: activación de «{card}» expiró — sigo"),
        }
    });
}

async fn activate_inner(card: &str) -> anyhow::Result<()> {
    use arje_bus::{BusRequest, BusResponse};
    let mut client = arje_bus::BusClient::from_env().await?;
    let req = BusRequest::SpawnCardFromDisk {
        name: card.to_string(),
    };
    match client.call(req).await? {
        BusResponse::Ok => Ok(()),
        other => anyhow::bail!("bus rechazó SpawnCardFromDisk: {other:?}"),
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
}
