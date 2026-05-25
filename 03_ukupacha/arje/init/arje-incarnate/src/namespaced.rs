//! Path namespaced: clone(2) + sync pipe + setup post-clone en padre + finalize en hijo.
//!
//! ## Protocolo padre↔hijo
//!
//! ```text
//!   parent               child
//!     |                    |
//!     |--- clone() ------->|   (child empieza dentro de los nuevos NS)
//!     |                    |
//!     |                    |---- read(sync_r, 1) ----  (bloquea)
//!     |                    |
//!     |  write uid_map     |
//!     |  write gid_map     |
//!     |  cgroup move       |
//!     |  cpu affinity      |
//!     |                    |
//!     |--- write(sync_w) ->|
//!     |                    |---- setrlimit
//!     |                    |---- mount(/, MS_PRIVATE | MS_REC)
//!     |                    |---- execve()
//! ```

use crate::child::{apply_rlimits, make_root_private};
use crate::cgroup::{ensure_cgroup, move_to_cgroup};
use crate::env::{build_env, EnvSpec};
use crate::error::{Degradation, IncarnateError};
use crate::pre_exec::{apply_unchecked, ChildSetup};
use crate::ChildStdio;
use card_core::{Card, NamespaceSet, Payload};
use nix::fcntl::OFlag;
use nix::sched::CloneFlags;
use nix::unistd::{pipe2, Pid};
use std::ffi::CString;
use std::os::fd::{IntoRawFd, RawFd};
use tracing::{info, warn};

pub fn needs_namespacing(ns: &NamespaceSet) -> bool {
    ns.mount || ns.pid || ns.net || ns.uts || ns.ipc || ns.user || ns.cgroup
}

pub fn build_clone_flags(ns: &NamespaceSet) -> CloneFlags {
    let mut f = CloneFlags::empty();
    if ns.mount  { f |= CloneFlags::CLONE_NEWNS; }
    if ns.pid    { f |= CloneFlags::CLONE_NEWPID; }
    if ns.net    { f |= CloneFlags::CLONE_NEWNET; }
    if ns.uts    { f |= CloneFlags::CLONE_NEWUTS; }
    if ns.ipc    { f |= CloneFlags::CLONE_NEWIPC; }
    if ns.user   { f |= CloneFlags::CLONE_NEWUSER; }
    if ns.cgroup { f |= CloneFlags::CLONE_NEWCGROUP; }
    f
}

