//! Stream de uevents del kernel vía NETLINK_KOBJECT_UEVENT.
//!
//! Bind requiere CAP_NET_ADMIN. En dev mode normal eso no está disponible —
//! el caller debe estar preparado para que `spawn_uevent_stream` falle, y
//! continuar sin grafo de dispositivos.

use anyhow::Context;
use arje_card::DeviceClass;
use nix::sys::socket::{bind, socket, AddressFamily, NetlinkAddr, SockFlag, SockProtocol, SockType};
use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;
use tracing::{trace, warn};

#[derive(Debug, Clone)]
pub struct UEvent {
    pub action: UAction,
    pub devpath: String,
    pub subsystem: Option<String>,
    pub device_class: Option<DeviceClass>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UAction {
    Add, Remove, Change, Move, Online, Offline, Bind, Unbind, Unknown,
}

pub fn spawn_uevent_stream() -> anyhow::Result<mpsc::Receiver<UEvent>> {
    let fd: OwnedFd = socket(
        AddressFamily::Netlink,
        SockType::Datagram,
        SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
        SockProtocol::NetlinkKObjectUEvent,
    ).context("netlink socket")?;

    // pid=0 → kernel asigna; groups=1 → multicast group del kernel uevent.
    let addr = NetlinkAddr::new(0, 1);
    bind(fd.as_raw_fd(), &addr).context("bind netlink uevent (CAP_NET_ADMIN?)")?;

    let async_fd = AsyncFd::new(NetlinkHandle(fd)).context("AsyncFd::new(netlink)")?;
    let (tx, rx) = mpsc::channel(128);

    tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let mut guard = match async_fd.readable().await {
                Ok(g) => g,
                Err(e) => { warn!(?e, "netlink readable"); return; }
            };
            let raw_fd = guard.get_inner().as_raw_fd();
            loop {
                let n = unsafe {
                    libc::recv(raw_fd, buf.as_mut_ptr() as *mut _, buf.len(), 0)
                };
                if n <= 0 { break; }
                if let Some(evt) = parse_uevent(&buf[..n as usize]) {
                    trace!(?evt.action, devpath = %evt.devpath, "uevent");
                    if tx.send(evt).await.is_err() { return; }
                }
            }
            guard.clear_ready();
        }
    });

    Ok(rx)
}

struct NetlinkHandle(OwnedFd);
impl AsRawFd for NetlinkHandle {
    fn as_raw_fd(&self) -> std::os::fd::RawFd { self.0.as_raw_fd() }
}

fn parse_uevent(buf: &[u8]) -> Option<UEvent> {
    // udev re-emite mensajes con magic "libudev\0..." al multicast group 2.
    // Si llegan al grupo 1 (improbable con bind groups=1) los filtramos igual.
    if buf.starts_with(b"libudev\0") {
        return None;
    }
    let mut parts = buf.split(|b| *b == 0).filter(|s| !s.is_empty());
    let header = std::str::from_utf8(parts.next()?).ok()?;
    let (action_s, devpath) = header.split_once('@')?;
    let mut env: HashMap<String, String> = HashMap::new();
    for kv in parts {
        if let Ok(s) = std::str::from_utf8(kv) {
            if let Some((k, v)) = s.split_once('=') {
                env.insert(k.to_string(), v.to_string());
            }
        }
    }
    let subsystem = env.remove("SUBSYSTEM");
    let device_class = subsystem.as_deref().and_then(map_device_class);
    Some(UEvent {
        action: parse_action(action_s),
        devpath: devpath.to_string(),
        subsystem,
        device_class,
    })
}

fn parse_action(s: &str) -> UAction {
    match s {
        "add" => UAction::Add,
        "remove" => UAction::Remove,
        "change" => UAction::Change,
        "move" => UAction::Move,
        "online" => UAction::Online,
        "offline" => UAction::Offline,
        "bind" => UAction::Bind,
        "unbind" => UAction::Unbind,
        _ => UAction::Unknown,
    }
}

fn map_device_class(subsys: &str) -> Option<DeviceClass> {
    Some(match subsys {
        "block" => DeviceClass::Block,
        "tty" => DeviceClass::Tty,
        "input" => DeviceClass::Input,
        "drm" => DeviceClass::Drm,
        "net" => DeviceClass::Net,
        "hidraw" => DeviceClass::Hidraw,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_uevent() {
        let buf = b"add@/devices/foo\0ACTION=add\0DEVPATH=/devices/foo\0SUBSYSTEM=block\0";
        let evt = parse_uevent(buf).expect("parsed");
        assert_eq!(evt.action, UAction::Add);
        assert_eq!(evt.devpath, "/devices/foo");
        assert_eq!(evt.subsystem.as_deref(), Some("block"));
        assert!(matches!(evt.device_class, Some(DeviceClass::Block)));
    }

    #[test]
    fn libudev_messages_filtered() {
        let buf = b"libudev\0\xfe\xed\xca\xfeadd@/devices/foo\0";
        assert!(parse_uevent(buf).is_none());
    }
}
