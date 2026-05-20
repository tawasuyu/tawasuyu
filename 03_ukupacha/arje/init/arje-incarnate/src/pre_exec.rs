//! Hook declarativo pre-execve para el hijo.
//!
//! Las ops corren EN EL HIJO, post-fork/clone, pre-execve. Reglas:
//! - sólo syscalls async-signal-safe.
//! - sin allocator (los CStrings ya están construidos por el padre).
//! - sin Drop con efectos.

use std::ffi::CString;

/// Operaciones declarativas aplicables pre-execve.
#[derive(Debug, Clone)]
pub enum ChildPreExec {
    /// `PR_SET_NO_NEW_PRIVS = 1` — bloquea escaladas futuras
    /// (suid bits, file caps, AT_SECURE). Recomendado en sandboxes.
    NoNewPrivs,
    /// `PR_SET_PDEATHSIG = sig` — el child recibe esta señal cuando su
    /// padre (PID 1 del namespace, o el que sea) muere. Útil para
    /// auto-cleanup de procesos huérfanos.
    ParentDeathSig(i32),
    /// `PR_SET_DUMPABLE` — controla si el proceso permite core dump.
    Dumpable(bool),
    /// `setsid()` — nuevo session/group leader (desconecta del controlling tty).
    NewSession,
    /// `chdir(path)` — cambiar working dir. Path pre-allocado.
    Chdir(CString),
    /// `umask(mode)` — fijar umask (octal, e.g. 0o022).
    Umask(libc::mode_t),
}

/// Setup completo del hijo. Default = sin ops.
#[derive(Debug, Clone, Default)]
pub struct ChildSetup {
    pub ops: Vec<ChildPreExec>,
}

impl ChildSetup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, op: ChildPreExec) -> &mut Self {
        self.ops.push(op);
        self
    }

    pub fn with(mut self, op: ChildPreExec) -> Self {
        self.ops.push(op);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// Aplica las ops en orden. SAFETY: ejecuta en el hijo, post-fork,
/// pre-execve. Sólo libc, sin allocator, sin Drop.
///
/// En caso de error, retorna el código de exit que el caller usará para
/// abortar el child (igual semántica que el resto de la closure de clone).
/// 0 = todo OK.
pub unsafe fn apply_unchecked(ops: &[ChildPreExec]) -> i32 {
    for op in ops {
        match op {
            ChildPreExec::NoNewPrivs => {
                // PR_SET_NO_NEW_PRIVS = 38 en Linux.
                let r = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1u64, 0u64, 0u64, 0u64) };
                if r != 0 {
                    return 110;
                }
            }
            ChildPreExec::ParentDeathSig(sig) => {
                let r = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, *sig as u64, 0u64, 0u64, 0u64) };
                if r != 0 {
                    return 111;
                }
            }
            ChildPreExec::Dumpable(yes) => {
                let v: u64 = if *yes { 1 } else { 0 };
                let r = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, v, 0u64, 0u64, 0u64) };
                if r != 0 {
                    return 112;
                }
            }
            ChildPreExec::NewSession => {
                let r = unsafe { libc::setsid() };
                if r < 0 {
                    return 113;
                }
            }
            ChildPreExec::Chdir(path) => {
                let r = unsafe { libc::chdir(path.as_ptr()) };
                if r != 0 {
                    return 114;
                }
            }
            ChildPreExec::Umask(mode) => {
                unsafe { libc::umask(*mode) };
            }
        }
    }
    0
}