pub fn incarnate_namespaced(
    card: &Card,
    env_spec: &EnvSpec,
    stdio: &ChildStdio,
    setup: &ChildSetup,
    degradations: &mut Vec<Degradation>,
) -> Result<Pid, IncarnateError> {
    let flags = build_clone_flags(&card.soma.namespaces);
    info!(label = %card.label, ?flags, "namespaced incarnation");

    let (exec, argv, base_envp) = match &card.payload {
        Payload::Native { exec, argv, envp } => (exec.clone(), argv.clone(), envp.clone()),
        Payload::Legacy { exec, argv, .. } => (exec.clone(), argv.clone(), Vec::new()),
        _ => return Err(IncarnateError::NonExecutablePayload),
    };

    // Pipe O_CLOEXEC: el read del lado hijo es lo que hace race-free el setup.
    // O_CLOEXEC garantiza cierre automático en execve.
    let (sync_r, sync_w) = pipe2(OFlag::O_CLOEXEC).map_err(IncarnateError::Pipe)?;
    let sync_r_raw: RawFd = sync_r.into_raw_fd();
    let sync_w_raw: RawFd = sync_w.into_raw_fd();

    let exec_c = CString::new(exec.clone()).map_err(|_| IncarnateError::InvalidArgv)?;
    let argv_c: Vec<CString> = std::iter::once(exec_c.clone())
        .chain(argv.iter().filter_map(|s| CString::new(s.as_str()).ok()))
        .collect();
    let argv_ptrs: Vec<*const libc::c_char> = argv_c
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    let env_pairs = build_env(card, &base_envp, env_spec);
    let envp_c: Vec<CString> = env_pairs
        .iter()
        .filter_map(|(k, v)| CString::new(format!("{k}={v}")).ok())
        .collect();
    let envp_ptrs: Vec<*const libc::c_char> = envp_c
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    let rlimits = card.soma.rlimits.clone();
    let mount_ns_enabled = card.soma.namespaces.mount;
    let stdin_fd = stdio.stdin_fd;
    let stdout_fd = stdio.stdout_fd;
    let stderr_fd = stdio.stderr_fd;
    let setup_ops = setup.ops.clone();

    // SAFETY: la clausura corre en stack nuevo dentro de un proceso recién
    // clonado, COW del padre. Sólo syscalls async-signal-safe; sin allocator,
    // sin Drop con efectos.
    let cb = Box::new(move || -> isize {
        unsafe { libc::close(sync_w_raw); }

        let mut byte = [0u8; 1];
        let n = unsafe { libc::read(sync_r_raw, byte.as_mut_ptr() as *mut _, 1) };
        if n != 1 {
            unsafe { libc::_exit(101); }
        }
        unsafe { libc::close(sync_r_raw); }

        unsafe { apply_rlimits(&rlimits); }

        if mount_ns_enabled {
            unsafe { make_root_private(); }
        }

        // dup2 declarativo: caller pasó fds que queremos como stdin/out/err.
        // dup2 es async-signal-safe (POSIX) y cierra el fd target si estaba
        // abierto. El fd source NO se cierra automáticamente — el padre
        // tiene su propia copia.
        if let Some(fd) = stdin_fd {
            unsafe {
                if libc::dup2(fd, 0) < 0 {
                    libc::_exit(103);
                }
            }
        }
        if let Some(fd) = stdout_fd {
            unsafe {
                if libc::dup2(fd, 1) < 0 {
                    libc::_exit(104);
                }
            }
        }
        if let Some(fd) = stderr_fd {
            unsafe {
                if libc::dup2(fd, 2) < 0 {
                    libc::_exit(105);
                }
            }
        }

        // Aplica las ops declarativas pre-execve (NoNewPrivs, chdir, etc.).
        if !setup_ops.is_empty() {
            let r = unsafe { apply_unchecked(&setup_ops) };
            if r != 0 {
                unsafe { libc::_exit(r) };
            }
        }

        unsafe {
            libc::execve(exec_c.as_ptr(), argv_ptrs.as_ptr(), envp_ptrs.as_ptr());
            libc::_exit(102);
        }
    });

    let mut stack = vec![0u8; 1024 * 1024];

    #[allow(deprecated)]
    let pid = unsafe { nix::sched::clone(cb, &mut stack, flags, Some(libc::SIGCHLD)) }
        .map_err(|e| {
            unsafe {
                libc::close(sync_r_raw);
                libc::close(sync_w_raw);
            }
            IncarnateError::Clone(e)
        })?;

    // Padre: cerrar el extremo de lectura.
    unsafe { libc::close(sync_r_raw); }

    // Setup post-clone. Errores aquí los registramos como degradations y
    // continuamos (la decisión strict_caps la toma el wrapper).
    if let Err(e) = configure_child(pid, card, degradations) {
        warn!(?e, ?pid, "configure_child errores");
    }

    // Despertar al hijo.
    let signal_byte = [b'x'];
    let written = unsafe { libc::write(sync_w_raw, signal_byte.as_ptr() as *const _, 1) };
    unsafe { libc::close(sync_w_raw); }
    if written != 1 {
        warn!(?pid, "write sync pipe devolvió {}", written);
    }

    // El hijo ya dup2-eó los fds del ChildStdio. La copia del padre no
    // sirve más y la cerramos para que el otro extremo del pipe reciba
    // EOF cuando corresponda.
    if let Some(fd) = stdio.stdin_fd {
        unsafe { libc::close(fd); }
    }
    if let Some(fd) = stdio.stdout_fd {
        unsafe { libc::close(fd); }
    }
    if let Some(fd) = stdio.stderr_fd {
        unsafe { libc::close(fd); }
    }

    Ok(pid)
}

