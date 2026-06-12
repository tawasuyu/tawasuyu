//! `shuma-shell-llimphi` — chasis del shell shuma sobre Llimphi.
//!
//! Shuma es la app standalone "normal" del workspace: una ventana con
//! tabs siempre visibles, monitores a la derecha, command-bar abajo. La
//! metáfora Quake-drawer (overlay sobre el escritorio + F12 para
//! invocar) vive en `mirada-launcher-llimphi`, no acá.
//!
//! **Layout** (sin `[main]` en shumarc):
//!
//! ```text
//!  ┌──────────────────────────────────────────────────┐
//!  │ TopBar · launcher (apps + shortcuts)             │
//!  ├────────────────────────────────┬─────────────────┤
//!  │ tabs: [shell] [lienzo] [matilda]│                 │
//!  ├────────────────────────────────┤ Monitores       │
//!  │                                │  CPU + MEM +    │
//!  │  contenido del tab activo      │  los del módulo │
//!  │                                │                 │
//!  ├────────────────────────────────┴─────────────────┤
//!  │ BottomBar · command-bar  › escribí…              │
//!  └──────────────────────────────────────────────────┘
//! ```
//!
//! Si el shumarc declara `[main]`, ese módulo ocupa toda el área central
//! a pantalla completa (sin tabs ni monitores) — útil para correr shuma
//! como wrapper de matilda standalone, por ejemplo.
//!
//! El chasis no conoce a sus módulos: el `Kind` estático enumera los
//! compilados. El shumarc elige cuáles activar y en qué slot.

#![forbid(unsafe_code)]

mod config;
mod hosts;

use std::time::Duration;

use llimphi_motion::{animate, motion, Tween};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, PathEl, Point, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{
    App, DragPhase, Handle, KeyEvent, KeyState, Modifiers, PaintRect, View, WheelDelta,
};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_stat_card::{stat_card_view, StatCardPalette};
use llimphi_widget_text_input::TextInputState;
use shuma_module::{ModuleContributions, MonitorSpec, ShortcutAction, Source};
use shuma_sysmon::{Snapshot, SystemSampler};
use std::collections::HashMap;

const HISTORY: usize = 60;
const TICK: Duration = Duration::from_secs(1);
/// Cadencia rápida para drenar el output del shell (streaming de
/// `shuma-exec`). 1 Hz se siente lento al ver `for i in …; do echo $i;
/// sleep 0.1; done`; 100 ms hace la salida sentirse en vivo sin
/// comerse CPU notable.
const SHELL_TICK: Duration = Duration::from_millis(100);
const MONITORS_INITIAL_WIDTH: f32 = 280.0;

/// Construye el cliente del rail hospedado si `SHUMA_DELEGATE_SIDEBAR` está
/// set. shuma publica sus tabs como dientes (cambian de tab al activarse) +
/// un diente "Monitores" que togglea el panel derecho. Cuando shuma tiene
/// foco, esos dientes aparecen en el rail global de pata; el área central
/// queda como puro lienzo (monitores ocultos por default). `app_id` debe ser
/// el mismo que reporta el compositor (`Shell::app_id`).
fn shuma_host(handle: &Handle<Msg>) -> Option<pata_host::HostClient> {
    if std::env::var_os("SHUMA_DELEGATE_SIDEBAR").is_none() {
        return None;
    }
    let teeth = host_tool_teeth();
    let h = handle.clone();
    pata_host::HostClient::connect("shuma.shell", "shuma", teeth, move |id| {
        h.dispatch(Msg::HostActivate(id))
    })
}

/// Dientes que shuma presta al rail de pata: uno por **herramienta** de la
/// sesión activa (id = índice en `Tool::ALL`).
fn host_tool_teeth() -> Vec<pata_host::HostedTooth> {
    Tool::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| pata_host::HostedTooth::new(i as u32, tool_icon_name(*t), t.label().to_string()))
        .collect()
}

/// Nombre de icono (vocabulario abierto de `pata`) para una herramienta.
fn tool_icon_name(t: Tool) -> &'static str {
    match t {
        Tool::History => "tools",
        Tool::Monitor => "system",
        Tool::Explorer => "files",
        Tool::Matilda => "settings",
    }
}

/// `Source` por defecto de la tab shell según las env vars del proceso —
/// para que `SHUMA_REMOTE*` enrute los comandos al daemon sin shumarc.
/// (rescate del `detect_remote_transport` del shell GPUI):
///
/// - `SHUMA_REMOTE_TCP_ADDR=host:port` + `SHUMA_REMOTE_TCP_PUB=<hex>`
///   → TCP autenticado Noise XK (`DaemonTcp`). La keypair propia la carga
///   `start_run` al conectar; acá sólo pasamos addr + pubkey del server.
/// - `SHUMA_REMOTE_SOCKET=/path` → daemon por ese Unix socket.
/// - `SHUMA_REMOTE=1` → daemon por el socket canónico (`socket: None`).
/// - sin ninguna → `Local` (ejecución directa).
fn default_shell_source() -> Source {
    let nonempty = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if let (Some(addr), Some(pub_hex)) = (
        nonempty("SHUMA_REMOTE_TCP_ADDR"),
        nonempty("SHUMA_REMOTE_TCP_PUB"),
    ) {
        return Source::DaemonTcp {
            addr,
            server_pub_hex: pub_hex,
            label: None,
        };
    }
    if let Some(path) = nonempty("SHUMA_REMOTE_SOCKET") {
        return Source::Daemon {
            socket: Some(std::path::PathBuf::from(path)),
            label: None,
        };
    }
    if std::env::var("SHUMA_REMOTE").as_deref() == Ok("1") {
        return Source::Daemon {
            socket: None,
            label: None,
        };
    }
    Source::Local
}

fn main() {
    rimay_localize::init();
    // Cablea `SUDO_ASKPASS` para que `sudo -A <cmd>` abra el popup Llimphi
    // en lugar de quedar colgado pidiendo pass en stdin del PTY. La env se
    // exporta en el proceso padre — los PTYs spawneados por shuma-exec la
    // heredan, igual que `TERM`.
    // Cablear el askpass para sudo + ssh + cualquier consumidor del
    // protocolo. El binario `shuma-askpass` lee la pass en una ventana
    // Llimphi y la imprime a stdout.
    if let Some(path) = resolve_askpass_path() {
        // En edition 2021 `set_var` es safe; corre antes de spawnear
        // ningún hilo. Los PTYs heredan la env vía shuma-exec.
        if std::env::var_os("SUDO_ASKPASS").is_none() {
            std::env::set_var("SUDO_ASKPASS", &path);
        }
        if std::env::var_os("SSH_ASKPASS").is_none() {
            std::env::set_var("SSH_ASKPASS", &path);
        }
        // OpenSSH 8.4+: `force` hace que ssh use SSH_ASKPASS aunque
        // haya tty (sin esto sólo lo usaría si DISPLAY está set y no
        // hay tty). En versiones viejas se ignora silenciosamente.
        if std::env::var_os("SSH_ASKPASS_REQUIRE").is_none() {
            std::env::set_var("SSH_ASKPASS_REQUIRE", "force");
        }
    }
    llimphi_ui::run::<Shell>();
}

/// `true` si el binario `podman` está disponible en `PATH`.
fn podman_disponible() -> bool {
    binary_disponible("podman")
}

/// `true` si el binario `bwrap` (bubblewrap) está disponible en `PATH`.
fn bwrap_disponible() -> bool {
    binary_disponible("bwrap")
}

/// `true` si `unshare` + `chroot` están en `PATH` (util-linux + coreutils).
/// Vienen instalados por default en cualquier Linux moderno.
fn unshare_disponible() -> bool {
    binary_disponible("unshare") && binary_disponible("chroot")
}

fn binary_disponible(name: &str) -> bool {
    let Some(path_env) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path_env) {
        if dir.join(name).exists() {
            return true;
        }
    }
    false
}

/// Engine preferido para containers de esta máquina, en orden:
/// 1. `unshare` — `util-linux` + `coreutils`, ya está en cualquier Linux
///    moderno con `unprivileged_userns_clone=1`. CERO instalación.
/// 2. `bwrap` — sin config, mejor aislamiento que unshare manual.
/// 3. `podman` — fallback OCI completo.
/// `None` si ninguno está; el form rebaja a Local con notice.
fn engine_preferido() -> Option<&'static str> {
    if unshare_disponible() {
        Some("unshare")
    } else if bwrap_disponible() {
        Some("bwrap")
    } else if podman_disponible() {
        Some("podman")
    } else {
        None
    }
}

/// Path donde shuma extrae rootfs LXC para usar con bwrap. Cada distro
/// queda en su subdirectorio. Persiste entre sesiones (sin re-descargar).
fn rootfs_root() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|b| b.data_local_dir().join("shuma").join("rootfs"))
}

/// Path donde la `distro` tiene su rootfs extraído. Usa el `label()` de
/// `Distro` en minúsculas como subdir.
fn rootfs_path_for(distro: Distro) -> Option<std::path::PathBuf> {
    rootfs_root().map(|r| r.join(distro.label().to_lowercase()))
}

/// `true` si el rootfs de esa distro ya está extraído (heurística: existe
/// `<rootfs>/bin/bash` o `<rootfs>/usr/bin/bash`).
fn rootfs_listo(distro: Distro) -> bool {
    let Some(root) = rootfs_path_for(distro) else {
        return false;
    };
    root.join("bin/bash").exists() || root.join("usr/bin/bash").exists()
}

/// Prepara un rootfs (unshare/bwrap) para que los gestores de paquetes
/// funcionen en un userns de UN SOLO uid. Ahí no se puede dropear privilegios
/// a `_apt` (apt) ni `alpm` (pacman) — ambos fallan con `seteuid`/`chown`. El
/// arreglo: que descarguen como root. Idempotente y best-effort (ignora
/// ausencia/permisos; el rootfs puede no traer ese gestor). Se edita el rootfs
/// en disco directamente, sin entrar al contenedor.
fn prepare_rootfs(root: &std::path::Path) {
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

/// Resuelve el path del binario `shuma-askpass`. Orden:
/// 1. `SHUMA_ASKPASS` override explícito.
/// 2. Hermano del binario actual (`current_exe().parent()/shuma-askpass`).
/// 3. Lookup en `$PATH` (`which`-like manual, solo nombre).
/// Devuelve `None` si no lo encuentra — `sudo -A` queda como antes.
fn resolve_askpass_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("SHUMA_ASKPASS") {
        let pb = std::path::PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("shuma-askpass");
            if sibling.exists() {
                return Some(sibling);
            }
        }
    }
    // Fallback: $PATH lookup mínimo — buscamos `shuma-askpass` en cada dir.
    if let Some(path_env) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_env) {
            let cand = dir.join("shuma-askpass");
            if cand.exists() {
                return Some(cand);
            }
        }
    }
    None
}

