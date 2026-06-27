//! ente-machined-compat: shim de `org.freedesktop.machine1`.
//!
//! Cada Ente del fractal con PID se expone como una "machine" — la analogía
//! con systemd-machined no es perfecta (allá una machine es una nspawn
//! container o VM, acá es cualquier Ente nativo) pero es la más honesta
//! sin un modelo de containerización propio. ListMachines / GetMachine /
//! GetMachineByPid consultan ListEntes vía bus interno; TerminateMachine
//! y KillMachine forwardean a KillEnte como systemd1.stop_unit. La
//! mutación que sí queda como NotSupported es RegisterMachine /
//! CreateMachine — un "registrar machine externa" no tiene contraparte
//! en un fractal donde toda Card pasa por la Semilla.

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::Capability;
use std::collections::HashMap;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use zbus::{fdo, interface, zvariant::{ObjectPath, OwnedValue}};

const BUS_NAME: &str = "org.freedesktop.machine1";
const OBJ_PATH: &str = "/org/freedesktop/machine1";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    bitacora::abrir("arje");
    init_tracing();
    info!("ente-machined-compat: arrancando");
    announce_to_fractal().await;

    let manager = MachineManager;
    let conn_result = zbus::connection::Builder::system()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJ_PATH, manager));
    match conn_result {
        Ok(builder) => match builder.build().await {
            Ok(_conn) => {
                info!(name = BUS_NAME, "name acquired, sirviendo");
                wait_for_term().await
            }
            Err(e) => { warn!(?e, "build conn falló — modo idle"); wait_for_term().await }
        },
        Err(e) => { warn!(?e, "builder D-Bus falló — modo idle"); wait_for_term().await }
    }
}

struct MachineManager;

/// Tipo del wire format de ListMachines: `(s, s, s, u, ay, ay, t, ay)` —
/// name, class, service, leader_pid, root_directory_path, id_unix, time_obtained,
/// machine_id_bytes. systemd usa este struct simplificado.
type Machine = (String, String, String, u32, String);

#[interface(name = "org.freedesktop.machine1.Manager")]
impl MachineManager {
    async fn list_machines(&self) -> fdo::Result<Vec<Machine>> {
        let entes = query_list_entes().await.unwrap_or_default();
        let out: Vec<Machine> = entes
            .into_iter()
            .filter_map(|e| {
                e.pid.map(|p| {
                    (
                        e.label.clone(),
                        "container".to_string(), // ver header — analogía pragmática
                        "arje".to_string(),       // service que registra: nosotros
                        p as u32,
                        String::new(),            // root_directory desconocido sin SomaSpec
                    )
                })
            })
            .collect();
        info!(count = out.len(), "ListMachines");
        Ok(out)
    }

    async fn get_machine(&self, name: String) -> fdo::Result<zbus::zvariant::OwnedObjectPath> {
        let entes = query_list_entes().await.unwrap_or_default();
        if entes.iter().any(|e| e.label == name && e.pid.is_some()) {
            let path = format!("/org/freedesktop/machine1/machine/{}", escape_unit_name(&name));
            return ObjectPath::try_from(path)
                .map(zbus::zvariant::OwnedObjectPath::from)
                .map_err(|e| fdo::Error::Failed(format!("path: {e}")));
        }
        Err(fdo::Error::Failed(format!("machine '{name}' no encontrada")))
    }

    async fn get_machine_by_pid(&self, pid: u32) -> fdo::Result<zbus::zvariant::OwnedObjectPath> {
        let entes = query_list_entes().await.unwrap_or_default();
        if let Some(e) = entes.iter().find(|e| e.pid == Some(pid as i32)) {
            let path = format!("/org/freedesktop/machine1/machine/{}", escape_unit_name(&e.label));
            return ObjectPath::try_from(path)
                .map(zbus::zvariant::OwnedObjectPath::from)
                .map_err(|err| fdo::Error::Failed(format!("path: {err}")));
        }
        Err(fdo::Error::Failed(format!("PID {pid} no asociado a ninguna machine")))
    }

