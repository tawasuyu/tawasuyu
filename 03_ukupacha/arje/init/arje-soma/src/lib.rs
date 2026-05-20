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
pub fn incarnate(card: &EntityCard) -> anyhow::Result<Pid> {
    let inc = INCARNATOR.get_or_init(|| Incarnator::new(IncarnatorConfig::default()));
    let out = inc.incarnate(card)?;
    for d in &out.degradations {
        warn!(?d, ?out.pid, "incarnation degradation");
    }
    Ok(out.pid)
}
