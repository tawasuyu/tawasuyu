//! Absorbe **runit**: descubre los servicios bajo supervisión.

use std::fs;
use std::path::Path;

use crate::model::{ForeignService, ServiceKind};

/// Descubre los servicios runit de `<root>`.
///
/// runit supervisa un directorio de servicios activos (el `runsvdir`
/// del runlevel). Cada entrada apunta a `/etc/sv/<nombre>`, cuyo script
/// `run` es un daemon en primer plano — el calce con arje es 1:1: arje
/// supervisa ese mismo `run`. Si no hay `runsvdir`, cae a `/etc/sv`
/// (todos los servicios definidos).
pub fn absorb(root: &Path) -> anyhow::Result<Vec<ForeignService>> {
    let runsvdir = [
        "etc/runit/runsvdir/default",
        "etc/runit/runsvdir/current",
        "service",
        "var/service",
        "etc/service",
    ]
    .into_iter()
    .map(|c| root.join(c))
    .find(|p| p.is_dir());

    let scan = match runsvdir {
        Some(d) => d,
        None => {
            let sv = root.join("etc/sv");
            anyhow::ensure!(
                sv.is_dir(),
                "no encontré servicios runit en {}",
                root.display()
            );
            sv
        }
    };

    let mut names: Vec<String> = fs::read_dir(&scan)?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();

    let mut out = Vec::new();
    for name in names {
        // El servicio vive en /etc/sv/<name>; su run-script es el exec.
        let run = root.join("etc/sv").join(&name).join("run");
        if !run.exists() {
            continue; // entrada sin run-script real — la saltamos
        }
        out.push(ForeignService {
            name: format!("runit-{name}"),
            exec: format!("/etc/sv/{name}/run"),
            argv: Vec::new(),
            env: Vec::new(),
            kind: ServiceKind::Daemon, // runit siempre supervisa
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sv(root: &Path, name: &str) {
        let d = root.join("etc/sv").join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("run"), "#!/bin/sh\nexec daemon\n").unwrap();
    }

    #[test]
    fn absorbs_from_etc_sv_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        make_sv(tmp.path(), "sshd");
        make_sv(tmp.path(), "dhcpcd");
        let svcs = absorb(tmp.path()).unwrap();
        assert_eq!(svcs.len(), 2);
        // Salida ordenada por nombre.
        assert_eq!(svcs[0].name, "runit-dhcpcd");
        assert_eq!(svcs[0].exec, "/etc/sv/dhcpcd/run");
        assert_eq!(svcs[0].kind, ServiceKind::Daemon);
    }

    #[test]
    fn absorbs_only_enabled_from_runsvdir() {
        let tmp = tempfile::tempdir().unwrap();
        for s in ["sshd", "dhcpcd", "apagado"] {
            make_sv(tmp.path(), s);
        }
        let rsv = tmp.path().join("etc/runit/runsvdir/default");
        fs::create_dir_all(&rsv).unwrap();
        // Sólo sshd y dhcpcd están habilitados (symlink en el runsvdir).
        for s in ["sshd", "dhcpcd"] {
            std::os::unix::fs::symlink(tmp.path().join("etc/sv").join(s), rsv.join(s))
                .unwrap();
        }
        let svcs = absorb(tmp.path()).unwrap();
        assert_eq!(svcs.len(), 2);
        assert!(svcs.iter().all(|s| s.name != "runit-apagado"));
    }

    #[test]
    fn errors_without_services() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(absorb(tmp.path()).is_err());
    }
}
