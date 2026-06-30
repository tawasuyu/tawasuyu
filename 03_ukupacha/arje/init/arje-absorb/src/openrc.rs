//! Absorbe **OpenRC**: descubre los servicios habilitados por runlevel.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::model::{ForeignService, ServiceKind};

/// Quita comillas envolventes de un valor `clave="valor"` de shell.
fn unquote(s: &str) -> String {
    s.trim().trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// Saca de un runscript OpenRC el daemon REAL (`command` / `command_args` /
/// `command_background`). La mayoría de los servicios modernos lo declaran así
/// (OpenRC los corre con start-stop-daemon). Devuelve `Some((exec, argv))` SÓLO
/// si el servicio corre en primer plano (`command_background=yes`) — ahí arje lo
/// supervisa directo, sin OpenRC ni el wrapper frágil. Si el script usa una
/// función `start()` propia, o el binario se auto-demoniza, o hay expansión de
/// variables, devuelve `None` y se cae al wrapper.
fn parse_real_daemon(script: &Path) -> Option<(String, Vec<String>)> {
    let text = fs::read_to_string(script).ok()?;
    let (mut command, mut args, mut background) = (None, Vec::new(), false);
    for line in text.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("command=") {
            let v = unquote(v);
            // Sólo paths literales (sin $VAR ni `cmd`): lo demás no es seguro.
            if v.starts_with('/') && !v.contains('$') && !v.contains('`') {
                command = Some(v);
            }
        } else if let Some(v) = l.strip_prefix("command_args=") {
            let v = unquote(v);
            if !v.contains('$') && !v.contains('`') {
                args = v.split_whitespace().map(str::to_string).collect();
            }
        } else if let Some(v) = l.strip_prefix("command_background=") {
            background = matches!(unquote(v).to_lowercase().as_str(), "yes" | "true" | "1");
        }
    }
    match command {
        Some(cmd) if background => Some((cmd, args)),
        _ => None,
    }
}

/// Descubre los servicios OpenRC habilitados en `<root>`.
///
/// OpenRC habilita un servicio con un symlink en `/etc/runlevels/<rl>/`. De cada
/// runscript intentamos sacar el **daemon real** ([`parse_real_daemon`]) para
/// supervisarlo directo bajo arje; si no se puede, caemos al wrapper one-shot
/// `/etc/init.d/<svc> start`. Recorremos `sysinit` → `boot` → `default` y
/// deduplicamos. Lo DESACTIVADO (sin symlink en un runlevel) no se absorbe.
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
            let script = root.join("etc/init.d").join(&svc);
            let service = match parse_real_daemon(&script) {
                // Daemon real (primer plano) → arje lo supervisa directo.
                Some((exec, argv)) => ForeignService {
                    name: format!("openrc-{svc}"),
                    exec,
                    argv,
                    env: Vec::new(),
                    kind: ServiceKind::Daemon,
                },
                // No parseable → wrapper one-shot (mejor que nada; frágil).
                None => ForeignService {
                    name: format!("openrc-{svc}"),
                    exec: format!("/etc/init.d/{svc}"),
                    argv: vec!["start".to_string()],
                    env: Vec::new(),
                    kind: ServiceKind::OneShot,
                },
            };
            out.push(service);
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

    fn habilita(tmp: &Path, svc: &str, script: &str) {
        let rl = tmp.join("etc/runlevels/default");
        fs::create_dir_all(&rl).unwrap();
        fs::write(rl.join(svc), "").unwrap();
        let initd = tmp.join("etc/init.d");
        fs::create_dir_all(&initd).unwrap();
        fs::write(initd.join(svc), script).unwrap();
    }

    #[test]
    fn saca_el_daemon_real_si_command_background() {
        // Un Caddy cualquiera (command_background=yes) → daemon directo, sin el
        // wrapper init.d. Demuestra que absorbe servicios obscuros sin conocerlos.
        let tmp = tempfile::tempdir().unwrap();
        habilita(
            tmp.path(),
            "caddy",
            "#!/sbin/openrc-run\ncommand=/usr/bin/caddy\ncommand_args=\"run --config /etc/caddy/Caddyfile\"\ncommand_background=yes\n",
        );
        let out = absorb(tmp.path()).unwrap();
        let c = out.iter().find(|s| s.name == "openrc-caddy").unwrap();
        assert_eq!(c.exec, "/usr/bin/caddy");
        assert_eq!(c.argv, ["run", "--config", "/etc/caddy/Caddyfile"]);
        assert_eq!(c.kind, ServiceKind::Daemon);
    }

    #[test]
    fn cae_al_wrapper_si_start_propio_o_se_autodemoniza() {
        // Sin command_background (se auto-demoniza) o con start() propio → wrapper.
        let tmp = tempfile::tempdir().unwrap();
        habilita(tmp.path(), "raro", "#!/sbin/openrc-run\nstart() {\n  /opt/raro &\n}\n");
        let out = absorb(tmp.path()).unwrap();
        let c = out.iter().find(|s| s.name == "openrc-raro").unwrap();
        assert_eq!(c.exec, "/etc/init.d/raro");
        assert_eq!(c.kind, ServiceKind::OneShot);
    }
}
