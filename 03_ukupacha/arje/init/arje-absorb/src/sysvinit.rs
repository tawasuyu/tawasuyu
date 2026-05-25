//! Absorbe **sysvinit**: parsea `/etc/inittab`.

use std::fs;
use std::path::Path;

use anyhow::Context;

use crate::model::{split_command, ForeignService, ServiceKind};

/// Lee `<root>/etc/inittab` y devuelve sus servicios.
///
/// Formato de cada línea: `id:runlevels:action:process`. Tomamos las
/// que tienen un `process` real: `respawn` → daemon supervisado;
/// `wait`/`once`/`boot`/`bootwait`/`sysinit` → one-shot. El resto de
/// acciones (`initdefault`, `ctrlaltdel`, `power*`, `off`, …) no lanzan
/// un servicio y se ignoran.
pub fn absorb(root: &Path) -> anyhow::Result<Vec<ForeignService>> {
    let path = root.join("etc/inittab");
    let text =
        fs::read_to_string(&path).with_context(|| format!("leyendo {}", path.display()))?;
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut f = line.splitn(4, ':');
        let (id, action, process) = match (f.next(), f.next(), f.next(), f.next()) {
            (Some(id), Some(_rl), Some(action), Some(process)) => (id, action, process),
            _ => continue, // línea malformada
        };
        let kind = match action.trim() {
            "respawn" => ServiceKind::Daemon,
            "wait" | "once" | "boot" | "bootwait" | "sysinit" => ServiceKind::OneShot,
            _ => continue,
        };
        // El proceso puede empezar con `+` (sysvinit: no escribir utmp).
        let process = process.trim().trim_start_matches('+').trim();
        let Some((exec, argv)) = split_command(process) else {
            continue;
        };
        out.push(ForeignService {
            name: format!("sysv-{}", id.trim()),
            exec,
            argv,
            env: Vec::new(),
            kind,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_inittab(content: &str) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("etc")).unwrap();
        fs::write(tmp.path().join("etc/inittab"), content).unwrap();
        tmp
    }

    #[test]
    fn parses_respawn_and_oneshot() {
        let tmp = with_inittab(
            "# consolas del sistema\n\
             id:3:initdefault:\n\
             si::sysinit:/etc/rc.d/rc.sysinit\n\
             1:2345:respawn:/sbin/agetty 38400 tty1 linux\n\
             rc::wait:/etc/rc.d/rc 3\n",
        );
        let svcs = absorb(tmp.path()).unwrap();
        // sysinit + respawn + wait — initdefault no cuenta.
        assert_eq!(svcs.len(), 3);
        let agetty = svcs.iter().find(|s| s.name == "sysv-1").unwrap();
        assert_eq!(agetty.kind, ServiceKind::Daemon);
        assert_eq!(agetty.exec, "/sbin/agetty");
        assert_eq!(agetty.argv, ["38400", "tty1", "linux"]);
        let si = svcs.iter().find(|s| s.name == "sysv-si").unwrap();
        assert_eq!(si.kind, ServiceKind::OneShot);
    }

    #[test]
    fn skips_comments_and_blank_lines() {
        let tmp = with_inittab("\n   \n# sólo comentarios\n");
        assert!(absorb(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn strips_leading_plus() {
        let tmp = with_inittab("x:2:respawn:+/sbin/getty tty2\n");
        let svcs = absorb(tmp.path()).unwrap();
        assert_eq!(svcs[0].exec, "/sbin/getty");
    }
}