/// Lista los contenedores locales (`podman ps -a`) en un hilo y entrega los
/// nombres por `Msg::ContainersLoaded`. Vacío si podman no está o falla.
fn spawn_list_containers(handle: &Handle<Msg>) {
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

/// Triple `(distro_slug, release, arch)` para construir la URL del LXC
/// image. Ver: https://images.linuxcontainers.org/ — el directorio
/// `<distro>/<release>/<arch>/default/` NO tiene un alias `current/`; sólo
/// builds con timestamp (`20260608_07:42/`). El build concreto se resuelve
/// en runtime tomando el último del índice (`spawn_pull_rootfs_lxc`).
fn lxc_image_triple(distro: Distro) -> (&'static str, &'static str, &'static str) {
    match distro {
        Distro::Ubuntu => ("ubuntu", "noble", "amd64"),
        Distro::Debian => ("debian", "bookworm", "amd64"),
        // Alpine versiona por número; las releases viejas se borran del
        // mirror. Mantener en una vigente (hoy 3.22).
        Distro::Alpine => ("alpine", "3.22", "amd64"),
        Distro::Arch => ("archlinux", "current", "amd64"),
    }
}

/// Descarga + extrae el rootfs LXC para `distro` en `~/.local/share/shuma/
/// rootfs/<distro>`. Hace un único `curl | tar -xJ` sin escribir el .tar.xz
/// intermedio. Al terminar, dispatcha `ContainerCreated(name)` para que la
/// sesión use el rootfs, o `ContainerFailed{reason}` si algo salió mal.
/// `name` aquí es el path absoluto del rootfs (como hace falta para bwrap).
fn spawn_pull_rootfs_lxc(handle: &Handle<Msg>, distro: Distro, mount: Option<String>) {
    let _ = mount; // mount lo aplica el run-time vía bwrap_args + --bind
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
        // 1. Crear el directorio.
        if let Err(e) = std::fs::create_dir_all(&root) {
            return Msg::ContainerFailed {
                name: name_for_msg,
                reason: format!("mkdir {}: {e}", root.display()),
            };
        }
        // 2. Base del directorio de builds. No hay alias `current/`: hay que
        //    leer el índice y quedarse con el último timestamp (`20260608_07%3A42/`).
        let base = format!(
            "https://images.linuxcontainers.org/images/{d}/{rel}/{arch}/default"
        );
        // 3. Pipe en bash:
        //    a) curl del índice → grep del último dir con timestamp (sort|tail).
        //    b) curl del rootfs.tar.xz de ese build → tar -xJ -C <root>.
        //    El `%3A` (':' codificado) se preserva tal cual en la URL.
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

/// Quote estilo Bourne para args a `bash -c '...'`. Idéntico al de
/// shuma-module-shell pero replicado para evitar exportarlo.
fn shell_quote_arg(s: &str) -> String {
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

/// Lista containers locales con su status + image (ventana gestora).
/// Usa `podman ps -a --format` con un formato Go simple y parsea por
/// líneas para no depender de `jq` ni del JSON output de podman.
fn spawn_list_containers_full(handle: &Handle<Msg>) {
    handle.spawn(|| {
        let mut infos: Vec<ContainerInfo> = Vec::new();
        // 1. Rootfs en disco (unshare/bwrap) — la lista PERSISTENTE, como los
        //    hosts. Cada subdir de `~/.local/share/shuma/rootfs/` es un
        //    contenedor local. Es lo que el usuario crea con unshare/bwrap.
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
        // 2. Containers podman/docker (si está instalado).
        let podman = std::process::Command::new("podman")
            .args([
                "ps",
                "-a",
                "--format",
                "{{.Names}}\t{{.Status}}\t{{.Image}}",
            ])
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

/// Borra un rootfs en disco (`~/.local/share/shuma/rootfs/<name>`) en bg y
/// refresca la lista del gestor. Es el equivalente a "borrar host".
fn spawn_remove_rootfs(handle: &Handle<Msg>, name: String) {
    handle.spawn(move || {
        if let Some(root) = rootfs_root() {
            let p = root.join(&name);
            // Sólo dentro de la carpeta de rootfs — nunca un path arbitrario.
            if p.starts_with(&root) && p.is_dir() {
                let _ = std::fs::remove_dir_all(&p);
            }
        }
        Msg::RefreshContainersFull
    });
}

/// Dispara `podman <action> <name>` en bg; al terminar, refresca la
/// lista. `action` ∈ {start, stop, rm} (con `rm` agregamos `-f`).
fn spawn_container_action(handle: &Handle<Msg>, action: &'static str, name: String) {
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

/// Se asegura de que el container `name` esté corriendo: prueba `podman
/// start name` (no-op si ya está vivo). Si el container no existe (sesión
/// persistida pero el storage local lo borró), emite `ContainerFailed`
/// para que el chasis caiga a Local con notice. Si arranca OK, emite
/// `ContainerCreated(name)` que dispara `apply_isolation` y conecta.
fn spawn_ensure_container(handle: &Handle<Msg>, name: String) {
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

/// Crea un contenedor `name` de la `image` dada (detached, `sleep infinity`)
/// en un hilo; `mount` opcional se monta como `/work` adentro (RW). Al volver,
/// emite `ContainerCreated(name)` para que la sesión active termine de
/// montarse — esto permite usarlo como Source::Container apenas esté listo.
fn spawn_create_container(
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
                // Capturamos stderr para que el usuario vea por qué falló
                // (imagen no existe, name ya en uso, rootless storage, etc.).
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

// ─── Tipos de módulos conocidos por este binario ───────────────────

/// Qué `Kind` puede ocupar cada slot. Una variante por módulo
/// compilado: agregar uno nuevo (p. ej. `matilda`) es una variante +
/// ramas en `update`/`view`. El static dispatch sortea la ausencia de
/// `View::map` en llimphi-ui.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Launcher,
    CommandBar,
    Shell,
    Matilda,
    Minga,
    Canvas,
}

impl Kind {
    /// `id` canónico — bloque 5 lo usa para matchear shumarc.
    #[allow(dead_code)]
    fn id(self) -> &'static str {
        match self {
            Kind::Launcher => shuma_module_launcher::ID,
            Kind::CommandBar => shuma_module_commandbar::ID,
            Kind::Shell => shuma_module_shell::ID,
            Kind::Matilda => shuma_module_matilda::ID,
            Kind::Minga => shuma_module_minga::ID,
            Kind::Canvas => shuma_module_canvas::ID,
        }
    }
}

/// Cuál de las tres instancias-módulo de una sesión direcciona un `Slot` o un
/// `Msg`. Las vistas Hosts y Vhosts comparten la instancia Matilda (mismo
/// inventario, distinto render).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Shell,
    Canvas,
    Matilda,
}

/// Dónde corre el shell de la sesión (la base del aislamiento). El contenedor
/// NO es exclusivo: es una capa opcional **encima** de Local o Remoto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Isolation {
    /// Directo sobre esta máquina.
    Local,
    /// Sobre una máquina remota por SSH.
    Remote,
}

impl Isolation {
    const ALL: [Isolation; 2] = [Isolation::Local, Isolation::Remote];
    /// Etiqueta corta (la rica con sublabel la arma `view::iso_items`).
    #[allow(dead_code)]
    fn label(self) -> &'static str {
        match self {
            Isolation::Local => "Local",
            Isolation::Remote => "Remoto",
        }
    }
}

/// Estado de conexión de la sesión — lo refleja su panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnState {
    /// En espera (aún no conectada — remoto sin conectar / contenedor sin crear).
    Pending,
    /// Conectada y lista.
    Connected,
    /// Estuvo conectada y se cayó. (Se setea al caerse SSH/contenedor — fase B/C.)
    #[allow(dead_code)]
    Disconnected,
}

impl ConnState {
    fn label(self) -> &'static str {
        match self {
            ConnState::Pending => "en espera",
            ConnState::Connected => "conectado",
            ConnState::Disconnected => "desconectado",
        }
    }
}

/// La distro del aislamiento (para contenedor/remoto).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Distro {
    Ubuntu,
    Debian,
    Alpine,
    Arch,
}

impl Distro {
    const ALL: [Distro; 4] = [Distro::Ubuntu, Distro::Debian, Distro::Alpine, Distro::Arch];
    fn label(self) -> &'static str {
        match self {
            Distro::Ubuntu => "Ubuntu",
            Distro::Debian => "Debian",
            Distro::Alpine => "Alpine",
            Distro::Arch => "Arch",
        }
    }
    /// Imagen OCI **fully-qualified** para `podman run`. Sin el prefijo
    /// `docker.io/library/`, podman rootless en distros sin
    /// `unqualified-search-registries` configurado (Artix por defecto)
    /// falla con: `Error: short-name "ubuntu:latest" did not resolve to
    /// an alias and no unqualified-search registries are defined in
    /// /etc/containers/registries.conf`. Con el FQN no necesita config.
    fn image(self) -> &'static str {
        match self {
            Distro::Ubuntu => "docker.io/library/ubuntu:latest",
            Distro::Debian => "docker.io/library/debian:latest",
            Distro::Alpine => "docker.io/library/alpine:latest",
            Distro::Arch => "docker.io/library/archlinux:latest",
        }
    }
}

/// Distro a partir del nombre de un rootfs (`"ubuntu"` → `Distro::Ubuntu`).
fn distro_from_name(name: &str) -> Option<Distro> {
    let n = name.to_lowercase();
    Distro::ALL.into_iter().find(|d| d.label().to_lowercase() == n)
}

/// Campo del form de conexión remota que tiene el foco de teclado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteField {
    Host,
    User,
    Port,
}

/// Campo del form de **creación de sesión nueva** (canvas grande) con foco.
/// Local/Remote reusan host/user/port de la sesión; acá solo va el mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingField {
    Mount,
}

/// Estado de un container listado en la ventana gestora — más rico que el
/// `Vec<String>` que usa el dropdown (que solo necesita names).
#[derive(Debug, Clone)]
struct ContainerInfo {
    pub name: String,
    pub status: String, // "Up 2 hours", "Exited (0) 3 days ago", etc.
    pub image: String,
    /// `true` = rootfs en disco (unshare/bwrap): no tiene start/stop, sólo se
    /// borra (rm del directorio). `false` = container podman/docker.
    pub rootfs: bool,
}

/// Un directorio del host montado dentro del contenedor (bind). Persistido.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Mount {
    /// Path en el host.
    host: String,
    /// Path donde aparece DENTRO del contenedor.
    target: String,
    /// `true` = sólo lectura.
    #[serde(default)]
    readonly: bool,
}

/// Config persistida de un contenedor. Vive en `~/.config/shuma/containers.json`.
/// `name` es la clave (para rootfs unshare/bwrap = el nombre del directorio,
/// p. ej. "ubuntu").
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ContainerCfg {
    name: String,
    /// Host al que PERTENECE el contenedor: `"local"` o el nombre de un host
    /// remoto. El select de contenedor de una sesión sólo ofrece los de su host.
    #[serde(default = "host_local")]
    host: String,
    engine: String,
    distro: Distro,
    #[serde(default)]
    mounts: Vec<Mount>,
}

fn host_local() -> String {
    "local".to_string()
}

fn containers_cfg_path() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|b| b.config_dir().join("shuma").join("containers.json"))
}

fn load_container_cfgs() -> Vec<ContainerCfg> {
    containers_cfg_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Vec<ContainerCfg>>(&s).ok())
        .unwrap_or_default()
}

fn save_container_cfgs(cfgs: &[ContainerCfg]) {
    let Some(path) = containers_cfg_path() else {
        return;
    };
    if let Ok(json) = serde_json::to_string_pretty(cfgs) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, json);
    }
}

/// Columna de un mount con foco de teclado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MountCol {
    Host,
    Target,
}

/// Una fila de mount en el editor.
#[derive(Debug, Clone)]
struct MountDraft {
    host: TextInputState,
    target: TextInputState,
    readonly: bool,
}

impl MountDraft {
    fn new() -> Self {
        Self {
            host: TextInputState::new(),
            target: TextInputState::new(),
            readonly: false,
        }
    }
    fn from_mount(m: &Mount) -> Self {
        let mut host = TextInputState::new();
        host.set_text(m.host.clone());
        let mut target = TextInputState::new();
        target.set_text(m.target.clone());
        Self { host, target, readonly: m.readonly }
    }
    fn to_mount(&self) -> Option<Mount> {
        let host = self.host.text();
        let target = self.target.text();
        if host.trim().is_empty() || target.trim().is_empty() {
            return None;
        }
        Some(Mount { host, target, readonly: self.readonly })
    }
}

/// Editor de contenedor del gestor. `editing = Some(name)` edita uno existente
/// (engine/distro quedan readonly); `None` = nuevo (engine/distro activos).
#[derive(Debug, Clone)]
struct ContainerDraft {
    editing: Option<String>,
    /// Host al que pertenece (clave). Por ahora editable sólo al crear.
    host: String,
    engine: String,
    distro: Distro,
    mounts: Vec<MountDraft>,
    /// (índice de mount, columna) con foco de teclado.
    focus: Option<(usize, MountCol)>,
}

impl ContainerDraft {
    /// Nuevo, ligado al host `host_key` (el de la sesión activa).
    fn new(host: String) -> Self {
        Self {
            editing: None,
            host,
            engine: engine_preferido().unwrap_or("unshare").to_string(),
            distro: Distro::Ubuntu,
            mounts: Vec::new(),
            focus: None,
        }
    }
    fn from_cfg(cfg: &ContainerCfg) -> Self {
        Self {
            editing: Some(cfg.name.clone()),
            host: cfg.host.clone(),
            engine: cfg.engine.clone(),
            distro: cfg.distro,
            mounts: cfg.mounts.iter().map(MountDraft::from_mount).collect(),
            focus: None,
        }
    }
    fn to_cfg(&self, name: String) -> ContainerCfg {
        ContainerCfg {
            name,
            host: self.host.clone(),
            engine: self.engine.clone(),
            distro: self.distro,
            mounts: self.mounts.iter().filter_map(MountDraft::to_mount).collect(),
        }
    }
}

/// Form embebido en la ventana de hosts para crear/editar uno nuevo.
#[derive(Debug, Clone)]
struct HostDraft {
    name: TextInputState,
    host: TextInputState,
    user: TextInputState,
    port: TextInputState,
    /// Modo de auth: `true` = password (askpass al conectar), `false` = PEM.
    use_password: bool,
    pem_path: TextInputState,
    /// Campo con foco de teclado dentro del draft (`None` = ninguno).
    focused: Option<HostDraftField>,
    /// Nombre original si se está EDITANDO uno existente (`None` = nuevo). Sirve
    /// para remarcar su fila y para borrar la entrada vieja si se renombra.
    editing: Option<String>,
}

impl HostDraft {
    fn new() -> Self {
        let mut port = TextInputState::new();
        port.set_text("22");
        Self {
            name: TextInputState::new(),
            host: TextInputState::new(),
            user: TextInputState::new(),
            port,
            use_password: true,
            pem_path: TextInputState::new(),
            focused: Some(HostDraftField::Name),
            editing: None,
        }
    }