    async fn register_machine(
        &self,
        name: String,
        _id: Vec<u8>,
        _service: String,
        class: String,
        _leader_pid: u32,
        _root_directory: String,
    ) -> fdo::Result<zbus::zvariant::OwnedObjectPath> {
        info!(%name, %class, "RegisterMachine (no-op)");
        Err(fdo::Error::NotSupported(
            "RegisterMachine no implementado — usar Cards del fractal".into()
        ))
    }

    async fn register_machine_with_network(
        &self,
        name: String,
        id: Vec<u8>,
        service: String,
        class: String,
        leader_pid: u32,
        root_directory: String,
        _network_interfaces: Vec<i32>,
    ) -> fdo::Result<zbus::zvariant::OwnedObjectPath> {
        self.register_machine(name, id, service, class, leader_pid, root_directory).await
    }

    async fn create_machine(
        &self,
        name: String,
        _id: Vec<u8>,
        _service: String,
        class: String,
        _leader_pid: u32,
        _root_directory: String,
        _scope_properties: Vec<(String, OwnedValue)>,
    ) -> fdo::Result<zbus::zvariant::OwnedObjectPath> {
        info!(%name, %class, "CreateMachine (no-op)");
        Err(fdo::Error::NotSupported(
            "CreateMachine no implementado".into()
        ))
    }

    async fn terminate_machine(&self, name: String) -> fdo::Result<()> {
        // Terminate = SIGTERM. La Supervision::Restart del Ente puede traerlo
        // de vuelta — comportamiento documentado, igual que systemd1.stop_unit.
        kill_machine_via_bus(&name, libc::SIGTERM).await
    }

    async fn kill_machine(&self, name: String, _who: String, signal: i32) -> fdo::Result<()> {
        kill_machine_via_bus(&name, signal).await
    }

    async fn get_machine_address(&self, name: String) -> fdo::Result<Vec<(i32, Vec<u8>)>> {
        warn!(%name, "GetMachineAddress (sin tracking, devuelvo vacío)");
        Ok(vec![])
    }

    async fn get_machine_osrelease(&self, name: String) -> fdo::Result<HashMap<String, String>> {
        warn!(%name, "GetMachineOSRelease (sin tracking)");
        Ok(HashMap::new())
    }

    /// Operaciones sobre la "host machine" (PID 1 namespace) — siempre
    /// disponibles. Usamos el path canónico `/org/freedesktop/machine1/machine/_host`.
    async fn open_machine_login(&self, _name: String) -> fdo::Result<(zbus::zvariant::OwnedObjectPath, zbus::zvariant::OwnedFd)> {
        Err(fdo::Error::NotSupported(
            "OpenMachineLogin no implementado".into()
        ))
    }

    async fn open_machine_shell(
        &self,
        _name: String,
        _user: String,
        _path: String,
        _args: Vec<String>,
        _environment: Vec<String>,
    ) -> fdo::Result<(zbus::zvariant::OwnedObjectPath, zbus::zvariant::OwnedFd)> {
        Err(fdo::Error::NotSupported("OpenMachineShell no implementado".into()))
    }
}

/// Resuelve `name` (label del Ente) a su Ulid via ListEntes y forwardea
/// KillEnte. Compartido por TerminateMachine y KillMachine — paralelo a
/// `kill_unit_via_bus` del shim systemd1.
async fn kill_machine_via_bus(name: &str, signal: i32) -> fdo::Result<()> {
    let entes = query_list_entes().await
        .ok_or_else(|| fdo::Error::Failed("bus interno no disponible".into()))?;
    let target = entes
        .into_iter()
        .find(|e| e.label == name)
        .ok_or_else(|| fdo::Error::Failed(format!("machine '{name}' no encontrada")))?;
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

/// Escape simple para nombres en object paths (parallel al de systemd1-compat).
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
                interface: arje_card::InterfaceId([0xa5; 16]),
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
        .unwrap_or_else(|_| EnvFilter::new("arje_machined_compat=info"));
    // try_init: bitacora::abrir ya puede haber instalado el subscriber global.
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_target(true).try_init();
}
