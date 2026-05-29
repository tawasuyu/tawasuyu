//! ente-systemd1-compat: shim de `org.freedesktop.systemd1.Manager`.
//!
//! Centro de control que `systemctl` consulta. Sin esto, `systemctl list-units`
//! falla con `Failed to connect to bus` aunque el sistema funcione.
//!
//! Mapeo: cada Ente vivo del fractal aparece como una "unit" cuyo nombre es
//! `<label>.service`. Estados:
//!   - `loaded` siempre (porque está en el grafo)
//!   - `active` si tiene PID o es Wasm corriendo, `inactive` si está virtual
//!   - sub_state: `running`/`exited`/`virtual`
//!
//! Métodos cubiertos del subset que `systemctl` típicamente llama al boot:
//!   - ListUnits (basis de `systemctl list-units`)
//!   - GetUnit / GetUnitByPID (object-path lookup; no servimos métodos del unit)
//!   - StartUnit / StopUnit / RestartUnit (forwardea al bus interno)
//!   - Subscribe / Unsubscribe (no-op)
//!   - Reload (no-op — Cards inmutables)
//!   - ListUnitFiles (vacío)
//!   - GetVersion / Environment / Architecture (properties)

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::Capability;
use std::collections::HashMap;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use zbus::{fdo, interface, zvariant::{ObjectPath, OwnedObjectPath, OwnedValue}};

const BUS_NAME: &str = "org.freedesktop.systemd1";
const OBJ_PATH: &str = "/org/freedesktop/systemd1";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    info!("ente-systemd1-compat: arrancando");
    announce_to_fractal().await;

    let manager = SystemdManager;
    let conn_result = zbus::connection::Builder::system()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJ_PATH, manager));
    match conn_result {
        Ok(builder) => match builder.build().await {
            Ok(_conn) => {
                info!(name = BUS_NAME, "name acquired, sirviendo systemctl");
                wait_for_term().await
            }
            Err(e) => { warn!(?e, "build conn falló — modo idle"); wait_for_term().await }
        },
        Err(e) => { warn!(?e, "builder D-Bus falló — modo idle"); wait_for_term().await }
    }
}

struct SystemdManager;

/// Wire format de un unit en `ListUnits`:
/// (name, description, load_state, active_state, sub_state, followed,
///  unit_path, job_id, job_type, job_path)
type UnitInfo = (
    String, String, String, String, String, String,
    OwnedObjectPath, u32, String, OwnedObjectPath,
);

#[interface(name = "org.freedesktop.systemd1.Manager")]
impl SystemdManager {
    async fn list_units(&self) -> fdo::Result<Vec<UnitInfo>> {
        let entes = match query_list_entes().await {
            Some(es) => es,
            None => return Ok(vec![]),
        };
        let unit_path = ObjectPath::try_from("/org/freedesktop/systemd1/unit/_invalid")
            .map_err(|e| fdo::Error::Failed(format!("path: {e}")))?;
        let job_path = ObjectPath::try_from("/")
            .map_err(|e| fdo::Error::Failed(format!("path: {e}")))?;

        let mut out = Vec::with_capacity(entes.len());
        for e in entes {
            let name = format!("{}.service", e.label);
            let description = format!("Ente: {} ({})", e.label, e.id);
            let active_state = if e.pid.is_some() { "active" } else { "active" };
            let sub_state = match e.pid {
                Some(_) => "running",
                None => "virtual",
            };
            out.push((
                name,
                description,
                "loaded".to_string(),
                active_state.to_string(),
                sub_state.to_string(),
                String::new(),  // followed_unit
                unit_path.clone().into(),
                0u32,           // job_id
                String::new(),  // job_type
                job_path.clone().into(),
            ));
        }
        info!(count = out.len(), "ListUnits");
        Ok(out)
    }

    async fn list_units_filtered(&self, _states: Vec<String>) -> fdo::Result<Vec<UnitInfo>> {
        // Subset simple: ignoramos el filtro y devolvemos todas.
        self.list_units().await
    }

    async fn list_units_by_names(&self, names: Vec<String>) -> fdo::Result<Vec<UnitInfo>> {
        let all = self.list_units().await?;
        let want: std::collections::HashSet<&String> = names.iter().collect();
        Ok(all.into_iter().filter(|u| want.contains(&u.0)).collect())
    }

    async fn get_unit(&self, name: String) -> fdo::Result<OwnedObjectPath> {
        if let Some(entes) = query_list_entes().await {
            if entes.iter().any(|e| format!("{}.service", e.label) == name) {
                let path = format!("/org/freedesktop/systemd1/unit/{}", escape_unit_name(&name));
                return ObjectPath::try_from(path)
                    .map(OwnedObjectPath::from)
                    .map_err(|e| fdo::Error::Failed(format!("path: {e}")));
            }
        }
        Err(fdo::Error::Failed(format!("Unit {name} not found")))
    }

