//! Hook declarativo pre-execve para el hijo.
//!
//! Las ops corren EN EL HIJO, post-fork/clone, pre-execve. Reglas:
//! - sólo syscalls async-signal-safe.
//! - sin allocator (los CStrings ya están construidos por el padre).
//! - sin Drop con efectos.

use crate::error::IncarnateError;
use std::ffi::CString;
use std::path::Path;

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
    /// `ioctl(0, TIOCSCTTY)` — hace del PTY que está en fd 0 el **controlling
    /// terminal** del proceso. Requiere ser session leader: combinar con
    /// `NewSession` *antes* en la lista. Habilita job control real (Ctrl-C /
    /// Ctrl-Z al foreground group, `/dev/tty`) en sesiones interactivas.
    ControllingTty,
    /// `chdir(path)` — cambiar working dir. Path pre-allocado.
    Chdir(CString),
    /// `umask(mode)` — fijar umask (octal, e.g. 0o022).
    Umask(libc::mode_t),
    /// Monta un OverlayFS en `target`. `options` es la cadena
    /// `lowerdir=...,upperdir=...,workdir=...` pre-construida por el padre.
    /// Requiere mount namespace (CLONE_NEWNS) ya establecido.
    MountOverlay { target: CString, options: CString },
    /// `pivot_root`: `new_root` pasa a ser `/`. El root viejo va a
    /// `put_old` (dir existente dentro de new_root) y se desmonta lazy
    /// (`MNT_DETACH`) tras pivotar. `old_root_after` es la ruta del root
    /// viejo YA pivotado (e.g. `/.oldroot`). Requiere mount namespace.
    PivotRoot {
        new_root: CString,
        put_old: CString,
        old_root_after: CString,
    },
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

    /// Agrega un mount OverlayFS: `lower` = capa base RO, `upper` = capa
    /// de sesión RW (persiste cambios), `work` = scratch interno de
    /// overlayfs, `merged` = mountpoint resultante.
    ///
    /// El padre debe garantizar que `upper`, `work` y `merged` existen
    /// antes de encarnar. Pensado para encadenarse antes de `with_pivot_root`.
    pub fn with_overlay(
        mut self,
        lower: &Path,
        upper: &Path,
        work: &Path,
        merged: &Path,
    ) -> Result<Self, IncarnateError> {
        let opts = format!(
            "lowerdir={},upperdir={},workdir={}",
            lower.display(),
            upper.display(),
            work.display(),
        );
        let target = path_cstring(merged)?;
        let options =
            CString::new(opts).map_err(|_| IncarnateError::InvalidRootfsPath)?;
        self.ops.push(ChildPreExec::MountOverlay { target, options });
        Ok(self)
    }

    /// Agrega un `pivot_root` a `new_root`. `put_old_name` es el nombre
    /// del subdirectorio (dentro de new_root, debe existir) que recibe el
    /// root viejo; tras pivotar se desmonta lazy.
    pub fn with_pivot_root(
        mut self,
        new_root: &Path,
        put_old_name: &str,
    ) -> Result<Self, IncarnateError> {
        let new_root_c = path_cstring(new_root)?;
        let put_old_c = path_cstring(&new_root.join(put_old_name))?;
        let old_root_after = CString::new(format!("/{put_old_name}"))
            .map_err(|_| IncarnateError::InvalidRootfsPath)?;
        self.ops.push(ChildPreExec::PivotRoot {
            new_root: new_root_c,
            put_old: put_old_c,
            old_root_after,
        });
        Ok(self)
    }
}

/// Convierte un `Path` a `CString` (rechaza NUL bytes interiores).
fn path_cstring(p: &Path) -> Result<CString, IncarnateError> {
    use std::os::unix::ffi::OsStrExt;
    CString::new(p.as_os_str().as_bytes())
        .map_err(|_| IncarnateError::InvalidRootfsPath)
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
            ChildPreExec::ControllingTty => {
                // fd 0 ya es el PTS (el dup del stdio corre antes que las ops).
                // arg 0 = no robar el tty de otra sesión.
                let r = unsafe { libc::ioctl(0, libc::TIOCSCTTY, 0) };
                if r != 0 {
                    return 120;
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
            ChildPreExec::MountOverlay { target, options } => {
                let r = unsafe {
                    libc::mount(
                        b"overlay\0".as_ptr() as *const libc::c_char,
                        target.as_ptr(),
                        b"overlay\0".as_ptr() as *const libc::c_char,
                        0,
                        options.as_ptr() as *const libc::c_void,
                    )
                };
                if r != 0 {
                    return 115;
                }
            }
            ChildPreExec::PivotRoot { new_root, put_old, old_root_after } => {
                // pivot_root exige que new_root sea un mount point:
                // bind-mount recursivo sobre sí mismo lo garantiza.
                let r = unsafe {
                    libc::mount(
                        new_root.as_ptr(),
                        new_root.as_ptr(),
                        std::ptr::null(),
                        libc::MS_BIND | libc::MS_REC,
                        std::ptr::null(),
                    )
                };
                if r != 0 {
                    return 116;
                }
                let r = unsafe {
                    libc::syscall(libc::SYS_pivot_root, new_root.as_ptr(), put_old.as_ptr())
                };
                if r != 0 {
                    return 117;
                }
                let r = unsafe { libc::chdir(b"/\0".as_ptr() as *const libc::c_char) };
                if r != 0 {
                    return 118;
                }
                // Desmontaje lazy del root viejo: se desliga del árbol ya;
                // los fds abiertos contra él siguen válidos hasta cerrarse.
                let r = unsafe {
                    libc::umount2(old_root_after.as_ptr(), libc::MNT_DETACH)
                };
                if r != 0 {
                    return 119;
                }
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_builds_correct_options() {
        let s = ChildSetup::new()
            .with_overlay(
                Path::new("/base"),
                Path::new("/sess"),
                Path::new("/work"),
                Path::new("/merged"),
            )
            .expect("overlay");
        match &s.ops[0] {
            ChildPreExec::MountOverlay { target, options } => {
                assert_eq!(target.to_str().unwrap(), "/merged");
                assert_eq!(
                    options.to_str().unwrap(),
                    "lowerdir=/base,upperdir=/sess,workdir=/work"
                );
            }
            other => panic!("esperaba MountOverlay, fue {other:?}"),
        }
    }

    #[test]
    fn pivot_root_builds_paths() {
        let s = ChildSetup::new()
            .with_pivot_root(Path::new("/newroot"), ".oldroot")
            .expect("pivot");
        match &s.ops[0] {
            ChildPreExec::PivotRoot { new_root, put_old, old_root_after } => {
                assert_eq!(new_root.to_str().unwrap(), "/newroot");
                assert_eq!(put_old.to_str().unwrap(), "/newroot/.oldroot");
                assert_eq!(old_root_after.to_str().unwrap(), "/.oldroot");
            }
            other => panic!("esperaba PivotRoot, fue {other:?}"),
        }
    }

    #[test]
    fn overlay_then_pivot_preserves_order() {
        let s = ChildSetup::new()
            .with_overlay(
                Path::new("/b"),
                Path::new("/u"),
                Path::new("/w"),
                Path::new("/m"),
            )
            .unwrap()
            .with_pivot_root(Path::new("/m"), ".oldroot")
            .unwrap();
        assert!(matches!(s.ops[0], ChildPreExec::MountOverlay { .. }));
        assert!(matches!(s.ops[1], ChildPreExec::PivotRoot { .. }));
    }
}
