//! Bootstrap del entorno kernel para PID 1: remonta `/` rw (sólo si hace
//! falta), monta procfs/sysfs/devtmpfs/cgroup2 y las superficies escribibles
//! volátiles (`/run`, `/tmp`, `/dev/pts`, `/dev/shm`), y registra al proceso
//! como subreaper para adoptar huérfanos.
//!
//! **Idempotente y convivente.** Antes de cada montaje:
//!   1. Se crea el directorio target (best-effort) — así no se cae por
//!      "el punto de montaje no existe".
//!   2. Se salta si el target **ya es un punto de montaje** (lo dejó el
//!      initramfs, OpenRC, systemd, o un arranque previo de arje).
//!
//! Esto es clave: `mount(2)` de un tmpfs nuevo sobre un `/run`/`/tmp`/`/proc`
//! ya montado **NO da EBUSY** — Linux lo **apila** y oculta el de abajo,
//! rompiendo lo que esperaba el contenido previo. El antiguo "se ignora con
//! EBUSY" era falso para ese caso y podía dejar el sistema atorado. El test
//! de punto-de-montaje (st_dev del dir ≠ st_dev del padre, igual que
//! `mountpoint(1)`) no depende de `/proc`, así que sirve incluso antes de
//! montar procfs.
//!
//! **Por qué importa `/run`:** el cmdline de arranque suele traer `ro`
//! (systemd remonta rw temprano; nosotros también debemos). Sin remontar
//! `/` y sin `/run` como tmpfs, crear el socket del bus interno falla con
//! EROFS — y PID 1 moriría, provocando un kernel panic. Esta función es
//! infalible a propósito: devuelve `Ok` siempre y sólo loggea los fallos.

use nix::mount::{mount, MsFlags};
use nix::sys::statvfs::{statvfs, FsFlags};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use tracing::{debug, info, warn};

/// ¿`path` es la raíz de un montaje? Test clásico de `mountpoint(1)`: un
/// directorio es punto de montaje si su `st_dev` difiere del de su padre.
/// No abre `/proc`, así que es usable antes de montar procfs. Ante cualquier
/// duda (target ausente, padre ilegible) devuelve `false` → se intenta montar.
fn is_mountpoint(path: &str) -> bool {
    let p = Path::new(path);
    let Ok(here) = std::fs::metadata(p) else { return false };
    let Some(parent) = p.parent() else { return false }; // "/" no tiene padre
    let Ok(up) = std::fs::metadata(parent) else { return false };
    here.dev() != up.dev()
}

/// ¿`/` está montado de sólo lectura ahora mismo? Si no podemos saberlo,
/// devolvemos `false` para **no** tocar una raíz que quizá ya está sana.
fn root_is_readonly() -> bool {
    match statvfs("/") {
        Ok(s) => s.flags().contains(FsFlags::ST_RDONLY),
        Err(_) => false,
    }
}

/// Monta `fstype` en `dst` de forma idempotente y best-effort: crea el dir,
/// respeta un montaje preexistente (no apila), y nunca aborta el arranque.
fn ensure_mount(src: &str, dst: &str, fstype: &str, flags: MsFlags, data: Option<&str>) {
    let _ = std::fs::create_dir_all(dst);
    if is_mountpoint(dst) {
        debug!(dst, fstype, "ya montado — respetado (no se apila)");
        return;
    }
    match mount::<str, str, str, str>(Some(src), dst, Some(fstype), flags, data) {
        Ok(()) => debug!(dst, fstype, "montado"),
        Err(e) => warn!(?e, dst, fstype, "mount falló — se continúa (best-effort)"),
    }
}

/// Prepara el entorno del kernel para PID 1. Nunca falla de forma dura:
/// cada paso es best-effort y los problemas se loggean, porque un `Err`
/// que llegue hasta `main` terminaría PID 1.
pub fn bootstrap_kernel_surface() -> anyhow::Result<()> {
    // 1) Remontar `/` rw SÓLO si está de sólo lectura. Si el initramfs u
    //    OpenRC ya la dejó rw, no la tocamos: un remount a ciegas puede pisar
    //    opciones de montaje ajenas que el sistema sí usa.
    if root_is_readonly() {
        match mount::<str, str, str, str>(None, "/", None, MsFlags::MS_REMOUNT, None) {
            Ok(()) => info!("/ remontado rw"),
            Err(e) => warn!(?e, "remount / rw falló — el sistema puede quedar de sólo lectura"),
        }
    } else {
        debug!("/ ya estaba rw — no se remonta");
    }

    // 2) Pseudo-filesystems del kernel. NOSUID/NOEXEC/NODEV donde aplica.
    //    El orden importa: `/sys` antes que `/sys/fs/cgroup`.
    let pseudo: [(&str, &str, &str, MsFlags); 4] = [
        ("proc", "/proc", "proc",
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV),
        ("sysfs", "/sys", "sysfs",
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV),
        ("devtmpfs", "/dev", "devtmpfs", MsFlags::MS_NOSUID),
        ("cgroup2", "/sys/fs/cgroup", "cgroup2",
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV),
    ];
    for (src, dst, fstype, flags) in pseudo {
        ensure_mount(src, dst, fstype, flags, None);
    }

    // 3) Superficies escribibles volátiles. `/run` como tmpfs es lo que
    //    permite crear el socket del bus interno aun con `/` de sólo lectura.
    let volatile: [(&str, &str, &str, MsFlags, &str); 4] = [
        ("tmpfs", "/run", "tmpfs",
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV, "mode=0755"),
        ("tmpfs", "/tmp", "tmpfs",
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV, "mode=1777"),
        ("devpts", "/dev/pts", "devpts",
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC, "mode=0620,gid=5"),
        ("tmpfs", "/dev/shm", "tmpfs",
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV, "mode=1777"),
    ];
    for (src, dst, fstype, flags, data) in volatile {
        ensure_mount(src, dst, fstype, flags, Some(data));
    }
    let _ = std::fs::create_dir_all("/run/lock");
    // XDG_RUNTIME_DIR del compositor gráfico (mirada-compositor --drm pone su
    // socket Wayland acá vía `bind_auto`, que NO crea el directorio). Las seeds
    // con DM (arje-host / arje-tawasuyu*) declaran XDG_RUNTIME_DIR=/run/arje; sin
    // este mkdir el compositor falla al abrir el socket. El compositor corre como
    // root, así que 0700 root alcanza.
    if let Ok(()) = std::fs::create_dir_all("/run/arje") {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions("/run/arje", std::fs::Permissions::from_mode(0o700));
    }

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

#[cfg(test)]
mod tests {
    use super::is_mountpoint;

    #[test]
    fn root_es_punto_de_montaje() {
        // `/` siempre es un montaje (aunque su padre sea él mismo, el caso
        // sin padre devuelve false; pero `/proc` y `/sys` deberían serlo en
        // cualquier host Linux donde corren los tests).
        assert!(is_mountpoint("/proc"), "/proc debería estar montado en el host de tests");
    }

    #[test]
    fn directorio_normal_no_es_punto_de_montaje() {
        // Un subdirectorio cualquiera de `/` comparte st_dev con `/` → no es
        // mountpoint. Usamos uno que existe seguro y rara vez es un montaje.
        assert!(!is_mountpoint("/etc"), "/etc no suele ser un punto de montaje");
    }

    #[test]
    fn target_inexistente_no_es_punto_de_montaje() {
        assert!(!is_mountpoint("/no/existe/este/path/arje"));
    }
}
