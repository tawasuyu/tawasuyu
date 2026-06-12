use super::*;

// --- Mounts de contenedor (espejo mínimo de containers.json) ----------------

#[derive(serde::Deserialize)]
pub(crate) struct ContainerCfgJson {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) mounts: Vec<MountJson>,
}

#[derive(serde::Deserialize, Clone)]
pub(crate) struct MountJson {
    pub(crate) host: String,
    pub(crate) target: String,
    #[serde(default)]
    pub(crate) readonly: bool,
}

/// Mounts configurados para el rootfs en `rootfs_path` (su basename es la clave
/// en `containers.json`). Vacío si no hay config.
pub(crate) fn container_mounts(rootfs_path: &str) -> Vec<MountJson> {
    let name = match std::path::Path::new(rootfs_path).file_name() {
        Some(n) => n.to_string_lossy().to_string(),
        None => return Vec::new(),
    };
    let Some(base) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    else {
        return Vec::new();
    };
    let path = base.join("shuma").join("containers.json");
    let Ok(txt) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(cfgs) = serde_json::from_str::<Vec<ContainerCfgJson>>(&txt) else {
        return Vec::new();
    };
    cfgs.into_iter()
        .find(|c| c.name == name)
        .map(|c| c.mounts)
        .unwrap_or_default()
}

/// Si `line` es un pipe «simple» de ≥2 etapas —sólo `Command`/`Argument`/
/// `Flag`/`Pipe`/espacio, sin comillas, variables, redirecciones,
/// operadores, globs (`* ? [ ] { }`) ni `~`— devuelve sus etapas como
/// [`StageSpec`] para correrlo por `Exec::Direct`. Si no, `None` (cae a
/// `sh -c`, que sí absorbe esa sintaxis). Un único comando también cae a
/// `sh -c`: el modo directo sólo aporta cuando hay tubería que interceptar.
///
/// Sólo toca la primera ocurrencia al principio del line — pipes / `&&` /
/// `;` van por su cuenta (el shell del PTY los maneja).
pub(crate) fn inject_askpass(line: &str) -> String {
    let trimmed = line.trim_start();
    let lead_len = line.len() - trimmed.len();
    let Some(rest_after_sudo) = trimmed.strip_prefix("sudo") else {
        return line.to_string();
    };
    // Exigir que `sudo` sea palabra completa (siguiente char espacio / EOL).
    let next = rest_after_sudo.chars().next();
    if !matches!(next, None | Some(' ') | Some('\t')) {
        return line.to_string();
    }
    // Heurística simple: si los tokens del comando contienen -A/-S/--askpass/
    // --stdin antes de cualquier `;|&` o salto de pipe, dejarlo como está.
    for tok in rest_after_sudo.split_whitespace() {
        if tok == "-A" || tok == "-S" || tok == "--askpass" || tok == "--stdin" {
            return line.to_string();
        }
        // Llegamos a un argumento que no es flag → dejamos de buscar (es
        // el comando ejecutado por sudo y sus flags son suyos).
        if !tok.starts_with('-') {
            break;
        }
    }
    let lead = &line[..lead_len];
    format!("{lead}sudo -A{rest_after_sudo}")
}

/// Envuelve `spec` en la invocación del **engine de aislamiento** elegido.
///
/// - `engine = "podman"` / `"docker"`: `name` es el nombre del container ya
///   creado; corremos `<engine> exec -i <name> bash -c <line>`.
/// - `engine = "bwrap"`: `name` es el PATH al rootfs en disco
///   (`~/.local/share/shuma/rootfs/<distro>`); corremos `bwrap` con los
///   binds estándar y `bash -c <line>` adentro. No requiere config
///   global — sólo el binario `bwrap` instalado.
///
/// En ambos casos el proceso hijo que ve `shuma-exec` sigue siendo local —
/// Comilla simple POSIX-segura: envuelve `s` en `'…'` escapando comillas
/// internas (`'` → `'\''`). Para componer el comando que viaja por SSH.
pub(crate) fn sh_squote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Comando a ejecutar **en el host remoto** para correr `line` dentro de un
/// contenedor de ese host. Para podman/docker entra con `<engine> exec`; para
/// un rootfs (unshare/bwrap) hace `chroot` al path. El `cwd` interior se aplica
/// con un `cd` dentro del shell del contenedor (no del host) — por eso
/// `start_run` le pasa "~" a `run_ssh`, para no anteponer un `cd` del host.
pub(crate) fn remote_container_command(line: &str, engine: &str, name: &str, cwd: &str) -> String {
    // El cwd interior sólo tiene sentido si es absoluto; si el dir no existe,
    // el `2>/dev/null` evita romper el comando.
    let inner = if cwd.starts_with('/') {
        format!("cd {} 2>/dev/null; {line}", sh_squote(cwd))
    } else {
        line.to_string()
    };
    match engine {
        // rootfs en el remoto: chroot al path (requiere privilegios allá).
        "unshare" | "bwrap" => format!(
            "chroot {} /bin/sh -lc {}",
            sh_squote(name),
            sh_squote(&inner)
        ),
        // podman/docker: exec contra el contenedor vivo.
        eng => format!(
            "{eng} exec -i {} /bin/sh -lc {}",
            sh_squote(name),
            sh_squote(&inner)
        ),
    }
}

