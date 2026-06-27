//! ente-logind-compat: el Ente que se hace pasar por systemd-logind.
//!
//! Vive FUERA de PID 1 — un parser D-Bus en el Init es una bomba con CVEs
//! históricos. Ejecutado como hijo del Ente #0 con `Restart` supervision.
//!
//! Implementa el subset del interface `org.freedesktop.login1.Manager` que
//! GNOME/KDE consultan en arranque. Cada método se traduce internamente en
//! una request al bus interno del fractal — capacidades tipadas, no nombres
//! D-Bus opacos hacia abajo.
//!
//! ## Lo que GNOME/KDE realmente llaman al boot
//!   - ListSessions, ListUsers, GetSession*
//!   - Inhibit (mantiene un fd vivo mientras la app está activa)
//!   - CanPowerOff/CanReboot/CanSuspend
//!   - PowerOff/Reboot/Suspend
//!   - Properties: IdleHint, Docked, etc.
//!
//! El stub responde "no hay sesiones" y "sí puedo apagar" — suficiente para
//! que GNOME complete arranque sin marcar fallo.

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::Capability;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use zbus::{fdo, interface, zvariant::OwnedObjectPath, Connection};

const BUS_NAME: &str = "org.freedesktop.login1";
const MANAGER_PATH: &str = "/org/freedesktop/login1";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    bitacora::abrir("arje");
    init_tracing();
    info!("ente-logind-compat: arrancando");

    let bus_addr = std::env::var("DBUS_SYSTEM_BUS_ADDRESS")
        .unwrap_or_else(|_| "unix:path=/var/run/dbus/system_bus_socket".into());
    let bus_path = bus_addr.strip_prefix("unix:path=").unwrap_or(&bus_addr);
    let bus_present = std::path::Path::new(bus_path).exists();
    info!(bus_addr, bus_present, "configuración D-Bus");

    // Anunciamos nuestra presencia al bus interno del fractal antes de
    // intentar registrar el nombre D-Bus. Esto sirve como handshake "estoy
    // vivo" independiente del estado del system bus.
    announce_to_fractal().await;

    if !bus_present {
        warn!("system bus no disponible — modo idle (esperando SIGTERM)");
        return wait_for_term().await;
    }

    let conn = match build_connection().await {
        Ok(c) => c,
        Err(e) => {
            warn!(?e, "fallo al registrar org.freedesktop.login1 — modo idle");
            // No retornamos error: la supervisión Restart entraría en bucle
            // si systemd-logind real ya posee el nombre. Esperar señal y salir.
            return wait_for_term().await;
        }
    };

    info!("logind compat corriendo — esperando señales");
    let _ = conn; // mantener viva la conexión hasta SIGTERM
    wait_for_term().await
}

async fn build_connection() -> anyhow::Result<Connection> {
    let manager = LogindManager::default();
    let conn = zbus::connection::Builder::system()?
        .name(BUS_NAME)?
        .serve_at(MANAGER_PATH, manager)?
        .build()
        .await?;
    info!(name = BUS_NAME, path = MANAGER_PATH, "name acquired + manager served");
    Ok(conn)
}

async fn announce_to_fractal() {
    match BusClient::from_env().await {
        Ok(mut client) => {
            let req = BusRequest::Announce {
                capabilities: vec![Capability::LegacyLogind],
            };
            match client.call(req).await {
                Ok(BusResponse::Ok) => info!("Announce → bus interno OK"),
                Ok(other) => warn!(?other, "Announce respuesta inesperada"),
                Err(e) => warn!(?e, "Announce falló"),
            }
        }
        Err(e) => warn!(?e, "no se pudo conectar al bus interno"),
    }
}

async fn forward_to_fractal(req: BusRequest) -> fdo::Result<()> {
    let mut client = BusClient::from_env().await
        .map_err(|e| fdo::Error::Failed(format!("bus client: {e}")))?;
    match client.call(req).await {
        Ok(BusResponse::Ok) => Ok(()),
        Ok(BusResponse::Error(s)) => Err(fdo::Error::Failed(s)),
        Ok(other) => Err(fdo::Error::Failed(format!("respuesta inesperada: {other:?}"))),
        Err(e) => Err(fdo::Error::Failed(format!("bus call: {e}"))),
    }
}

