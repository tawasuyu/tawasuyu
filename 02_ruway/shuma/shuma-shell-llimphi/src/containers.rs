//! Helpers de contenedores, rootfs y detección de engines.
//!
//! Funciones de spawn para listar, crear, iniciar, parar y borrar
//! contenedores (locales y remotos), más la gestión de rootfs LXC
//! para unshare/bwrap.
//!
//! Las funciones de detección de engines (`binary_disponible`,
//! `engine_preferido`, etc.) viven en `env.rs`; este módulo las re-exporta
//! para comodidad de los llamadores que sólo hacen `use super::containers::*`.

pub(crate) use crate::env::{
    binary_disponible, bwrap_disponible, engine_preferido, podman_disponible,
    unshare_disponible,
};

use crate::types::{ContainerInfo, Distro, ExplorerEntry, Msg};
use llimphi_ui::Handle;
use shuma_module::Source;

// ─── Rootfs (unshare / bwrap) ───────────────────────────────────────

/// Path donde shuma extrae rootfs LXC para usar con bwrap/unshare.
pub(crate) fn rootfs_root() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|b| b.data_local_dir().join("shuma").join("rootfs"))
}

/// Path donde la `distro` tiene su rootfs extraído.
pub(crate) fn rootfs_path_for(distro: Distro) -> Option<std::path::PathBuf> {
    rootfs_root().map(|r| r.join(distro.label().to_lowercase()))
}

/// `true` si el rootfs de esa distro ya está extraído.
pub(crate) fn rootfs_listo(distro: Distro) -> bool {
    let Some(root) = rootfs_path_for(distro) else {
        return false;
    };
    root.join("bin/bash").exists() || root.join("usr/bin/bash").exists()
}

