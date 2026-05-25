//! Absorbe **dinit**: parsea los archivos de `/etc/dinit.d`.

use std::fs;
use std::path::Path;

use crate::model::{split_command, ForeignService, ServiceKind};

/// Descubre los servicios dinit de `<root>/etc/dinit.d`.
///
/// Cada archivo es un servicio con líneas `clave = valor`. Absorbemos
/// los de `type` = `process`/`bgprocess` (daemon) y `scripted`
/// (one-shot) que declaren un `command`. `internal`/`triggered` no
/// encarnan un proceso y se omiten — entre ellos el servicio `boot`.
pub fn absorb(root: &Path) -> anyhow::Result<Vec<ForeignService>> {
    let dir = root.join("etc/dinit.d");
    anyhow::ensure!(
        dir.is_dir(),
        "no encontré /etc/dinit.d en {}",
        root.display()
    );
    let mut names: Vec<String> = fs::read_dir(&dir)?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();

    let mut out = Vec::new();
    for name in names {
        let text = fs::read_to_string(dir.join(&name))?;
        if let Some(svc) = parse_service(&name, &text) {
            out.push(svc);
        }
    }
    Ok(out)
}

/// Parsea un archivo de servicio dinit. `None` si no encarna un proceso.
fn parse_service(name: &str, text: &str) -> Option<ForeignService> {
    let mut ty = String::new();
    let mut command = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "type" => ty = val.trim().to_string(),
            "command" => command = val.trim().to_string(),
            _ => {}
        }
    }
    let kind = match ty.as_str() {
        "process" | "bgprocess" => ServiceKind::Daemon,
        "scripted" => ServiceKind::OneShot,
        _ => return None, // internal, triggered, o sin `type`
    };
    let (exec, argv) = split_command(&command)?;
    Some(ForeignService {
        name: format!("dinit-{name}"),
        exec,
        argv,
        env: Vec::new(),
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absorbs_process_and_scripted_skips_internal() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("etc/dinit.d");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("boot"), "type = internal\nwaits-for = sshd\n").unwrap();
        fs::write(
            d.join("sshd"),
            "# servicio ssh\ntype = process\ncommand = /usr/sbin/sshd -D\nrestart = true\n",
        )
        .unwrap();
        fs::write(d.join("fsck"), "type = scripted\ncommand = /sbin/fsck -A\n").unwrap();

        let svcs = absorb(tmp.path()).unwrap();
        // sshd + fsck — boot (internal) se omite.
        assert_eq!(svcs.len(), 2);
        let sshd = svcs.iter().find(|s| s.name == "dinit-sshd").unwrap();
        assert_eq!(sshd.kind, ServiceKind::Daemon);
        assert_eq!(sshd.exec, "/usr/sbin/sshd");
        assert_eq!(sshd.argv, ["-D"]);
        let fsck = svcs.iter().find(|s| s.name == "dinit-fsck").unwrap();
        assert_eq!(fsck.kind, ServiceKind::OneShot);
    }

    #[test]
    fn errors_without_dinit_d() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(absorb(tmp.path()).is_err());
    }
}
