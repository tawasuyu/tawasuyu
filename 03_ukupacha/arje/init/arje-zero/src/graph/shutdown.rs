//! Cascade shutdown: SIGTERM en orden topológico (hojas primero), grace
//! period, SIGKILL para stragglers, reap final.

use super::EnteGraph;
use nix::errno::Errno;
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Tiempo que damos a los Entes tras SIGTERM antes de escalar a SIGKILL.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

impl EnteGraph {
    pub async fn cascade_shutdown(&mut self) {
        let order = self.topo_order();
        let pids: Vec<Pid> = order.iter()
            .filter_map(|id| self.incarnated.get(id).and_then(|i| i.pid))
            .collect();

        if pids.is_empty() {
            info!("cascade shutdown: ningún Ente encarnado, salida limpia");
            return;
        }

        info!(
            count = pids.len(), grace_ms = SHUTDOWN_GRACE.as_millis() as u64,
            "SIGTERM cascade (topológico, hojas primero)"
        );
        for pid in &pids {
            match kill(*pid, Signal::SIGTERM) {
                Ok(()) => {}
                Err(Errno::ESRCH) => {} // ya muerto, lo cosecharemos abajo
                Err(e) => warn!(?pid, ?e, "kill SIGTERM falló"),
            }
        }

        let deadline = Instant::now() + SHUTDOWN_GRACE;
        while Instant::now() < deadline {
            if !self.incarnated.values().any(|i| i.pid.is_some()) {
                break;
            }
            match waitpid(None, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(pid, code)) => {
                    self.reap_during_shutdown(pid);
                    debug!(?pid, code, "reaped (exited)");
                }
                Ok(WaitStatus::Signaled(pid, sig, _)) => {
                    self.reap_during_shutdown(pid);
                    debug!(?pid, ?sig, "reaped (signaled)");
                }
                Ok(WaitStatus::StillAlive) | Err(Errno::EINTR) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(Errno::ECHILD) => return,
                Ok(_) => {}
                Err(e) => {
                    warn!(?e, "waitpid fallo en shutdown grace");
                    break;
                }
            }
        }

        let stragglers: Vec<Pid> = self.incarnated.values()
            .filter_map(|i| i.pid)
            .collect();

        if stragglers.is_empty() {
            info!("cascade shutdown completo (todos los Entes terminaron en gracia)");
            return;
        }

        warn!(count = stragglers.len(), "stragglers post-SIGTERM, escalando a SIGKILL");
        for pid in &stragglers {
            let _ = kill(*pid, Signal::SIGKILL);
        }
        loop {
            match waitpid(None, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(pid, _)) | Ok(WaitStatus::Signaled(pid, _, _)) => {
                    self.reap_during_shutdown(pid);
                }
                Ok(WaitStatus::StillAlive) => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(Errno::ECHILD) => break,
                _ => break,
            }
            if !self.incarnated.values().any(|i| i.pid.is_some()) { break; }
        }
        info!("cascade shutdown completo (con SIGKILL)");
    }

    fn reap_during_shutdown(&mut self, pid: Pid) {
        let Some(id) = self.by_pid.remove(&pid.as_raw()) else { return };
        if let Some(inc) = self.incarnated.remove(&id) {
            self.unregister_provider(&inc.card);
        }
    }
}
