use super::*;

pub(crate) fn apply_cd(mut s: State, rest: &str) -> State {
    // En un contenedor el `cd` es contra el FS de ADENTRO, no el del host:
    // resolvemos el path de forma léxica (sin `canonicalize`, que miraría el
    // host) y actualizamos el cwd interior. Sin verificación de existencia —
    // el siguiente comando (que corre con `cd <cwd>` adentro) reporta el error
    // si el dir no existe.
    if matches!(s.source, Source::Container { .. } | Source::RemoteContainer { .. }) {
        let trimmed = rest.trim();
        let base = if trimmed.is_empty() {
            PathBuf::from("/root") // HOME del root dentro del contenedor
        } else if trimmed.starts_with('/') {
            PathBuf::from(trimmed)
        } else {
            s.cwd.join(trimmed)
        };
        s.cwd = normalize_lexical(&base);
        s.completion_source = crate::completion_source_for(&s.source, &s.cwd);
        return s;
    }
    // Remoto (SSH): cada comando es un `ssh exec` (shell nuevo en $HOME). v1:
    // sólo persistimos `cd` a rutas ABSOLUTAS (un `cd` relativo no tiene contra
    // qué resolver sin un round-trip). El cwd se antepone como `cd` en run_ssh.
    if matches!(s.source, Source::Remote { .. }) {
        let trimmed = rest.trim();
        if trimmed.is_empty() {
            s.cwd = PathBuf::from("~");
        } else if trimmed.starts_with('/') {
            s.cwd = normalize_lexical(&PathBuf::from(trimmed));
        } else {
            s.push_output(OutputLine::notice(
                "cd remoto (v1): usá una ruta absoluta (p. ej. cd /var/log)",
            ));
        }
        return s;
    }
    let target = if rest.trim().is_empty() {
        // `cd` sin args → HOME (convención bash/zsh).
        match std::env::var("HOME") {
            Ok(h) => PathBuf::from(h),
            Err(_) => {
                s.push_output(OutputLine::notice("cd: HOME no está definido"));
                return s;
            }
        }
    } else {
        let trimmed = rest.trim();
        let p = PathBuf::from(trimmed);
        if p.is_absolute() {
            p
        } else {
            s.cwd.join(p)
        }
    };
    match std::fs::canonicalize(&target) {
        Ok(canonical) => {
            if canonical.is_dir() {
                s.cwd = canonical;
            } else {
                s.push_output(OutputLine::notice(format!(
                    "cd: no es un directorio: {}",
                    target.display()
                )));
            }
        }
        Err(e) => {
            s.push_output(OutputLine::notice(format!("cd: {}: {e}", target.display())));
        }
    }
    s
}

/// Resuelve `.`/`..` de forma puramente léxica (sin tocar el FS ni seguir
/// symlinks). Para el `cd` dentro de un contenedor, donde el path es del FS
/// de adentro y `canonicalize` (host) no aplica.
fn normalize_lexical(p: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let mut out: Vec<std::ffi::OsString> = Vec::new();
    for comp in p.components() {
        match comp {
            Component::RootDir | Component::Prefix(_) => {}
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(c) => out.push(c.to_os_string()),
        }
    }
    let mut res = PathBuf::from("/");
    for c in out {
        res.push(c);
    }
    res
}

pub(crate) fn split_first_word(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    if line.is_empty() {
        return None;
    }
    match line.find(char::is_whitespace) {
        Some(i) => Some((&line[..i], &line[i + 1..])),
        None => Some((line, "")),
    }
}

/// Acciona el click sobre una decoración del output. Ninguna acción
/// bloquea la UI: `xdg-open` se forkea detached, y los cambios al
/// state (cwd, input) son in-memory.
pub(crate) fn open_decoration(mut s: State, kind: shuma_line::DecorationKind) -> State {
    use shuma_line::DecorationKind as Dk;
    match kind {
        Dk::Path {
            abs,
            is_dir,
            is_executable,
            ..
        } => {
            if is_dir {
                // Directorios → cd. Cambia el cwd y lo refleja en el
                // header sin "ejecutar" un comando.
                if abs.is_dir() {
                    s.cwd = abs;
                    s.completion_source = crate::completion_source_for(&s.source, &s.cwd);
                }
            } else if is_executable {
                // Binarios → pre-llenar el input con el path; el
                // usuario decide los args y Enter.
                s.input.set_text(abs.display().to_string());
            } else {
                // Archivos regulares → xdg-open detached.
                spawn_detached("xdg-open", &[abs.display().to_string().as_str()]);
            }
        }
        Dk::Url(url) => {
            spawn_detached("xdg-open", &[&url]);
        }
        Dk::GrepRef { abs, line_no, col } => {
            // `$EDITOR +line file` para vim/neovim/helix; si no hay
            // EDITOR, xdg-open al archivo y listo.
            if let Ok(editor) = std::env::var("EDITOR") {
                let line_flag = format!("+{line_no}");
                let path = abs.display().to_string();
                let args: Vec<&str> = match col {
                    Some(_) => vec![&line_flag, &path],
                    None => vec![&line_flag, &path],
                };
                spawn_detached(&editor, &args);
            } else {
                spawn_detached("xdg-open", &[abs.display().to_string().as_str()]);
            }
        }
        Dk::GitSha(sha) => {
            // Pre-llenar `git show <sha>` — la acción más útil 99% del tiempo.
            s.input.set_text(format!("git show {sha}"));
        }
        Dk::IssueRef(_)
        | Dk::BoxDraw
        | Dk::Number
        | Dk::DateTime
        | Dk::Severity(_)
        | Dk::Version
        | Dk::Percent
        | Dk::PermMask => {
            // Sin acción asociada — coloreo puro.
        }
    }
    s
}

/// Lanza un proceso "detached" — no esperamos, no leemos su output,
/// y el padre puede morir sin matarlo (`process_group(0)` para
/// despegarlo de la sesión de shuma). Usado para `xdg-open` y `$EDITOR`
/// disparados desde clicks.
pub(crate) fn spawn_detached(program: &str, args: &[&str]) {
    use std::os::unix::process::CommandExt;
    let _ = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
}
