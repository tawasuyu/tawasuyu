//! `ente-soma` — wrapper histórico sobre [`arje_incarnate`].
//!
//! La rutina de namespacing fue extraída a `ente-incarnate` para que
//! shuma, exploradores y cualquier supervisor no-PID-1 puedan reusarla.
//! Este crate sobrevive como compat para `ente-zero` y otros que importan
//! `arje_soma::{set_bus_sock, incarnate}`.
//!
//! Semántica preservada:
//! - `BUS_SOCK_PATH` global vía `OnceLock` (init lo setea una vez).
//! - `NOTIFY_SOCKET=/run/systemd/notify` se inyecta automáticamente.
//! - `strict_caps = false` (errores no-fatales se loguean, encarnación sigue).

use arje_card::EntityCard;
use arje_incarnate::{Incarnator, IncarnatorConfig};
use nix::unistd::Pid;
use std::path::PathBuf;
use std::sync::OnceLock;
use tracing::warn;

static INCARNATOR: OnceLock<Incarnator> = OnceLock::new();

/// Establece el path del socket del bus interno. Se llama una sola vez al
/// arrancar PID 1 (después de que el listener bind exitoso). Cada hijo
/// encarnado recibirá este path en `ENTE_BUS_SOCK`.
pub fn set_bus_sock(path: String) {
    let cfg = IncarnatorConfig {
        bus_sock: Some(PathBuf::from(path)),
        notify_socket: Some(PathBuf::from("/run/systemd/notify")),
        extra_env: Vec::new(),
        strict_caps: false,
    };
    let _ = INCARNATOR.set(Incarnator::new(cfg));
}

/// Encarna un EntityCard. Si `set_bus_sock` no fue invocado todavía,
/// usa un Incarnator default (sin bus, sin notify).
///
/// Telemetría: el stdout/stderr de cada Ente se captura a
/// `/var/log/arje/ente-<label>.log` (el «por qué» de una caída — panic/error
/// del binario — que de otro modo se pierde con la consola al reiniciar).
/// Excepción: los Entes interactivos (getty/shell) conservan su TTY heredado,
/// porque redirigir su stdio los rompería.
pub fn incarnate(card: &EntityCard) -> anyhow::Result<Pid> {
    let inc = INCARNATOR.get_or_init(|| Incarnator::new(IncarnatorConfig::default()));
    let out = match open_ente_log(card) {
        Some(file) => {
            use std::os::unix::io::AsRawFd;
            let fd = file.as_raw_fd();
            let stdio = arje_incarnate::ChildStdio {
                stdin_fd: None,
                stdout_fd: Some(fd),
                stderr_fd: Some(fd),
            };
            let r = inc.incarnate_with(card, stdio);
            // El hijo ya hizo dup2(fd→1,2); soltamos nuestra copia.
            drop(file);
            r?
        }
        None => inc.incarnate(card)?,
    };
    for d in &out.degradations {
        warn!(?d, ?out.pid, "incarnation degradation");
    }
    Ok(out.pid)
}

/// Abre el log por-Ente para capturar su stdout/stderr. Devuelve `None` para
/// Entes interactivos (necesitan su TTY) — esos heredan la consola como antes.
fn open_ente_log(card: &EntityCard) -> Option<std::fs::File> {
    use arje_card::Payload;
    let interactive = matches!(
        &card.payload,
        Payload::Native { exec, .. } if matches!(
            exec.rsplit('/').next().unwrap_or(exec.as_str()),
            "agetty" | "getty" | "mingetty" | "sh" | "bash" | "login"
        )
    );
    if interactive {
        return None;
    }
    let _ = std::fs::create_dir_all("/var/log/arje");
    let safe: String = card
        .label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("/var/log/arje/ente-{safe}.log"))
        .ok()
}
