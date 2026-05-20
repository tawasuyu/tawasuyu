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
        warn!(%name, "StartUnit no implementado — Cards no se 'start' tras boot");
        Err(fdo::Error::NotSupported(
            "StartUnit: el fractal usa Cards cargadas al boot, no unit files dinámicos".into()
        ))
    }

    async fn stop_unit(&self, name: String, _mode: String) -> fdo::Result<OwnedObjectPath> {
        warn!(%name, "StopUnit (stub: TODO via bus capability)");
        // TODO: bus → graph → kill PID por label. Por ahora no-op.
        let path = ObjectPath::try_from("/").unwrap();
        Ok(path.into())
    }

    async fn restart_unit(&self, name: String, mode: String) -> fdo::Result<OwnedObjectPath> {
        info!(%name, "RestartUnit (delega a StopUnit)");
        self.stop_unit(name, mode).await
    }

    async fn reload_unit(&self, name: String, _mode: String) -> fdo::Result<OwnedObjectPath> {
        info!(%name, "ReloadUnit (no-op — Cards inmutables)");
        let path = ObjectPath::try_from("/").unwrap();
        Ok(path.into())
    }

    async fn kill_unit(&self, name: String, _who: String, _signal: i32) -> fdo::Result<()> {
        warn!(%name, "KillUnit (stub)");
        Ok(())
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