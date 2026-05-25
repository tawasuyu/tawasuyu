//! Bootstrap del entorno kernel para PID 1: remonta `/` rw, monta
//! procfs/sysfs/devtmpfs/cgroup2 y las superficies escribibles volátiles
//! (`/run`, `/tmp`, `/dev/pts`, `/dev/shm`), y registra al proceso como
//! subreaper para adoptar huérfanos.
//!
//! Idempotente: si un punto de montaje ya existe (lo montó el initramfs),
//! el segundo mount falla con EBUSY y simplemente se ignora.
//!
//! **Por qué importa `/run`:** el cmdline de arranque suele traer `ro`
//! (systemd remonta rw temprano; nosotros también debemos). Sin remontar
//! `/` y sin `/run` como tmpfs, crear el socket del bus interno falla con
//! EROFS — y PID 1 moriría, provocando un kernel panic. Esta función es
//! infalible a propósito: devuelve `Ok` siempre y sólo loggea los fallos.

use nix::mount::{mount, MsFlags};
use tracing::{debug, warn};

/// Prepara el entorno del kernel para PID 1. Nunca falla de forma dura:
/// cada paso es best-effort y los problemas se loggean, porque un `Err`
/// que llegue hasta `main` terminaría PID 1.
pub fn bootstrap_kernel_surface() -> anyhow::Result<()> {
    // 1) Remontar `/` lectura-escritura. El cmdline casi siempre trae
    //    `ro`; sin esto el resto del sistema queda de sólo lectura.
    if let Err(e) = mount::<str, str, str, str>(
        None, "/", None, MsFlags::MS_REMOUNT, None,
    ) {
        warn!(?e, "remount / rw falló — el sistema puede quedar de sólo lectura");
    }

    // 2) Pseudo-filesystems del kernel. NOSUID/NOEXEC/NODEV donde aplica.
    let pseudo: [(&str, &str, &str, MsFlags); 4] = [
        ("proc", "/proc", "proc",
            MsFlags::MS_NOSUID.union(MsFlags::MS_NOEXEC).union(MsFlags::MS_NODEV)),
        ("sysfs", "/sys", "sysfs",
            MsFlags::MS_NOSUID.union(MsFlags::MS_NOEXEC).union(MsFlags::MS_NODEV)),
        ("devtmpfs", "/dev", "devtmpfs", MsFlags::MS_NOSUID),
        ("cgroup2", "/sys/fs/cgroup", "cgroup2",
            MsFlags::MS_NOSUID.union(MsFlags::MS_NOEXEC).union(MsFlags::MS_NODEV)),
    ];
    for (src, dst, fstype, flags) in pseudo {
        let _ = mount::<str, str, str, str>(Some(src), dst, Some(fstype), flags, None);
    }

    // 3) Superficies escribibles volátiles. `/run` como tmpfs es lo que
    //    permite crear el socket del bus interno aun con `/` de sólo
    //    lectura. `mkdir` best-effort antes de cada montaje.
    let volatile: [(&str, &str, &str, MsFlags, &str); 4] = [
        ("tmpfs", "/run", "tmpfs",
            MsFlags::MS_NOSUID.union(MsFlags::MS_NODEV), "mode=0755"),
        ("tmpfs", "/tmp", "tmpfs",
            MsFlags::MS_NOSUID.union(MsFlags::MS_NODEV), "mode=1777"),
        ("devpts", "/dev/pts", "devpts",
            MsFlags::MS_NOSUID.union(MsFlags::MS_NOEXEC), "mode=0620,gid=5"),
        ("tmpfs", "/dev/shm", "tmpfs",
            MsFlags::MS_NOSUID.union(MsFlags::MS_NODEV), "mode=1777"),
    ];
    for (src, dst, fstype, flags, data) in volatile {
        let _ = std::fs::create_dir_all(dst);
        let _ = mount::<str, str, str, str>(Some(src), dst, Some(fstype), flags, Some(data));
    }
    let _ = std::fs::create_dir_all("/run/lock");

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
