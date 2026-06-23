//! Handoff sin parpadeo desde el splash de arranque (`arje-splash`, Fase 2).
//!
//! Cuando mirada arranca como greeter sobre DRM, puede que `arje-splash` esté
//! mostrando el splash del arranque y **tenga el DRM master**. Antes de pelear
//! por el device, le avisamos por un socket Unix que estamos por tomar la
//! pantalla (`READY`) y esperamos a que haga su fade-out y suelte el master
//! (`RELEASED`). Recién entonces seguimos con libseat + `DrmDevice::new`.
//!
//! Best-effort y con timeout corto: si no hay splash (socket ausente) o no
//! contesta a tiempo, seguimos solos — degradación elegante, mirada nunca se
//! queda esperando un splash que no está.
//!
//! Contrato espejo de `arje-splash::handoff` (mismo socket, mismos mensajes).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

/// Ruta por defecto del socket (override por `ARJE_SPLASH_SOCK`). Igual que en
/// `arje-splash`.
const DEFAULT_SOCK: &str = "/run/arje-splash.sock";
const MSG_READY: &str = "READY";
const MSG_RELEASED: &str = "RELEASED";
/// Cuánto esperamos el `RELEASED` antes de seguir solos.
const WAIT: Duration = Duration::from_secs(3);

fn sock_path() -> PathBuf {
    std::env::var_os("ARJE_SPLASH_SOCK")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCK))
}

/// Coordina el traspaso de la pantalla con `arje-splash` si está presente.
/// Bloquea hasta recibir `RELEASED` o agotar el timeout. No falla nunca: ante
/// cualquier problema, vuelve y mirada sigue su arranque normal.
pub fn esperar_release_del_splash() {
    let path = sock_path();
    let Ok(mut stream) = UnixStream::connect(&path) else {
        // Sin splash escuchando — caso normal si arrancás mirada a mano.
        return;
    };
    println!("[handoff] arje-splash presente — pido la pantalla y espero RELEASED");
    if stream.write_all(format!("{MSG_READY}\n").as_bytes()).is_err() {
        return;
    }
    let _ = stream.flush();
    let _ = stream.set_read_timeout(Some(WAIT));
    let mut tmp = [0u8; 64];
    match stream.read(&mut tmp) {
        Ok(n) if n > 0 && String::from_utf8_lossy(&tmp[..n]).contains(MSG_RELEASED) => {
            println!("[handoff] RELEASED — el splash soltó el DRM, tomo la pantalla");
        }
        _ => {
            println!("[handoff] sin RELEASED a tiempo — sigo igual (degradación elegante)");
        }
    }
}
