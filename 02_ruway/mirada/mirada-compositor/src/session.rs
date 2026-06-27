//! Coordinador de sesión (arranque del session-manager).
//!
//! Toma las apps de la sesión del usuario (autostart / config / la sesión
//! elegida) y, en vez de lanzarlas como procesos crudos, se las entrega a arje
//! como **Entes supervisados que dependen del piso** (`wayland_floor()`):
//!
//!   - `supervision: Restart` ⇒ si la app crashea, arje la reinicia.
//!   - `requires: [wayland_floor()]` ⇒ si el compositor cae y se lleva a la app,
//!     arje la APARCA y la re-erige (re-floor) cuando el compositor vuelve. El
//!     usuario ve su sesión reconstruirse sola, en orden, sin relanzar nada.
//!
//! Estado: **arranque**. El coordinador (armar Card + `RunCard`) está hecho y
//! testeado. Falta UN enabler en arje para volverlo el camino por defecto: arje
//! corre los Entes como **root** (PID 1) y el modelo de Card todavía NO tiene
//! drop de **uid/gid**, así que una app de usuario como Ente correría como root
//! — regresión frente al `spawn_command` actual (que hace `setuid` al usuario).
//! Por eso acá va **opt-in** (`MIRADA_SESSION_ENTES=1`); pasa a default cuando
//! `Payload::Native`/`SomaSpec` ganen `run_as { uid, gid }` y arje-incarnate lo
//! honre. Ver el seam en `utilidades::spawn_command`.

use std::time::Duration;

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::{wayland_floor, EntityCard, FsPolicy, NetworkingPolicy, Payload, Supervision, WireCard};

/// ¿Está activado el modo "apps de sesión como Entes de arje"? Opt-in mientras
/// arje no dropee uid/gid (ver doc del módulo).
pub(crate) fn ente_mode() -> bool {
    std::env::var_os("ENTE_BUS_SOCK").is_some()
        && std::env::var("MIRADA_SESSION_ENTES").map(|v| v == "1").unwrap_or(false)
}

/// Construye la Card de un Ente de sesión: corre `cmd` vía `sh -c` con el
/// entorno de la sesión, depende del piso y se reinicia si cae.
pub(crate) fn session_app_card(label: &str, cmd: &str, session_env: &[(String, String)]) -> EntityCard {
    let mut card = EntityCard::new(format!("session.{label}"));
    card.requires = std::iter::once(wayland_floor()).collect();
    card.supervision = Supervision::Restart {
        initial: Duration::from_millis(500),
        max: Duration::from_secs(30),
    };
    // Una app de escritorio necesita red, FS y poder lanzar hijos.
    card.permissions.networking = NetworkingPolicy::Full;
    card.permissions.filesystem = FsPolicy::ReadWrite;
    card.permissions.processes = true;
    card.payload = Payload::Native {
        exec: "/bin/sh".into(),
        argv: vec!["-c".into(), cmd.into()],
        envp: session_env.to_vec(),
    };
    card
}

/// Intenta lanzar `cmd` como Ente de sesión vía `RunCard` (best-effort,
/// síncrono — el bucle es calloop). Devuelve `true` si arje lo aceptó (el caller
/// se saltea el spawn crudo); `false` para caer al camino normal.
pub(crate) fn try_spawn_as_ente(label: &str, cmd: &str, session_env: &[(String, String)]) -> bool {
    let card = session_app_card(label, cmd, session_env);
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(_) => return false,
    };
    rt.block_on(async move {
        let mut client = match BusClient::from_env().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("mirada/session: sin bus para RunCard — sigo crudo: {e}");
                return false;
            }
        };
        let wire: WireCard = card.into();
        match client.call(BusRequest::RunCard { card: wire }).await {
            Ok(BusResponse::Ok) => {
                eprintln!("mirada/session: «{label}» lanzado como Ente (supervisado + re-floor)");
                true
            }
            Ok(other) => {
                eprintln!("mirada/session: arje rechazó RunCard de «{label}» — sigo crudo: {other:?}");
                false
            }
            Err(e) => {
                eprintln!("mirada/session: RunCard de «{label}» falló — sigo crudo: {e}");
                false
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arje_card::Capability;

    #[test]
    fn la_card_depende_del_piso_y_se_reinicia() {
        let env = vec![("WAYLAND_DISPLAY".to_string(), "wayland-1".to_string())];
        let card = session_app_card("foot", "foot --server", &env);
        // Depende del piso ⇒ park & re-floor.
        assert!(card.requires.contains(&wayland_floor()));
        // Se reinicia si cae.
        assert!(matches!(card.supervision, Supervision::Restart { .. }));
        // Corre el comando vía sh -c con el entorno de sesión.
        match &card.payload {
            Payload::Native { exec, argv, envp } => {
                assert_eq!(exec, "/bin/sh");
                assert_eq!(argv, &vec!["-c".to_string(), "foot --server".to_string()]);
                assert!(envp.iter().any(|(k, _)| k == "WAYLAND_DISPLAY"));
            }
            other => panic!("payload no es Native: {other:?}"),
        }
        // La Card es válida.
        card.validate().unwrap();
        let _ = Capability::Spawn; // (sanity: el tipo está en scope)
    }

    #[test]
    fn label_namespaced() {
        let card = session_app_card("editor", "nada", &[]);
        assert_eq!(card.label, "session.editor");
    }
}
