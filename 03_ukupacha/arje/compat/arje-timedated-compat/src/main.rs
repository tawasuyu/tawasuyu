//! ente-timedated-compat: shim de `org.freedesktop.timedate1`.
//!
//! GNOME settings panel "Date & Time" llama aquí. Properties read-only se
//! mapean a syscalls/lecturas del sistema; setters log + forward.

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::Capability;
use arje_compat_common::atomic_write;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use zbus::{fdo, interface};

const BUS_NAME: &str = "org.freedesktop.timedate1";
const OBJ_PATH: &str = "/org/freedesktop/timedate1";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    info!("ente-timedated-compat: arrancando");
    announce_to_fractal().await;

    let manager = TimedateManager::default();
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
struct TimedateManager;

#[interface(name = "org.freedesktop.timedate1")]
impl TimedateManager {
    // ----- Properties -----

    /// Timezone configurada. Por defecto leemos el target de /etc/localtime
    /// (un symlink a /usr/share/zoneinfo/<TZ>).
    #[zbus(property)]
    async fn timezone(&self) -> String {
        std::fs::read_link("/etc/localtime")
            .ok()
            .and_then(|p| {
                let s = p.to_string_lossy().into_owned();
                s.strip_prefix("/usr/share/zoneinfo/").map(String::from)
                    .or_else(|| s.split("/zoneinfo/").nth(1).map(String::from))
            })
            .unwrap_or_else(|| "UTC".into())
    }

    /// True si el RTC del hardware está en local time. Convención moderna
    /// es UTC (false). Reportamos false como default.
    #[zbus(property)]
    async fn local_rtc(&self) -> bool { false }

    /// Si el sistema puede gestionar NTP. `false`: arje no administra un
    /// daemon de sincronización propio, así que GNOME deja el toggle
    /// «hora automática» deshabilitado en vez de fingir que funciona.
    #[zbus(property)]
    async fn can_ntp(&self) -> bool { false }

    /// Si NTP está activo. Sin daemon real bajo nuestro control no podemos
    /// consultarlo con precisión — false como default seguro.
    #[zbus(property)]
    async fn ntp(&self) -> bool { false }

    #[zbus(property)]
    async fn ntpsynchronized(&self) -> bool { false }

    /// Timestamp actual en microsegundos desde epoch.
    #[zbus(property)]
    async fn time_usec(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0)
    }

    #[zbus(property)]
    async fn rtctime_usec(&self) -> u64 {
        // El RTC real requiere ioctl a /dev/rtc — usamos system clock como aprox.
        SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0)
    }

    // ----- Setters -----

    async fn set_time(&self, usec_utc: i64, relative: bool, _interactive: bool) -> fdo::Result<()> {
        // `relative`: el valor es un delta sobre el reloj actual.
        let target = if relative {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_micros() as i64)
                .unwrap_or(0);
            now + usec_utc
        } else {
            usec_utc
        };
        if target < 0 {
            return Err(fdo::Error::InvalidArgs("el tiempo resultante es negativo".into()));
        }
        let ts = libc::timespec {
            tv_sec: (target / 1_000_000) as libc::time_t,
            tv_nsec: ((target % 1_000_000) * 1_000) as _,
        };
        // SEGURIDAD: `clock_settime` con un timespec válido vivo en la
        // pila durante la llamada.
        let r = unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &ts) };
        if r != 0 {
            let e = std::io::Error::last_os_error();
            warn!(%e, "clock_settime falló (¿falta CAP_SYS_TIME?)");
            return Err(fdo::Error::Failed(format!("clock_settime: {e}")));
        }
        info!(target, "SetTime aplicado al reloj del sistema");
        Ok(())
    }

    async fn set_timezone(&self, timezone: String, _interactive: bool) -> fdo::Result<()> {
        // Validar contra zoneinfo: el archivo destino debe existir.
        let zoneinfo = format!("/usr/share/zoneinfo/{timezone}");
        if !std::path::Path::new(&zoneinfo).exists() {
            return Err(fdo::Error::InvalidArgs(format!("timezone desconocida: {timezone}")));
        }
        // Atomic relink: crear localtime.tmp como symlink, rename.
        let tmp = "/etc/localtime.tmp";
        let _ = std::fs::remove_file(tmp);
        if let Err(e) = std::os::unix::fs::symlink(&zoneinfo, tmp) {
            return Err(fdo::Error::Failed(format!("symlink: {e}")));
        }
        if let Err(e) = std::fs::rename(tmp, "/etc/localtime") {
            return Err(fdo::Error::Failed(format!("rename: {e}")));
        }
        info!(%timezone, "SetTimezone → /etc/localtime");
        Ok(())
    }

    async fn set_local_rtc(&self, local_rtc: bool, _fix_system: bool, _interactive: bool) -> fdo::Result<()> {
        let mode = if local_rtc { "LOCAL" } else { "UTC" };
        // `/etc/adjtime` tiene tres líneas; sólo la tercera (UTC|LOCAL)
        // nos concierne. Se conservan las dos primeras si ya existían.
        let existing = std::fs::read_to_string("/etc/adjtime").unwrap_or_default();
        let mut lines = existing.lines();
        let l0 = lines.next().unwrap_or("0.0 0 0.0");
        let l1 = lines.next().unwrap_or("0");
        atomic_write("/etc/adjtime", format!("{l0}\n{l1}\n{mode}\n").as_bytes())
            .map_err(|e| fdo::Error::Failed(format!("write /etc/adjtime: {e}")))?;
        info!(local_rtc, "SetLocalRTC → /etc/adjtime");
        Ok(())
    }

    async fn set_ntp(&self, ntp: bool, _interactive: bool) -> fdo::Result<()> {
        // arje no administra un daemon NTP propio. En vez de fingir
        // éxito, se rechaza con honestidad — y como `can_ntp` es `false`,
        // GNOME deja el toggle deshabilitado y normalmente no llega acá.
        warn!(ntp, "SetNTP rechazado — arje no gestiona un daemon NTP");
        Err(fdo::Error::NotSupported(
            "arje no gestiona un daemon NTP (timesyncd/chrony)".into(),
        ))
    }

    async fn list_timezones(&self) -> fdo::Result<Vec<String>> {
        // Listar /usr/share/zoneinfo recursivamente. Hacemos un best-effort.
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir("/usr/share/zoneinfo") {
            for entry in rd.flatten() {
                if let Ok(name) = entry.file_name().into_string() {
                    if !name.starts_with(|c: char| c.is_lowercase()) && name != "posix" && name != "right" {
                        out.push(name);
                    }
                }
            }
        }
        Ok(out)
    }
}

async fn announce_to_fractal() {
    if let Ok(mut client) = BusClient::from_env().await {
        let req = BusRequest::Announce {
            capabilities: vec![Capability::Endpoint {
                interface: arje_card::InterfaceId([0xa1; 16]),
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
        .unwrap_or_else(|_| EnvFilter::new("arje_timedated_compat=info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).init();
}