/// reusamos la maquinaria de PTY / capture / kill de `Source::Local`.
pub(crate) fn wrap_spec_for_container(mut spec: CommandSpec, engine: &str, name: &str) -> CommandSpec {
    if engine == "unshare" {
        return wrap_spec_for_unshare(spec, name);
    }
    if engine == "bwrap" {
        return wrap_spec_for_bwrap(spec, name);
    }
    let eng = engine.to_string();
    let nm = name.to_string();
    spec.exec = match spec.exec {
        Exec::Shell { line, program } => {
            // bash local que dispara `engine exec` con `program -c "line"`
            // adentro. Mantenemos Exec::Shell para preservar captura por
            // líneas (no PTY).
            let inner = format!(
                "{eng} exec -i {nm} {prog} -c {q}",
                eng = shell_quote(&eng),
                nm = shell_quote(&nm),
                prog = shell_quote(&program),
                q = shell_quote(&line),
            );
            Exec::Shell {
                line: inner,
                program: "bash".into(),
            }
        }
        Exec::Pty { program, args, cols, rows } => {
            // PTY local que ejecuta `engine exec -it name <program> <args...>`.
            let mut new_args = vec!["exec".to_string(), "-it".into(), nm, program];
            new_args.extend(args);
            Exec::Pty {
                program: eng,
                args: new_args,
                cols,
                rows,
            }
        }
        Exec::Direct { stages } => {
            // Reconstruimos la pipe como una sola line bash y la disparamos
            // dentro del contenedor; perdemos la captura de etapas (tee) —
            // tradeoff aceptable para el MVP del cableo container.
            let mut line = String::new();
            for (i, st) in stages.iter().enumerate() {
                if i > 0 {
                    line.push_str(" | ");
                }
                line.push_str(&shell_quote(&st.program));
                for a in &st.args {
                    line.push(' ');
                    line.push_str(&shell_quote(a));
                }
            }
            let inner = format!(
                "{eng} exec -i {nm} bash -c {q}",
                eng = shell_quote(&eng),
                nm = shell_quote(&nm),
                q = shell_quote(&line),
            );
            Exec::Shell {
                line: inner,
                program: "bash".into(),
            }
        }
    };
    spec
}

/// Script `sh` (`$1 = rootfs_path`, `$2 = línea bash`) que aísla por `unshare`
/// + `chroot`. Monta `/proc`, `/dev`, `/sys` y bind-mountea `/etc/resolv.conf`
/// del host (para que apt/pacman alcancen la red) + los directorios que el
/// usuario configuró en el gestor, cada uno en su `target` (ro o rw).
///
/// El `|| true` tras cada `mount` evita abortar si ya había algo montado
/// (re-entry tras crash) o un dir no existe.
pub(crate) fn unshare_script(mounts: &[MountJson]) -> String {
    let mut s = String::from(
        "mount -t proc proc \"$1/proc\" 2>/dev/null || true; \
         mount --bind /dev \"$1/dev\" 2>/dev/null || true; \
         mount --bind /sys \"$1/sys\" 2>/dev/null || true; \
         mount --bind /etc/resolv.conf \"$1/etc/resolv.conf\" 2>/dev/null || true; ",
    );
    for m in mounts {
        if m.host.trim().is_empty() || m.target.trim().is_empty() {
            continue;
        }
        let hq = shell_quote(&m.host);
        let tq = shell_quote(&m.target);
        s.push_str(&format!("mkdir -p \"$1\"{tq} 2>/dev/null || true; "));
        s.push_str(&format!("mount --bind {hq} \"$1\"{tq} 2>/dev/null || true; "));
        if m.readonly {
            s.push_str(&format!(
                "mount -o remount,bind,ro \"$1\"{tq} 2>/dev/null || true; "
            ));
        }
    }
    s.push_str("exec chroot \"$1\" /bin/bash -c \"$2\"");
    s
}

