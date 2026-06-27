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
//! El enabler de uid/gid YA ESTÁ en arje: `SomaSpec.run_as` (card-core) +
//! `ChildPreExec::DropPrivileges` (arje-incarnate, setgroups→setgid→setuid). Así
//! el Ente de sesión corre **como el usuario logueado**, no como root — paridad
//! con el `setuid` del `spawn_command` crudo.
//!
//! Estado: **arranque, opt-in** (`MIRADA_SESSION_ENTES=1`). Sigue opt-in —y con
//! fallback al spawn crudo— hasta verificar en HARDWARE la paridad fina de sesión
//! (cwd al home, `setsid`, entorno completo) con apps reales; al confirmar, pasa
//! a default. Seam: `utilidades::spawn_command` (chokepoint de todas las apps).

use std::time::Duration;

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::{
    wayland_floor, EntityCard, FsPolicy, NetworkingPolicy, Payload, RunAs, Supervision, WireCard,
};
use auth_core::UserInfo;

/// ¿Está activado el modo "apps de sesión como Entes de arje"? Opt-in mientras
/// arje no dropee uid/gid (ver doc del módulo).
pub(crate) fn ente_mode() -> bool {
    std::env::var_os("ENTE_BUS_SOCK").is_some()
        && std::env::var("MIRADA_SESSION_ENTES").map(|v| v == "1").unwrap_or(false)
}

/// Grupos suplementarios del usuario (NSS via `getgrouplist`; corre en el padre,
/// async-signal-safe NO requerido acá). Cae al gid primario si falla.
fn user_groups(user: &UserInfo) -> Vec<u32> {
    use nix::unistd::Gid;
    std::ffi::CString::new(user.name.as_bytes())
        .ok()
        .and_then(|name| nix::unistd::getgrouplist(&name, Gid::from_raw(user.gid)).ok())
        .map(|gs| gs.into_iter().map(|g| g.as_raw()).collect())
        .unwrap_or_else(|| vec![user.gid])
}

/// Construye la Card de un Ente de sesión: corre `cmd` vía `sh -c` con el
/// entorno de la sesión, depende del piso, se reinicia si cae y —si hay
/// `user`— BAJA privilegios a ese usuario (`run_as`) para no correr como root.
pub(crate) fn session_app_card(
    label: &str,
    cmd: &str,
    user: Option<&UserInfo>,
    session_env: &[(String, String)],
) -> EntityCard {
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

    // Entorno: tema + sesión + identidad del usuario (paridad con spawn_command).
    let mut envp: Vec<(String, String)> = crate::utilidades::THEME_ENV
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    envp.extend(session_env.iter().cloned());
    if let Some(u) = user {
        envp.push(("HOME".into(), u.home.display().to_string()));
        envp.push(("USER".into(), u.name.clone()));
        envp.push(("LOGNAME".into(), u.name.clone()));
        envp.push(("SHELL".into(), u.shell.display().to_string()));
        // Drop de privilegios al usuario: el Ente corre como él, no como root.
        card.soma.run_as = Some(RunAs {
            uid: u.uid,
            gid: u.gid,
            groups: user_groups(u),
        });
    }

    card.payload = Payload::Native {
        exec: "/bin/sh".into(),
        argv: vec!["-c".into(), cmd.into()],
        envp,
    };
    card
}

/// Intenta lanzar `cmd` como Ente de sesión vía `RunCard` (best-effort,
/// síncrono — el bucle es calloop). Devuelve `true` si arje lo aceptó (el caller
/// se saltea el spawn crudo); `false` para caer al camino normal.
pub(crate) fn try_spawn_as_ente(
    label: &str,
    cmd: &str,
    user: Option<&UserInfo>,
    session_env: &[(String, String)],
) -> bool {
    let card = session_app_card(label, cmd, user, session_env);
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

    fn user(uid: u32) -> UserInfo {
        UserInfo {
            name: "sergio".into(),
            uid,
            gid: uid,
            home: "/home/sergio".into(),
            shell: "/bin/sh".into(),
        }
    }

    #[test]
    fn la_card_depende_del_piso_y_se_reinicia() {
        let env = vec![("WAYLAND_DISPLAY".to_string(), "wayland-1".to_string())];
        let card = session_app_card("foot", "foot --server", None, &env);
        // Depende del piso ⇒ park & re-floor.
        assert!(card.requires.contains(&wayland_floor()));
        // Se reinicia si cae.
        assert!(matches!(card.supervision, Supervision::Restart { .. }));
        // Corre el comando vía sh -c con el entorno de sesión.
        match &card.payload {
            Payload::Native { exec, argv, envp } => {
                assert_eq!(exec, "/bin/sh");
                assert_eq!(argv, &vec!["-c".to_string(), "foot --server".to_string()]);
                assert!(envp.iter().any(|(k, v)| k == "WAYLAND_DISPLAY" && v == "wayland-1"));
            }
            other => panic!("payload no es Native: {other:?}"),
        }
        // Sin user ⇒ corre con la identidad del Init (sin run_as).
        assert!(card.soma.run_as.is_none());
        card.validate().unwrap();
        let _ = Capability::Spawn; // (sanity: el tipo está en scope)
    }

    #[test]
    fn con_usuario_baja_privilegios() {
        let card = session_app_card("foot", "foot", Some(&user(1000)), &[]);
        let ra = card.soma.run_as.expect("debe bajar privilegios al usuario");
        assert_eq!(ra.uid, 1000);
        assert_eq!(ra.gid, 1000);
        // Inyecta la identidad del usuario en el entorno.
        if let Payload::Native { envp, .. } = &card.payload {
            assert!(envp.iter().any(|(k, v)| k == "HOME" && v == "/home/sergio"));
            assert!(envp.iter().any(|(k, v)| k == "USER" && v == "sergio"));
        }
    }

    #[test]
    fn label_namespaced() {
        let card = session_app_card("editor", "nada", None, &[]);
        assert_eq!(card.label, "session.editor");
    }
}
