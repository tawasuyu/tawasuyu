//! `arje-activate`: disparador de activación perezosa de un shim de compat.
//!
//! El **dbus-daemon del host** activa este binario cuando una app pide un
//! nombre `org.freedesktop.*` cuyo `.service` de activación apunta acá
//! (`Exec=/usr/lib/arje/arje-activate <card>`). `arje-activate` **no**
//! reclama el nombre: le pide a `arje-zero` que encarne la Card del shim
//! (`SpawnCardFromDisk { name: <card> }`) y sale. El shim, al vivir,
//! reclama el nombre en el bus del host y el dbus-daemon entrega el mensaje
//! que tenía en espera.
//!
//! Así arje sigue siendo la **única autoridad de spawn y supervisión** —el
//! shim entra al grafo, con su `Restart`/telemetría—; el dbus-daemon queda
//! como mero *sensor de borde* que traduce "alguien pidió el nombre X" en un
//! evento del bus de arje. Es el patrón `SystemdService=` de D-Bus, pero con
//! arje en lugar de systemd. (Ver `03_ukupacha/arje/seeds/fragments/`.)
//!
//! Contrato de deploy: `arje-zero` debe correr con
//! `ENTE_BUS_SOCK=/run/arje/bus.sock` (o el path que sea), y `arje-activate`
//! toma ese mismo path de su env o cae al default `/run/arje/bus.sock` —el
//! entorno de activación del dbus-daemon no hereda el del fractal.
//!
//! **Auth**: `SpawnCardFromDisk` exige identidad autenticada (SO_PEERCRED).
//! `arje-activate` NO es un ente del grafo (lo spawnea el dbus-daemon), así
//! que no puede reclamar identidad. Funciona porque el `.service` de
//! activación lo corre como **root** (`User=root`) y `arje-zero` permite las
//! operaciones de card-store a un peer root sin identidad: sólo puede nombrar
//! cards que root instaló en `/etc/arje/cards.d/`, sin escalada (ver
//! `graph::bus_mediator::is_store_op`).

use std::path::PathBuf;
use std::process::ExitCode;

use arje_bus::{BusClient, BusRequest, BusResponse};

/// Path fijo del socket de arje cuando el entorno de activación no trae
/// `ENTE_BUS_SOCK` (el caso normal: el dbus-daemon del host no hereda el env
/// del fractal). El deploy debe arrancar `arje-zero` con este mismo path.
const DEFAULT_SOCK: &str = "/run/arje/bus.sock";

/// Resuelve el socket: `ENTE_BUS_SOCK` si está y no vacío, si no el default.
fn resolve_socket(env: Option<String>) -> PathBuf {
    match env {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(DEFAULT_SOCK),
    }
}

/// Misma validación que el handler del bus: sin `/`, `..`, ni vacío.
fn valid_card_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains("..")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    bitacora::abrir("arje");
    let name = match std::env::args().nth(1) {
        Some(n) if valid_card_name(&n) => n,
        Some(n) => {
            eprintln!("arje-activate: nombre de card inválido: {n:?}");
            return ExitCode::from(2);
        }
        None => {
            eprintln!("uso: arje-activate <card>");
            return ExitCode::from(2);
        }
    };
    let sock = resolve_socket(std::env::var(arje_bus::ENV_BUS_SOCK).ok());
    let mut client = match BusClient::connect(&sock).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("arje-activate: no conecté al bus en {}: {e}", sock.display());
            return ExitCode::from(1);
        }
    };
    match client.call(BusRequest::SpawnCardFromDisk { name: name.clone() }).await {
        Ok(BusResponse::Ok) => {
            eprintln!("arje-activate: {name} activado");
            ExitCode::SUCCESS
        }
        Ok(other) => {
            eprintln!("arje-activate: el bus rechazó {name}: {other:?}");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("arje-activate: la llamada al bus falló: {e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_default_y_override() {
        assert_eq!(resolve_socket(None), PathBuf::from(DEFAULT_SOCK));
        assert_eq!(resolve_socket(Some(String::new())), PathBuf::from(DEFAULT_SOCK));
        assert_eq!(
            resolve_socket(Some("/run/arje/bus.sock".into())),
            PathBuf::from("/run/arje/bus.sock")
        );
        assert_eq!(resolve_socket(Some("/x/y.sock".into())), PathBuf::from("/x/y.sock"));
    }

    #[test]
    fn nombres_validos() {
        assert!(valid_card_name("compat-logind"));
        assert!(!valid_card_name(""));
        assert!(!valid_card_name("../etc/passwd"));
        assert!(!valid_card_name("a/b"));
    }
}