/// Variante de [`wrap_spec_for_container`] para `engine = "unshare"`. El
/// `rootfs_path` es un filesystem extraído en disco local; `unshare -r`
/// + `chroot` lo activan sin necesidad de root ni bwrap ni podman — sólo
/// requiere `util-linux` + `coreutils` (instalados en todo Linux moderno).
///
/// Funciona en distros con `kernel.unprivileged_userns_clone = 1` (default
/// en kernels >= 5.10 mayoritarios). Si está deshabilitado, el `unshare -r`
/// falla con "Operation not permitted" y el caller verá el stderr en el
/// notice.
pub(crate) fn wrap_spec_for_unshare(mut spec: CommandSpec, rootfs_path: &str) -> CommandSpec {
    fn base_args(rootfs: &str, inner_line: &str, script: &str) -> Vec<String> {
        vec![
            "-r".into(),       // map root in user ns
            "-m".into(),       // mount ns (para mount -t proc etc.)
            "-u".into(),       // uts ns
            "-i".into(),       // ipc ns
            "-p".into(),       // pid ns
            "-f".into(),       // fork (necesario con -p)
            "--kill-child".into(), // los hijos mueren con el padre
            "--".into(),
            "/bin/sh".into(), "-c".into(), script.to_string(),
            "_".into(),                  // $0
            rootfs.to_string(),          // $1
            inner_line.to_string(),      // $2
        ]
    }
    let rootfs = rootfs_path.to_string();
    // Script con los binds estándar + los directorios montados por el usuario
    // (de containers.json). El basename del rootfs es la clave de config.
    let script = unshare_script(&container_mounts(rootfs_path));
    // Prefijo común para TODO comando dentro del contenedor: HOME del root y
    // `cd` al cwd interior que trackea shuma (`spec.cwd`). Sin esto el comando
    // corría en `/` con PWD heredado del host → `pwd`/`ls`/el prompt se
    // contradecían. `|| true` para no abortar si el dir no existe (el comando
    // igual reporta su propio error).
    let prelude = format!(
        "export HOME=/root; cd {} 2>/dev/null || true; ",
        shell_quote(&spec.cwd)
    );
    spec.exec = match spec.exec {
        Exec::Shell { line, program: _ } => {
            // No-TUI: corremos `unshare` como UNA etapa `Exec::Direct` para
            // capturar stdout/stderr por líneas y renderizarlas como bloques,
            // igual que un comando local. (Antes se forzaba `Exec::Pty`, pero
            // sin `TuiSession` el drenado descartaba los `Bytes` del PTY → el
            // comando corría sin mostrar NADA, con la card en verde/✘ sin
            // motivo. Los TUI fullscreen sí van por la rama `Exec::Pty` de
            // abajo, que sí trae su emulador.)
            let inner = format!("{prelude}{line}");
            let args = base_args(&rootfs, &inner, &script);
            Exec::Direct { stages: vec![StageSpec { program: "unshare".into(), args }] }
        }
        Exec::Pty { program, args, cols, rows } => {
            // Para Exec::Pty (TUI fullscreen tipo vim) armamos el `bash -c`
            // con el program + args ya quoteados.
            let mut inner = prelude.clone();
            inner.push_str(&shell_quote(&program));
            for a in &args {
                inner.push(' ');
                inner.push_str(&shell_quote(a));
            }
            let args = base_args(&rootfs, &inner, &script);
            Exec::Pty { program: "unshare".into(), args, cols, rows }
        }
        Exec::Direct { stages } => {
            let mut line = String::new();
            for (i, st) in stages.iter().enumerate() {
                if i > 0 {
                    line.push_str(" | ");
                }
                line.push_str(&shell_quote(&st.program));
                for a in &st.args {
                    line.push(' ');
                    line.push_str(&shell_quote(a));
                }
            }
            // Pipe simple → también por `Exec::Direct` (captura por líneas).
            let inner = format!("{prelude}{line}");
            let args = base_args(&rootfs, &inner, &script);
            Exec::Direct { stages: vec![StageSpec { program: "unshare".into(), args }] }
        }
    };
    spec
}