/// Setup que requiere capacidades del padre: uid_map, gid_map, cgroup move.
/// Estos archivos en `/proc/<pid>/*` tienen reglas de propiedad que sólo el
/// padre puede satisfacer mientras el hijo está suspendido en el sync pipe.
fn configure_child(
    pid: Pid,
    card: &Card,
    degradations: &mut Vec<Degradation>,
) -> Result<(), IncarnateError> {
    if card.soma.namespaces.user {
        // Desde kernel 3.19 hay que escribir "deny" a setgroups antes de
        // poder escribir gid_map sin CAP_SETGID. Ignorar errores aquí: en
        // kernels antiguos el archivo no existe.
        let _ = std::fs::write(format!("/proc/{}/setgroups", pid.as_raw()), "deny");

        let uid = nix::unistd::getuid().as_raw();
        let gid = nix::unistd::getgid().as_raw();
        if let Err(e) = std::fs::write(
            format!("/proc/{}/uid_map", pid.as_raw()),
            format!("0 {uid} 1"),
        ) {
            degradations.push(Degradation::UidMapFailed {
                reason: format!("uid_map: {e}"),
            });
        }
        if let Err(e) = std::fs::write(
            format!("/proc/{}/gid_map", pid.as_raw()),
            format!("0 {gid} 1"),
        ) {
            degradations.push(Degradation::UidMapFailed {
                reason: format!("gid_map: {e}"),
            });
        }
    }

    if !card.soma.cgroup.path.is_empty() {
        match ensure_cgroup(&card.soma.cgroup) {
            Ok(abs) => {
                if let Err(e) = move_to_cgroup(&abs, pid) {
                    degradations.push(Degradation::CgroupSkipped {
                        path: abs,
                        reason: format!("{e}"),
                    });
                }
            }
            Err(e) => degradations.push(Degradation::CgroupSkipped {
                path: std::path::PathBuf::from(&card.soma.cgroup.path),
                reason: format!("{e}"),
            }),
        }
    }

    if let Some(cpus) = &card.soma.cpu_affinity {
        if let Err(e) = set_cpu_affinity(pid, cpus) {
            degradations.push(Degradation::CpuAffinitySkipped {
                reason: format!("{e}"),
            });
        }
    }

    Ok(())
}

fn set_cpu_affinity(pid: Pid, cpus: &[u32]) -> Result<(), std::io::Error> {
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    unsafe { libc::CPU_ZERO(&mut set); }
    for &c in cpus {
        unsafe { libc::CPU_SET(c as usize, &mut set); }
    }
    let r = unsafe {
        libc::sched_setaffinity(pid.as_raw(), std::mem::size_of::<libc::cpu_set_t>(), &set)
    };
    if r != 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use card_core::NamespaceSet;

    #[test]
    fn empty_ns_does_not_need_namespacing() {
        let ns = NamespaceSet::default();
        assert!(!needs_namespacing(&ns));
    }

    #[test]
    fn any_ns_triggers_namespacing() {
        let mut ns = NamespaceSet::default();
        ns.user = true;
        assert!(needs_namespacing(&ns));
    }

    #[test]
    fn flags_match_namespace_bools() {
        let mut ns = NamespaceSet::default();
        ns.user = true;
        ns.pid = true;
        let f = build_clone_flags(&ns);
        assert!(f.contains(CloneFlags::CLONE_NEWUSER));
        assert!(f.contains(CloneFlags::CLONE_NEWPID));
        assert!(!f.contains(CloneFlags::CLONE_NEWNET));
    }
}
