//! Absorbe **OpenRC**: descubre los servicios habilitados por runlevel.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::model::{ForeignService, ServiceKind};

/// Descubre los servicios OpenRC habilitados en `<root>`.
///
/// OpenRC habilita un servicio con un symlink en `/etc/runlevels/<rl>/`.
/// Los scripts de `/etc/init.d/` son shell completo —no se parsean—; se
/// absorben como tarea one-shot `/etc/init.d/<svc> start`, que arranca
/// el daemon (OpenRC mismo lo lleva a segundo plano). Recorremos los
/// runlevels `sysinit` → `boot` → `default` y deduplicamos.
pub fn absorb(root: &Path) -> anyhow::Result<Vec<ForeignService>> {
    let runlevels = ["sysinit", "boot", "default"];
    let mut found_any = false;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for rl in runlevels {
        let dir = root.join("etc/runlevels").join(rl);
        if !dir.is_dir() {
            continue;
        }
        found_any = true;
        let mut names: Vec<String> = fs::read_dir(&dir)?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        for svc in names {
            if !seen.insert(svc.clone()) {
                continue; // ya absorbido en un runlevel anterior
            }
            out.push(ForeignService {
                name: format!("openrc-{svc}"),
                exec: format!("/etc/init.d/{svc}"),
                argv: vec!["start".to_string()],
                env: Vec::new(),
                kind: ServiceKind::OneShot,
            });
        }
    }
    anyhow::ensure!(
        found_any,
        "no encontré /etc/runlevels/* en {}",
        root.display()
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absorbs_enabled_services_deduped() {
        let tmp = tempfile::tempdir().unwrap();
        for (rl, svcs) in [
            ("boot", ["bootmisc", "hostname"].as_slice()),
            ("default", ["sshd", "hostname"].as_slice()),
        ] {
            let d = tmp.path().join("etc/runlevels").join(rl);
            fs::create_dir_all(&d).unwrap();
            for s in svcs {
                // Un archivo simple basta: sólo leemos los nombres.
                fs::write(d.join(s), "").unwrap();
            }
        }
        let out = absorb(tmp.path()).unwrap();
        // bootmisc, hostname, sshd — `hostname` deduplicado entre runlevels.
        assert_eq!(out.len(), 3);
        let sshd = out.iter().find(|s| s.name == "openrc-sshd").unwrap();
        assert_eq!(sshd.exec, "/etc/init.d/sshd");
        assert_eq!(sshd.argv, ["start"]);
        assert_eq!(sshd.kind, ServiceKind::OneShot);
    }

    #[test]
    fn errors_without_runlevels() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(absorb(tmp.path()).is_err());
    }
}