    async fn get_unit_by_pid(&self, pid: u32) -> fdo::Result<OwnedObjectPath> {
        if let Some(entes) = query_list_entes().await {
            if let Some(e) = entes.iter().find(|e| e.pid == Some(pid as i32)) {
                let path = format!("/org/freedesktop/systemd1/unit/{}",
                    escape_unit_name(&format!("{}.service", e.label)));
                return ObjectPath::try_from(path)
                    .map(OwnedObjectPath::from)
                    .map_err(|e| fdo::Error::Failed(format!("path: {e}")));
            }
        }
        Err(fdo::Error::Failed(format!("PID {pid} not in any unit")))
    }

    async fn start_unit(&self, name: String, _mode: String) -> fdo::Result<OwnedObjectPath> {
        // Mapeo: `foo.service` → card store `<ARJE_CARDS_DIR>/foo.json`.
        // arje-zero parsea y encarna; idempotente si ya hay un Ente con
        // ese label en cuanto al efecto observable (otro Ente con mismo
        // label se materializa — la Card es la fuente de verdad).
        let stem = name.strip_suffix(".service").unwrap_or(&name).to_string();
        let mut client = BusClient::from_env().await
            .map_err(|e| fdo::Error::Failed(format!("bus connect: {e}")))?;
        match client.call(BusRequest::SpawnCardFromDisk { name: stem.clone() }).await {
            Ok(BusResponse::Ok) => {
                info!(%name, %stem, "StartUnit aplicado");
                no_job_path()
            }
            Ok(BusResponse::Error(e)) => {
                warn!(%name, %e, "StartUnit rechazado por el bus");
                Err(fdo::Error::Failed(e))
            }
            Ok(other) => {
                warn!(%name, ?other, "StartUnit respuesta inesperada");
                Err(fdo::Error::Failed("respuesta inesperada del bus".into()))
            }
            Err(e) => Err(fdo::Error::Failed(format!("bus call: {e}"))),
        }
    }

    async fn stop_unit(&self, name: String, _mode: String) -> fdo::Result<OwnedObjectPath> {
        // Stop = SIGTERM al PID del Ente. La supervisión decide si vuelve:
        //  - Supervision::OneShot|Delegate → muere y queda muerto.
        //  - Supervision::Restart → reaparece tras el backoff.
        // El último caso difiere de systemd (donde Stop es definitivo) pero
        // refleja la realidad del fractal — el supervisor es soberano.
        kill_unit_via_bus(&name, libc::SIGTERM).await?;
        no_job_path()
    }

    async fn restart_unit(&self, name: String, _mode: String) -> fdo::Result<OwnedObjectPath> {
        // Restart = SIGTERM y dejamos que la Supervision::Restart de la Card
        // lo levante. Si la Card no es Restart, el "restart" es efectivamente
        // un stop. Honesto pero asimétrico con systemd — lo documentamos en
        // el log.
        kill_unit_via_bus(&name, libc::SIGTERM).await?;
        info!(%name, "RestartUnit: SIGTERM emitido; el supervisor decide si vuelve");
        no_job_path()
    }

    async fn reload_unit(&self, name: String, _mode: String) -> fdo::Result<OwnedObjectPath> {
        info!(%name, "ReloadUnit (no-op — Cards inmutables)");
        let path = ObjectPath::try_from("/").unwrap();
        Ok(path.into())
    }

    async fn kill_unit(&self, name: String, _who: String, signal: i32) -> fdo::Result<()> {
        kill_unit_via_bus(&name, signal).await
    }

    async fn subscribe(&self) -> fdo::Result<()> { Ok(()) }
    async fn unsubscribe(&self) -> fdo::Result<()> { Ok(()) }

    async fn reload(&self) -> fdo::Result<()> {
        info!("Reload: trigger re-read (no-op — Cards no se recargan tras boot)");
        Ok(())
    }

    async fn list_unit_files(&self) -> fdo::Result<Vec<(String, String)>> {
        // Empty: no usamos unit files. Cards en su lugar.
        Ok(vec![])
    }

    async fn list_jobs(&self) -> fdo::Result<Vec<(u32, String, String, String, OwnedObjectPath, OwnedObjectPath)>> {
        Ok(vec![])
    }

    async fn get_default_target(&self) -> fdo::Result<String> {
        Ok("multi-user.target".into())
    }

    async fn set_default_target(&self, _name: String, _force: bool) -> fdo::Result<(Vec<String>, Vec<String>, Vec<String>)> {
        Err(fdo::Error::NotSupported("default target gestionado por Card de Semilla".into()))
    }

    // ----- Properties -----

