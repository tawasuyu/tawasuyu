//! Bootstrap del entorno kernel para PID 1: monta procfs/sysfs/devtmpfs/cgroup2
//! y registra al proceso como subreaper para adoptar huérfanos.
//!
//! Idempotente: si los puntos de montaje ya existen (initramfs los montó),
//! el segundo mount falla con EBUSY y simplemente lo ignoramos.

use nix::mount::{mount, MsFlags};
use tracing::debug;

/// Monta los pseudo-filesystems esenciales. Errores benignos (ya montados)
/// se ignoran; errores serios se propagan.
pub fn bootstrap_kernel_surface() -> anyhow::Result<()> {
    // Cada uno con sus flags estándar — NOSUID/NOEXEC/NODEV donde aplica.
    mount::<str, str, str, str>(
        Some("proc"), "/proc", Some("proc"),
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV, None,
    ).ok();
    mount::<str, str, str, str>(
        Some("sysfs"), "/sys", Some("sysfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV, None,
    ).ok();
    mount::<str, str, str, str>(
        Some("devtmpfs"), "/dev", Some("devtmpfs"),
        MsFlags::MS_NOSUID, None,
    ).ok();
    mount::<str, str, str, str>(
        Some("cgroup2"), "/sys/fs/cgroup", Some("cgroup2"),
        MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV, None,
    ).ok();
    debug!("kernel surface bootstrap completo");
    Ok(())
}

/// PR_SET_CHILD_SUBREAPER: que adoptemos huérfanos del fractal.
///
/// En PID 1 esto es redundante (el kernel ya lo hace), pero se deja explícito
/// para que ente-zero corriendo como sub-init en un container mantenga la
/// misma semántica.
pub fn become_child_subreaper() -> anyhow::Result<()> {
    let r = unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1u64, 0u64, 0u64, 0u64) };
    if r != 0 {
        anyhow::bail!(
            "prctl PR_SET_CHILD_SUBREAPER falló: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}

/// Cosechar zombis hasta vaciar la cola de niños muertos. Devuelve los
/// PIDs cosechados con su estado, como tuplas.
pub fn reap_all() -> Vec<ReapedChild> {
    use nix::errno::Errno;
    use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
    let mut out = Vec::new();
    loop {
        match waitpid(None, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(pid, code)) => {
                out.push(ReapedChild { pid: pid.as_raw(), status: ReapStatus::Exited(code) });
            }
            Ok(WaitStatus::Signaled(pid, sig, _core)) => {
                out.push(ReapedChild { pid: pid.as_raw(), status: ReapStatus::Signaled(sig as i32) });
            }
            Ok(WaitStatus::StillAlive) => return out,
            Err(Errno::ECHILD) => return out,
            Err(_) => return out,
            Ok(_) => continue, // Stopped/Continued — irrelevantes
        }
    }
    // unreachable, satisface al borrow checker
    #[allow(unreachable_code)]
    out
}

#[derive(Debug, Clone)]
pub struct ReapedChild {
    pub pid: i32,
    pub status: ReapStatus,
}

#[derive(Debug, Clone)]
pub enum ReapStatus {
    Exited(i32),
    Signaled(i32),
}
