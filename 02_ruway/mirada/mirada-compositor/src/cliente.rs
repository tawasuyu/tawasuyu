// Datos por cliente Wayland.
use crate::*;
use smithay::wayland::compositor::CompositorClientState;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};

#[derive(Default)]
pub struct ClientState {
    pub(crate) compositor_state: CompositorClientState,
    /// PID del proceso cliente, leído de las credenciales del socket Unix al
    /// aceptarlo (`SO_PEERCRED`). Alimenta el linaje de las *constelaciones*.
    /// `None` si el backend no expone credenciales.
    pub(crate) pid: Option<i32>,
}
impl ClientState {
    /// Estado de cliente con su PID (de `UnixStream::peer_cred`).
    pub fn with_pid(pid: Option<i32>) -> Self {
        Self { pid, ..Default::default() }
    }
}
impl ClientData for ClientState {
    fn initialized(&self, _id: ClientId) {}
    fn disconnected(&self, _id: ClientId, _reason: DisconnectReason) {}
}

/// El PID del cliente al otro extremo de un socket Unix, vía `SO_PEERCRED`.
/// `None` si el kernel no lo expone (no debería pasar en sockets locales). Es la
/// raíz del linaje de las *constelaciones*. Se llama en `pub(crate)` desde el
/// backend DRM, que tiene su propio bucle de `accept`.
pub(crate) fn peer_pid(stream: &std::os::unix::net::UnixStream) -> Option<i32> {
    use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
    getsockopt(stream, PeerCredentials)
        .ok()
        .map(|c| c.pid())
        .filter(|&p| p > 0)
}

/// La cadena de PIDs ancestros de un proceso (padre inmediato primero), leída de
/// `/proc/<pid>/stat`. Acotada a 32 saltos por si /proc miente o cicla. Vacía si
/// Toma un `RwLock` para lectura **tolerando el veneno**: si otro hilo paniqueó
/// con el lock tomado, Rust lo marca envenenado y `read().unwrap()` propagaría
/// el panic — tumbando la sesión entera (justo lo que la auditoría de robustez
/// evita). Los datos que protegemos así (bitfield de capacidades, datos planos
/// de smithay) no tienen invariantes que un panic a medias pueda romper, así que
/// recuperamos el guard con `into_inner()` y seguimos.
pub(crate) fn leer_tolerante<T>(l: &std::sync::RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}

/// Variante de escritura de [`leer_tolerante`].
pub(crate) fn escribir_tolerante<T>(l: &std::sync::RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

/// Variante para `Mutex` de [`leer_tolerante`].
pub(crate) fn lock_tolerante<T>(l: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    l.lock().unwrap_or_else(|e| e.into_inner())
}

/// no se puede leer (el proceso ya murió, no es Linux…). Alimenta el linaje de
/// las *constelaciones* del Cerebro.
pub(crate) fn process_ancestors(pid: i32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut cur = pid;
    for _ in 0..32 {
        let Some(ppid) = read_ppid(cur) else { break };
        if ppid <= 0 || ppid == cur {
            break;
        }
        out.push(ppid as u32);
        if ppid == 1 {
            break; // init: la raíz del árbol de procesos
        }
        cur = ppid;
    }
    out
}

/// El ejecutable real de un proceso, leído de `/proc/<pid>/exe` (un symlink al
/// binario). Es la **identidad honesta** del cliente para decidir capacidades:
/// la da el kernel, no el cliente (a diferencia del `app_id`, que es aserción).
/// `None` si el proceso ya murió o `/proc` no expone el enlace.
pub(crate) fn exe_de_pid(pid: i32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// El PPID de un proceso desde `/proc/<pid>/stat` (campo 4). El `comm` (campo 2)
/// puede llevar espacios y paréntesis, así que se parsea desde el último ')':
/// tras él viene " <state> <ppid> …", y el ppid es el segundo token.
pub(crate) fn read_ppid(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(1)?.parse().ok()
}