    fn from_host(h: &hosts::RemoteHost) -> Self {
        let mut name = TextInputState::new();
        name.set_text(h.name.clone());
        let mut host = TextInputState::new();
        host.set_text(h.host.clone());
        let mut user = TextInputState::new();
        user.set_text(h.user.clone());
        let mut port = TextInputState::new();
        port.set_text(h.port.to_string());
        let (use_password, pem) = match &h.auth {
            hosts::HostAuth::Password => (true, String::new()),
            hosts::HostAuth::Key { path } => (false, path.clone()),
        };
        let mut pem_path = TextInputState::new();
        pem_path.set_text(pem);
        Self {
            name,
            host,
            user,
            port,
            use_password,
            pem_path,
            focused: Some(HostDraftField::Name),
            editing: Some(h.name.clone()),
        }
    }

    fn to_host(&self) -> Option<hosts::RemoteHost> {
        let name = self.name.text();
        let host = self.host.text();
        let user = self.user.text();
        if name.trim().is_empty() || host.trim().is_empty() || user.trim().is_empty() {
            return None;
        }
        let port: u16 = self.port.text().trim().parse().unwrap_or(22);
        let auth = if self.use_password {
            hosts::HostAuth::Password
        } else {
            let path = self.pem_path.text();
            hosts::HostAuth::Key { path }
        };
        Some(hosts::RemoteHost { name, host, user, port, auth })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostDraftField {
    Name,
    Host,
    User,
    Port,
    Pem,
}

/// Cuál dropdown de la config de sesión está abierto (overlay del select).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropKind {
    Isolation,
    Distro,
    /// Suscribir a un contenedor existente / crear uno nuevo.
    Container,
    /// Elegir el engine de aislamiento (unshare / bwrap / podman) — sólo
    /// muestra los disponibles en el `PATH` del proceso.
    Engine,
    /// Elegir un host remoto guardado en el form de sesión nueva. El menú
    /// flota centrado (el form vive en el canvas, no en el panel lateral).
    Host,
}

/// El tipo de una sesión — define el icono de su diente (rail izquierdo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionKind {
    /// La sesión por defecto, local y sin aislamiento — "no toca nada". No
    /// lleva número de insignia. Es la primera y siempre está.
    Draft,
    /// Sesión local creada por el usuario (con número de insignia).
    Local,
    /// Sesión remota (SSH/daemon) — aislamiento remoto. Aún no la crea nadie
    /// (el `+` hace local); el form de aislamiento remoto es la fase 4.
    #[allow(dead_code)]
    Remote,
}

/// Las **herramientas** de la sesión activa — un diente del rail DERECHO. Cada
/// una abre su panel operando sobre la sesión activa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Tool {
    History,
    Monitor,
    Explorer,
    Matilda,
}

impl Tool {
    /// Orden de los dientes en el rail derecho (debe seguir a `host_tool_teeth`).
    const ALL: [Tool; 4] = [Tool::History, Tool::Monitor, Tool::Explorer, Tool::Matilda];

    fn label(self) -> &'static str {
        match self {
            Tool::History => "Historial",
            Tool::Monitor => "Monitor",
            Tool::Explorer => "Explorer",
            Tool::Matilda => "Matilda",
        }
    }
}

/// State vivo de un módulo. Una variante por `Kind` para evitar trait
/// objects (cada módulo trae su propio `Msg` que no es object-safe).
enum ModuleState {
    Launcher(shuma_module_launcher::State),
    CommandBar(shuma_module_commandbar::State),
    Shell(shuma_module_shell::State),
    // `State` de matilda lleva el inventory entero (varios cientos
    // de bytes); boxearlo mantiene el enum ModuleState compacto.
    Matilda(Box<shuma_module_matilda::State>),
    Minga(shuma_module_minga::State),
    Canvas(shuma_module_canvas::State),
}

/// Una instancia activa de un módulo. `kind` + `state` deben coincidir
/// (lo invariante lo garantiza el constructor).
struct Instance {
    kind: Kind,
    /// Etiqueta del módulo. El título de la vista lo arma la sesión
    /// (`nombre · vista`); los constructores la setean y queda disponible.
    #[allow(dead_code)]
    label: String,
    state: ModuleState,
}

impl Instance {
    fn launcher(state: shuma_module_launcher::State) -> Self {
        Self {
            kind: Kind::Launcher,
            label: rimay_localize::t("shuma-label-launcher"),
            state: ModuleState::Launcher(state),
        }
    }

    fn command_bar(state: shuma_module_commandbar::State) -> Self {
        Self {
            kind: Kind::CommandBar,
            label: rimay_localize::t("shuma-label-command"),
            state: ModuleState::CommandBar(state),
        }
    }

    fn shell(label: String, source: Source) -> Self {
        Self {
            kind: Kind::Shell,
            label,
            state: ModuleState::Shell(shuma_module_shell::State::new(source)),
        }
    }

    fn matilda(label: String, source: Source) -> Self {
        Self::matilda_with_inventory(label, source, None)
    }

    fn matilda_with_inventory(
        label: String,
        source: Source,
        inventory: Option<&std::path::Path>,
    ) -> Self {
        let state = match inventory {
            Some(p) => {
                let inv = load_matilda_inventory(p).unwrap_or_else(example_inventory_fallback);
                shuma_module_matilda::State::with_inventory_path(source, inv, p.to_path_buf())
            }
            None => shuma_module_matilda::State::new(source),
        };
        Self {
            kind: Kind::Matilda,
            label,
            state: ModuleState::Matilda(Box::new(state)),
        }
    }

    fn minga(label: String, source: Source) -> Self {
        Self {
            kind: Kind::Minga,
            label,
            state: ModuleState::Minga(shuma_module_minga::State::new(source)),
        }
    }

    fn canvas(label: String) -> Self {
        Self {
            kind: Kind::Canvas,
            label,
            state: ModuleState::Canvas(shuma_module_canvas::State::new()),
        }
    }
}

/// Una **sesión de trabajo**: un ambiente con su aislamiento (local o remoto)
/// y sus tres vistas (shell, lienzo, inventario matilda). Cambiar de sesión
/// (tab superior) cambia todo el ambiente; el rail derecho elige la vista.
struct Session {
    name: String,
    kind: SessionKind,
    /// Número de insignia del diente (None para la draft).
    number: Option<u32>,
    /// Base del aislamiento (Local/Remoto).
    isolation: Isolation,
    /// Capa de contenedor OPCIONAL (encima de Local o Remoto). El colapsable
    /// del panel la crea/conecta.
    distro: Distro,
    /// Contenedor suscrito (`None` = sin contenedor). Cuando `use_container`
    /// está activo, el shell ejecuta dentro de él vía `Source::Container`.
    container: Option<String>,
    /// `true` si esta sesión usa un contenedor (la capa OCI por encima del
    /// aislamiento base). El form de creación lo togglea; `apply_isolation`
    /// lo lee para resolver el `Source` real.
    use_container: bool,
    /// Engine que ejecuta el container: "bwrap" (default si está) o "podman"
    /// (fallback). El usuario puede sobreescribirlo desde el form.
    container_engine: String,
    /// Si el colapsable de contenedor está abierto en el panel.
    container_open: bool,
    /// Estado de conexión de la sesión (lo refleja el panel).
    conn: ConnState,
    /// Nombre del host remoto elegido (`None` = Local). Es la CLAVE que liga la
    /// sesión a un host: el select de contenedor sólo ofrece los de este host.
    host_label: Option<String>,
    /// Campos del form de conexión remota (sólo se usan con `Isolation::Remote`).
    host: TextInputState,
    user: TextInputState,
    port: TextInputState,
    /// `true` mientras la sesión está siendo configurada en el form grande
    /// (canvas) y todavía no se materializó su shell. Al `ConfirmNewSession`
    /// pasa a `false` y arranca el shell con el `source` resuelto.
    pending: bool,
    /// Path único a montar dentro del contenedor (MVP — Vec viene después).
    /// Solo se usa con `Isolation::Local` + container ≠ None.
    mount: TextInputState,
    /// Cuál campo del form de creación tiene foco (`None` = ninguno).
    pending_focus: Option<PendingField>,
    /// El origen de ejecución del shell + matilda (Local / Daemon / Remote).
    /// (El enforcement real del aislamiento contenedor/remoto es deuda; hoy el
    /// shell corre con este `source`.)
    #[allow(dead_code)]
    source: Source,
    shell: Instance,
    canvas: Instance,
    matilda: Instance,
}

impl Session {
    fn build(name: String, kind: SessionKind, number: Option<u32>, source: Source) -> Self {
        Self {
            shell: Instance::shell(name.clone(), source.clone()),
            canvas: Instance::canvas(rimay_localize::t("shuma-label-canvas")),
            matilda: Instance::matilda(name.clone(), source.clone()),
            name,
            kind,
            number,
            isolation: Isolation::Local,
            distro: Distro::Ubuntu,
            container: None,
            use_container: false,
            container_engine: engine_preferido().unwrap_or("bwrap").to_string(),
            container_open: false,
            pending: false,
            mount: TextInputState::new(),
            pending_focus: None,
            // Local arranca conectado; remoto en espera hasta conectar.
            conn: ConnState::Connected,
            host_label: None,
            host: TextInputState::new(),
            user: TextInputState::new(),
            port: {
                let mut p = TextInputState::new();
                p.set_text("22");
                p
            },
            source,
        }
    }

    /// La sesión por defecto: local, sin número. **Es utilizable directo** —
    /// arranca con shell vivo (Source::Local / Daemon según env). El form
    /// grande de creación se dispara con el botón `+`, no editando esta.
    fn draft() -> Self {
        Self::build("draft".to_string(), SessionKind::Draft, None, default_shell_source())
    }

    /// Sesión nueva en modo **configuración** — el canvas muestra el form
    /// grande (aislamiento / distro / mount / container). El shell se
    /// construye recién en `ConfirmNewSession`. Mientras `pending`, el shell
    /// interno es un placeholder Local que nadie ve.
    fn new_pending(n: u32) -> Self {
        let mut s = Self::build(
            format!("local {n}"),
            SessionKind::Local,
            Some(n),
            Source::Local,
        );
        s.pending = true;
        s
    }

    /// Clave del host de la sesión: `"local"` o el nombre del host remoto
    /// elegido. Liga la sesión a un host: el contenedor pertenece a este host.
    fn host_key(&self) -> String {
        self.host_label.clone().unwrap_or_else(|| "local".to_string())
    }

    /// `true` si la sesión está moviendo datos ahora (comando corriendo) — para
    /// el puntito LED del diente.
    fn active_data(&self) -> bool {
        matches!(&self.shell.state, ModuleState::Shell(s) if s.is_running())
    }

    /// Reconstruye el shell + matilda con el `source` que dicta el aislamiento
    /// elegido. Pierde el shell anterior a propósito: reconfigurar el aislamiento
    /// = ambiente nuevo. Si `use_container` y `container = Some(name)`, el
    /// shell corre **dentro** del contenedor vía `Source::Container`.
    fn apply_isolation(&mut self) {
        let base = match self.isolation {
            Isolation::Local => Source::Local,
            Isolation::Remote => default_shell_source(),
        };
        let source = if self.use_container {
            if let Some(name) = self.container.clone() {
                // Rootfs unshare/bwrap: lo preparamos para que los gestores de
                // paquetes funcionen en un userns de un solo uid (no pueden
                // dropear privilegios a `_apt`/`alpm`). Idempotente.
                if matches!(self.container_engine.as_str(), "unshare" | "bwrap") {
                    prepare_rootfs(std::path::Path::new(&name));
                }
                Source::Container {
                    engine: self.container_engine.clone(),
                    name,
                    label: None,
                }
            } else {
                base
            }
        } else {
            base
        };
        // Container/remote arrancan en espera (hasta ContainerCreated /
        // connect_remote); local está listo de entrada.
        self.conn = if self.use_container {
            ConnState::Pending
        } else {
            match self.isolation {
                Isolation::Local => ConnState::Connected,
                Isolation::Remote => ConnState::Pending,
            }
        };
        self.shell = Instance::shell(self.name.clone(), source.clone());
        self.matilda = Instance::matilda(self.name.clone(), source.clone());
        self.source = source;
    }

    fn instance(&self, w: Which) -> &Instance {
        match w {
            Which::Shell => &self.shell,
            Which::Canvas => &self.canvas,
            Which::Matilda => &self.matilda,
        }
    }

    fn instance_mut(&mut self, w: Which) -> &mut Instance {
        match w {
            Which::Shell => &mut self.shell,
            Which::Canvas => &mut self.canvas,
            Which::Matilda => &mut self.matilda,
        }
    }

    /// Config persistible de la sesión (sin las instancias-módulo vivas).
    fn to_config(&self) -> SessionConfig {
        SessionConfig {
            name: self.name.clone(),
            number: self.number,
            isolation: self.isolation,
            distro: self.distro,
            container: self.container.clone(),
            use_container: self.use_container,
            container_engine: self.container_engine.clone(),
            mount: self.mount.text(),
            host_label: self.host_label.clone(),
            host: self.host.text(),
            user: self.user.text(),
            port: self.port.text(),
        }
    }

