use super::*;

/// Mapea `action_id` de `ShortcutAction::ModuleAction` al `Msg`.
pub fn dispatch(action_id: &str) -> Option<Msg> {
    match action_id {
        "shell.clear" => Some(Msg::Clear),
        "shell.cancel" => Some(Msg::Cancel),
        _ => None,
    }
}

/// Traduce un `KeyEvent` a una llamada sobre `LineState`. Devuelve
/// `true` si tocó el state. No maneja Enter, Tab, Up/Down ni Ctrl-C
/// (esos los intercepta el `update` del módulo).
pub(crate) fn apply_key_to_line(line: &mut LineState, ev: &KeyEvent) -> bool {
    match &ev.key {
        Key::Named(NamedKey::Backspace) => {
            line.backspace();
            true
        }
        Key::Named(NamedKey::Delete) => {
            line.delete();
            true
        }
        Key::Named(NamedKey::ArrowLeft) => {
            if ev.modifiers.ctrl {
                line.move_word_left();
            } else {
                line.move_left();
            }
            true
        }
        Key::Named(NamedKey::ArrowRight) => {
            if ev.modifiers.ctrl {
                line.move_word_right();
            } else {
                line.move_right();
            }
            true
        }
        Key::Named(NamedKey::Home) => {
            line.move_home();
            true
        }
        Key::Named(NamedKey::End) => {
            line.move_end();
            true
        }
        Key::Named(NamedKey::Space) => {
            line.insert(" ");
            true
        }
        _ => {
            if let Some(text) = &ev.text {
                if !text.is_empty() && !text.chars().any(|c| c.is_control()) {
                    line.insert(text);
                    return true;
                }
            }
            false
        }
    }
}

pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::Key(ev) => {
            if ev.state != KeyState::Pressed {
                return s;
            }
            // Si hay un TUI activo, las teclas van al stdin del PTY
            // (no al input). El usuario sale tipeando dentro del TUI
            // (`:q` en vim, `q` en less, etc.).
            if is_tui_active(&s) {
                // Shift+Insert siempre pega. Ctrl-V también — en TUIs
                // tipo less/vim no suele ser un binding (vim usa Ctrl-V
                // para visual-block en normal mode; al editar dentro
                // de insert mode tampoco). Si choca con un usuario
                // específico, en el futuro lo gateamos por allowlist.
                let paste = (ev.modifiers.ctrl
                    && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("v")))
                    || (ev.modifiers.shift
                        && matches!(&ev.key, Key::Named(NamedKey::Insert)));
                if paste {
                    forward_paste_to_pty(&s);
                    return s;
                }
                forward_key_to_pty(&s, &ev);
                return s;
            }
            // Si el overlay de búsqueda está abierto, las teclas van ahí.
            if s.history_search.is_some() {
                return handle_search_key(s, &ev);
            }
            // Ctrl-C: si hay run vivo, mandarle SIGTERM y comer la tecla.
            if ev.modifiers.ctrl
                && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("c"))
            {
                if s.running.is_some() {
                    return cancel_running(s);
                }
            }
            // Ctrl-V (o Shift+Insert): pega del clipboard al input.
            // (Si hay TUI, lo intercepta `is_tui_active` arriba; ese
            // camino tiene su propio paste.)
            let is_paste = (ev.modifiers.ctrl
                && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("v")))
                || (ev.modifiers.shift
                    && matches!(&ev.key, Key::Named(NamedKey::Insert)));
            if is_paste {
                if let Some(text) = read_clipboard() {
                    s.input.insert(&text);
                }
                return s;
            }
            // Ctrl-R: abrir overlay de búsqueda de historial.
            if ev.modifiers.ctrl
                && matches!(&ev.key, Key::Character(c) if c.eq_ignore_ascii_case("r"))
            {
                s.history_search = Some(HistorySearch::default());
                return s;
            }
            // Enter: ejecuta — pero si el texto deja una construcción
            // abierta (quote, paren, heredoc, `\` final, pipe pendiente),
            // insertamos un salto de línea y seguimos editando.
            // Shift+Enter fuerza salto de línea siempre.
            if let Key::Named(NamedKey::Enter) = ev.key {
                let pending = shuma_line::needs_continuation(s.input.text());
                if pending || ev.modifiers.shift {
                    s.input.insert("\n");
                    s.history_cursor = None;
                    return s;
                }
                s.history_cursor = None;
                s = run_submitted(s);
                return s;
            }
            // Tab: completion.
            if let Key::Named(NamedKey::Tab) = ev.key {
                return apply_completion_msg(s);
            }
            // Up/Down: navegación de historial.
            if let Key::Named(NamedKey::ArrowUp) = ev.key {
                return navigate_history(s, shuma_history::Nav::Older);
            }
            if let Key::Named(NamedKey::ArrowDown) = ev.key {
                return navigate_history(s, shuma_history::Nav::Newer);
            }
            // Flecha derecha al final de línea con ghost visible: acepta ghost.
            if let Key::Named(NamedKey::ArrowRight) = ev.key {
                if !ev.modifiers.ctrl && s.input.cursor() == s.input.text().len() {
                    if let Some(suffix) = current_ghost(&s) {
                        if !suffix.is_empty() {
                            s.input.insert(&suffix);
                            return s;
                        }
                    }
                }
            }
            apply_key_to_line(&mut s.input, &ev);
            // Cualquier edición rompe el cursor de navegación de historial.
            s.history_cursor = None;
        }
        Msg::FocusInput => {
            s.focused = true;
        }
        Msg::Clear => {
            s.output.clear();
        }
        Msg::Tick => {
            s = drain_run(s);
        }
        Msg::Cancel => {
            if s.running.is_some() {
                s = cancel_running(s);
            }
        }
        Msg::OpenDecoration(kind) => {
            s = open_decoration(s, kind);
        }
        Msg::InsertAtCursor(text) => {
            // Cerramos cualquier overlay activo para que el texto
            // pegado quede visible sin tener que cerrar el Ctrl-R a mano.
            s.history_search = None;
            s.history_cursor = None;
            s.input.insert(&text);
            s.focused = true;
        }
        Msg::VimPaste => {
            // Sólo aplica si hay un TUI vivo; `forward_paste_to_pty` es
            // no-op silencioso si no.
            forward_paste_to_pty(&s);
        }
    }
    s
}

