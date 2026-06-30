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
    /// Monta un tmpfs vacío en `target` (creando el mountpoint si falta).
    /// `options` es la cadena `size=...` pre-construida por el padre, o vacía
    /// (`""`) para el default del kernel. Requiere mount namespace.
    MountTmpfs { target: CString, options: CString },
    /// Bind-monta `source` sobre `target` (creando el target — dir si
    /// `source_is_dir`, archivo vacío si no — y sus padres). Si `ro`, remonta
    /// el bind sólo-lectura. Requiere mount namespace. Pensado para inyectar
    /// secretos/dotfiles ya materializados (en claro hoy; desde tmpfs en RAM
    /// en Fase 2) dentro del `$HOME` privado del Ente.
    BindMount { source: CString, target: CString, source_is_dir: bool, ro: bool },
    /// Baja privilegios al usuario antes de `execve`: `setgroups` (suplementarios)
    /// → `setgid` → `setuid`, EN ESE ORDEN (tras `setuid` se pierde el privilegio
    /// de cambiar gid/groups). Va al FINAL de la lista (después de mounts/pivot,
    /// que necesitan privilegio). Los gids ya están allocados por el padre.
    DropPrivileges { uid: u32, gid: u32, groups: Vec<u32> },
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

    /// Compila un [`MountPlan`](card_core::MountPlan) declarativo a la secuencia
    /// de ops `mount`/`tmpfs`/`bind` que lo realizan, en orden: primero el
    /// `$HOME` (para que los binds posteriores caigan dentro de un home tmpfs
    /// recién montado), luego los tmpfs extra, luego los binds. Statea el origen
    /// de cada bind AQUÍ (en el padre, con allocator) para decidir dir/archivo —
    /// el hijo, async-signal-safe, sólo crea el target y monta.
    ///
    /// No-op si `plan.is_empty()`. Requiere que la Card tenga `namespaces.mount`.
    pub fn with_mount_plan(mut self, plan: &card_core::MountPlan) -> Result<Self, IncarnateError> {
        use card_core::HomeSpec;
        match &plan.home {
            HomeSpec::Heredar => {
                if let Some(home) = &plan.hide_home_real {
                    self.ops.push(tmpfs_op(Path::new(home), None)?);
                }
            }
            HomeSpec::Tmpfs { destino, size_bytes } => {
                self.ops.push(tmpfs_op(Path::new(destino), *size_bytes)?);
            }
            HomeSpec::Subdir { origen, destino } => {
                self.ops.push(bind_op(Path::new(origen), Path::new(destino), false)?);
            }
        }
        for t in &plan.tmpfs {
            self.ops.push(tmpfs_op(Path::new(&t.destino), t.size_bytes)?);
        }
        for b in &plan.binds {
            self.ops.push(bind_op(Path::new(&b.origen), Path::new(&b.destino), b.ro)?);
        }
        Ok(self)
    }
}

/// Construye una op `MountTmpfs` (resuelve la cadena `size=...`).
fn tmpfs_op(target: &Path, size_bytes: Option<u64>) -> Result<ChildPreExec, IncarnateError> {
    let opts = match size_bytes {
        Some(n) => format!("size={n}"),
        None => String::new(),
    };
    Ok(ChildPreExec::MountTmpfs {
        target: path_cstring(target)?,
        options: CString::new(opts).map_err(|_| IncarnateError::InvalidRootfsPath)?,
    })
}

/// Construye una op `BindMount`, stateando el origen para saber si es dir.
fn bind_op(source: &Path, target: &Path, ro: bool) -> Result<ChildPreExec, IncarnateError> {
    let source_is_dir = std::fs::metadata(source).map(|m| m.is_dir()).unwrap_or(false);
    Ok(ChildPreExec::BindMount {
        source: path_cstring(source)?,
        target: path_cstring(target)?,
        source_is_dir,
        ro,
    })
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
            ChildPreExec::MountTmpfs { target, options } => {
                if unsafe { mkdir_prefixes(target.as_ptr(), true) } != 0 {
                    return 140;
                }
                let data = if options.as_bytes().is_empty() {
                    std::ptr::null()
                } else {
                    options.as_ptr() as *const libc::c_void
                };
                let r = unsafe {
                    libc::mount(
                        b"tmpfs\0".as_ptr() as *const libc::c_char,
                        target.as_ptr(),
                        b"tmpfs\0".as_ptr() as *const libc::c_char,
                        0,
                        data,
                    )
                };
                if r != 0 {
                    return 141;
                }
            }
            ChildPreExec::BindMount { source, target, source_is_dir, ro } => {
                // Crear el mountpoint: dir si el origen es dir, archivo vacío si no.
                if *source_is_dir {
                    if unsafe { mkdir_prefixes(target.as_ptr(), true) } != 0 {
                        return 142;
                    }
                } else {
                    if unsafe { mkdir_prefixes(target.as_ptr(), false) } != 0 {
                        return 142;
                    }
                    let fd = unsafe {
                        libc::open(
                            target.as_ptr(),
                            libc::O_CREAT | libc::O_WRONLY | libc::O_CLOEXEC,
                            0o600,
                        )
                    };
                    if fd < 0 {
                        let e = unsafe { *libc::__errno_location() };
                        if e != libc::EEXIST {
                            return 143;
                        }
                    } else {
                        unsafe { libc::close(fd) };
                    }
                }
                let r = unsafe {
                    libc::mount(
                        source.as_ptr(),
                        target.as_ptr(),
                        std::ptr::null(),
                        libc::MS_BIND,
                        std::ptr::null(),
                    )
                };
                if r != 0 {
                    return 144;
                }
                if *ro {
                    // El bind RO necesita un segundo mount con MS_REMOUNT.
                    let r = unsafe {
                        libc::mount(
                            source.as_ptr(),
                            target.as_ptr(),
                            std::ptr::null(),
                            libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY,
                            std::ptr::null(),
                        )
                    };
                    if r != 0 {
                        return 145;
                    }
                }
            }
            ChildPreExec::DropPrivileges { uid, gid, groups } => {
                // ORDEN: setgroups → setgid → setuid. Tras bajar el uid se pierde
                // el privilegio de cambiar gid/groups, así que el uid va ÚLTIMO.
                let r = unsafe {
                    libc::setgroups(groups.len() as libc::size_t, groups.as_ptr() as *const libc::gid_t)
                };
                if r != 0 {
                    return 130;
                }
                if unsafe { libc::setgid(*gid as libc::gid_t) } != 0 {
                    return 131;
                }
                if unsafe { libc::setuid(*uid as libc::uid_t) } != 0 {
                    return 132;
                }
            }
        }
    }
    0
}

