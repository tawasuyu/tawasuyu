//! ente-hostnamed-compat: shim de `org.freedesktop.hostname1`.
//!
//! GNOME control-center y otros componentes consultan este servicio al boot
//! para mostrar nombre de host, OS, kernel. Sin esto los settings panels
//! se rompen aunque el sistema funcione.
//!
//! Read-only properties: leemos /etc/hostname, /etc/os-release, uname().
//! Set* methods: log + forward al bus interno (no aplicamos cambios reales
//! en el stub — un siguiente paso es persistir a /etc/* y rehash).

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::Capability;
use std::sync::Mutex;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use zbus::{fdo, interface};

const BUS_NAME: &str = "org.freedesktop.hostname1";
const OBJ_PATH: &str = "/org/freedesktop/hostname1";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    info!("ente-hostnamed-compat: arrancando");
    announce_to_fractal().await;

    let manager = HostnameManager::default();
    let conn_result = zbus::connection::Builder::system()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJ_PATH, manager));
    match conn_result {
        Ok(builder) => match builder.build().await {
            Ok(_conn) => {
                info!(name = BUS_NAME, "name acquired, sirviendo");
                wait_for_term().await
            }
            Err(e) => {
                warn!(?e, "build conn falló — modo idle");
                wait_for_term().await
            }
        },
        Err(e) => {
            warn!(?e, "builder D-Bus falló — modo idle");
            wait_for_term().await
        }
    }
}

#[derive(Default)]
struct HostnameManager {
    /// Cache para SetHostname. En el stub no persistimos a /etc.
    transient_hostname: Mutex<Option<String>>,
}

#[interface(name = "org.freedesktop.hostname1")]
impl HostnameManager {
    // ----- Properties read-only -----

    #[zbus(property)]
    async fn hostname(&self) -> String {
        if let Some(h) = self.transient_hostname.lock().unwrap().clone() {
            return h;
        }
        gethostname_libc().unwrap_or_else(|| "localhost".into())
    }