/// Acciona el click sobre una decoración del output. Ninguna acción
/// bloquea la UI: `xdg-open` se forkea detached, y los cambios al
/// state (cwd, input) son in-memory.
pub(crate) fn open_decoration(mut s: State, kind: shuma_line::DecorationKind) -> State {
    use shuma_line::DecorationKind as Dk;
    match kind {
        Dk::Path { abs, is_dir, is_executable, .. } => {
            if is_dir {
                // Directorios → cd. Cambia el cwd y lo refleja en el
                // header sin "ejecutar" un comando.
                if abs.is_dir() {
                    s.cwd = abs;
                    s.completion_source = Arc::new(ShellSource::new(&s.cwd));
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
        Dk::IssueRef(_) | Dk::BoxDraw => {
            // Sin acción asociada.
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

/// Aplica un Tab: completion en la posición del cursor.
/// - 0 candidatos: no hace nada.
/// - 1 candidato: lo inserta directo.
/// - N candidatos: inserta el prefijo común y deja al usuario tipear más.
pub(crate) fn apply_completion_msg(mut s: State) -> State {
    let comp = s.input.complete(s.completion_source.as_ref());
    if comp.is_empty() {
        return s;
    }
    let candidate: String = if comp.candidates.len() == 1 {
        comp.candidates[0].clone()
    } else {
        common_prefix(&comp.candidates)
    };
    if candidate.is_empty() {
        return s;
    }
    s.input.apply_completion(&comp, &candidate);
    s
}

/// Prefijo común más largo de un slice de strings — usado en completion
/// cuando hay múltiples candidatos.
pub(crate) fn common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else {
        return String::new();
    };
    let mut end = first.len();
    for s in &items[1..] {
        let bytes = s.as_bytes();
        let fbytes = first.as_bytes();
        let mut i = 0;
        while i < end && i < bytes.len() && bytes[i] == fbytes[i] {
            i += 1;
        }
        end = i;
        if end == 0 {
            break;
        }
    }
    // Asegurarse de cortar en límite de carácter UTF-8.
    while end > 0 && !first.is_char_boundary(end) {
        end -= 1;
    }
    first[..end].to_string()
}

/// Navega el historial por Up/Down.
pub(crate) fn navigate_history(mut s: State, dir: shuma_history::Nav) -> State {
    let next = {
        let history = s.history.lock().unwrap();
        history.navigate(s.history_cursor, dir).map(|(i, e)| (i, e.line.clone()))
    };
    if let Some((i, line)) = next {
        s.history_cursor = Some(i);
        s.input.set_text(line);
    } else if matches!(dir, shuma_history::Nav::Newer) {
        // Salir del historial al final: línea vacía.
        s.history_cursor = None;
        s.input.clear();
    }
    s
}

/// Maneja teclas mientras el overlay Ctrl-R está abierto.
pub(crate) fn handle_search_key(mut s: State, ev: &KeyEvent) -> State {
    let Some(mut search) = s.history_search.take() else {
        return s;
    };
    match &ev.key {
        Key::Named(NamedKey::Escape) => {
            // Salida sin aceptar.
            return s;
        }
        Key::Named(NamedKey::Enter) => {
            // Acepta el seleccionado: pasa a la línea (sin ejecutar).
            let pick = {
                let history = s.history.lock().unwrap();
                history
                    .fuzzy_search(&search.query, 50)
                    .get(search.selected)
                    .map(|e| e.line.clone())
            };
            if let Some(line) = pick {
                s.input.set_text(line);
            }
            return s;
        }
        Key::Named(NamedKey::Backspace) => {
            search.query.pop();
            search.selected = 0;
        }
        Key::Named(NamedKey::ArrowDown) => {
            let history = s.history.lock().unwrap();
            let max = history.fuzzy_search(&search.query, 50).len();
            if max > 0 && search.selected + 1 < max {
                search.selected += 1;
            }
        }
        Key::Named(NamedKey::ArrowUp) => {
            search.selected = search.selected.saturating_sub(1);
        }
        _ => {
            if let Some(text) = &ev.text {
                if !text.is_empty() && !text.chars().any(|c| c.is_control()) {
                    search.query.push_str(text);
                    search.selected = 0;
                }
            }
        }
    }
    s.history_search = Some(search);
    s
}

/// `true` si hay un `ActiveRun` en modo TUI (PTY + vt100). Las teclas
/// van al stdin del PTY mientras esto sea cierto.
pub(crate) fn is_tui_active(s: &State) -> bool {
    let Some(arc) = s.running.as_ref() else {
        return false;
    };
    let g = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    g.tui.is_some()
}

/// Traduce una tecla a su secuencia de bytes para el PTY (xterm-compat).
/// Las TUIs esperan estos códigos.
pub(crate) fn key_to_pty_bytes(ev: &KeyEvent) -> Vec<u8> {
    match &ev.key {
        Key::Named(NamedKey::Enter) => b"\r".to_vec(),
        Key::Named(NamedKey::Tab) => b"\t".to_vec(),
        Key::Named(NamedKey::Backspace) => b"\x7f".to_vec(),
        Key::Named(NamedKey::Escape) => b"\x1b".to_vec(),
        Key::Named(NamedKey::ArrowUp) => b"\x1b[A".to_vec(),
        Key::Named(NamedKey::ArrowDown) => b"\x1b[B".to_vec(),
        Key::Named(NamedKey::ArrowRight) => b"\x1b[C".to_vec(),
        Key::Named(NamedKey::ArrowLeft) => b"\x1b[D".to_vec(),
        Key::Named(NamedKey::Home) => b"\x1b[H".to_vec(),
        Key::Named(NamedKey::End) => b"\x1b[F".to_vec(),
        Key::Named(NamedKey::PageUp) => b"\x1b[5~".to_vec(),
        Key::Named(NamedKey::PageDown) => b"\x1b[6~".to_vec(),
        Key::Named(NamedKey::Delete) => b"\x1b[3~".to_vec(),
        Key::Named(NamedKey::Space) => b" ".to_vec(),
        _ => {
            // Ctrl-<x>: codifica el byte 0x01..0x1a para letras.
            if ev.modifiers.ctrl {
                if let Key::Character(c) = &ev.key {
                    if let Some(ch) = c.chars().next() {
                        let lo = ch.to_ascii_lowercase();
                        if ('a'..='z').contains(&lo) {
                            return vec![(lo as u8) - b'a' + 1];
                        }
                    }
                }
            }
            ev.text.as_deref().unwrap_or("").as_bytes().to_vec()
        }
    }
}

/// Lee el clipboard del SO (vía `arboard`). Devuelve `None` si no hay
/// display server, está vacío, o el contenido no es texto. No cachea —
/// el sistema tiene su propio TTL.
pub(crate) fn read_clipboard() -> Option<String> {
    let mut clip = arboard::Clipboard::new().ok()?;
    clip.get_text().ok()
}

/// Pega el contenido del clipboard en el PTY del run activo. Si el TUI
/// hijo está en bracketed-paste mode (DECSET 2004), envuelve la
/// secuencia en `\x1b[200~...\x1b[201~` para que vim, less y emacs
/// distingan "tipeé esto" de "pegué esto" (auto-indent, paste-mode,
/// etc.). No-op silencioso si no hay TUI o el clipboard está vacío.
pub(crate) fn forward_paste_to_pty(s: &State) {
    let Some(arc) = s.running.as_ref() else {
        return;
    };
    let Some(text) = read_clipboard() else {
        return;
    };
    if text.is_empty() {
        return;
    }
    let guard = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let bracketed = guard
        .tui
        .as_ref()
        .map(|t| t.parser.screen().bracketed_paste())
        .unwrap_or(false);
    let payload: Vec<u8> = if bracketed {
        let mut buf: Vec<u8> = b"\x1b[200~".to_vec();
        buf.extend_from_slice(text.as_bytes());
        buf.extend_from_slice(b"\x1b[201~");
        buf
    } else {
        text.into_bytes()
    };
    guard.handle.write_input(payload);
}

/// Manda los bytes de la tecla al PTY del run activo. No-op si no hay
/// tui activo.
pub(crate) fn forward_key_to_pty(s: &State, ev: &KeyEvent) {
    let Some(arc) = s.running.as_ref() else {
        return;
    };
    let bytes = key_to_pty_bytes(ev);
    if bytes.is_empty() {
        return;
    }
    let guard = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.handle.write_input(bytes);
}

/// Sugerencia "ghost" para la línea actual — el prefijo histórico más
/// reciente que extiende el texto que ya está tipeado.
pub(crate) fn current_ghost(s: &State) -> Option<String> {
    let text = s.input.text();
    if text.is_empty() || s.input.cursor() != text.len() {
        return None;
    }
    let history = s.history.lock().ok()?;
    let corpus: Vec<String> = history.entries().iter().rev().map(|e| e.line.clone()).collect();
    shuma_line::ghost_suggestion(text, &corpus)
}

pub(crate) fn run_submitted(mut s: State) -> State {
    let line = s.input.text().to_string();
    let trimmed = line.trim().to_string();
    s.input.clear();
    if trimmed.is_empty() {
        return s;
    }
    push_line(&mut s.output, OutputLine::prompt(format!("$ {trimmed}")));

    // Append al historial — todo lo que el usuario Enter-eó queda
    // registrado, builtins incluidos (para que `cd ../foo` reaparezca
    // por Up). `IgnoreConsecutive` evita ráfagas iguales.
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = shuma_history::Entry::new(
            trimmed.clone(),
            s.cwd.display().to_string(),
            now,
        );
        if let Ok(mut h) = s.history.lock() {
            let _ = h.append(entry);
        }
    }

    // Builtins primero — no spawnean proceso, corren aunque haya run vivo.
    if let Some((cmd, rest)) = split_first_word(&trimmed) {
        match cmd {
            "cd" => {
                return apply_cd(s, rest);
            }
            "pwd" => {
                let cwd_str = s.cwd.display().to_string();
                push_line(&mut s.output, OutputLine::stdout(cwd_str));
                return s;
            }
            "clear" => {
                s.output.clear();
                return s;
            }
            "exit" => {
                push_line(
                    &mut s.output,
                    OutputLine::notice("exit: el chasis maneja la salida (F12 para cerrar)"),
                );
                return s;
            }
            ":jobs" => return apply_jobs_list(s),
            ":term" => return apply_jobs_signal(s, rest, JobSignal::Term),
            ":stop" => return apply_jobs_signal(s, rest, JobSignal::Stop),
            ":cont" => return apply_jobs_signal(s, rest, JobSignal::Cont),
            _ => {}
        }
    }

    // Sufijo `&` (con espacios opcionales antes) → background. El
    // background siempre arranca, sin encolar; no hay límite.
    if let Some(stripped) = trimmed.strip_suffix('&') {
        let cmd = stripped.trim_end().to_string();
        if cmd.is_empty() {
            return s;
        }
        return start_bg(s, cmd);
    }

    // Comando externo foreground. Si ya hay uno corriendo, lo encolamos;
    // si no, arrancamos ahora mismo.
    if s.running.is_some() {
        s.queue.push_back(trimmed);
        push_line(
            &mut s.output,
            OutputLine::notice("⌛ en cola — esperando a que el comando actual termine"),
        );
        return s;
    }
    start_run(s, trimmed)
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum JobSignal {
    Term,
    Stop,
    Cont,
}

/// Lista los bg_jobs con su índice y comando. Marca finalizados.
pub(crate) fn apply_jobs_list(mut s: State) -> State {
    if s.bg_jobs.is_empty() {
        push_line(&mut s.output, OutputLine::notice("(sin jobs en background)"));
        return s;
    }
    for (i, arc) in s.bg_jobs.iter().enumerate() {
        let (cmd, status) = match arc.lock() {
            Ok(g) => (
                g.command.clone(),
                if g.handle.is_finished() { "done" } else { "running" },
            ),
            Err(p) => {
                let g = p.into_inner();
                (g.command.clone(), if g.handle.is_finished() { "done" } else { "running" })
            }
        };
        push_line(
            &mut s.output,
            OutputLine::notice(format!("[{i}] {status}  {cmd}")),
        );
    }
    s
}

/// Aplica `:term N` / `:stop N` / `:cont N` al job de índice `N`.
/// Stop/Cont son no-op en jobs sin `Killer` (remotos vía daemon).
pub(crate) fn apply_jobs_signal(mut s: State, rest: &str, sig: JobSignal) -> State {
    let idx: usize = match rest.trim().parse() {
        Ok(n) => n,
        Err(_) => {
            push_line(
                &mut s.output,
                OutputLine::notice("uso: :term N | :stop N | :cont N"),
            );
            return s;
        }
    };
    let Some(arc) = s.bg_jobs.get(idx).cloned() else {
        push_line(
            &mut s.output,
            OutputLine::notice(format!("no hay job [{idx}]")),
        );
        return s;
    };
    let guard = match arc.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let acted = match sig {
        JobSignal::Term => match guard.killer.as_ref() {
            Some(k) => {
                k.term();
                true
            }
            None => {
                // Remoto: cancel via stream close.
                guard.handle.kill();
                true
            }
        },
        JobSignal::Stop => guard.killer.as_ref().map(|k| k.stop()).unwrap_or(false),
        JobSignal::Cont => guard.killer.as_ref().map(|k| k.cont()).unwrap_or(false),
    };
    let label = match sig {
        JobSignal::Term => "TERM",
        JobSignal::Stop => "STOP",
        JobSignal::Cont => "CONT",
    };
    drop(guard);
    push_line(
        &mut s.output,
        OutputLine::notice(if acted {
            format!("[{idx}] SIG{label} enviado")
        } else {
            format!("[{idx}] no se pudo enviar SIG{label}")
        }),
    );
    s
}

/// Variante de `start_run` que arranca como job background. La salida
/// se mergea al output buffer prefijada por `[N]`. Devuelve `s` con el
/// nuevo job en `bg_jobs`.
pub(crate) fn start_bg(mut s: State, line: String) -> State {
    let cwd_str = s.cwd.display().to_string();
    let (spec, _tui) = build_spec(&line, &cwd_str);
    // Background no soporta TUI (no le pintamos el grid; el panel
    // sería robado al foreground). Si la línea era TUI, la corremos
    // sin PTY igual — el binario podrá quejarse, pero al menos no
    // tira la UI.
    let bg_spec = if matches!(spec.exec, Exec::Pty { .. }) {
        let mut s2 = spec.clone();
        s2.exec = Exec::Shell {
            line: line.clone(),
            program: "bash".into(),
        };
        s2
    } else {
        spec
    };
    let handle = shuma_exec::run(&bg_spec);
    let killer = handle.killer();
    let idx = s.bg_jobs.len();
    push_line(
        &mut s.output,
        OutputLine::notice(format!("[{idx}] background  {line}")),
    );
    let active = ActiveRun {
        handle: BackendHandle::Local(handle),
        killer: Some(killer),
        command: line,
        tui: None,
    };
    s.bg_jobs.push(Arc::new(Mutex::new(active)));
    s
}

pub(crate) fn start_run(mut s: State, line: String) -> State {
    let cwd_str = s.cwd.display().to_string();
    let (spec, tui) = build_spec(&line, &cwd_str);
    // Registramos la intención antes de hacer spawn — si el spawn
    // remoto falla, igual queda el nodo `%cN` con status `Failed`
    // marcado más abajo (vía el RunEvent::Failed que retorna el
    // backend). El lienzo refleja el intento.
    s.current_run_node = Some(s.intent_graph.record(line.clone()));
    s.current_run_bytes = 0;
    let active = match &s.source {
        Source::Local => {
            // Camino histórico — exec directo sobre esta máquina.
            let handle = shuma_exec::run(&spec);
            let killer = handle.killer();
            ActiveRun {
                handle: BackendHandle::Local(handle),
                killer: Some(killer),
                command: line,
                tui,
            }
        }
        Source::Daemon { socket, .. } => {
            // PTY remoto no soportado; fallback a local con notice.
            if tui.is_some() {
                push_line(
                    &mut s.output,
                    OutputLine::notice(
                        "PTY remoto no soportado por el daemon — corro local",
                    ),
                );
                let handle = shuma_exec::run(&spec);
                let killer = handle.killer();
                ActiveRun {
                    handle: BackendHandle::Local(handle),
                    killer: Some(killer),
                    command: line,
                    tui,
                }
            } else {
                let sock = socket
                    .clone()
                    .unwrap_or_else(shuma_protocol::default_socket_path);
                match shuma_remote_exec::run(&spec, &sock) {
                    Ok(h) => ActiveRun {
                        handle: BackendHandle::Remote(h),
                        killer: None,
                        command: line,
                        tui: None,
                    },
                    Err(e) => {
                        push_line(
                            &mut s.output,
                            OutputLine::notice(format!("✘ daemon: {e}")),
                        );
                        fail_pending_intent(&mut s);
                        return s;
                    }
                }
            }
        }
        Source::DaemonTcp { addr, server_pub_hex, .. } => {
            if tui.is_some() {
                push_line(
                    &mut s.output,
                    OutputLine::notice("PTY remoto no soportado — corro local"),
                );
                let handle = shuma_exec::run(&spec);
                let killer = handle.killer();
                ActiveRun {
                    handle: BackendHandle::Local(handle),
                    killer: Some(killer),
                    command: line,
                    tui,
                }
            } else {
                let kp = match load_or_create_identity() {
                    Ok(kp) => kp,
                    Err(e) => {
                        push_line(
                            &mut s.output,
                            OutputLine::notice(format!("✘ identity: {e}")),
                        );
                        fail_pending_intent(&mut s);
                        return s;
                    }
                };
                let server_pub = match parse_pub_hex(server_pub_hex) {
                    Ok(p) => p,
                    Err(e) => {
                        push_line(
                            &mut s.output,
                            OutputLine::notice(format!("✘ server_pub_hex: {e}")),
                        );
                        fail_pending_intent(&mut s);
                        return s;
                    }
                };
                match shuma_remote_exec::run_tcp(&spec, addr, kp, server_pub) {
                    Ok(h) => ActiveRun {
                        handle: BackendHandle::Remote(h),
                        killer: None,
                        command: line,
                        tui: None,
                    },
                    Err(e) => {
                        push_line(
                            &mut s.output,
                            OutputLine::notice(format!("✘ daemon tcp: {e}")),
                        );
                        fail_pending_intent(&mut s);
                        return s;
                    }
                }
            }
        }
        Source::Remote { .. } => {
            // SSH (matilda usa esta variante para otra cosa). El shell
            // no tiene un transporte SSH para comandos arbitrarios aún;
            // fallback a local con notice claro.
            push_line(
                &mut s.output,
                OutputLine::notice(
                    "shell vía SSH no implementado todavía — corro local",
                ),
            );
            let handle = shuma_exec::run(&spec);
            let killer = handle.killer();
            ActiveRun {
                handle: BackendHandle::Local(handle),
                killer: Some(killer),
                command: line,
                tui,
            }
        }
    };
    s.running = Some(Arc::new(Mutex::new(active)));
    s
}

/// Cierra el nodo `%cN` registrado por `start_run` como fallido cuando
/// el spawn no llega a colocar el `RunHandle` (errores de socket/identity/
/// pub-hex/tcp). Sin esto el lienzo mostraría el comando como "running"
/// para siempre. Limpiá también el contador de bytes.
pub(crate) fn fail_pending_intent(s: &mut State) {
    if let Some(id) = s.current_run_node.take() {
        s.intent_graph.complete(id, false, 0);
    }
    s.current_run_bytes = 0;
}

/// Carga el `Keypair` del shell desde el archivo de identidad,
/// creando uno nuevo si no existe. Usa el path por defecto de
/// `shuma-link::Keypair::default_path()` (`~/.config/shuma/keys/identity`).
pub(crate) fn load_or_create_identity() -> Result<shuma_link::Keypair, String> {
    let path = shuma_link::Keypair::default_path()
        .ok_or_else(|| "no se pudo derivar el path de identidad".to_string())?;
    shuma_link::Keypair::load_or_generate(&path).map_err(|e| e.to_string())
}

pub(crate) fn parse_pub_hex(hex_str: &str) -> Result<shuma_link::PublicKey, String> {
    shuma_link::PublicKey::from_hex(hex_str).map_err(|e| e.to_string())
}

/// Decide cómo lanzar `line`: si el primer token está en la allowlist
/// TUI (o el usuario lo prefijó con `:tui`), abre un PTY; si no, va por
/// el shell normal (streaming Stdout/Stderr).
pub(crate) fn build_spec(line: &str, cwd: &str) -> (CommandSpec, Option<TuiSession>) {
    // Prefijo explícito `:tui <comando>`.
    let (cmd_line, force_tui) = match line.strip_prefix(":tui ") {
        Some(rest) => (rest.trim(), true),
        None => (line, false),
    };
    let first_word = cmd_line.split_whitespace().next().unwrap_or("");
    let is_tui = force_tui || TUI_ALLOWLIST.contains(&first_word);
    if !is_tui {
        return (CommandSpec::shell(line, cwd), None);
    }
    // Bajo PTY: parseamos en stages básicos por whitespace. No soporta
    // pipes ni redirecciones — un TUI fullscreen no los usa.
    let parts: Vec<String> = cmd_line.split_whitespace().map(String::from).collect();
    if parts.is_empty() {
        return (CommandSpec::shell(line, cwd), None);
    }
    let program = parts[0].clone();
    let args = parts[1..].to_vec();
    let spec = CommandSpec {
        exec: Exec::Pty {
            program,
            args,
            cols: PTY_COLS,
            rows: PTY_ROWS,
        },
        cwd: cwd.to_string(),
        capture_limit: 0,
        spill_path: None,
        stdin_data: None,
    };
    // Stage marker — usamos `parts` para sintaxis, no para ejecutar; el
    // Exec::Pty arma el spawn directo. La conversión a `StageSpec`
    // queda como guía visual del tooltip si después la queremos
    // exponer (hoy `Exec::Pty` no usa stages).
    let _ = StageSpec {
        program: parts[0].clone(),
        args: parts[1..].to_vec(),
    };
    // `program` ya se movió al `Exec::Pty`; usamos `parts[0]` (sigue vivo).
    (spec, Some(TuiSession::new(&parts[0], PTY_ROWS, PTY_COLS)))
}

pub(crate) fn drain_run(mut s: State) -> State {
    let Some(active_arc) = s.running.clone() else {
        return s;
    };
    let mut finished_with: Option<RunEvent> = None;
    {
        let mut guard = match active_arc.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        // Resize del PTY si el rect del panel cambió desde el último
        // tick. Cell size aproximado: 7.5 px ancho × 16 px alto (12 pt
        // monoespacio en Llimphi default). Si el panel se redimensiona
        // el TUI hace SIGWINCH al child.
        let want_resize: Option<(u16, u16)> = if let Some(tui) = guard.tui.as_ref() {
            let (w, h) = match s.last_tui_rect.lock() {
                Ok(g) => *g,
                Err(p) => *p.into_inner(),
            };
            if w > 1.0 && h > 1.0 {
                let cols = ((w / 7.5).floor() as i32).clamp(20, 400) as u16;
                let rows = ((h / 16.0).floor() as i32).clamp(5, 200) as u16;
                if rows != tui.rows || cols != tui.cols {
                    Some((rows, cols))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        if let Some((rows, cols)) = want_resize {
            guard.handle.resize(rows, cols);
            if let Some(tui) = guard.tui.as_mut() {
                tui.set_size(rows, cols);
            }
        }
        let events = guard.handle.try_events();
        for ev in events {
            match ev {
                RunEvent::Stdout(line) => {
                    // +1 por el `\n` implícito de cada línea drenada.
                    s.current_run_bytes =
                        s.current_run_bytes.saturating_add(line.len() as u64 + 1);
                    push_line(&mut s.output, OutputLine::stdout(line));
                }
                RunEvent::Stderr(line) => {
                    s.current_run_bytes =
                        s.current_run_bytes.saturating_add(line.len() as u64 + 1);
                    push_line(&mut s.output, OutputLine::stderr(line));
                }
                RunEvent::Truncated => push_line(
                    &mut s.output,
                    OutputLine::notice("… (salida truncada por límite de captura)"),
                ),
                RunEvent::Spilled(path) => push_line(
                    &mut s.output,
                    OutputLine::notice(format!("… (resto volcado a {path})")),
                ),
                RunEvent::Bytes(bytes) => {
                    s.current_run_bytes =
                        s.current_run_bytes.saturating_add(bytes.len() as u64);
                    if let Some(tui) = guard.tui.as_mut() {
                        tui.parser.process(&bytes);
                    }
                }
                ev @ (RunEvent::Exited(_) | RunEvent::Failed(_)) => {
                    finished_with = Some(ev);
                }
            }
        }
    }
    if let Some(ev) = finished_with {
        let ok = matches!(ev, RunEvent::Exited(0));
        let notice = match ev {
            RunEvent::Exited(0) => "✔ exit 0".to_string(),
            RunEvent::Exited(code) => format!("✘ exit {code}"),
            RunEvent::Failed(e) => format!("✘ no se pudo spawnear: {e}"),
            _ => unreachable!(),
        };
        push_line(&mut s.output, OutputLine::notice(notice));
        // Cerrá el nodo del grafo de intenciones — el lienzo lo refleja
        // como verde/rojo en el próximo render.
        if let Some(id) = s.current_run_node.take() {
            s.intent_graph.complete(id, ok, s.current_run_bytes);
        }
        s.current_run_bytes = 0;
        s.running = None;
        // Si quedó algo en cola, arrancarlo ya — sin esperar otro Tick.
        if let Some(next) = s.queue.pop_front() {
            s = start_run(s, next);
        }
    }
    // Drenado de jobs background — cada uno aporta sus líneas
    // prefijadas por `[N]`. Los terminados se eliminan del Vec.
    s = drain_bg_jobs(s);
    s
}

/// Drena los `bg_jobs` y los limpia. Las líneas se prefijan `[N]`
/// para distinguir su origen.
pub(crate) fn drain_bg_jobs(mut s: State) -> State {
    let mut next_jobs: Vec<Arc<Mutex<ActiveRun>>> = Vec::with_capacity(s.bg_jobs.len());
    for (i, arc) in s.bg_jobs.iter().enumerate() {
        let mut keep = true;
        let prefix = format!("[{i}] ");
        let mut finished: Option<RunEvent> = None;
        {
            let mut guard = match arc.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            for ev in guard.handle.try_events() {
                match ev {
                    RunEvent::Stdout(line) => push_line(
                        &mut s.output,
                        OutputLine::stdout(format!("{prefix}{line}")),
                    ),
                    RunEvent::Stderr(line) => push_line(
                        &mut s.output,
                        OutputLine::stderr(format!("{prefix}{line}")),
                    ),
                    RunEvent::Truncated => push_line(
                        &mut s.output,
                        OutputLine::notice(format!("{prefix}… (truncada)")),
                    ),
                    RunEvent::Spilled(path) => push_line(
                        &mut s.output,
                        OutputLine::notice(format!("{prefix}… (volcado a {path})")),
                    ),
                    RunEvent::Bytes(_) => {
                        // Background sin PTY — no debería emitir Bytes.
                    }
                    ev @ (RunEvent::Exited(_) | RunEvent::Failed(_)) => {
                        finished = Some(ev);
                    }
                }
            }
        }
        if let Some(ev) = finished {
            let notice = match ev {
                RunEvent::Exited(0) => format!("{prefix}✔ exit 0"),
                RunEvent::Exited(code) => format!("{prefix}✘ exit {code}"),
                RunEvent::Failed(e) => format!("{prefix}✘ failed: {e}"),
                _ => unreachable!(),
            };
            push_line(&mut s.output, OutputLine::notice(notice));
            keep = false;
        }
        if keep {
            next_jobs.push(arc.clone());
        }
    }
    s.bg_jobs = next_jobs;
    s
}

pub(crate) fn cancel_running(mut s: State) -> State {
    if let Some(arc) = s.running.as_ref() {
        let guard = match arc.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        // Local: SIGKILL al grupo entero — Ctrl-C debe doler en una UI.
        // Remoto: cerrar el stream — el daemon detecta EOF y mata al
        // hijo. La forma del notice no cambia.
        if let Some(killer) = guard.killer.as_ref() {
            killer.kill();
        } else {
            guard.handle.kill();
        }
        // El próximo Tick observará `RunEvent::Exited` y limpiará el handle.
    }
    push_line(&mut s.output, OutputLine::notice("⏹ cancel (SIGKILL enviado)"));
    s
}

pub(crate) fn apply_cd(mut s: State, rest: &str) -> State {
    let target = if rest.trim().is_empty() {
        // `cd` sin args → HOME (convención bash/zsh).
        match std::env::var("HOME") {
            Ok(h) => PathBuf::from(h),
            Err(_) => {
                push_line(
                    &mut s.output,
                    OutputLine::notice("cd: HOME no está definido"),
                );
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
                push_line(
                    &mut s.output,
                    OutputLine::notice(format!(
                        "cd: no es un directorio: {}",
                        target.display()
                    )),
                );
            }
        }
        Err(e) => {
            push_line(
                &mut s.output,
                OutputLine::notice(format!("cd: {}: {e}", target.display())),
            );
        }
    }
    s
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

pub(crate) fn push_line(buf: &mut Vec<OutputLine>, line: OutputLine) {
    buf.push(line);
    let len = buf.len();
    if len > MAX_OUTPUT_LINES {
        buf.drain(0..len - MAX_OUTPUT_LINES);
    }
}