async fn wait_for_term() -> anyhow::Result<()> {
    let mut term = signal(SignalKind::terminate())?;
    let mut int_ = signal(SignalKind::interrupt())?;
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.tick().await; // descartar el primer tick inmediato
    loop {
        tokio::select! {
            _ = term.recv() => { info!("SIGTERM — cierre ordenado"); return Ok(()); }
            _ = int_.recv() => { info!("SIGINT — cierre"); return Ok(()); }
            _ = tick.tick() => { info!("heartbeat"); }
        }
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("arje_logind_compat=info"));
    // try_init: bitacora::abrir ya puede haber instalado el subscriber global.
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_target(true).try_init();
}

/// Un inhibidor activo: el cliente sostiene un fd vivo; mientras no lo
/// cierre, esta entrada permanece en la tabla.
struct Inhibitor {
    id: u32,
    what: String,
    who: String,
    why: String,
    mode: String,
    uid: u32,
    pid: u32,
}

/// Une los `what` de los inhibidores de un `mode` dado en una lista de
/// tokens únicos separados por `:` — el format de las propiedades
/// `BlockInhibited` / `DelayInhibited`.
fn inhibited_what(inhibitors: &[Inhibitor], mode: &str) -> String {
    let mut tokens: Vec<&str> = Vec::new();
    for inh in inhibitors.iter().filter(|i| i.mode == mode) {
        for t in inh.what.split(':').filter(|t| !t.is_empty()) {
            if !tokens.contains(&t) {
                tokens.push(t);
            }
        }
    }
    tokens.join(":")
}

#[derive(Default)]
struct LogindManager {
    /// Contador monótono — fuente de ids de inhibidores.
    inhibit_counter: AtomicU32,
    /// Inhibidores activos. La tarea guardiana del fd de cada uno quita
    /// su entrada cuando el cliente cierra el descriptor.
    inhibitors: Arc<Mutex<Vec<Inhibitor>>>,
}

/// Tipos del wire format de `org.freedesktop.login1.Manager`.
type SessionTuple = (String, u32, String, String, OwnedObjectPath);
type UserTuple = (u32, String, OwnedObjectPath);

#[interface(name = "org.freedesktop.login1.Manager")]
impl LogindManager {
    // ---- Listado / lookup ----

    async fn list_sessions(&self) -> fdo::Result<Vec<SessionTuple>> {
        Ok(vec![])
    }

    async fn list_users(&self) -> fdo::Result<Vec<UserTuple>> {
        Ok(vec![])
    }

    async fn get_session(&self, _session_id: String) -> fdo::Result<OwnedObjectPath> {
        Err(fdo::Error::Failed("no sessions in fractal".into()))
    }

    async fn get_session_by_pid(&self, _pid: u32) -> fdo::Result<OwnedObjectPath> {
        Err(fdo::Error::Failed("no sessions in fractal".into()))
    }

    async fn get_user(&self, _uid: u32) -> fdo::Result<OwnedObjectPath> {
        Err(fdo::Error::Failed("no users in fractal".into()))
    }

    async fn get_user_by_pid(&self, _pid: u32) -> fdo::Result<OwnedObjectPath> {
        Err(fdo::Error::Failed("no users in fractal".into()))
    }

    // ---- Inhibit ----
    //
    // Devuelve un fd que el cliente mantiene abierto mientras quiere
    // inhibir. Un pipe: el cliente se queda el extremo de escritura;
    // este shim, el de lectura. Cuando el cliente cierra el suyo —o
    // muere—, nuestra lectura ve EOF y la tarea guardiana retira el
    // inhibidor de la tabla.