    /// Reconstruye una sesión desde su config persistida.
    fn from_config(c: SessionConfig) -> Self {
        let kind = match c.isolation {
            Isolation::Remote => SessionKind::Remote,
            Isolation::Local => SessionKind::Local,
        };
        let source = match c.isolation {
            Isolation::Local => Source::Local,
            Isolation::Remote => default_shell_source(),
        };
        let mut s = Session::build(c.name, kind, c.number, source);
        s.isolation = c.isolation;
        s.distro = c.distro;
        s.container = c.container;
        s.use_container = c.use_container;
        // Si la sesión persistió "podman" pero ya no está instalado,
        // rebajamos al preferido del sistema actual.
        if !c.container_engine.is_empty() && binary_disponible(&c.container_engine) {
            s.container_engine = c.container_engine;
        } else if let Some(pref) = engine_preferido() {
            s.container_engine = pref.to_string();
        }
        s.mount.set_text(c.mount);
        s.host_label = c.host_label;
        s.host.set_text(c.host);
        s.user.set_text(c.user);
        if !c.port.is_empty() {
            s.port.set_text(c.port);
        }
        // Aplica el aislamiento real (incluye Source::Container si toca).
        s.apply_isolation();
        s
    }

    /// El campo de input del form remoto (mutable, para `apply_key`).
    fn remote_field_mut(&mut self, f: RemoteField) -> &mut TextInputState {
        match f {
            RemoteField::Host => &mut self.host,
            RemoteField::User => &mut self.user,
            RemoteField::Port => &mut self.port,
        }
    }

    /// Conecta el aislamiento remoto: arma `Source::Remote{host,user,port}` con
    /// lo que hay en los campos y reconstruye el shell. Conn → Connected.
    fn connect_remote(&mut self) {
        let host = self.host.text();
        let user = self.user.text();
        if host.trim().is_empty() || user.trim().is_empty() {
            return; // sin host/usuario no hay a dónde conectar
        }
        let port: u16 = self.port.text().trim().parse().unwrap_or(22);
        let source = Source::Remote {
            host,
            user,
            port,
            label: None,
        };
        self.shell = Instance::shell(self.name.clone(), source.clone());
        self.matilda = Instance::matilda(self.name.clone(), source.clone());
        self.source = source;
        self.conn = ConnState::Connected;
    }

    /// (Re)conecta la sesión: reconstruye el shell con el `Source` que le toca
    /// según su estado. Para un host remoto rearma `Source::Remote`; para una
    /// sesión con contenedor reentra al contenedor; en Local simplemente forja
    /// un shell fresco. Es la acción del botón "Conectar/Reconectar" del panel.
    fn reconnect(&mut self) {
        if self.host_label.is_some() {
            self.connect_remote();
        } else {
            // Local o contenedor: `apply_isolation` ya resuelve el Source y deja
            // `conn` en Connected (o Pending si el contenedor aún no está listo).
            self.apply_isolation();
        }
    }
}

/// Config persistible de una sesión (lo que sobrevive a reiniciar shuma).
#[derive(serde::Serialize, serde::Deserialize)]
struct SessionConfig {
    name: String,
    #[serde(default)]
    number: Option<u32>,
    isolation: Isolation,
    distro: Distro,
    #[serde(default)]
    container: Option<String>,
    #[serde(default)]
    use_container: bool,
    #[serde(default)]
    container_engine: String,
    #[serde(default)]
    mount: String,
    #[serde(default)]
    host_label: Option<String>,
    #[serde(default)]
    host: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    port: String,
}

/// `$XDG_CONFIG_HOME/shuma/sessions.json`.
fn sessions_path() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|b| b.config_dir().join("shuma").join("sessions.json"))
}

/// Guarda las sesiones reales (no la draft) para reiniciarlas en el próximo
/// arranque. Silencioso ante errores de IO.
fn save_sessions(m: &Model) {
    let Some(path) = sessions_path() else {
        return;
    };
    let cfgs: Vec<SessionConfig> = m
        .sessions
        .iter()
        .filter(|s| s.kind != SessionKind::Draft && !s.pending)
        .map(|s| s.to_config())
        .collect();
    if let Ok(json) = serde_json::to_string_pretty(&cfgs) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, json);
    }
}

/// Lee las sesiones persistidas (vacío si no hay archivo o no parsea).
fn load_sessions() -> Vec<SessionConfig> {
    sessions_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Vec<SessionConfig>>(&s).ok())
        .unwrap_or_default()
}

/// Estado de chrome persistible: qué paneles están abiertos y qué pestaña
/// (sesión) está activa, para reabrir shuma como lo dejaste. Separado de
/// `sessions.json` para no acoplar el layout de UI al de las sesiones.
#[derive(serde::Serialize, serde::Deserialize)]
struct ChromeState {
    /// Herramienta abierta a la derecha (`None` = rail derecho colapsado).
    /// Por defecto colapsado — el panel derecho no se impone al arrancar.
    #[serde(default)]
    active_tool: Option<Tool>,
    /// Panel de config de la sesión (izquierda) desplegado.
    #[serde(default = "yes")]
    session_panel_open: bool,
    /// Índice de la sesión/pestaña activa.
    #[serde(default)]
    active_session: usize,
    /// Ancho (px) del panel de config de la sesión (izquierda). Parte de la
    /// **disposición**: el splitter se reabre donde lo dejaste.
    #[serde(default = "default_session_w")]
    session_w: f32,
    /// Ancho (px) del panel de herramienta/monitores (derecha).
    #[serde(default = "default_monitors_width")]
    monitors_width: f32,
}

fn yes() -> bool {
    true
}

fn default_session_w() -> f32 {
    240.0
}

fn default_monitors_width() -> f32 {
    MONITORS_INITIAL_WIDTH
}

impl Default for ChromeState {
    fn default() -> Self {
        // Default pedido: panel derecho colapsado, panel de config abierto.
        Self {
            active_tool: None,
            session_panel_open: true,
            active_session: 0,
            session_w: default_session_w(),
            monitors_width: default_monitors_width(),
        }
    }
}

/// `$XDG_CONFIG_HOME/shuma/chrome.json`.
fn chrome_path() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|b| b.config_dir().join("shuma").join("chrome.json"))
}

/// Guarda el estado de chrome (paneles + pestaña activa). Silencioso ante IO.
fn save_chrome(m: &Model) {
    let Some(path) = chrome_path() else {
        return;
    };
    let state = ChromeState {
        active_tool: m.active_tool,
        session_panel_open: m.session_panel_open,
        active_session: m.active_session,
        session_w: m.session_w,
        monitors_width: m.monitors_width,
    };
    if let Ok(json) = serde_json::to_string_pretty(&state) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, json);
    }
}

/// Lee el estado de chrome persistido (default si no hay archivo o no parsea).
fn load_chrome() -> ChromeState {
    chrome_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<ChromeState>(&s).ok())
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
enum ModuleMsg {
    Launcher(shuma_module_launcher::Msg),
    CommandBar(shuma_module_commandbar::Msg),
    #[allow(dead_code)]
    Shell(shuma_module_shell::Msg),
    Matilda(shuma_module_matilda::Msg),
    Minga(shuma_module_minga::Msg),
    Canvas(shuma_module_canvas::Msg),
}

// ─── Slot del chasis al que va un Msg de módulo ────────────────────

/// Identifica de dónde viene un `ModuleMsg`. Los slots únicos (TopBar/
/// Bottombar/Main) se identifican por sí mismos; el Tab lleva el
/// índice del tab para enrutar al instance correcto.
#[derive(Debug, Clone)]
enum Slot {
    TopBar,
    BottomBar,
    #[allow(dead_code)]
    Main,
    /// Una instancia-módulo de la sesión `idx` (cuál, lo dice `Which`).
    Session(usize, Which),
}

// ─── Modelo + Msg ───────────────────────────────────────────────────

struct Model {
    theme: Theme,

    // Slots fijos (únicos):
    topbar: Option<Instance>,
    bottombar: Option<Instance>,
    /// Si está set, ocupa toda el área central (sin tabs). Útil para
    /// configurar shuma como wrapper de una sola app (matilda standalone,
    /// editor, etc.) vía shumarc.
    main: Option<Instance>,

    // Sesiones de trabajo (tabs superiores cuando `main` está vacío). Cambiar
    // de sesión cambia todo el ambiente; `active_view` (rail derecho) elige la
    // vista de la sesión activa.
    sessions: Vec<Session>,
    active_session: usize,
    /// Diente de sesión bajo el cursor (`None` = ninguno). Lo alimentan los
    /// `on_pointer_enter/leave` del rail y lo lee la barra de estado para
    /// mostrar el nombre completo de la sesión hovereada (los dientes sólo
    /// muestran icono + número). Puramente efímero: no se persiste.
    hovered_session: Option<usize>,
    /// Herramienta abierta a la derecha (`None` = sin panel de herramienta).
    active_tool: Option<Tool>,
    /// Si el panel de la sesión activa (su configuración, a la izquierda) está
    /// desplegado. Cada diente de sesión ES su panel: al seleccionarlo se abre;
    /// re-clickear el activo lo cierra.
    session_panel_open: bool,
    /// Dropdown de config abierto (overlay del select), o `None`.
    dropdown_open: Option<DropKind>,
    /// Contenedores locales descubiertos (`podman ps -a`) — para suscribir.
    containers: Vec<String>,
    /// Lista detallada de containers para el gestor (name + state).
    containers_full: Vec<ContainerInfo>,
    /// Config persistida de contenedores (engine/distro/mounts), por nombre.
    container_cfgs: Vec<ContainerCfg>,
    /// Campo del form remoto con foco de teclado (`None` = ninguno).
    focused_field: Option<RemoteField>,
    /// Hosts remotos guardados (`$XDG_CONFIG_HOME/shuma/hosts.json`).
    hosts: Vec<hosts::RemoteHost>,
    /// Draft del nuevo host que se está creando en la ventana gestora.
    /// `None` = no hay form abierto; `Some(...)` lo pinta.
    host_draft: Option<HostDraft>,
    /// Draft del nuevo container que se está creando en la ventana de
    /// gestión de containers.
    container_draft: Option<ContainerDraft>,
    /// El diálogo bloqueante de hosts está abierto (modal centrado, no una
    /// ventana del SO). Pinta lista + borrar + form de alta.
    hosts_modal_open: bool,
    /// El diálogo bloqueante de containers está abierto (modal centrado).
    containers_modal_open: bool,
    /// Tamaño del viewport en px lógicos, para centrar los modales y anclar
    /// los selects del form de sesión nueva. Lo actualiza `on_resize`.
    viewport: (f32, f32),

    // Anchos resizables de los paneles laterales (px).
    session_w: f32,
    sysmon: SystemSampler,
    last_snapshot: Option<Snapshot>,
    monitors_width: f32,
    /// Historial por monitor extra (los que aportan los módulos vía
    /// `contributions()`). La clave es `"<slot>/<spec.id>"`. El chasis
    /// los muestrea en cada `Tick` y los acumula como `f32`.
    extra_history: HashMap<String, Vec<f32>>,
    /// Último `Sample::display` por monitor — se pinta como subtítulo
    /// de la stat-card.
    extra_display: HashMap<String, String>,
    /// Watcher del bus de config wawa. Vive lo que vive el modelo —
    /// al dropear se cierran los notify::RecommendedWatcher y el thread
    /// de debounce sale silenciosamente. Ningún read directo desde
    /// el código de update — sólo recibe callbacks que se traducen a
    /// `Msg::WawaConfigChanged`.
    _wawa_watcher: Option<wawa_config::ConfigWatcher>,

    /// Menú principal: índice del menú raíz abierto (`None` = cerrado).
    menu_open: Option<usize>,
    /// Fila activa (resaltada por teclado) del dropdown del menú principal.
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal (0→1).
    menu_anim: Tween<f32>,
    /// Menú contextual de terminal: ancla `(x, y)` en ventana (`None` =
    /// cerrado). Se abre con right-click sobre el área de trabajo.
    ctx_menu: Option<(f32, f32)>,

    /// Cliente del rail hospedado: con `SHUMA_DELEGATE_SIDEBAR`, shuma presta
    /// sus tabs + el toggle de monitores al rail de pata. Kept-alive (las
    /// activaciones llegan por callback → `Msg::HostActivate`); el `_` evita
    /// el lint de campo sin leer, como `_wawa_watcher`.
    _host: Option<pata_host::HostClient>,
}

impl Model {
    /// La sesión activa (la primera si el índice quedó fuera de rango).
    fn active(&self) -> Option<&Session> {
        self.sessions.get(self.active_session).or_else(|| self.sessions.first())
    }

    /// Instancia-módulo `w` de la sesión `idx`, si existe.
    fn session_instance(&self, idx: usize, w: Which) -> Option<&Instance> {
        self.sessions.get(idx).map(|s| s.instance(w))
    }

