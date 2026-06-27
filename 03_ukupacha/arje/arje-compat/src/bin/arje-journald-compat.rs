//! ente-journald-compat: stub que absorbe escrituras al journal socket.
//!
//! Listen en `/run/systemd/journal/socket` (datagram) — todo lo que llega
//! se decodifica best-effort y se emite como tracing event.
//!
//! Sin esto, apps que usan `sd_journal_send` o syslog fallan al escribir.
//! Para una implementación real: persistir a CAS por timestamp+sha,
//! exponer query API, indexar por unidad/usuario.

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::Capability;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio::io::unix::AsyncFd;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

const JOURNAL_SOCKET: &str = "/run/systemd/journal/socket";
const DEV_LOG: &str = "/dev/log";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    bitacora::abrir("arje");
    init_tracing();
    info!("ente-journald-compat: arrancando");
    announce_to_fractal().await;

    // Intentamos vincular ambos sockets. Cada uno puede fallar
    // independientemente; si alguno funciona, seguimos.
    let mut bound = 0usize;
    if let Some(stream) = bind_dgram(JOURNAL_SOCKET) {
        bound += 1;
        spawn_listener(stream, "journal");
    } else {
        warn!(path = JOURNAL_SOCKET, "no se pudo bind — necesita CAP_NET_BIND_SERVICE o /run writable");
    }
    if let Some(stream) = bind_dgram(DEV_LOG) {
        bound += 1;
        spawn_listener(stream, "syslog");
    } else {
        warn!(path = DEV_LOG, "no se pudo bind /dev/log");
    }

    if bound == 0 {
        warn!("ningún socket bound — modo idle");
    } else {
        info!(sockets_bound = bound, "journald-compat listening");
    }

    wait_for_term().await
}

fn bind_dgram(path: &str) -> Option<AsyncFd<OwnedFdWrap>> {
    use nix::sys::socket::{bind, socket, AddressFamily, SockFlag, SockType, UnixAddr};
    let _ = std::fs::remove_file(path);
    if let Some(parent) = Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let fd = match socket(
        AddressFamily::Unix,
        SockType::Datagram,
        SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
        None,
    ) {
        Ok(f) => f,
        Err(e) => { warn!(?e, "socket() falló"); return None; }
    };
    let addr = match UnixAddr::new(path) {
        Ok(a) => a,
        Err(e) => { warn!(?e, "UnixAddr falló"); return None; }
    };
    if let Err(e) = bind(fd.as_raw_fd(), &addr) {
        warn!(?e, %path, "bind falló");
        return None;
    }
    AsyncFd::new(OwnedFdWrap(fd)).ok()
}

struct OwnedFdWrap(OwnedFd);
impl AsRawFd for OwnedFdWrap {
    fn as_raw_fd(&self) -> std::os::fd::RawFd { self.0.as_raw_fd() }
}

fn spawn_listener(async_fd: AsyncFd<OwnedFdWrap>, source: &'static str) {
    tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut guard = match async_fd.readable().await {
                Ok(g) => g,
                Err(e) => { warn!(?e, source, "readable failed"); return; }
            };
            let raw_fd = guard.get_inner().as_raw_fd();
            loop {
                let n = unsafe {
                    libc::recv(raw_fd, buf.as_mut_ptr() as *mut _, buf.len(), 0)
                };
                if n <= 0 { break; }
                handle_message(&buf[..n as usize], source);
            }
            guard.clear_ready();
        }
    });
}

/// Mutex sobre el archivo index para escrituras concurrentes desde
/// múltiples listeners (journal + syslog).
static INDEX_FILE: Mutex<()> = Mutex::new(());

/// Path del index file: `$XDG_DATA_HOME/ente/journal/index.log` (default
/// `~/.local/share/ente/journal/index.log`).
fn index_path() -> PathBuf {
    let base = if let Ok(d) = std::env::var("XDG_DATA_HOME") { d }
        else if let Ok(h) = std::env::var("HOME") { format!("{h}/.local/share") }
        else { "/var/lib".into() };
    PathBuf::from(base).join("ente").join("journal").join("index.log")
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Persiste el blob crudo al CAS y appendea una línea al index:
/// `<timestamp_ms>:<source>:<unit>:<sha_hex>`. Errores se logean pero
/// no abortan — perder un mensaje no debe romper journald.
fn persist_to_cas(buf: &[u8], source: &'static str, unit: Option<&str>) {
    let sha = match arje_cas::store(buf) {
        Ok(s) => s,
        Err(e) => { warn!(?e, "CAS store falló"); return; }
    };
    let line = format!(
        "{}:{}:{}:{}\n",
        now_ms(), source, unit.unwrap_or("-"), arje_cas::hex(&sha)
    );
    let path = index_path();
    let _guard = INDEX_FILE.lock().unwrap();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    use std::io::Write;
    let mut f = match std::fs::OpenOptions::new()
        .create(true).append(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) => { warn!(?e, path = %path.display(), "abrir index"); return; }
    };
    if let Err(e) = f.write_all(line.as_bytes()) {
        warn!(?e, "write index");
    }
}

/// Decodifica best-effort. Formato journald nativo: lines de "KEY=value"
/// (binario para values con newlines, pero raro). Formato syslog: texto
/// con prefijo "<priority>tag: message".
fn handle_message(buf: &[u8], source: &'static str) {
    if let Ok(s) = std::str::from_utf8(buf) {
        if s.contains('=') && s.lines().any(|l| l.contains('=')) {
            let mut message = None;
            let mut priority = None;
            let mut unit: Option<String> = None;
            for line in s.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    match k {
                        "MESSAGE" => message = Some(v.to_string()),
                        "PRIORITY" => priority = Some(v.to_string()),
                        "_SYSTEMD_UNIT" | "UNIT" => unit = Some(v.to_string()),
                        _ => {}
                    }
                }
            }
            persist_to_cas(buf, source, unit.as_deref());
            if let Some(msg) = message {
                info!(target: "journal", source, ?priority, ?unit, "{msg}");
            } else {
                debug!(source, len = buf.len(), "journal native sin MESSAGE");
            }
        } else {
            persist_to_cas(buf, source, None);
            info!(target: "syslog", source, "{}", s.trim_end());
        }
    } else {
        persist_to_cas(buf, source, None);
        debug!(source, len = buf.len(), "journal binario (no UTF-8)");
    }
}

async fn announce_to_fractal() {
    if let Ok(mut client) = BusClient::from_env().await {
        let req = BusRequest::Announce {
            capabilities: vec![Capability::Journal],
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
        .unwrap_or_else(|_| EnvFilter::new("arje_journald_compat=info,journal=info,syslog=info"));
    // try_init: bitacora::abrir ya puede haber instalado el subscriber global.
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_target(true).try_init();
}