/// Prepara un rootfs para que los gestores de paquetes funcionen en
/// un userns de un solo uid. Idempotente y best-effort.
pub(crate) fn prepare_rootfs(root: &std::path::Path) {
    // apt (Debian/Ubuntu): drop-in que desactiva el sandbox de descarga.
    let apt_dir = root.join("etc/apt/apt.conf.d");
    if apt_dir.is_dir() {
        let f = apt_dir.join("99shuma-nosandbox");
        if !f.exists() {
            let _ = std::fs::write(&f, "APT::Sandbox::User \"root\";\n");
        }
    }
    // pacman (Arch): comentar `DownloadUser` para que descargue como root.
    let pac = root.join("etc/pacman.conf");
    if let Ok(txt) = std::fs::read_to_string(&pac) {
        let activa = |l: &str| {
            let t = l.trim_start();
            !t.starts_with('#') && t.starts_with("DownloadUser")
        };
        if txt.lines().any(activa) {
            let nuevo: String = txt
                .lines()
                .map(|l| {
                    if activa(l) {
                        format!("#{l}  # shuma: descarga como root (userns de 1 uid)")
                    } else {
                        l.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            let _ = std::fs::write(&pac, format!("{nuevo}\n"));
        }
    }
}

// ─── LXC image ─────────────────────────────────────────────────────

/// Triple `(distro_slug, release, arch)` para construir la URL del LXC image.
fn lxc_image_triple(distro: Distro) -> (&'static str, &'static str, &'static str) {
    match distro {
        Distro::Ubuntu => ("ubuntu", "noble", "amd64"),
        Distro::Debian => ("debian", "bookworm", "amd64"),
        Distro::Alpine => ("alpine", "3.22", "amd64"),
        Distro::Arch => ("archlinux", "current", "amd64"),
    }
}

/// Quote estilo Bourne para args a `bash -c '...'`.
pub(crate) fn shell_quote_arg(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Descarga + extrae el rootfs LXC para `distro`. Al terminar, dispatcha
/// `ContainerCreated(name)` o `ContainerFailed{reason}`.
pub(crate) fn spawn_pull_rootfs_lxc(handle: &Handle<Msg>, distro: Distro, mount: Option<String>) {
    let _ = mount;
    let (d, rel, arch) = lxc_image_triple(distro);
    let Some(root) = rootfs_path_for(distro) else {
        let name = format!("rootfs:{}", distro.label().to_lowercase());
        handle.spawn(move || Msg::ContainerFailed {
            name,
            reason: "no se pudo resolver $XDG_DATA_HOME".into(),
        });
        return;
    };
    let root_str = root.display().to_string();
    let name_for_msg = root_str.clone();
    handle.spawn(move || {
        if let Err(e) = std::fs::create_dir_all(&root) {
            return Msg::ContainerFailed {
                name: name_for_msg,
                reason: format!("mkdir {}: {e}", root.display()),
            };
        }
        let base = format!(
            "https://images.linuxcontainers.org/images/{d}/{rel}/{arch}/default"
        );
        let cmd = format!(
            "set -o pipefail; \
             dir=$(curl -fsSL {base}/ | grep -oE '[0-9]{{8}}_[0-9]{{2}}%3A[0-9]{{2}}/' | sort | tail -1); \
             test -n \"$dir\" || {{ echo 'no encontré builds en el índice LXC' >&2; exit 1; }}; \
             curl -L -fsSL {base}/\"$dir\"rootfs.tar.xz | tar -xJ -C {root}",
            base = shell_quote_arg(&base),
            root = shell_quote_arg(&root.display().to_string()),
        );
        match std::process::Command::new("bash")
            .args(["-c", &cmd])
            .output()
        {
            Ok(out) if out.status.success() => Msg::ContainerCreated(name_for_msg),
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .last()
                    .unwrap_or("curl|tar salió con status no-cero")
                    .to_string();
                Msg::ContainerFailed { name: name_for_msg, reason: err }
            }
            Err(e) => Msg::ContainerFailed {
                name: name_for_msg,
                reason: format!("no pude ejecutar bash: {e}"),
            },
        }
    });
}

// ─── Spawn: containers locales ──────────────────────────────────────

/// Lista los contenedores locales (`podman ps -a`) y entrega los nombres
/// por `Msg::ContainersLoaded`.
pub(crate) fn spawn_list_containers(handle: &Handle<Msg>) {
    handle.spawn(|| {
        let names = std::process::Command::new("podman")
            .args(["ps", "-a", "--format", "{{.Names}}"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Msg::ContainersLoaded(names)
    });
}

/// Lista containers locales con su status + image (ventana gestora).
pub(crate) fn spawn_list_containers_full(handle: &Handle<Msg>) {
    handle.spawn(|| {
        let mut infos: Vec<ContainerInfo> = Vec::new();
        // 1. Rootfs en disco (unshare/bwrap) — la lista PERSISTENTE.
        if let Some(root) = rootfs_root() {
            if let Ok(rd) = std::fs::read_dir(&root) {
                let mut dirs: Vec<_> = rd
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect();
                dirs.sort();
                for name in dirs {
                    let p = root.join(&name);
                    let listo = p.join("bin/bash").exists() || p.join("usr/bin/bash").exists();
                    infos.push(ContainerInfo {
                        name,
                        status: if listo { "listo".into() } else { "incompleto".into() },
                        image: "rootfs · unshare/bwrap".into(),
                        rootfs: true,
                    });
                }
            }
        }
        // 2. Containers podman/docker.
        let podman = std::process::Command::new("podman")
            .args(["ps", "-a", "--format", "{{.Names}}\t{{.Status}}\t{{.Image}}"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| {
                        let mut it = l.splitn(3, '\t');
                        let name = it.next()?.trim().to_string();
                        let status = it.next().unwrap_or("").trim().to_string();
                        let image = it.next().unwrap_or("").trim().to_string();
                        if name.is_empty() {
                            None
                        } else {
                            Some(ContainerInfo { name, status, image, rootfs: false })
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        infos.extend(podman);
        Msg::ContainersFullLoaded(infos)
    });
}

/// Borra un rootfs en disco en bg y refresca la lista.
pub(crate) fn spawn_remove_rootfs(handle: &Handle<Msg>, name: String) {
    handle.spawn(move || {
        if let Some(root) = rootfs_root() {
            let p = root.join(&name);
            if p.starts_with(&root) && p.is_dir() {
                let _ = std::fs::remove_dir_all(&p);
            }
        }
        Msg::RefreshContainersFull
    });
}

/// Dispara `podman <action> <name>` en bg; al terminar, refresca la lista.
pub(crate) fn spawn_container_action(handle: &Handle<Msg>, action: &'static str, name: String) {
    handle.spawn(move || {
        let mut args: Vec<String> = if action == "rm" {
            vec!["rm".into(), "-f".into()]
        } else {
            vec![action.into()]
        };
        args.push(name.clone());
        let _ = std::process::Command::new("podman").args(&args).output();
        Msg::RefreshContainersFull
    });
}

/// Se asegura de que el container `name` esté corriendo.
pub(crate) fn spawn_ensure_container(handle: &Handle<Msg>, name: String) {
    handle.spawn(move || {
        match std::process::Command::new("podman")
            .args(["start", &name])
            .output()
        {
            Ok(out) if out.status.success() => Msg::ContainerCreated(name),
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .next()
                    .unwrap_or("podman start salió con status no-cero")
                    .to_string();
                Msg::ContainerFailed { name, reason: err }
            }
            Err(e) => Msg::ContainerFailed {
                name,
                reason: format!("no pude ejecutar podman: {e}"),
            },
        }
    });
}

/// Crea un contenedor `name` de la `image` dada (detached, `sleep infinity`).
pub(crate) fn spawn_create_container(
    handle: &Handle<Msg>,
    image: &'static str,
    name: String,
    mount: Option<String>,
) {
    handle.spawn(move || {
        let mut args: Vec<String> = vec![
            "run".into(),
            "-d".into(),
            "--name".into(),
            name.clone(),
        ];
        if let Some(m) = mount.as_ref().map(|m| m.trim()).filter(|m| !m.is_empty()) {
            args.push("-v".into());
            args.push(format!("{m}:/work"));
            args.push("-w".into());
            args.push("/work".into());
        }
        args.push(image.into());
        args.push("sleep".into());
        args.push("infinity".into());
        match std::process::Command::new("podman").args(&args).output() {
            Ok(out) if out.status.success() => Msg::ContainerCreated(name),
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .next()
                    .unwrap_or("podman run salió con status no-cero")
                    .to_string();
                Msg::ContainerFailed { name, reason: err }
            }
            Err(e) => Msg::ContainerFailed {
                name,
                reason: format!("no pude ejecutar podman: {e}"),
            },
        }
    });
}

// ─── Spawn: containers remotos ──────────────────────────────────────

/// Lista los contenedores de un host remoto vía `ssh`.
pub(crate) fn spawn_list_remote_containers(
    handle: &Handle<Msg>,
    host: String,
    user: String,
    port: u16,
    engine: String,
) {
    handle.spawn(move || {
        let eng = if matches!(engine.as_str(), "podman" | "docker") {
            engine.as_str()
        } else {
            "podman"
        };
        let target = format!("{user}@{host}");
        let names = std::process::Command::new("ssh")
            .args([
                "-p",
                &port.to_string(),
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=8",
                &target,
                "--",
                eng,
                "ps",
                "-a",
                "--format",
                "{{.Names}}",
            ])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Msg::RemoteContainersLoaded(names)
    });
}

/// Corre `<engine> <action> <name>` en el host remoto por `ssh`.
pub(crate) fn spawn_remote_engine_action(
    handle: &Handle<Msg>,
    host: String,
    user: String,
    port: u16,
    engine: String,
    action: &'static str,
    name: String,
) {
    if !matches!(engine.as_str(), "podman" | "docker") {
        return;
    }
    handle.spawn(move || {
        let target = format!("{user}@{host}");
        let mut args: Vec<String> = vec![
            "-p".into(),
            port.to_string(),
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            "ConnectTimeout=8".into(),
            target,
            "--".into(),
            engine,
            action.into(),
        ];
        if action == "rm" {
            args.push("-f".into());
        }
        args.push(name);
        let _ = std::process::Command::new("ssh").args(&args).output();
        Msg::RefreshRemoteContainers
    });
}

/// Crea un contenedor en el host remoto.
pub(crate) fn spawn_create_remote_container(
    handle: &Handle<Msg>,
    host: String,
    user: String,
    port: u16,
    engine: String,
    image: &'static str,
    name: String,
) {
    if !matches!(engine.as_str(), "podman" | "docker") {
        return;
    }
    handle.spawn(move || {
        let target = format!("{user}@{host}");
        let _ = std::process::Command::new("ssh")
            .args([
                "-p",
                &port.to_string(),
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=8",
                &target,
                "--",
                &engine,
                "run",
                "-d",
                "--name",
                &name,
                image,
                "sleep",
                "infinity",
            ])
            .output();
        Msg::RefreshRemoteContainers
    });
}

// ─── Spawn: listado del Explorer (cwd remoto por SSH) ───────────────

/// Lista el `cwd` de una sesión remota (Remote / RemoteContainer) por `ssh`
/// (BatchMode, igual que el gestor de contenedores remotos) y entrega el
/// resultado por `Msg::ExplorerLoaded`. El trabajo de red va off-thread:
/// `read_dir` local no alcanza al filesystem del host remoto.
pub(crate) fn spawn_explorer_list(
    handle: &Handle<Msg>,
    session: usize,
    source: Source,
    cwd: String,
) {
    handle.spawn(move || {
        let result = explorer_list_blocking(&source, &cwd);
        Msg::ExplorerLoaded { session, path: cwd, result }
    });
}

/// Construye el comando `ls`, lo manda por SSH al host de `source` y parsea
/// la salida. Sólo Remote / RemoteContainer; otras fuentes son un bug del
/// llamador (el panel local usa `read_dir`).
fn explorer_list_blocking(source: &Source, cwd: &str) -> Result<Vec<ExplorerEntry>, String> {
    let (host, user, port, remote_cmd) = match source {
        Source::Remote { host, user, port, .. } => {
            // `ls -1Ap`: una entrada por línea, incluye ocultos (sin ./..),
            // y sufija `/` a los directorios — así sabemos el tipo sin `stat`.
            let cmd = if cwd.starts_with('/') {
                format!("ls -1Ap -- {}", shell_quote_arg(cwd))
            } else {
                "ls -1Ap".to_string() // cwd "~"/relativo → home de la sesión SSH
            };
            (host.as_str(), user.as_str(), *port, cmd)
        }
        Source::RemoteContainer { host, user, port, engine, name, .. } => {
            (host.as_str(), user.as_str(), *port, remote_container_ls_cmd(engine, name, cwd))
        }
        _ => return Err("la sesión no es remota".into()),
    };
    let out = ssh_capture(host, user, port, &remote_cmd)?;
    Ok(parse_ls_output(&out))
}

/// Comando que, ejecutado **en el host remoto**, lista el cwd interior de un
/// contenedor de ese host. Espejo mínimo de `remote_container_command` del
/// shell: `<engine> exec` para podman/docker, `chroot` para rootfs.
fn remote_container_ls_cmd(engine: &str, name: &str, cwd: &str) -> String {
    let inner = if cwd.starts_with('/') {
        format!("cd {} 2>/dev/null; ls -1Ap", shell_quote_arg(cwd))
    } else {
        "ls -1Ap".to_string()
    };
    match engine {
        "unshare" | "bwrap" => format!(
            "chroot {} /bin/sh -lc {}",
            shell_quote_arg(name),
            shell_quote_arg(&inner)
        ),
        eng => format!(
            "{eng} exec -i {} /bin/sh -lc {}",
            shell_quote_arg(name),
            shell_quote_arg(&inner)
        ),
    }
}

/// Corre `remote_cmd` en `user@host:port` por `ssh` y devuelve su stdout, o
/// la primera línea de stderr como error. BatchMode evita prompts colgados.
fn ssh_capture(host: &str, user: &str, port: u16, remote_cmd: &str) -> Result<String, String> {
    let target = format!("{user}@{host}");
    let out = std::process::Command::new("ssh")
        .args([
            "-p",
            &port.to_string(),
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=8",
            &target,
            remote_cmd,
        ])
        .output()
        .map_err(|e| format!("no pude ejecutar ssh: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("ssh falló")
            .to_string())
    }
}

/// Parsea la salida de `ls -1Ap`: directorios sufijados con `/`, ordenados
/// con dirs primero y acotados a 200 (igual que el panel local).
fn parse_ls_output(out: &str) -> Vec<ExplorerEntry> {
    let mut entradas: Vec<ExplorerEntry> = out
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.is_empty())
        .map(|l| match l.strip_suffix('/') {
            Some(n) => ExplorerEntry { is_dir: true, name: n.to_string() },
            None => ExplorerEntry { is_dir: false, name: l.to_string() },
        })
        .collect();
    entradas.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    entradas.truncate(200);
    entradas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ls_separates_dirs_and_sorts() {
        // `ls -1Ap` sufija `/` a los dirs; el resto son archivos. Incluye un
        // `\r` colgado y una línea vacía para validar el trim/filtro.
        let out = "zeta.txt\nsrc/\n.config\nbin/\nalpha.md\r\n\n";
        let e = parse_ls_output(out);
        // Dirs primero (alfabético), luego archivos (alfabético, ocultos incl.).
        let got: Vec<(bool, &str)> = e.iter().map(|x| (x.is_dir, x.name.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (true, "bin"),
                (true, "src"),
                (false, ".config"),
                (false, "alpha.md"),
                (false, "zeta.txt"),
            ]
        );
    }

    #[test]
    fn remote_container_ls_uses_engine_exec_or_chroot() {
        // podman/docker → `<engine> exec`. El cwd interior va dentro del `-lc`
        // (las comillas internas quedan escapadas por el quoting anidado, así
        // que verificamos el contenido lógico, no la forma exacta del escape).
        let podman = remote_container_ls_cmd("podman", "caja", "/work");
        assert!(podman.starts_with("podman exec -i 'caja' /bin/sh -lc"));
        assert!(podman.contains("/work") && podman.contains("ls -1Ap"));
        // rootfs (unshare/bwrap) → chroot al path.
        let rootfs = remote_container_ls_cmd("bwrap", "/home/u/.local/share/shuma/rootfs/arch", "~");
        assert!(rootfs.starts_with("chroot '/home/u/.local/share/shuma/rootfs/arch' /bin/sh -lc"));
        // cwd "~" (no absoluto) → sin `cd`, lista el home del contenedor.
        assert!(!rootfs.contains("cd "));
    }
}