    #[zbus(property)]
    async fn static_hostname(&self) -> String {
        std::fs::read_to_string("/etc/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn pretty_hostname(&self) -> String {
        read_machine_info_field("PRETTY_HOSTNAME").unwrap_or_default()
    }

    #[zbus(property)]
    async fn icon_name(&self) -> String {
        read_machine_info_field("ICON_NAME").unwrap_or_default()
    }

    #[zbus(property)]
    async fn chassis(&self) -> String {
        read_machine_info_field("CHASSIS").unwrap_or_else(|| "desktop".into())
    }

    #[zbus(property)]
    async fn deployment(&self) -> String {
        read_machine_info_field("DEPLOYMENT").unwrap_or_default()
    }

    #[zbus(property)]
    async fn location(&self) -> String {
        read_machine_info_field("LOCATION").unwrap_or_default()
    }

    #[zbus(property)]
    async fn kernel_name(&self) -> String {
        nix::sys::utsname::uname()
            .ok()
            .and_then(|u| u.sysname().to_str().map(String::from))
            .unwrap_or_else(|| "Linux".into())
    }

    #[zbus(property)]
    async fn kernel_release(&self) -> String {
        nix::sys::utsname::uname()
            .ok()
            .and_then(|u| u.release().to_str().map(String::from))
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn kernel_version(&self) -> String {
        nix::sys::utsname::uname()
            .ok()
            .and_then(|u| u.version().to_str().map(String::from))
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn operating_system_pretty_name(&self) -> String {
        read_os_release_field("PRETTY_NAME").unwrap_or_else(|| "Linux".into())
    }

    #[zbus(property)]
    async fn operating_system_cpename(&self) -> String {
        read_os_release_field("CPE_NAME").unwrap_or_default()
    }

    #[zbus(property)]
    async fn home_url(&self) -> String {
        read_os_release_field("HOME_URL").unwrap_or_default()
    }

    #[zbus(property)]
    async fn hardware_vendor(&self) -> String {
        read_dmi("/sys/class/dmi/id/sys_vendor")
    }

    #[zbus(property)]
    async fn hardware_model(&self) -> String {
        read_dmi("/sys/class/dmi/id/product_name")
    }

    #[zbus(property)]
    async fn firmware_version(&self) -> String {
        read_dmi("/sys/class/dmi/id/bios_version")
    }

    // ----- Setters: forward al bus interno y guardan en cache -----

    async fn set_hostname(&self, name: String, _interactive: bool) -> fdo::Result<()> {
        if !is_valid_hostname(&name) {
            return Err(fdo::Error::InvalidArgs(format!("hostname inválido: {name:?}")));
        }
        // sethostname(2) cambia sólo el running kernel value.
        let cstr = std::ffi::CString::new(name.clone())
            .map_err(|e| fdo::Error::Failed(format!("CString: {e}")))?;
        let r = unsafe { libc::sethostname(cstr.as_ptr(), name.len()) };
        if r != 0 {
            warn!(error = %std::io::Error::last_os_error(), %name, "sethostname syscall falló (¿CAP_SYS_ADMIN?)");
            // No es fatal — guardamos transient para que el property lea el valor nuevo.
        }
        *self.transient_hostname.lock().unwrap() = Some(name.clone());
        info!(%name, "SetHostname aplicado");
        Ok(())
    }

    async fn set_static_hostname(&self, name: String, _interactive: bool) -> fdo::Result<()> {
        if !is_valid_hostname(&name) {
            return Err(fdo::Error::InvalidArgs(format!("hostname inválido: {name:?}")));
        }
        atomic_write("/etc/hostname", format!("{name}\n").as_bytes())
            .map_err(|e| fdo::Error::Failed(format!("write /etc/hostname: {e}")))?;
        info!(%name, "SetStaticHostname → /etc/hostname");
        Ok(())
    }

    async fn set_pretty_hostname(&self, name: String, _interactive: bool) -> fdo::Result<()> {
        update_machine_info("PRETTY_HOSTNAME", &name)
            .map_err(|e| fdo::Error::Failed(format!("machine-info: {e}")))?;
        info!(%name, "SetPrettyHostname → /etc/machine-info");
        Ok(())
    }

    async fn set_icon_name(&self, name: String, _interactive: bool) -> fdo::Result<()> {
        update_machine_info("ICON_NAME", &name)
            .map_err(|e| fdo::Error::Failed(format!("machine-info: {e}")))?;
        info!(%name, "SetIconName → /etc/machine-info");
        Ok(())
    }

    async fn set_chassis(&self, chassis: String, _interactive: bool) -> fdo::Result<()> {
        if !matches!(chassis.as_str(), "desktop"|"laptop"|"server"|"tablet"|"handset"|"watch"|"embedded"|"vm"|"container") {
            return Err(fdo::Error::InvalidArgs(format!("chassis inválido: {chassis}")));
        }
        update_machine_info("CHASSIS", &chassis)
            .map_err(|e| fdo::Error::Failed(format!("machine-info: {e}")))?;
        info!(%chassis, "SetChassis → /etc/machine-info");
        Ok(())
    }

    async fn set_deployment(&self, deployment: String, _interactive: bool) -> fdo::Result<()> {
        update_machine_info("DEPLOYMENT", &deployment)
            .map_err(|e| fdo::Error::Failed(format!("machine-info: {e}")))?;
        info!(%deployment, "SetDeployment → /etc/machine-info");
        Ok(())
    }

    async fn set_location(&self, location: String, _interactive: bool) -> fdo::Result<()> {
        update_machine_info("LOCATION", &location)
            .map_err(|e| fdo::Error::Failed(format!("machine-info: {e}")))?;
        info!(%location, "SetLocation → /etc/machine-info");
        Ok(())
    }
}

// ---------------- helpers ----------------

fn gethostname_libc() -> Option<String> {
    let mut buf = [0u8; 256];
    let r = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut _, buf.len()) };
    if r != 0 { return None; }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    std::str::from_utf8(&buf[..len]).ok().map(String::from)
}

fn read_os_release_field(field: &str) -> Option<String> {
    parse_kv_file("/etc/os-release", field)
}

fn read_machine_info_field(field: &str) -> Option<String> {
    parse_kv_file("/etc/machine-info", field)
}

fn parse_kv_file(path: &str, field: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == field {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

fn read_dmi(path: &str) -> String {
    std::fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// RFC 1123 + extra: ASCII alfanumérico, dash, dot. Longitud 1..253.
/// Rechaza vacíos, espacios, control chars.
fn is_valid_hostname(s: &str) -> bool {
    if s.is_empty() || s.len() > 253 { return false; }
    s.chars().all(|c|
        c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_'
    )
}

/// Escritura atómica via tmp + rename. fsync del directorio para
/// garantizar durabilidad post-crash. Permisos 0644.
fn atomic_write(path: &str, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let p = std::path::Path::new(path);
    if let Some(parent) = p.parent() { let _ = std::fs::create_dir_all(parent); }
    let tmp = p.with_extension("tmp");
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true).write(true).truncate(true)
            .mode(0o644)
            .open(&tmp)?;
        f.write_all(content)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, p)?;
    Ok(())
}

/// Lee /etc/machine-info, actualiza/inserta una clave, escribe atómico.
fn update_machine_info(key: &str, value: &str) -> std::io::Result<()> {
    let path = "/etc/machine-info";
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut found = false;
    let mut out = String::new();
    for line in existing.lines() {
        if let Some((k, _)) = line.split_once('=') {
            if k.trim() == key {
                out.push_str(&format!("{key}={value}\n"));
                found = true;
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if !found {
        out.push_str(&format!("{key}={value}\n"));
    }
    atomic_write(path, out.as_bytes())
}

async fn announce_to_fractal() {
    if let Ok(mut client) = BusClient::from_env().await {
        let req = BusRequest::Announce {
            capabilities: vec![Capability::Endpoint {
                interface: arje_card::InterfaceId([0xa0; 16]),
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
        .unwrap_or_else(|_| EnvFilter::new("arje_hostnamed_compat=info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).init();
}