    #[zbus(property)]
    async fn version(&self) -> String { format!("ente-systemd1-compat {}", env!("CARGO_PKG_VERSION")) }

    #[zbus(property)]
    async fn architecture(&self) -> String { std::env::consts::ARCH.into() }

    #[zbus(property)]
    async fn features(&self) -> String { "+ENTE-FRACTAL".into() }

    #[zbus(property)]
    async fn virtualization(&self) -> String { String::new() }

    #[zbus(property)]
    async fn confined(&self) -> bool { false }

    #[zbus(property)]
    async fn environment(&self) -> Vec<String> {
        std::env::vars().map(|(k, v)| format!("{k}={v}")).collect()
    }

    #[zbus(property)]
    async fn n_names(&self) -> u32 { 0 }

    #[zbus(property)]
    async fn n_jobs(&self) -> u32 { 0 }

    #[zbus(property)]
    async fn progress(&self) -> f64 { 1.0 }
}

/// Resuelve `<name>.service` a su Ulid via ListEntes y forwardea KillEnte
/// al bus interno. Compartido por StopUnit, RestartUnit y KillUnit.
async fn kill_unit_via_bus(name: &str, signal: i32) -> fdo::Result<()> {
    let entes = query_list_entes().await
        .ok_or_else(|| fdo::Error::Failed("bus interno no disponible".into()))?;
    let target = entes
        .into_iter()
        .find(|e| format!("{}.service", e.label) == name)
        .ok_or_else(|| fdo::Error::Failed(format!("Unit {name} no encontrada")))?;
    let mut client = BusClient::from_env().await
        .map_err(|e| fdo::Error::Failed(format!("bus connect: {e}")))?;
    match client.call(BusRequest::KillEnte { target: target.id, signal }).await {
        Ok(BusResponse::Ok) => {
            info!(%name, ulid = %target.id, signal, "KillEnte aplicado");
            Ok(())
        }
        Ok(BusResponse::Error(e)) => {
            warn!(%name, %e, "KillEnte rechazado por el bus");
            Err(fdo::Error::Failed(e))
        }
        Ok(other) => {
            warn!(%name, ?other, "KillEnte respuesta inesperada");
            Err(fdo::Error::Failed("respuesta inesperada del bus".into()))
        }
        Err(e) => Err(fdo::Error::Failed(format!("bus call: {e}"))),
    }
}

/// systemd devuelve un object path de `job` por Start/Stop/Restart. El
/// fractal no tiene jobs (las mutaciones son síncronas desde la vista del
/// caller), así que devolvemos "/" — convención zbus para "no job".
fn no_job_path() -> fdo::Result<OwnedObjectPath> {
    ObjectPath::try_from("/")
        .map(OwnedObjectPath::from)
        .map_err(|e| fdo::Error::Failed(format!("path: {e}")))
}

/// Pregunta al bus interno por la lista de Entes vivos.
async fn query_list_entes() -> Option<Vec<arje_bus::EnteInfo>> {
    let mut client = match BusClient::from_env().await {
        Ok(c) => c,
        Err(e) => { warn!(?e, "no bus client — devuelvo vacío"); return None; }
    };
    match client.call(BusRequest::ListEntes).await {
        Ok(BusResponse::Entes(entes)) => Some(entes),
        Ok(other) => { warn!(?other, "ListEntes respuesta inesperada"); None }
        Err(e) => { warn!(?e, "ListEntes call falló"); None }
    }
}

/// Escape de nombres de units para object paths según convención systemd:
/// `.` → `_2e`, `-` → `_2d`, etc. Para el demo usamos un escape simple.
fn escape_unit_name(name: &str) -> String {
    name.chars().map(|c| match c {
        c if c.is_ascii_alphanumeric() => c.to_string(),
        c => format!("_{:02x}", c as u32),
    }).collect()
}

async fn announce_to_fractal() {
    if let Ok(mut client) = BusClient::from_env().await {
        let req = BusRequest::Announce {
            capabilities: vec![Capability::Endpoint {
                interface: arje_card::InterfaceId([0xa6; 16]),
                version: 1,
            }],
        };
        match client.call(req).await {
            Ok(BusResponse::Ok) => info!("Announce → bus interno OK"),
            Ok(other) => warn!(?other, "Announce respuesta inesperada"),
            Err(e) => warn!(?e, "Announce falló"),
        }
    }
}

async fn wait_for_term() -> anyhow::Result<()> {
    let mut term = signal(SignalKind::terminate())?;
    let mut int_ = signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = term.recv() => info!("SIGTERM"),
        _ = int_.recv() => info!("SIGINT"),
    }
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("arje_systemd1_compat=info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).init();
}

#[allow(dead_code)]
fn _suppress(_: HashMap<String, OwnedValue>) {} // mantener import si se reduce