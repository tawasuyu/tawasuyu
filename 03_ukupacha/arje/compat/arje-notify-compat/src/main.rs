//! ente-notify-compat: NOTIFY_SOCKET listener para apps `Type=notify`.
//!
//! systemd convention: el servicio escribe `KEY=value\n` lines a un socket
//! datagram cuya path está en `$NOTIFY_SOCKET`. Keys típicos:
//!   - READY=1            (servicio listo para recibir requests)
//!   - STATUS=text        (descripción del estado)
//!   - WATCHDOG=1          (heartbeat)
//!   - STOPPING=1          (cierre ordenado)
//!   - MAINPID=<pid>       (cambio de PID principal)
//!
//! Path canonical: /run/systemd/notify. Bindeable sólo con CAP_NET_BIND_SERVICE
//! o si /run es writable.
//!
//! Para que las apps lo usen, ente-soma debe inyectar `NOTIFY_SOCKET=<path>`
//! en el envp de cada Ente encarnado. Eso ya lo hace via build_env() —
//! aquí sólo necesitamos que el path sea coherente.

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::Capability;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
use tokio::io::unix::AsyncFd;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

const NOTIFY_SOCKET_PATH: &str = "/run/systemd/notify";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    info!(path = NOTIFY_SOCKET_PATH, "ente-notify-compat: arrancando");
    announce_to_fractal().await;

    let stream = match bind_dgram(NOTIFY_SOCKET_PATH) {
        Some(s) => s,
        None => {
            warn!("no se pudo bind — modo idle (apps Type=notify caerán a no-op)");
            return wait_for_term().await;
        }
    };
    info!("NOTIFY_SOCKET listening");
    spawn_listener(stream);
    wait_for_term().await
}

fn bind_dgram(path: &str) -> Option<AsyncFd<OwnedFdWrap>> {
    use nix::sys::socket::{bind, socket, AddressFamily, SockFlag, SockType, UnixAddr};
    let _ = std::fs::remove_file(path);
    if let Some(parent) = Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let fd = socket(
        AddressFamily::Unix,
        SockType::Datagram,
        SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
        None,
    ).ok()?;
    let addr = UnixAddr::new(path).ok()?;
    if let Err(e) = bind(fd.as_raw_fd(), &addr) {
        warn!(?e, %path, "bind");
        return None;
    }
    // Permisos abiertos: cualquier proceso debería poder escribir notificaciones.
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666));
    AsyncFd::new(OwnedFdWrap(fd)).ok()
}

struct OwnedFdWrap(OwnedFd);
impl AsRawFd for OwnedFdWrap {
    fn as_raw_fd(&self) -> std::os::fd::RawFd { self.0.as_raw_fd() }
}

fn spawn_listener(async_fd: AsyncFd<OwnedFdWrap>) {
    tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let mut guard = match async_fd.readable().await {
                Ok(g) => g,
                Err(e) => { warn!(?e, "readable"); return; }
            };
            let raw_fd = guard.get_inner().as_raw_fd();
            loop {
                let n = unsafe { libc::recv(raw_fd, buf.as_mut_ptr() as *mut _, buf.len(), 0) };
                if n <= 0 { break; }
                handle_notification(&buf[..n as usize]);
            }
            guard.clear_ready();
        }
    });
}

fn handle_notification(buf: &[u8]) {
    let s = match std::str::from_utf8(buf) {
        Ok(s) => s,
        Err(_) => { debug!(len = buf.len(), "notify binario, skip"); return; }
    };
    let mut ready = false;
    let mut status = None;
    let mut mainpid = None;
    let mut watchdog = false;
    let mut stopping = false;
    let mut other_keys = Vec::new();
    for line in s.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k {
                "READY" if v == "1" => ready = true,
                "STATUS" => status = Some(v.to_string()),
                "MAINPID" => mainpid = v.parse::<u32>().ok(),
                "WATCHDOG" if v == "1" => watchdog = true,
                "STOPPING" if v == "1" => stopping = true,
                _ => other_keys.push(format!("{k}={v}")),
            }
        }
    }
    if ready {
        info!(?status, ?mainpid, "sd_notify READY");
    } else if stopping {
        info!(?status, "sd_notify STOPPING");
    } else if watchdog {
        debug!("sd_notify WATCHDOG");
    } else if let Some(s) = status {
        info!(%s, "sd_notify STATUS");
    } else if !other_keys.is_empty() {
        debug!(keys = ?other_keys, "sd_notify (other)");
    }
}

async fn announce_to_fractal() {
    if let Ok(mut client) = BusClient::from_env().await {
        let req = BusRequest::Announce {
            capabilities: vec![Capability::Endpoint {
                interface: arje_card::InterfaceId([0xa7; 16]),
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
        .unwrap_or_else(|_| EnvFilter::new("arje_notify_compat=info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).init();
}
