//! ente-machined-compat: shim de `org.freedesktop.machine1`.
//!
//! systemd-machined trackea VMs y containers (typically managed por systemd-nspawn).
//! En el fractal cada Ente con namespaces es candidato a "machine", pero la
//! correspondencia no es 1:1 — un Ente puede tener menos aislamiento que una
//! container completa.
//!
//! Este shim devuelve listas vacías para no romper clientes (gnome-boxes,
//! virt-manager, etc) que llaman a `ListMachines` durante boot. Métodos de
//! mutación (RegisterMachine, KillMachine) se aceptan como no-op con audit
//! log via tracing.
//!
//! Producción real: integrar con el graph del fractal — ListMachines query
//! BusRequest::ListEntes filtrado por `card.soma.namespaces.pid`.

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::Capability;
use std::collections::HashMap;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use zbus::{fdo, interface, zvariant::OwnedValue};

const BUS_NAME: &str = "org.freedesktop.machine1";
const OBJ_PATH: &str = "/org/freedesktop/machine1";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
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
    /// Lista vacía — no trackeamos containers todavía.
    async fn list_machines(&self) -> fdo::Result<Vec<Machine>> {
        Ok(vec![])
    }

    /// Devuelve siempre NotFound — sin machines registradas.
    async fn get_machine(&self, name: String) -> fdo::Result<zbus::zvariant::OwnedObjectPath> {
        Err(fdo::Error::Failed(format!("machine '{name}' no encontrada")))
    }

    async fn get_machine_by_pid(&self, pid: u32) -> fdo::Result<zbus::zvariant::OwnedObjectPath> {
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
        info!(%name, "TerminateMachine (no-op)");
        Ok(())
    }

    async fn kill_machine(&self, name: String, _who: String, _signal: i32) -> fdo::Result<()> {
        info!(%name, "KillMachine (no-op)");
        Ok(())
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
    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).init();
}