/// `mkdir -p` async-signal-safe sobre un path absoluto. Crea cada componente
/// terminado en `/` con modo 0755, ignorando `EEXIST`. Si `include_final`, crea
/// también el componente final. Devuelve 0 OK, -1 si el path no entra en el
/// buffer o un `mkdir` falla por algo distinto de `EEXIST`.
///
/// SAFETY: corre en el hijo post-clone, pre-execve. Sólo libc; buffer en stack,
/// sin allocator.
unsafe fn mkdir_prefixes(path: *const libc::c_char, include_final: bool) -> i32 {
    let mut buf = [0u8; 4096];
    let mut len = 0usize;
    loop {
        let c = unsafe { *path.add(len) };
        if c == 0 {
            break;
        }
        if len >= buf.len() - 1 {
            return -1;
        }
        buf[len] = c as u8;
        len += 1;
    }
    let mut i = 1usize; // saltear el '/' raíz
    while i < len {
        if buf[i] == b'/' {
            buf[i] = 0;
            if unsafe { mkdir_one(buf.as_ptr() as *const libc::c_char) } != 0 {
                return -1;
            }
            buf[i] = b'/';
        }
        i += 1;
    }
    if include_final && len > 0 && buf[len - 1] != b'/' {
        if unsafe { mkdir_one(buf.as_ptr() as *const libc::c_char) } != 0 {
            return -1;
        }
    }
    0
}

/// `mkdir(path, 0755)` que tolera `EEXIST`. 0 OK / -1 error real.
unsafe fn mkdir_one(path: *const libc::c_char) -> i32 {
    if unsafe { libc::mkdir(path, 0o755) } != 0 {
        let e = unsafe { *libc::__errno_location() };
        if e != libc::EEXIST {
            return -1;
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
    fn mount_plan_compila_home_tmpfs_luego_binds() {
        use card_core::{BindSpec, HomeSpec, MountPlan, TmpfsSpec};
        let plan = MountPlan {
            home: HomeSpec::Tmpfs { destino: "/home/u".into(), size_bytes: Some(1024) },
            tmpfs: vec![TmpfsSpec { destino: "/home/u/.cache".into(), size_bytes: None }],
            binds: vec![BindSpec { origen: "/etc/hostname".into(), destino: "/home/u/.host".into(), ro: true }],
            hide_home_real: None,
        };
        let s = ChildSetup::new().with_mount_plan(&plan).unwrap();
        // Orden: home tmpfs → tmpfs extra → binds.
        match &s.ops[0] {
            ChildPreExec::MountTmpfs { target, options } => {
                assert_eq!(target.to_str().unwrap(), "/home/u");
                assert_eq!(options.to_str().unwrap(), "size=1024");
            }
            o => panic!("op[0] esperaba MountTmpfs home, fue {o:?}"),
        }
        match &s.ops[1] {
            ChildPreExec::MountTmpfs { target, options } => {
                assert_eq!(target.to_str().unwrap(), "/home/u/.cache");
                assert_eq!(options.to_str().unwrap(), ""); // sin size
            }
            o => panic!("op[1] esperaba MountTmpfs cache, fue {o:?}"),
        }
        match &s.ops[2] {
            ChildPreExec::BindMount { source, target, source_is_dir, ro } => {
                assert_eq!(source.to_str().unwrap(), "/etc/hostname");
                assert_eq!(target.to_str().unwrap(), "/home/u/.host");
                assert!(!source_is_dir, "/etc/hostname es archivo");
                assert!(ro);
            }
            o => panic!("op[2] esperaba BindMount, fue {o:?}"),
        }
    }

    #[test]
    fn mount_plan_vacio_no_agrega_ops() {
        let s = ChildSetup::new().with_mount_plan(&card_core::MountPlan::default()).unwrap();
        assert!(s.is_empty());
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