    fn session_instance_mut(&mut self, idx: usize, w: Which) -> Option<&mut Instance> {
        self.sessions.get_mut(idx).map(|s| s.instance_mut(w))
    }

}

#[derive(Clone)]
enum Msg {
    Tick,
    /// Tick rápido que drena la salida del shell (~100 ms) sin tocar
    /// el muestreo de sysmon.
    ShellTick,
    /// El viewport cambió de tamaño (winit `Resized`). Guardamos las
    /// dimensiones para centrar los modales y anclar los selects del form.
    Resized(f32, f32),
    /// Click en un diente de sesión (rail izquierdo): cambia el ambiente.
    SelectSession(usize),
    /// El cursor entró (`Some(i)`) o salió (`None`) de un diente de sesión.
    /// Sólo actualiza el resaltado/hint de la barra de estado; no toca foco.
    HoverSession(Option<usize>),
    /// Click en un diente de herramienta (rail derecho): abre/cierra su panel.
    SelectTool(Tool),
    /// Abrir/cerrar un dropdown de config (aislamiento o distro).
    ToggleDropdown(DropKind),
    /// Cerrar el dropdown (scrim / Esc).
    DismissDropdown,
    /// Elegir el aislamiento en el panel de config. Sobre la draft, configurar
    /// la promueve a sesión propia (y nace un draft nuevo); sobre una sesión
    /// real, edita su config.
    SetIsolation(Isolation),
    /// Elegir la distro del aislamiento (idem promoción del draft).
    SetDistro(Distro),
    /// No hace nada. Lo usan los scrims de los modales bloqueantes como
    /// `on_dismiss`: un clic afuera NO los cierra (bloquean la app); se
    /// cierran sólo con su botón «Listo» o con Esc.
    Noop,
    /// Abrir/cerrar el colapsable de contenedor (capa opcional).
    ToggleContainer,
    /// Dar foco a un campo del form remoto (click).
    FocusField(RemoteField),
    /// Tecla al campo remoto focado (Esc desenfoca, Enter conecta).
    RemoteKey(KeyEvent),
    /// Conectar el aislamiento remoto con los datos del form.
    ConnectRemote,
    /// (Re)conectar la sesión `idx`: reconstruye el shell con el `Source` que
    /// le corresponde (remoto / contenedor / local). Botón del panel.
    ReconnectSession(usize),
    /// Cerrar (descartar) la sesión `idx`. La draft (0) no se cierra.
    CloseSession(usize),
    /// Click en el botón `+` del rail: crea una sesión `pending` y la activa,
    /// para que el canvas muestre el form grande de creación.
    OpenNewSessionForm,
    /// Confirma la sesión pending activa: arma su `Source` real, monta el
    /// shell y persiste. Sale del modo pending.
    ConfirmNewSession,
    /// Descarta la sesión pending activa (sin crear nada).
    CancelNewSession,
    /// Foco a un campo del form de creación (mount, etc.). Click sobre el
    /// input lo dispara.
    FocusPendingField(PendingField),
    /// Tecla a un campo del form de creación cuando el foco lo apunta.
    PendingKey(KeyEvent),
    /// Re-listar los contenedores locales (`docker ps -a`).
    RefreshContainers,
    /// Resultado del listado de contenedores.
    ContainersLoaded(Vec<String>),
    /// Suscribir la sesión activa al contenedor `idx` de la lista.
    SubscribeContainer(usize),
    /// Crear un contenedor nuevo con la distro de la sesión y suscribirla.
    CreateContainer,
    /// Toggle del checkbox "Aislar en contenedor" del form de creación.
    ToggleUseContainer,
    /// Cambia el engine de aislamiento de la sesión activa (unshare /
    /// bwrap / podman) desde el dropdown del form.
    SetEngine(String),
    /// Click sobre un rootfs presente en disco (filas en el form de
    /// container) — asocia la sesión activa con ese rootfs y arranca.
    PickRootfs(Distro),
    /// El thread de `podman run` terminó OK — la sesión que lo esperaba
    /// queda lista (conectada) y, si era pending, ya tiene su shell montado.
    ContainerCreated(String),
    /// El thread de `podman run` falló (podman ausente / imagen / nombre).
    /// El motivo (primera línea de stderr) se muestra al usuario en el
    /// shell de la sesión que pidió el container.
    ContainerFailed { name: String, reason: String },
    /// Al reabrir un workspace con `use_container=true`, despachamos esto
    /// para reactivar el container (podman start name) sin tener que
    /// recrearlo. Si falla por inexistencia, el `ContainerFailed`
    /// rebaja la sesión a Local con notice.
    EnsureContainer(String),

    // ─── Diálogo bloqueante de containers ─────────────────────────────
    /// Abre el modal de containers (centrado, bloqueante) y arranca un
    /// draft de alta. Carga la lista con `podman ps -a`. Reemplaza la
    /// vieja ventana secundaria del SO.
    OpenContainersWindow,
    /// Cierra el modal de containers (scrim / Esc / Listo) y descarta el draft.
    CloseContainersModal,
    /// El thread de `podman ps -a --format json` terminó. Reemplaza la
    /// lista completa de containers en el modelo.
    ContainersFullLoaded(Vec<ContainerInfo>),
    /// Refresca la lista (re-spawn de `podman ps -a`).
    RefreshContainersFull,
    /// `podman start <name>` para un container parado.
    StartContainer(String),
    /// `podman stop <name>` para un container corriendo.
    StopContainer(String),
    /// `podman rm -f <name>` — destructivo, borra el container.
    RemoveContainer(String),
    /// Borra un rootfs en disco (unshare/bwrap) — destructivo, rm del dir.
    RemoveRootfs(String),