/// Args base de bwrap para correr un comando dentro de `rootfs_path`. La
/// idea: aislar mount/pid/uts/ipc pero **compartir net** del host (para
/// que `apt update`, `pacman -Sy`, etc. lleguen al mundo). El `/work`
/// queda como bind del cwd del host cuando aplica.
pub(crate) fn bwrap_args(rootfs_path: &str) -> Vec<String> {
    let mut a: Vec<String> = vec![
        // Root del container.
        "--bind".into(), rootfs_path.into(), "/".into(),
        // Filesystems internos.
        "--proc".into(), "/proc".into(),
        "--dev".into(), "/dev".into(),
        "--tmpfs".into(), "/tmp".into(),
        // DNS funcional: copia el resolv.conf del host (ro).
        "--ro-bind-try".into(), "/etc/resolv.conf".into(), "/etc/resolv.conf".into(),
        // Aislamiento: namespaces propios menos net (compartido).
        "--unshare-pid".into(),
        "--unshare-uts".into(),
        "--unshare-ipc".into(),
        // El process tree muere si el padre muere — no quedan zombies.
        "--die-with-parent".into(),
        // Env mínimo razonable para un shell vacío.
        "--setenv".into(), "HOME".into(), "/root".into(),
        "--setenv".into(), "USER".into(), "root".into(),
        "--setenv".into(), "PATH".into(),
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
        "--setenv".into(), "TERM".into(), "xterm-256color".into(),
    ];
    // Directorios montados por el usuario (containers.json): `--ro-bind` o
    // `--bind` del host a su `target` dentro del contenedor.
    for m in container_mounts(rootfs_path) {
        if m.host.trim().is_empty() || m.target.trim().is_empty() {
            continue;
        }
        a.push(if m.readonly { "--ro-bind".into() } else { "--bind".into() });
        a.push(m.host.clone());
        a.push(m.target.clone());
    }
    // Si existe ~/work en el rootfs, lo usamos como cwd; sino /.
    a.push("--chdir".into());
    a.push("/".into());
    a
}

/// Variante de [`wrap_spec_for_container`] para `engine = "bwrap"`. El
/// `rootfs_path` es el filesystem extraído (LXC image) en disco local.
pub(crate) fn wrap_spec_for_bwrap(mut spec: CommandSpec, rootfs_path: &str) -> CommandSpec {
    let base = bwrap_args(rootfs_path);
    // Igual que unshare: HOME del root + `cd` al cwd interior trackeado.
    let prelude = format!(
        "export HOME=/root; cd {} 2>/dev/null || true; ",
        shell_quote(&spec.cwd)
    );
    spec.exec = match spec.exec {
        Exec::Shell { line, program: _ } => {
            // No-TUI → `Exec::Direct` (una etapa bwrap) para capturar
            // stdout/stderr por líneas y renderizar como bloques. Forzar PTY
            // sin TuiSession descartaba el output (ver wrap_spec_for_unshare).
            // Los TUI fullscreen van por la rama `Exec::Pty` de abajo.
            let mut args = base;
            args.push("--".into());
            args.push("bash".into());
            args.push("-c".into());
            args.push(format!("{prelude}{line}"));
            Exec::Direct { stages: vec![StageSpec { program: "bwrap".into(), args }] }
        }
        Exec::Pty { program, args, cols, rows } => {
            // TUI: envolvemos en `bash -c` para poder hacer el `cd` interior.
            let mut inner = prelude.clone();
            inner.push_str("exec ");
            inner.push_str(&shell_quote(&program));
            for a in &args {
                inner.push(' ');
                inner.push_str(&shell_quote(a));
            }
            let mut new_args = base;
            new_args.push("--".into());
            new_args.push("bash".into());
            new_args.push("-c".into());
            new_args.push(inner);
            Exec::Pty {
                program: "bwrap".into(),
                args: new_args,
                cols,
                rows,
            }
        }
        Exec::Direct { stages } => {
            // Serialize pipe as a single bash line (mismo tradeoff que podman).
            let mut line = String::new();
            for (i, st) in stages.iter().enumerate() {
                if i > 0 {
                    line.push_str(" | ");
                }
                line.push_str(&shell_quote(&st.program));
                for a in &st.args {
                    line.push(' ');
                    line.push_str(&shell_quote(a));
                }
            }
            let mut args = base;
            args.push("--".into());
            args.push("bash".into());
            args.push("-c".into());
            args.push(format!("{prelude}{line}"));
            Exec::Direct { stages: vec![StageSpec { program: "bwrap".into(), args }] }
        }
    };
    spec
}

/// Quote básico estilo Bourne para envolver en `'…'`. Sustituye `'` por
/// `'\''`. Suficiente para inyectar paths/comandos del usuario al wrap del
/// container; no pretende ser un parser POSIX completo.
pub(crate) fn shell_quote(s: &str) -> String {
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