    async fn inhibit(
        &self,
        what: String,
        who: String,
        why: String,
        mode: String,
    ) -> fdo::Result<zbus::zvariant::OwnedFd> {
        let (reader, writer) = std::io::pipe()
            .map_err(|e| fdo::Error::Failed(format!("pipe: {e}")))?;
        let id = self.inhibit_counter.fetch_add(1, Ordering::Relaxed);
        // uid/pid del llamante quedan en 0/0: obtenerlos exige consultar
        // las credenciales de la conexión D-Bus, y para el shim no es
        // crítico (ListInhibitors los expone sólo de forma informativa).
        self.inhibitors.lock().unwrap().push(Inhibitor {
            id,
            what: what.clone(),
            who: who.clone(),
            why: why.clone(),
            mode: mode.clone(),
            uid: 0,
            pid: 0,
        });
        info!(id, %what, %who, %why, %mode, "Inhibit registrado");
        let inhibitors = self.inhibitors.clone();
        tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut reader = reader;
            let mut buf = [0u8; 64];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break, // EOF: el cliente soltó el fd
                    Ok(_) => continue,       // datos espurios: seguir
                }
            }
            inhibitors.lock().unwrap().retain(|i| i.id != id);
            info!(id, "inhibidor liberado (el cliente cerró el fd)");
        });
        let raw: std::os::fd::OwnedFd = writer.into();
        Ok(raw.into())
    }

    /// Los inhibidores activos: `(what, who, why, mode, uid, pid)`.
    async fn list_inhibitors(
        &self,
    ) -> fdo::Result<Vec<(String, String, String, String, u32, u32)>> {
        Ok(self
            .inhibitors
            .lock()
            .unwrap()
            .iter()
            .map(|i| {
                (
                    i.what.clone(),
                    i.who.clone(),
                    i.why.clone(),
                    i.mode.clone(),
                    i.uid,
                    i.pid,
                )
            })
            .collect())
    }

    // ---- Power management ----

    async fn power_off(&self, interactive: bool) -> fdo::Result<()> {
        info!(interactive, "PowerOff D-Bus → bus interno");
        forward_to_fractal(BusRequest::PowerOff { interactive }).await
    }

    async fn reboot(&self, interactive: bool) -> fdo::Result<()> {
        info!(interactive, "Reboot D-Bus → bus interno");
        forward_to_fractal(BusRequest::Reboot { interactive }).await
    }

    async fn suspend(&self, interactive: bool) -> fdo::Result<()> {
        info!(interactive, "Suspend D-Bus → bus interno");
        forward_to_fractal(BusRequest::Suspend { interactive }).await
    }

    async fn hibernate(&self, interactive: bool) -> fdo::Result<()> {
        info!(interactive, "Hibernate D-Bus → bus interno");
        forward_to_fractal(BusRequest::Hibernate { interactive }).await
    }

    async fn can_power_off(&self) -> fdo::Result<String> {
        Ok("yes".into())
    }

    async fn can_reboot(&self) -> fdo::Result<String> {
        Ok("yes".into())
    }

    async fn can_suspend(&self) -> fdo::Result<String> {
        // "challenge" = válido, requiere autenticación. GNOME muestra el
        // botón pero pide PIN/contraseña antes de invocar Suspend.
        Ok("challenge".into())
    }

    async fn can_hibernate(&self) -> fdo::Result<String> {
        Ok("challenge".into())
    }

    // ---- Properties mínimas ----

    #[zbus(property)]
    async fn idle_hint(&self) -> bool { false }

    #[zbus(property)]
    async fn idle_since_hint(&self) -> u64 { 0 }

    #[zbus(property)]
    async fn idle_since_hint_monotonic(&self) -> u64 { 0 }

    #[zbus(property)]
    async fn block_inhibited(&self) -> String {
        inhibited_what(&self.inhibitors.lock().unwrap(), "block")
    }

    #[zbus(property)]
    async fn delay_inhibited(&self) -> String {
        inhibited_what(&self.inhibitors.lock().unwrap(), "delay")
    }

    #[zbus(property)]
    async fn docked(&self) -> bool { false }

    #[zbus(property)]
    async fn lid_closed(&self) -> bool { false }

    #[zbus(property)]
    async fn on_external_power(&self) -> bool { true }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inh(what: &str, mode: &str) -> Inhibitor {
        Inhibitor {
            id: 0,
            what: what.into(),
            who: "app".into(),
            why: "test".into(),
            mode: mode.into(),
            uid: 0,
            pid: 0,
        }
    }

    #[test]
    fn inhibited_what_une_tokens_unicos_por_modo() {
        let v = vec![
            inh("sleep:shutdown", "block"),
            inh("idle", "delay"),
            inh("shutdown:handle-lid-switch", "block"),
        ];
        let block = inhibited_what(&v, "block");
        // sleep, shutdown, handle-lid-switch — `shutdown` no se duplica.
        assert_eq!(block.split(':').count(), 3, "block = {block}");
        assert!(block.contains("sleep"));
        assert!(block.contains("handle-lid-switch"));
        assert_eq!(inhibited_what(&v, "delay"), "idle");
        assert_eq!(inhibited_what(&[], "block"), "");
    }
}