    // ─── Diálogo bloqueante de hosts ──────────────────────────────────
    /// Abre el modal de hosts (centrado, bloqueante) y arranca un draft
    /// de alta para que el form de «nuevo host» esté listo. Reemplaza la
    /// vieja ventana secundaria del SO.
    OpenHostsWindow,
    /// Cierra el modal de hosts (scrim / Esc / Listo) y descarta el draft.
    CloseHostsModal,
    /// "Nuevo": draft de host fresco (deselecciona la lista).
    HostDraftStart,
    /// Elegir el host `idx` de la lista para editarlo (carga el draft).
    HostEdit(usize),
    /// Cancela el draft del host (cierra el form).
    HostDraftCancel,
    /// Guarda el draft del host en disco + en el modelo.
    HostDraftSave,
    /// Foco a un campo del draft del host.
    HostDraftFocus(HostDraftField),
    /// Tecla al campo del draft focado.
    HostDraftKey(KeyEvent),
    /// Cambia el modo de auth del draft (Password ↔ Key).
    HostDraftToggleAuth,
    /// Borrar el host `idx` de la lista guardada.
    HostDelete(usize),
    // ─── Editor CRUD de contenedores (ventana de gestión) ──────────────
    /// "Nuevo": deselecciona la lista y abre un draft con engine/distro activos.
    ContainerDraftNew,
    /// Cancela el draft (cierra el form de edición).
    ContainerDraftCancel,
    /// Elegir la fila `idx` de la lista para editarla (engine/distro readonly).
    ContainerEdit(usize),
    /// Cambia el engine del draft (sólo si es nuevo).
    ContainerDraftSetEngine(String),
    /// Cambia la distro del draft (sólo si es nuevo).
    ContainerDraftSetDistro(Distro),
    /// Agrega una fila de mount vacía.
    ContainerDraftAddMount,
    /// Quita la fila de mount `idx`.
    ContainerDraftRemoveMount(usize),
    /// Alterna readonly del mount `idx`.
    ContainerDraftToggleMountRo(usize),
    /// Foco a una columna (host/target) del mount `idx`.
    ContainerDraftFocusMount(usize, MountCol),
    /// Guarda el draft (crea el recurso si es nuevo) y persiste su config.
    ContainerDraftSave,
    /// Tecla al input del mount focado del draft.
    ContainerDraftKey(KeyEvent),
    /// Elegir el host de la sesión desde el select único: `None` = Local,
    /// `Some(i)` = el host guardado `i`. Liga la sesión a ese host.
    PickHost(Option<usize>),
    /// Aplicar un host guardado al form de la sesión actual (rellena
    /// host/user/port + dispara connect si la sesión está pending).
    HostApply(usize),
    /// Reordenar dientes por drag: mover la sesión `from` a la posición `to`.
    /// La draft (0) queda fija.
    ReorderSession(usize, usize),
    /// Resize del panel de sesión (izq) / de herramienta (der), por drag del
    /// divisor del `splitter`.
    SetSessionWidth(f32),
    SetToolWidth(f32),
    /// Click en una línea del historial: carga ese comando en el input del
    /// shell de la sesión activa (el usuario confirma con Enter).
    RunFromHistory(String),
    /// Botón ▶ de una fila del historial: re-ejecuta ese comando YA en el
    /// shell de la sesión activa (sin esperar Enter).
    RunFromHistoryNow(String),
    /// Msg de un módulo. El chasis lo enruta a `update` según `slot`.
    Module(Slot, ModuleMsg),
    /// Click en un botón de acción (matilda: discover/dry-run/apply/reload).
    /// `slot` es el módulo emisor; lo resuelve `handle_shortcut`.
    ShortcutClicked(Slot, ShortcutAction),
    /// La config de wawa (`$XDG_CONFIG_HOME/wawa/config.json`) cambió;
    /// rearmamos el theme, accent y locale sin reiniciar. Boxed por
    /// tamaño (la config tiene un BTreeMap de módulos).
    WawaConfigChanged(Box<wawa_config::WawaConfig>),

    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` = cerrar).
    MenuOpen(Option<usize>),
    /// Navegación de teclado en el dropdown del menú principal (±1 fila).
    MenuNav(i32),
    /// Enter sobre la fila activa del menú principal.
    MenuActivate,
    /// Tick de re-render para la animación de aparición del dropdown.
    MenuTick,
    /// Comando elegido en el menú principal o contextual — se traduce al
    /// `Msg`/acción real del chasis o del módulo shell focado.
    MenuCommand(String),
    /// Right-click sobre el área de trabajo → abre el menú contextual de
    /// terminal en `(x, y)` de ventana.
    ContextMenuOpen(f32, f32),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,

    /// Rail hospedado de pata: el usuario activó un diente. `id < tabs.len()`
    /// selecciona esa tab; `MONITORS_TOOTH` togglea el panel de monitores.
    HostActivate(u32),
}

struct Shell;

impl App for Shell {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "shuma"
    }

    fn app_id() -> Option<&'static str> {
        Some("shuma.shell")
    }

    fn initial_size() -> (u32, u32) {
        (1280, 800)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        handle.spawn_periodic(TICK, || Msg::Tick);
        handle.spawn_periodic(SHELL_TICK, || Msg::ShellTick);

        // wawa-config (bus de preferencias del SO) — theme/accent/lang.
        // Lo cargamos antes de armar las instancias para que el primer
        // render ya tenga el theme correcto. El watcher avisa cambios
        // posteriores con `Msg::WawaConfigChanged`.
        let wawa = wawa_config::WawaConfig::load();
        let theme = wawa_config_llimphi::theme_from_wawa(&wawa, &Theme::dark());
        let _ = rimay_localize::set_locale(&wawa.lang);
        let wawa_watcher = {
            let handle = handle.clone();
            wawa_config::ConfigWatcher::spawn(move |cfg| {
                handle.dispatch(Msg::WawaConfigChanged(Box::new(cfg)));
            })
            .ok()
        };

        let cfg = config::ShumaConfig::load_default();
        let topbar = resolve_slot(cfg.topbar.as_ref()).or_else(|| {
            Some(Instance::launcher(
                shuma_module_launcher::State::from_apps_dir(),
            ))
        });
        let bottombar = resolve_slot(cfg.bottombar.as_ref()).or_else(|| {
            Some(Instance::command_bar(
                shuma_module_commandbar::State::default(),
            ))
        });
        let main = resolve_slot(cfg.main.as_ref());

        // La draft (índice 0) + las sesiones persistidas del último arranque.
        let mut sessions = vec![Session::draft()];
        for c in load_sessions() {
            sessions.push(Session::from_config(c));
        }

        // Reactivar containers persistidos: por cada sesión con
        // `use_container=true && container=Some(name)`, dispatchamos
        // `EnsureContainer(name)`. El thread bg corre `podman start name`;
        // si el container existe queda listo, si no, ContainerFailed
        // rebaja la sesión a Local con notice.
        for s in &sessions {
            if s.use_container {
                if let Some(name) = s.container.clone() {
                    handle.dispatch(Msg::EnsureContainer(name));
                }
            }
        }

        // Estado de chrome (paneles + pestaña) del último arranque. El default
        // deja el panel derecho colapsado. La pestaña activa se clampa por si
        // se borraron sesiones desde el último guardado.
        let chrome = load_chrome();
        let active_session = chrome.active_session.min(sessions.len().saturating_sub(1));

        // Rail hospedado: si `SHUMA_DELEGATE_SIDEBAR` está set, prestamos las
        // HERRAMIENTAS de la sesión activa al rail de pata.
        let host = shuma_host(handle);

        Model {
            theme,
            topbar,
            bottombar,
            main,
            sessions,
            active_session,
            hovered_session: None,
            // Panel derecho: lo que se dejó la última vez (default colapsado).
            active_tool: chrome.active_tool,
            // Panel de config (izquierda): idem, default abierto.
            session_panel_open: chrome.session_panel_open,
            dropdown_open: None,
            containers: Vec::new(),
            containers_full: Vec::new(),
            container_cfgs: load_container_cfgs(),
            focused_field: None,
            hosts: hosts::load_hosts(),
            host_draft: None,
            container_draft: None,
            hosts_modal_open: false,
            containers_modal_open: false,
            viewport: (1280.0, 800.0),
            session_w: chrome.session_w,
            sysmon: SystemSampler::new(HISTORY),
            last_snapshot: None,
            monitors_width: chrome.monitors_width,
            extra_history: HashMap::new(),
            extra_display: HashMap::new(),
            _wawa_watcher: wawa_watcher,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            ctx_menu: None,
            _host: host,
        }
    }

    fn on_resize(_model: &Self::Model, width: u32, height: u32) -> Option<Self::Msg> {
        Some(Msg::Resized(width as f32, height as f32))
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Los modales bloqueantes capturan TODO el teclado (prioridad máxima):
        // Esc cierra; el resto va al draft que están editando.
        if model.hosts_modal_open {
            if let llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) = &e.key {
                return Some(Msg::CloseHostsModal);
            }
            return Some(Msg::HostDraftKey(e.clone()));
        }
        if model.containers_modal_open {
            if let llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) = &e.key {
                return Some(Msg::CloseContainersModal);
            }
            return Some(Msg::ContainerDraftKey(e.clone()));
        }
        // Con un campo del form remoto focado, las teclas van ahí (no al shell).
        if model.focused_field.is_some() {
            return Some(Msg::RemoteKey(e.clone()));
        }
        // Con un dropdown de config abierto, Esc lo cierra (no va al shell).
        if model.dropdown_open.is_some() {
            if let llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) = &e.key {
                return Some(Msg::DismissDropdown);
            }
        }
        // Con un menú abierto, Esc lo cierra y se come la tecla (no va al
        // shell). El resto de teclas siguen su curso normal.
        if let Some(msg) = menu::intercept_key(model, e) {
            return Some(msg);
        }
        // Reenvía teclas al módulo focado. Hoy sólo el shell consume
        // teclas (input del REPL); el resto de módulos siguen sin
        // recibirlas hasta que las necesiten.
        forward_key_to_focused_shell(model, e)
    }

    fn on_wheel(
        model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        modifiers: Modifiers,
    ) -> Option<Self::Msg> {
        // Ctrl+rueda = zoom del texto del shell (paso ~10% por click).
        if modifiers.ctrl && delta.y != 0.0 {
            let factor = if delta.y > 0.0 { 1.0 / 1.1 } else { 1.1 };
            return Some(Msg::Module(
                Slot::Session(model.active_session, Which::Shell),
                ModuleMsg::Shell(shuma_module_shell::Msg::ZoomBy(factor)),
            ));
        }
        // Shift+rueda = scroll horizontal del shell (útil cuando zoom-in
        // hace líneas más largas que el viewport). 40 px por click.
        if modifiers.shift && delta.y != 0.0 {
            let dx = delta.y * 40.0;
            return Some(Msg::Module(
                Slot::Session(model.active_session, Which::Shell),
                ModuleMsg::Shell(shuma_module_shell::Msg::ScrollHoriz(dx)),
            ));
        }
        // `delta.y` viene en líneas (positivo = hacia abajo). El scroll
        // del shell mide px desde el fondo, donde positivo = ver
        // historial, así que invertimos y escalamos a ~40 px por línea.
        let dpx = -delta.y * 40.0;
        if dpx == 0.0 {
            return None;
        }
        forward_wheel_to_focused_shell(model, dpx)
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                m.last_snapshot = Some(m.sysmon.sample());
                sample_extra_monitors(&mut m);
            }
            Msg::ShellTick => {
                drain_shell_instances(&mut m);
            }
            Msg::Resized(w, h) => {
                if w > 0.0 && h > 0.0 {
                    m.viewport = (w, h);
                }
            }
            Msg::WawaConfigChanged(cfg) => {
                // Re-armar el theme con el nuevo variant + accent. El
                // fallback es el theme actual — si la nueva config tiene
                // un variant raro, conservamos lo de antes.
                m.theme = wawa_config_llimphi::theme_from_wawa(&cfg, &m.theme);
                // Locale activo — `set_locale` es no-op si el lang no
                // está en el catálogo; los próximos `t(...)` ya devuelven
                // strings en el nuevo idioma sin necesidad de reiniciar
                // (los labels in-memory siguen siendo viejos hasta que
                // el módulo correspondiente vuelva a rehidratarlos,
                // pero todo lo que se calcula en cada `view()` se
                // refresca al instante).
                let _ = rimay_localize::set_locale(&cfg.lang);
            }
            // Click en un diente de sesión: lo selecciona y abre su panel.
            // Re-clickear el activo cierra/abre su panel (cada diente ES su panel).
            Msg::SelectSession(i) => {
                if i < m.sessions.len() {
                    if i == m.active_session {
                        m.session_panel_open = !m.session_panel_open;
                    } else {
                        m.active_session = i;
                        m.session_panel_open = true;
                    }
                    save_chrome(&m);
                }
            }
            // Hover sobre un diente: sólo guarda el índice para el hint de la
            // barra de estado. Efímero, no se persiste ni cambia el foco.
            Msg::HoverSession(idx) => {
                m.hovered_session = idx.filter(|&i| i < m.sessions.len());
            }
            // Click en una herramienta: toggle de su panel (re-click cierra).
            Msg::SelectTool(t) => {
                m.active_tool = if m.active_tool == Some(t) { None } else { Some(t) };
                save_chrome(&m);
            }
            Msg::RunFromHistory(cmd) => {
                let slot = Slot::Session(m.active_session, Which::Shell);
                m = apply_module_msg(
                    m,
                    slot,
                    ModuleMsg::Shell(shuma_module_shell::Msg::InsertAtCursor(cmd)),
                );
            }
            Msg::RunFromHistoryNow(cmd) => {
                let slot = Slot::Session(m.active_session, Which::Shell);
                m = apply_module_msg(
                    m,
                    slot,
                    ModuleMsg::Shell(shuma_module_shell::Msg::RunLine(cmd)),
                );
            }
            Msg::ToggleDropdown(kind) => {
                m.dropdown_open = if m.dropdown_open == Some(kind) { None } else { Some(kind) };
            }
            Msg::DismissDropdown => m.dropdown_open = None,
            // Config del aislamiento. Cambia el aislamiento de la sesión activa
            // y reconstruye su shell con el `Source` correspondiente. La draft
            // ya no se promueve al tocarla — crear sesiones nuevas pasa por el
            // botón `+` (`Msg::OpenNewSessionForm`).
            Msg::SetIsolation(iso) => {
                m.dropdown_open = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.isolation = iso;
                    // Sesiones pending no tienen shell todavía — sólo actualizan el form.
                    if !s.pending {
                        s.apply_isolation();
                    } else {
                        s.conn = match iso {
                            Isolation::Local => ConnState::Connected,
                            Isolation::Remote => ConnState::Pending,
                        };
                    }
                }
                save_sessions(&m);
            }
            // Abrir/cerrar el colapsable de contenedor (capa opcional). Al abrir,
            // listamos los contenedores locales.
            Msg::ToggleContainer => {
                let mut opening = false;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.container_open = !s.container_open;
                    opening = s.container_open;
                }
                if opening {
                    spawn_list_containers(handle);
                }
            }
            Msg::SetDistro(d) => {
                m.dropdown_open = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.distro = d;
                }
                save_sessions(&m);
            }
            Msg::FocusField(f) => {
                m.focused_field = Some(f);
                m.dropdown_open = None;
            }
            Msg::RemoteKey(e) => {
                let Some(f) = m.focused_field else { return m };
                match &e.key {
                    llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) => {
                        m.focused_field = None;
                    }
                    llimphi_ui::Key::Named(llimphi_ui::NamedKey::Enter) => {
                        if let Some(s) = m.sessions.get_mut(m.active_session) {
                            s.connect_remote();
                        }
                        m.focused_field = None;
                        save_sessions(&m);
                    }
                    _ => {
                        if let Some(s) = m.sessions.get_mut(m.active_session) {
                            s.remote_field_mut(f).apply_key(&e);
                        }
                    }
                }
            }
            Msg::ConnectRemote => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.connect_remote();
                }
                m.focused_field = None;
                save_sessions(&m);
            }
            Msg::ReconnectSession(idx) => {
                if let Some(s) = m.sessions.get_mut(idx) {
                    s.reconnect();
                }
                m.focused_field = None;
                save_sessions(&m);
            }
            Msg::RefreshContainers => spawn_list_containers(handle),
            Msg::ContainersLoaded(v) => m.containers = v,
            Msg::SubscribeContainer(i) => {
                m.dropdown_open = None;
                if let Some(name) = m.containers.get(i).cloned() {
                    if let Some(s) = m.sessions.get_mut(m.active_session) {
                        s.container = Some(name);
                        s.conn = ConnState::Connected;
                    }
                }
                save_sessions(&m);
            }
            Msg::CreateContainer => {
                m.dropdown_open = None;
                // Si podman no está instalado, no tiene sentido seguir —
                // mostramos error en lugar de spawnear y dejar la sesión
                // colgada en Pending.
                if !podman_disponible() {
                    if let Some(s) = m.sessions.get_mut(m.active_session) {
                        s.conn = ConnState::Disconnected;
                        let slot = Slot::Session(m.active_session, Which::Shell);
                        m = apply_module_msg(
                            m,
                            slot,
                            ModuleMsg::Shell(shuma_module_shell::Msg::PushNotice(
                                "✘ podman no encontrado en PATH — instalá podman o desactivá 'Aislar en contenedor'".into(),
                            )),
                        );
                    }
                    return m;
                }
                let (distro, n, mount) = m
                    .sessions
                    .get(m.active_session)
                    .map(|s| (s.distro, s.number.unwrap_or(0), s.mount.text()))
                    .unwrap_or((Distro::Ubuntu, 0, String::new()));
                let name = format!("shuma-{}-{n}", distro.label().to_lowercase());
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.container = Some(name.clone());
                    s.use_container = true;
                    s.conn = ConnState::Pending; // hasta que `ContainerCreated`
                    // Aplica isolation YA — el shell se reconstruye con
                    // Source::Container apuntando al name. Los primeros
                    // comandos pueden fallar si podman aún no creó el
                    // container; al llegar `ContainerCreated` ya está vivo.
                    s.apply_isolation();
                }
                let mount_opt = if mount.trim().is_empty() { None } else { Some(mount) };
                spawn_create_container(handle, distro.image(), name, mount_opt);
                save_sessions(&m);
            }
            Msg::ToggleUseContainer => {
                let mut activado = false;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.use_container = !s.use_container;
                    // Si el engine persistido ya no está disponible, lo
                    // refrescamos al preferido en el momento de activar.
                    if s.use_container {
                        activado = true;
                        if let Some(pref) = engine_preferido() {
                            if !binary_disponible(&s.container_engine) {
                                s.container_engine = pref.to_string();
                            }
                        }
                    }
                    // Sesión viva: el cambio debe TOMAR EFECTO ya (antes el
                    // toggle no reconstruía el shell, así que desmarcar dejaba
                    // los comandos corriendo dentro del contenedor igual).
                    // - apagar → soltar el contenedor y volver a shell base.
                    // - encender con uno ya elegido → re-entrar al contenedor.
                    // - encender SIN contenedor elegido → no tocar el shell;
                    //   se reconstruye al elegir uno (PickRootfs/Subscribe).
                    if !s.pending {
                        if !s.use_container {
                            s.container = None;
                            s.apply_isolation();
                        } else if s.container.is_some() {
                            s.apply_isolation();
                        }
                    }
                }
                // Poblar el select de contenedores con los podman existentes.
                if activado {
                    spawn_list_containers(handle);
                }
                save_sessions(&m);
            }
            Msg::SetEngine(name) => {
                m.dropdown_open = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    // Sólo aceptamos engines realmente disponibles.
                    if binary_disponible(&name)
                        || name == "unshare"
                        || name == "bwrap"
                        || name == "podman"
                    {
                        s.container_engine = name;
                    }
                }
            }
            Msg::PickRootfs(distro) => {
                // El usuario eligió un rootfs presente en disco. Asocia
                // a la sesión activa y, si está pending, confirma. Si no,
                // aplica isolation y conecta.
                m.dropdown_open = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.use_container = true;
                    s.distro = distro;
                    if !binary_disponible(&s.container_engine) {
                        if let Some(pref) = engine_preferido() {
                            s.container_engine = pref.to_string();
                        }
                    }
                    let path = rootfs_path_for(distro)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    s.container = Some(path);
                    s.apply_isolation();
                    s.conn = ConnState::Connected;
                    if s.pending {
                        // Si estaba en form de creación, lo confirmamos.
                        s.pending = false;
                        s.pending_focus = None;
                        m.session_panel_open = true;
                    }
                }
                save_sessions(&m);
            }
            Msg::EnsureContainer(name) => {
                // Ramificamos por engine de la sesión que lo pidió. Para
                // unshare/bwrap, el "name" es el path al rootfs — si ya
                // existe en disco, marcamos conectada de una; sino, repull.
                let engine = m
                    .sessions
                    .iter()
                    .find(|s| s.container.as_deref() == Some(name.as_str()))
                    .map(|s| (s.container_engine.clone(), s.distro))
                    .unwrap_or_else(|| ("podman".into(), Distro::Ubuntu));
                match engine.0.as_str() {
                    "unshare" | "bwrap" => {
                        if rootfs_listo(engine.1) {
                            // No bg work — emit el "ya está listo" inline.
                            handle.dispatch(Msg::ContainerCreated(name));
                        } else {
                            spawn_pull_rootfs_lxc(handle, engine.1, None);
                        }
                    }
                    _ /* podman */ => spawn_ensure_container(handle, name),
                }
            }
            Msg::OpenContainersWindow => {
                // Modal bloqueante: abre mostrando la LISTA (sin editor). El
                // editor aparece al tocar "Nuevo" o una fila — así la lista
                // siempre se ve y el "seleccionar para editar arriba" es claro.
                m.containers_modal_open = true;
                m.container_draft = None;
                spawn_list_containers_full(handle);
            }
            Msg::Noop => {}
            Msg::CloseContainersModal => {
                m.containers_modal_open = false;
                m.container_draft = None;
            }
            Msg::ContainersFullLoaded(v) => {
                m.containers_full = v;
            }
            Msg::RefreshContainersFull => spawn_list_containers_full(handle),
            Msg::StartContainer(name) => spawn_container_action(handle, "start", name),
            Msg::StopContainer(name) => spawn_container_action(handle, "stop", name),
            Msg::RemoveContainer(name) => spawn_container_action(handle, "rm", name),
            Msg::RemoveRootfs(name) => spawn_remove_rootfs(handle, name),
            Msg::OpenHostsWindow => {
                // Modal bloqueante: abre mostrando la LISTA. El editor aparece
                // al tocar "Nuevo" o una fila (mismo mecanismo que contenedores).
                m.hosts_modal_open = true;
                m.host_draft = None;
            }
            Msg::CloseHostsModal => {
                m.hosts_modal_open = false;
                m.host_draft = None;
            }
            Msg::HostDraftStart => {
                // "Nuevo": draft fresco (deselecciona la lista).
                m.host_draft = Some(HostDraft::new());
            }
            Msg::HostEdit(idx) => {
                if let Some(h) = m.hosts.get(idx).cloned() {
                    m.host_draft = Some(HostDraft::from_host(&h));
                }
            }
            Msg::HostDraftCancel => {
                m.host_draft = None;
            }
            Msg::HostDraftSave => {
                if let Some(draft) = m.host_draft.clone() {
                    if let Some(h) = draft.to_host() {
                        // Si renombró un host existente, borrar la entrada vieja.
                        if let Some(old) = &draft.editing {
                            if old != &h.name {
                                m.hosts.retain(|x| &x.name != old);
                            }
                        }
                        if let Some(idx) = m.hosts.iter().position(|x| x.name == h.name) {
                            m.hosts[idx] = h.clone();
                        } else {
                            m.hosts.push(h.clone());
                        }
                        hosts::save_hosts(&m.hosts);
                        // Quedamos editando el guardado (remarcado en la lista).
                        m.host_draft = Some(HostDraft::from_host(&h));
                    }
                }
            }
            Msg::HostDraftFocus(f) => {
                if let Some(d) = m.host_draft.as_mut() {
                    d.focused = Some(f);
                }
            }
            Msg::HostDraftKey(e) => {
                if let Some(d) = m.host_draft.as_mut() {
                    let Some(f) = d.focused else { return m };
                    match &e.key {
                        llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) => {
                            d.focused = None;
                        }
                        llimphi_ui::Key::Named(llimphi_ui::NamedKey::Enter) => {
                            handle.dispatch(Msg::HostDraftSave);
                        }
                        llimphi_ui::Key::Named(llimphi_ui::NamedKey::Tab) => {
                            // Tab cicla los campos del draft.
                            let next = match f {
                                HostDraftField::Name => HostDraftField::Host,
                                HostDraftField::Host => HostDraftField::User,
                                HostDraftField::User => HostDraftField::Port,
                                HostDraftField::Port => {
                                    if d.use_password { HostDraftField::Name } else { HostDraftField::Pem }
                                }
                                HostDraftField::Pem => HostDraftField::Name,
                            };
                            d.focused = Some(next);
                        }
                        _ => {
                            let target = match f {
                                HostDraftField::Name => &mut d.name,
                                HostDraftField::Host => &mut d.host,
                                HostDraftField::User => &mut d.user,
                                HostDraftField::Port => &mut d.port,
                                HostDraftField::Pem => &mut d.pem_path,
                            };
                            let _ = target.apply_key(&e);
                        }
                    }
                }
            }
            Msg::HostDraftToggleAuth => {
                if let Some(d) = m.host_draft.as_mut() {
                    d.use_password = !d.use_password;
                }
            }
            Msg::HostDelete(idx) => {
                if idx < m.hosts.len() {
                    m.hosts.remove(idx);
                    hosts::save_hosts(&m.hosts);
                }
            }
            Msg::ContainerDraftNew => {
                // "Nuevo": deselecciona la lista y activa engine + distro,
                // ligado al host de la sesión activa.
                let host = m
                    .sessions
                    .get(m.active_session)
                    .map(|s| s.host_key())
                    .unwrap_or_else(host_local);
                m.container_draft = Some(ContainerDraft::new(host));
            }
            Msg::ContainerDraftCancel => {
                m.container_draft = None;
            }
            Msg::ContainerEdit(idx) => {
                // Elegir una fila (rootfs) de la lista para editarla: engine y
                // distro quedan readonly; los mounts, editables.
                if let Some(info) = m.containers_full.get(idx) {
                    if info.rootfs {
                        let name = info.name.clone();
                        let host = m
                            .sessions
                            .get(m.active_session)
                            .map(|s| s.host_key())
                            .unwrap_or_else(host_local);
                        let cfg = m
                            .container_cfgs
                            .iter()
                            .find(|c| c.name == name)
                            .cloned()
                            .unwrap_or_else(|| ContainerCfg {
                                name: name.clone(),
                                host,
                                engine: engine_preferido().unwrap_or("unshare").to_string(),
                                distro: distro_from_name(&name).unwrap_or(Distro::Ubuntu),
                                mounts: Vec::new(),
                            });
                        m.container_draft = Some(ContainerDraft::from_cfg(&cfg));
                    }
                }
            }
            Msg::ContainerDraftSetEngine(name) => {
                if let Some(d) = m.container_draft.as_mut() {
                    if d.editing.is_none() {
                        d.engine = name; // readonly al editar
                    }
                }
            }
            Msg::ContainerDraftSetDistro(distro) => {
                if let Some(d) = m.container_draft.as_mut() {
                    if d.editing.is_none() {
                        d.distro = distro; // readonly al editar
                    }
                }
            }
            Msg::ContainerDraftAddMount => {
                if let Some(d) = m.container_draft.as_mut() {
                    d.mounts.push(MountDraft::new());
                    d.focus = Some((d.mounts.len() - 1, MountCol::Host));
                }
            }
            Msg::ContainerDraftRemoveMount(i) => {
                if let Some(d) = m.container_draft.as_mut() {
                    if i < d.mounts.len() {
                        d.mounts.remove(i);
                        d.focus = None;
                    }
                }
            }
            Msg::ContainerDraftToggleMountRo(i) => {
                if let Some(d) = m.container_draft.as_mut() {
                    if let Some(md) = d.mounts.get_mut(i) {
                        md.readonly = !md.readonly;
                    }
                }
            }
            Msg::ContainerDraftFocusMount(i, col) => {
                if let Some(d) = m.container_draft.as_mut() {
                    if i < d.mounts.len() {
                        d.focus = Some((i, col));
                    }
                }
            }
            Msg::ContainerDraftSave => {
                if let Some(d) = m.container_draft.clone() {
                    let nuevo = d.editing.is_none();
                    // Clave: si edita, el name existente; si es nuevo, el de la
                    // distro (rootfs) o un nombre podman único.
                    let name = d.editing.clone().unwrap_or_else(|| {
                        if matches!(d.engine.as_str(), "unshare" | "bwrap") {
                            d.distro.label().to_lowercase()
                        } else {
                            (1..1000)
                                .map(|n| format!("shuma-{}-{n}", d.distro.label().to_lowercase()))
                                .find(|cand| !m.container_cfgs.iter().any(|c| &c.name == cand))
                                .unwrap_or_else(|| format!("shuma-{}", d.distro.label().to_lowercase()))
                        }
                    });
                    let cfg = d.to_cfg(name.clone());
                    if let Some(slot) = m.container_cfgs.iter_mut().find(|c| c.name == name) {
                        *slot = cfg.clone();
                    } else {
                        m.container_cfgs.push(cfg.clone());
                    }
                    save_container_cfgs(&m.container_cfgs);
                    // Si es NUEVO, crear el recurso real.
                    if nuevo {
                        match d.engine.as_str() {
                            "unshare" | "bwrap" => {
                                if !rootfs_listo(d.distro) {
                                    spawn_pull_rootfs_lxc(handle, d.distro, None);
                                }
                            }
                            _ /* podman */ => {
                                spawn_create_container(handle, d.distro.image(), name.clone(), None);
                            }
                        }
                    }
                    // Quedamos EDITANDO el guardado (remarcado en la lista).
                    m.container_draft = Some(ContainerDraft::from_cfg(&cfg));
                    spawn_list_containers_full(handle);
                }
            }
            Msg::ContainerDraftKey(e) => {
                if let Some(d) = m.container_draft.as_mut() {
                    let Some((idx, col)) = d.focus else { return m };
                    match &e.key {
                        llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) => {
                            d.focus = None;
                        }
                        llimphi_ui::Key::Named(llimphi_ui::NamedKey::Enter) => {
                            handle.dispatch(Msg::ContainerDraftSave);
                        }
                        _ => {
                            if let Some(md) = d.mounts.get_mut(idx) {
                                let input = match col {
                                    MountCol::Host => &mut md.host,
                                    MountCol::Target => &mut md.target,
                                };
                                let _ = input.apply_key(&e);
                            }
                        }
                    }
                }
            }
            Msg::PickHost(choice) => {
                m.dropdown_open = None;
                let host = choice.and_then(|i| m.hosts.get(i).cloned());
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    // Cambiar de host invalida el contenedor (es de OTRO host).
                    s.container = None;
                    match host {
                        None => {
                            // Local.
                            s.isolation = Isolation::Local;
                            s.host_label = None;
                            if !s.pending {
                                s.apply_isolation();
                            } else {
                                s.conn = ConnState::Connected;
                            }
                        }
                        Some(h) => {
                            s.isolation = Isolation::Remote;
                            s.host_label = Some(h.name.clone());
                            s.host.set_text(h.host);
                            s.user.set_text(h.user);
                            s.port.set_text(h.port.to_string());
                            if !s.pending {
                                s.connect_remote();
                            } else {
                                s.conn = ConnState::Pending;
                            }
                        }
                    }
                }
                save_sessions(&m);
            }
            Msg::HostApply(idx) => {
                m.dropdown_open = None;
                let h = match m.hosts.get(idx).cloned() {
                    Some(h) => h,
                    None => return m,
                };
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.isolation = Isolation::Remote;
                    s.host_label = Some(h.name.clone());
                    s.host.set_text(h.host);
                    s.user.set_text(h.user);
                    s.port.set_text(h.port.to_string());
                    if !s.pending {
                        s.connect_remote();
                    }
                }
                save_sessions(&m);
            }
            Msg::ContainerCreated(name) => {
                // Encontrar la sesión que está esperando este contenedor y
                // montarle el shell con `Source::Container`.
                let idx = m
                    .sessions
                    .iter()
                    .position(|s| s.container.as_deref() == Some(name.as_str()));
                if let Some(i) = idx {
                    if let Some(s) = m.sessions.get_mut(i) {
                        s.conn = ConnState::Connected;
                        if s.use_container && !s.pending {
                            s.apply_isolation();
                        }
                    }
                }
                // Re-listar para reflejar el nuevo contenedor en el dropdown.
                spawn_list_containers(handle);
                save_sessions(&m);
            }
            Msg::ContainerFailed { name, reason } => {
                // Rebajamos a Local: si dejábamos `use_container=true` con
                // `container=Some(name)`, el shell seguiría disparando
                // `podman exec` que falla por cada comando — peor que
                // simplemente caer a Local con una notice clara.
                let idx = m
                    .sessions
                    .iter()
                    .position(|s| s.container.as_deref() == Some(name.as_str()));
                if let Some(i) = idx {
                    // El verbo del aviso depende del engine real: unshare/bwrap
                    // bajan un rootfs; podman/docker corren `run`. Hardcodear
                    // "podman" mentía cuando el engine era otro.
                    let engine = m
                        .sessions
                        .get(i)
                        .map(|s| s.container_engine.clone())
                        .unwrap_or_default();
                    let accion = match engine.as_str() {
                        "unshare" | "bwrap" => "la descarga del rootfs",
                        other if !other.is_empty() => "el arranque del contenedor",
                        _ => "el contenedor",
                    };
                    if let Some(s) = m.sessions.get_mut(i) {
                        s.conn = ConnState::Disconnected;
                        s.container = None;
                        s.use_container = false;
                        s.apply_isolation(); // shell vuelve a Source::Local
                    }
                    let slot = Slot::Session(i, Which::Shell);
                    m = apply_module_msg(
                        m,
                        slot,
                        ModuleMsg::Shell(shuma_module_shell::Msg::PushNotice(format!(
                            "✘ {accion} ({engine}) falló: {reason} — caí a shell local."
                        ))),
                    );
                }
                save_sessions(&m);
            }
            Msg::CloseSession(idx) => {
                // La draft (0) no se cierra; las demás se descartan.
                if idx > 0 && idx < m.sessions.len() {
                    m.sessions.remove(idx);
                    m.active_session = m.active_session.min(m.sessions.len() - 1);
                }
                save_sessions(&m);
                save_chrome(&m);
            }
            Msg::OpenNewSessionForm => {
                // Numero la nueva por la cantidad de sesiones reales + 1.
                let n = m.sessions.iter().filter(|s| s.number.is_some()).count() as u32 + 1;
                let mut s = Session::new_pending(n);
                s.pending_focus = Some(PendingField::Mount);
                m.sessions.push(s);
                m.active_session = m.sessions.len() - 1;
                m.session_panel_open = false; // el form grande absorbe la config
                save_chrome(&m);
            }
            Msg::ConfirmNewSession => {
                // Resuelve engine + arma el container. Plan se ejecuta tras
                // setear el state de la sesión.
                enum CreatePlan {
                    Rootfs { distro: Distro, mount: Option<String> },
                    Podman { image: &'static str, name: String, mount: Option<String> },
                    /// Container podman ya creado (en el modal): sólo asegurar
                    /// que arranque, sin recrearlo con otro nombre.
                    PodmanEnsure { name: String },
                }
                let mut plan: Option<CreatePlan> = None;
                let mut notice: Option<String> = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    if s.pending {
                        s.pending = false;
                        s.pending_focus = None;
                        if s.use_container {
                            // Resolver engine: usar el que el usuario eligió
                            // en el form si está disponible; sino, el
                            // preferido del sistema. Si no hay ninguno,
                            // rebajar a Local con notice.
                            let chosen: Option<String> = if binary_disponible(&s.container_engine) {
                                Some(s.container_engine.clone())
                            } else {
                                engine_preferido().map(|e| e.to_string())
                            };
                            match chosen.as_deref() {
                                None => {
                                    s.use_container = false;
                                    s.container = None;
                                    notice = Some(
                                        "✘ ningún engine de aislamiento está disponible (faltan `unshare`/`bwrap`/`podman`). Arrancó como shell local.".into(),
                                    );
                                }
                                Some("unshare") | Some("bwrap") => {
                                    let engine = chosen.unwrap();
                                    s.container_engine = engine.clone();
                                    let mount = s.mount.text();
                                    let mount_opt =
                                        if mount.trim().is_empty() { None } else { Some(mount) };
                                    // Respetar el rootfs ya ligado (creado/elegido
                                    // en el modal); sólo caer al rootfs default de
                                    // la distro si la sesión no tiene ninguno.
                                    let via_modal = s.container.is_some();
                                    if s.container.is_none() {
                                        let path = rootfs_path_for(s.distro)
                                            .map(|p| p.display().to_string())
                                            .unwrap_or_default();
                                        s.container = Some(path);
                                    }
                                    if rootfs_listo(s.distro) {
                                        s.conn = ConnState::Connected;
                                    } else {
                                        // No listo. Si vino del modal, la descarga
                                        // ya está en vuelo (la lanzó el modal) y
                                        // `ContainerCreated` conectará — no la
                                        // dupliquemos. Si no, la arrancamos acá.
                                        s.conn = ConnState::Pending;
                                        if !via_modal {
                                            plan = Some(CreatePlan::Rootfs {
                                                distro: s.distro,
                                                mount: mount_opt,
                                            });
                                        }
                                    }
                                }
                                Some(_) /* "podman" */ => {
                                    s.container_engine = "podman".into();
                                    s.conn = ConnState::Pending;
                                    let mount = s.mount.text();
                                    let mount_opt =
                                        if mount.trim().is_empty() { None } else { Some(mount) };
                                    match s.container.clone() {
                                        // Ya creado/seleccionado en el modal:
                                        // asegurarlo, no recrear con otro nombre.
                                        Some(name) => {
                                            plan = Some(CreatePlan::PodmanEnsure { name });
                                        }
                                        None => {
                                            let n = s.number.unwrap_or(0);
                                            let name = format!(
                                                "shuma-{}-{n}",
                                                s.distro.label().to_lowercase()
                                            );
                                            s.container = Some(name.clone());
                                            plan = Some(CreatePlan::Podman {
                                                image: s.distro.image(),
                                                name,
                                                mount: mount_opt,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        s.apply_isolation();
                        // `apply_isolation` deja conn=Pending para todo
                        // use_container; un rootfs unshare/bwrap ya presente está
                        // listo de una, así que lo marcamos conectado.
                        if s.use_container
                            && matches!(s.container_engine.as_str(), "unshare" | "bwrap")
                            && rootfs_listo(s.distro)
                        {
                            s.conn = ConnState::Connected;
                        }
                        if s.isolation == Isolation::Remote {
                            if !s.host.text().trim().is_empty() && !s.user.text().trim().is_empty() {
                                s.connect_remote();
                            }
                        }
                        m.session_panel_open = true;
                    }
                }
                if let Some(text) = notice {
                    let slot = Slot::Session(m.active_session, Which::Shell);
                    m = apply_module_msg(
                        m,
                        slot,
                        ModuleMsg::Shell(shuma_module_shell::Msg::PushNotice(text)),
                    );
                }
                match plan {
                    Some(CreatePlan::Rootfs { distro, mount }) => {
                        let slot = Slot::Session(m.active_session, Which::Shell);
                        m = apply_module_msg(
                            m,
                            slot,
                            ModuleMsg::Shell(shuma_module_shell::Msg::PushNotice(format!(
                                "⬇ descargando rootfs LXC ({}) — ~50 MB, esto tarda unos segundos…",
                                distro.label()
                            ))),
                        );
                        spawn_pull_rootfs_lxc(handle, distro, mount);
                    }
                    Some(CreatePlan::Podman { image, name, mount }) => {
                        spawn_create_container(handle, image, name, mount);
                    }
                    Some(CreatePlan::PodmanEnsure { name }) => {
                        spawn_ensure_container(handle, name);
                    }
                    None => {}
                }
                save_sessions(&m);
                save_chrome(&m);
            }
            Msg::CancelNewSession => {
                if let Some(s) = m.sessions.get(m.active_session) {
                    if s.pending {
                        let idx = m.active_session;
                        m.sessions.remove(idx);
                        m.active_session = m.active_session.min(m.sessions.len().saturating_sub(1));
                    }
                }
                save_chrome(&m);
            }
            Msg::FocusPendingField(f) => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.pending_focus = Some(f);
                }
                m.dropdown_open = None;
            }
            Msg::PendingKey(e) => {
                let Some(s) = m.sessions.get_mut(m.active_session) else {
                    return m;
                };
                let Some(f) = s.pending_focus else { return m };
                match &e.key {
                    llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) => {
                        s.pending_focus = None;
                    }
                    llimphi_ui::Key::Named(llimphi_ui::NamedKey::Enter) => {
                        // Enter en el form = confirmar la sesión (siguiente tick).
                        handle.dispatch(Msg::ConfirmNewSession);
                    }
                    _ => match f {
                        PendingField::Mount => {
                            let _ = s.mount.apply_key(&e);
                        }
                    },
                }
            }
            Msg::ReorderSession(from, to) => {
                // La draft (0) queda fija; el resto se reordena.
                let len = m.sessions.len();
                if from > 0 && from < len && to > 0 && to < len && from != to {
                    let s = m.sessions.remove(from);
                    m.sessions.insert(to, s);
                    m.active_session = to;
                }
                save_sessions(&m);
                save_chrome(&m);
            }
            Msg::SetSessionWidth(dx) => {
                m.session_w = (m.session_w + dx).clamp(180.0, 480.0);
                // Persistir la geometría: el splitter reabre donde lo dejaste.
                save_chrome(&m);
            }
            Msg::SetToolWidth(dx) => {
                m.monitors_width = (m.monitors_width - dx).clamp(180.0, 480.0);
                save_chrome(&m);
            }
            Msg::Module(slot, mmsg) => {
                // Hook: SelectRoot del módulo minga dispara la carga
                // de la fuente reconstruida en un thread aparte. El
                // mensaje se sigue propagando para que el state marque
                // `selected = Some(alpha)` y `selected_source = None`
                // mientras carga.
                if let ModuleMsg::Minga(shuma_module_minga::Msg::SelectRoot(alpha)) = &mmsg {
                    if let Some(repo_path) = minga_repo_path(&slot, &m) {
                        let alpha = *alpha;
                        let slot_back = slot.clone();
                        handle.spawn(move || {
                            let result = shuma_module_minga::load_root_source(&repo_path, alpha);
                            Msg::Module(
                                slot_back,
                                ModuleMsg::Minga(shuma_module_minga::Msg::SourceLoaded {
                                    alpha,
                                    result,
                                }),
                            )
                        });
                    }
                }
                m = apply_module_msg(m, slot, mmsg);
            }
            Msg::ShortcutClicked(slot, action) => {
                m = handle_shortcut(m, slot, action, handle);
            }
            Msg::MenuOpen(idx) => {
                m.menu_open = idx;
                m.menu_active = usize::MAX;
                // Abrir el menú principal cierra el contextual (y viceversa).
                m.ctx_menu = None;
                // Animación de aparición/swap: cada vez que se abre (o se
                // cambia de) menú, el dropdown se funde+desliza de nuevo.
                if idx.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = m.menu_open {
                    let menu = menu::app_menu(&m);
                    m.menu_active =
                        llimphi_widget_menubar::menubar_nav(&menu, mi, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = m.menu_open {
                    let menu = menu::app_menu(&m);
                    if let Some(cmd) =
                        llimphi_widget_menubar::menubar_command_at(&menu, mi, m.menu_active)
                    {
                        m = menu::handle_command(m, &cmd);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::ContextMenuOpen(x, y) => {
                m.ctx_menu = Some((x, y));
                m.menu_open = None;
                m.menu_active = usize::MAX;
            }
            Msg::CloseMenus => {
                m.menu_open = None;
                m.menu_active = usize::MAX;
                m.ctx_menu = None;
            }
            Msg::MenuCommand(cmd) => {
                m = menu::handle_command(m, &cmd);
            }
            Msg::HostActivate(id) => {
                // Rail hospedado: un diente de herramienta abre/cierra su panel.
                if let Some(t) = Tool::ALL.get(id as usize) {
                    m.active_tool = if m.active_tool == Some(*t) { None } else { Some(*t) };
                    save_chrome(&m);
                }
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = &model.theme;

        let menubar = menu::menubar_row(model, theme);
        let topbar = render_topbar(model, theme);
        let main_area = render_main_area(model, theme);
        let bottombar = render_bottombar(model, theme);

        // El right-click se engancha en la raíz (origen 0,0 → las coords
        // locales que llegan al handler ya son de ventana) y abre el menú
        // contextual de terminal. Un nodo hijo con su propio handler de
        // right-click ganaría; hoy ninguno lo pone, así que la raíz es el
        // catch-all.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, topbar, main_area, bottombar])
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // Prioridad: diálogo bloqueante (modal) > dropdown de select > menú.
        if model.hosts_modal_open {
            return Some(view::hosts_modal(model, &model.theme));
        }
        if model.containers_modal_open {
            return Some(view::containers_modal(model, &model.theme));
        }
        view::dropdown_overlay(model).or_else(|| menu::overlay(model))
    }
}

// Helpers partidos del monolito (regla dura #1, 1522 LOC): update + view.
mod menu;
mod update;
mod view;

use update::*;
use view::*;
