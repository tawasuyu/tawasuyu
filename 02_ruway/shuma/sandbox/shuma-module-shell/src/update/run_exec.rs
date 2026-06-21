use super::*;

pub(crate) fn run_submitted(mut s: State) -> State {
    let line = s.input.text().to_string();
    let trimmed = line.trim().to_string();
    s.input.clear();
    if trimmed.is_empty() {
        return s;
    }
    // Modelo de input paralelo: el Enter se dirige por `input_focus`.
    //   • Foco en un comando VIVO (foreground o bg) y la línea no fuerza bg
    //     (`&`) → el Enter es respuesta a SU stdin (apt Y/n, sudo password,
    //     prompts custom). Escribimos `<line>\n` a ese job.
    //   • Foco en la LÍNEA (`input_focus == None`) → el Enter arranca un
    //     comando NUEVO aunque haya otros vivos (no bloquea: para volver a la
    //     línea basta click en el prompt/cabezal → `FocusInput`).
    // Antes esto miraba `s.running.is_some()` e ignoraba `input_focus`: con un
    // comando que no termina (una app GUI, `ssh`…) TODO lo tipeado iba a su
    // stdin y no había forma de lanzar otro — el "se queda bloqueado".
    if let (Some(fb), false, false) =
        (s.input_focus, trimmed.ends_with('&'), trimmed.starts_with(':'))
    {
        // Los meta-comandos `:` (`:jobs`, `:kill`, `:term`…) son control del
        // shell, no datos: siempre ejecutan, aunque el foco esté en un job.
        if let Some(active_arc) = s.job_by_block(fb).filter(|_| s.block_has_live_job(fb)) {
            let bytes = {
                let mut v = line.clone().into_bytes();
                v.push(b'\n');
                v
            };
            if let Ok(guard) = active_arc.lock() {
                guard.handle.write_input(bytes);
            }
            // Echo discreto de lo enviado para que el usuario vea qué tipeó.
            s.push_output(OutputLine::notice(format!("← {line}")));
            return s;
        }
    }
    // E3 — cada submit del usuario re-arma la regla on_exit_nonzero. (El
    // comando de la regla la re-desarma después de su run.)
    s.exit_rule_fired = false;
    // El comando que estaba en foco recede al historial: se pliega para que
    // el nuevo nazca expandido y la vista no sea un volcado plano. Sólo los
    // que tienen cuerpo (los sin salida no se pliegan; se ven distinto).
    let prev = s.current_block;
    if prev != 0 && !body_lines_for_block(&s, prev).is_empty() {
        s.collapsed.insert(prev);
    }
    s.push_output(OutputLine::prompt(format!("$ {trimmed}")));

    // Append al historial — todo lo que el usuario Enter-eó queda
    // registrado, builtins incluidos (para que `cd ../foo` reaparezca
    // por Up). `IgnoreConsecutive` evita ráfagas iguales.
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = shuma_history::Entry::new(trimmed.clone(), s.cwd.display().to_string(), now);
        if let Ok(mut h) = s.history.lock() {
            let _ = h.append(entry);
        }
    }
    // Recalcula los patrones emergentes con el historial ya actualizado —
    // alimentan la predicción del ghost para el próximo comando.
    refresh_patterns(&mut s);

    // Expansión de aliases del `.shumarc`: la primera palabra se reemplaza si
    // está declarada. Los meta-comandos del shell (`:save`, `:limit`, …) se
    // dejan SIN expandir para que el rc no pueda secuestrarlos. Lo que se
    // muestra (`$ trimmed`) y se persiste en el historial es lo que el usuario
    // tipeó; lo que se ejecuta es `exec_line` ya resuelto.
    let exec_line = if trimmed.starts_with(':') {
        trimmed.clone()
    } else {
        s.config.expand_aliases(&trimmed).into_owned()
    };

    // Builtins primero — no spawnean proceso, corren aunque haya run vivo.
    if let Some((cmd, rest)) = split_first_word(&exec_line) {
        match cmd {
            "cd" => {
                return apply_cd(s, rest);
            }
            "pwd" => {
                let cwd_str = s.cwd.display().to_string();
                s.push_output(OutputLine::stdout(cwd_str));
                return s;
            }
            "clear" => {
                s.clear_output();
                return s;
            }
            "exit" => {
                s.push_output(OutputLine::notice(
                    "exit: el chasis maneja la salida (F12 para cerrar)",
                ));
                return s;
            }
            ":jobs" => return apply_jobs_list(s),
            ":term" => return apply_jobs_signal(s, rest, JobSignal::Term),
            ":kill" => return apply_jobs_signal(s, rest, JobSignal::Kill),
            ":stop" => return apply_jobs_signal(s, rest, JobSignal::Stop),
            ":cont" => return apply_jobs_signal(s, rest, JobSignal::Cont),
            ":env" => return apply_env(s, rest),
            ":persist" => return apply_persist(s, rest),
            ":limit" => return apply_capture_limit(s, rest),
            ":spill" => return apply_spill(s, rest),
            ":scrollback" => return apply_scrollback(s, rest),
            ":save" => return save_group(s, rest),
            ":write" => return apply_write(s, rest),
            ":yank" | ":copy" => return apply_yank(s, rest),
            ":groups" => return apply_groups_list(s),
            ":macro" => return apply_macro(s, rest),
            ":macros" => return list_macros(s),
            ":stats" => return apply_stats(s, rest),
            ":?" => return apply_ask(s, rest),
            ":explica" | ":explain" => return apply_explain(s, rest, false),
            ":resume" | ":resumen" => return apply_explain(s, rest, true),
            ":spawn" => return apply_spawn_session(s, rest),
            ":sessions" => return apply_sessions(s, rest),
            ":attach" => return apply_attach_session(s, rest),
            ":kill-session" => return apply_kill_session(s, rest),
            _ => {}
        }
    }

    // Sufijo `&` (con espacios opcionales antes) → background. El
    // background siempre arranca, sin encolar; no hay límite.
    if let Some(stripped) = exec_line.strip_suffix('&') {
        let cmd = stripped.trim_end().to_string();
        if cmd.is_empty() {
            return s;
        }
        return start_bg(s, cmd);
    }

    // Comando externo foreground. Si ya hay uno corriendo, el nuevo
    // arranca en background paralelo (job nuevo) — no encolar. Esto
    // evita que un comando colgado (fastfetch, ssh, etc.) bloquee el
    // shell. El usuario ve un notice "▶ job N en background" y puede
    // seguir tipeando; `:jobs` los lista, `:kill N` los mata, `:fg N`
    // los traería al foreground (TODO).
    if s.running.is_some() {
        let cmd = exec_line.clone();
        s.push_output(OutputLine::notice(format!(
            "▶ corre en background (hay otro comando vivo) — {cmd}"
        )));
        let mut s = start_bg(s, exec_line);
        // Auto-bg (lanzado con otro vivo): se lleva el foco del input, como el
        // foreground. El `&` explícito NO foca (fire-and-forget, seguís en la
        // línea). Volver a la línea: click en el prompt/cabezal (`FocusInput`).
        let blk = s.bg_jobs.last().and_then(|j| j.lock().ok().map(|g| g.block));
        if let Some(b) = blk {
            s.input_focus = Some(b);
        }
        return s;
    }
    start_run(s, exec_line)
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
    // Cada job de fondo vive en SU propia card (bloque propio). Sin esto
    // su salida se intercalaba en la card del comando de foreground.
    let bg_block = s.open_block();
    s.push_in_block(bg_block, OutputLine::prompt(format!("[{idx}] $ {line} &")));
    let active = ActiveRun {
        handle: BackendHandle::Local(handle),
        killer: Some(killer),
        command: line,
        tui: None,
        block: bg_block,
    };
    s.bg_jobs.push(Arc::new(Mutex::new(active)));
    s
}

/// E2 — el scrollback como base de datos: resuelve las etapas-referencia
/// `%cN`/`%pN` de una línea materializando el stdout de esos bloques. Devuelve
/// `(línea_ejecutable, stdin_inyectado)`:
/// - `%c12 | grep error | sort` → ejecuta `grep error | sort` con el stdout del
///   bloque 12 como stdin (la ref es la fuente de datos del pipeline).
/// - `%c12` solo → `cat` con ese stdin (re-muestra el bloque, consultable).
/// - sin refs → la línea tal cual, sin inyección.
///
/// Tanto `%cN` (comando) como `%pN` (buffer) referencian el stdout del bloque
/// `N`; el shell no materializa buffers intermedios aparte. El chip `» stdin`
/// (reprocess) es el caso degenerado de esto.
pub(crate) fn resolve_injects(s: &State, line: &str) -> (String, Option<String>) {
    let intention = shuma_intent::Intention::parse(line);
    let tiene_ref = intention
        .stages
        .iter()
        .any(|st| matches!(st, shuma_intent::Stage::Inject(_)));
    if !tiene_ref {
        return (line.to_string(), None);
    }
    let mut data = String::new();
    let mut exec_stages: Vec<String> = Vec::new();
    for st in &intention.stages {
        match st {
            shuma_intent::Stage::Inject(r) => {
                let block = match r {
                    shuma_intent::Ref::Command(n) | shuma_intent::Ref::Buffer(n) => *n as u64,
                };
                data.push_str(&gather_block_stdout(s, block));
            }
            shuma_intent::Stage::Exec(cmd) => exec_stages.push(cmd.clone()),
        }
    }
    // Sólo refs (sin comando) → `cat` re-muestra el contenido inyectado.
    let exec_line = if exec_stages.is_empty() {
        "cat".to_string()
    } else {
        exec_stages.join(" | ")
    };
    (exec_line, Some(data))
}

pub(crate) fn start_run(mut s: State, line: String) -> State {
    let cwd_str = s.cwd.display().to_string();
    // E2 — resolución de `%cN`/`%pN`: la línea ejecutable puede diferir de la
    // tipeada (las refs se sacan del pipe y su stdout va al stdin).
    let (exec_line, injected_stdin) = resolve_injects(&s, &line);
    let (mut spec, tui) = build_spec(&exec_line, &cwd_str);
    // Config de captura y reprocess sólo aplican a runs no-PTY (los TUI
    // capturan a vt100, no a buffer, y no consumen stdin reprocesado).
    if tui.is_none() {
        spec.capture_limit = s.capture_limit_bytes;
        spec.spill_path = (s.spill && s.capture_limit_bytes > 0).then(|| {
            std::env::temp_dir().join(format!(
                "shuma-spill-{}-{}.log",
                std::process::id(),
                s.current_block
            ))
        });
        // E2 — stdin inyectado por `%cN`/`%pN` (tiene prioridad sobre el
        // reprocess del chip). Si la línea trajo refs, ya desarmamos cualquier
        // reprocess pendiente: la fuente explícita manda.
        if let Some(data) = injected_stdin {
            if !data.is_empty() {
                spec.stdin_data = Some(data);
            }
            s.reprocess_source = None;
        } else if let Some(src) = s.reprocess_source.take() {
            // Reprocess armado: el stdout del bloque fuente alimenta el stdin.
            let data = gather_block_stdout(&s, src);
            if !data.is_empty() {
                spec.stdin_data = Some(data);
            }
        }
    } else {
        // Un run TUI desarma cualquier reprocess pendiente (no aplica).
        s.reprocess_source = None;
    }
    // Registramos la intención antes de hacer spawn — si el spawn
    // remoto falla, igual queda el nodo `%cN` con status `Failed`
    // marcado más abajo (vía el RunEvent::Failed que retorna el
    // backend). El lienzo refleja el intento.
    s.current_run_node = Some(s.intent_graph.record(line.clone()));
    s.current_run_bytes = 0;
    // El prompt de este run ya abrió su bloque (current_block); fijamos
    // que TODA su salida —drenada en ticks futuros— vaya a esa card.
    let run_block = s.current_block;
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
                block: run_block,
            }
        }
        Source::Daemon { socket, .. } => {
            let sock = socket
                .clone()
                .unwrap_or_else(shuma_protocol::default_socket_path);
            // PTY remoto full-duplex: conservamos la `TuiSession` para
            // pintar el terminal localmente; las teclas/resize viajan al
            // daemon por el asa remota.
            if tui.is_some() {
                match shuma_remote_exec::run_pty(&spec, &sock) {
                    Ok(h) => ActiveRun {
                        handle: BackendHandle::Remote(h),
                        killer: None,
                        command: line,
                        tui,
                        block: run_block,
                    },
                    Err(e) => {
                        s.push_output(OutputLine::notice(format!("✘ daemon pty: {e}")));
                        fail_pending_intent(&mut s);
                        return s;
                    }
                }
            } else {
                match shuma_remote_exec::run(&spec, &sock) {
                    Ok(h) => ActiveRun {
                        handle: BackendHandle::Remote(h),
                        killer: None,
                        command: line,
                        tui: None,
                        block: run_block,
                    },
                    Err(e) => {
                        s.push_output(OutputLine::notice(format!("✘ daemon: {e}")));
                        fail_pending_intent(&mut s);
                        return s;
                    }
                }
            }
        }
        Source::DaemonTcp {
            addr,
            server_pub_hex,
            ..
        } => {
            // Identidad y pubkey del server hacen falta en ambos caminos
            // (PTY y no-PTY); las resolvemos una vez antes de ramificar.
            let kp = match load_or_create_identity() {
                Ok(kp) => kp,
                Err(e) => {
                    s.push_output(OutputLine::notice(format!("✘ identity: {e}")));
                    fail_pending_intent(&mut s);
                    return s;
                }
            };
            let server_pub = match parse_pub_hex(server_pub_hex) {
                Ok(p) => p,
                Err(e) => {
                    s.push_output(OutputLine::notice(format!("✘ server_pub_hex: {e}")));
                    fail_pending_intent(&mut s);
                    return s;
                }
            };
            if tui.is_some() {
                match shuma_remote_exec::run_pty_tcp(&spec, addr, kp, server_pub) {
                    Ok(h) => ActiveRun {
                        handle: BackendHandle::Remote(h),
                        killer: None,
                        command: line,
                        tui,
                        block: run_block,
                    },
                    Err(e) => {
                        s.push_output(OutputLine::notice(format!("✘ daemon tcp pty: {e}")));
                        fail_pending_intent(&mut s);
                        return s;
                    }
                }
            } else {
                match shuma_remote_exec::run_tcp(&spec, addr, kp, server_pub) {
                    Ok(h) => ActiveRun {
                        handle: BackendHandle::Remote(h),
                        killer: None,
                        command: line,
                        tui: None,
                        block: run_block,
                    },
                    Err(e) => {
                        s.push_output(OutputLine::notice(format!("✘ daemon tcp: {e}")));
                        fail_pending_intent(&mut s);
                        return s;
                    }
                }
            }
        }
        Source::Remote { host, user, port, .. } => {
            // Shell SSH (russh vía shuma-remote-exec::run_ssh). v1: cada comando
            // es un `ssh exec` (sin PTY/streaming). La auth sale de hosts.json.
            match resolve_ssh_auth(host, user) {
                Ok(auth) => {
                    let cwd = s.cwd.display().to_string();
                    let handle =
                        shuma_remote_exec::run_ssh(&line, &cwd, host, user, *port, auth);
                    ActiveRun {
                        handle: BackendHandle::Remote(handle),
                        killer: None,
                        command: line,
                        tui: None,
                        block: run_block,
                    }
                }
                Err(e) => {
                    s.push_output(OutputLine::notice(format!("✘ SSH: {e}")));
                    fail_pending_intent(&mut s);
                    return s;
                }
            }
        }
        Source::Container { engine, name, .. } => {
            // Envuelve el spec en `<engine> exec` contra el contenedor. El wrap
            // ya leyó `spec.cwd` como cwd INTERIOR (lo inyecta como `cd` adentro).
            let mut wrapped = wrap_spec_for_container(spec.clone(), engine, name);
            // El cwd del SPAWN en el host debe ser válido y accesible: el cwd
            // interior (p.ej. `/root`) no existe/no es accesible en el host y
            // haría fallar el spawn. `/` siempre sirve; el chroot/bind manda.
            wrapped.cwd = "/".to_string();
            let handle = shuma_exec::run(&wrapped);
            let killer = handle.killer();
            ActiveRun {
                handle: BackendHandle::Local(handle),
                killer: Some(killer),
                command: line,
                tui,
                block: run_block,
            }
        }
        Source::RemoteContainer {
            host, user, port, engine, name, ..
        } => {
            // El comando viaja por SSH y allá se envuelve en `<engine> exec`
            // (o `chroot` para rootfs). El cwd interior va DENTRO del wrap (un
            // `cd` en el shell del contenedor); a `run_ssh` le pasamos "~" para
            // que no anteponga un `cd` del lado del HOST remoto. v1: sin PTY.
            match resolve_ssh_auth(host, user) {
                Ok(auth) => {
                    let cwd = s.cwd.display().to_string();
                    let cmd = remote_container_command(&line, engine, name, &cwd);
                    let handle = shuma_remote_exec::run_ssh(&cmd, "~", host, user, *port, auth);
                    ActiveRun {
                        handle: BackendHandle::Remote(handle),
                        killer: None,
                        command: line,
                        tui: None,
                        block: run_block,
                    }
                }
                Err(e) => {
                    s.push_output(OutputLine::notice(format!("✘ SSH: {e}")));
                    fail_pending_intent(&mut s);
                    return s;
                }
            }
        }
    };
    s.running = Some(Arc::new(Mutex::new(active)));
    // El comando recién arrancado recibe el foco del input: el Enter siguiente
    // alimenta SU stdin. Para lanzar otro, el usuario vuelve a la línea
    // (click en el prompt/cabezal → `FocusInput`).
    s.input_focus = Some(run_block);
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

pub(crate) fn drain_run(mut s: State) -> State {
    let Some(active_arc) = s.running.clone() else {
        return s;
    };
    let mut finished_with: Option<RunEvent> = None;
    // Bloque de ESTE run — toda su salida va a su card, aunque el usuario
    // haya tipeado otros comandos (que movieron `current_block`) mientras
    // corría.
    let run_block;
    {
        let mut guard = match active_arc.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        run_block = guard.block;
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
        // Limitamos los eventos por tick. Un `ls -alR /` puede escupir miles
        // de líneas en un solo flush; procesar todo dentro del lock de
        // `active_arc` pasma la pantalla porque el render llama `try_lock`
        // en cada frame. Con un tope, el lock se libera entre Ticks y la UI
        // se actualiza sin esperar al final del comando. Los restantes
        // QUEDAN EN LA COLA del backend para el próximo Tick (no se pierden).
        const DRAIN_BUDGET: usize = 512;
        let events = guard.handle.try_events_limit(DRAIN_BUDGET);
        for ev in events.into_iter() {
            match ev {
                RunEvent::Stdout(line) => {
                    // +1 por el `\n` implícito de cada línea drenada.
                    s.current_run_bytes = s.current_run_bytes.saturating_add(line.len() as u64 + 1);
                    // Strip ANSI escapes: comandos como `fastfetch`, `ls
                    // --color`, `git --color=always` emiten SGR + `\r` para
                    // sobrescribir. `strip_ansi` colapsa `\r` y descarta
                    // códigos sin perder el contenido visible. El coloreo
                    // real (style runs) queda para una iteración futura.
                    let clean = shuma_line::ansi::strip_ansi(&line);
                    s.push_in_block(run_block, OutputLine::stdout(clean));
                }
                RunEvent::StageStdout { stage, line } => {
                    // Salida de una etapa intermedia (tee). NO suma a
                    // `current_run_bytes` (el grafo cuenta la salida final);
                    // queda guardada para el desplegable de su etapa.
                    let clean = shuma_line::ansi::strip_ansi(&line);
                    s.push_in_block(run_block, OutputLine::stage_stdout(stage, clean));
                }
                RunEvent::Stderr(line) => {
                    s.current_run_bytes = s.current_run_bytes.saturating_add(line.len() as u64 + 1);
                    let clean = shuma_line::ansi::strip_ansi(&line);
                    s.push_in_block(run_block, OutputLine::stderr(clean));
                }
                RunEvent::Truncated => s.push_in_block(
                    run_block,
                    OutputLine::notice("… (salida truncada por límite de captura)"),
                ),
                RunEvent::Spilled(path) => s.push_in_block(
                    run_block,
                    OutputLine::notice(format!("… (resto volcado a {path})")),
                ),
                RunEvent::Bytes(bytes) => {
                    s.current_run_bytes = s.current_run_bytes.saturating_add(bytes.len() as u64);
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
        s.push_in_block(run_block, OutputLine::notice(notice));
        // Sella el cierre para el titular semáforo del header colapsado
        // (duración = ended − started).
        let ended = now_unix_secs();
        s.block_ended.insert(run_block, ended);
        // A6 — comando largo terminado.
        register_long_command(&mut s, run_block, ended);
        // A4 — si falló por `command not found`, ofrecé la corrección.
        if !ok {
            detect_did_you_mean(&mut s, run_block);
        }
        // El comando terminado queda EXPANDIDO; sólo recede (se pliega) al
        // perderse en el historial cuando arranca uno nuevo (ver
        // `recede_previous_blocks` en `run_submitted`).
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
        // E3 — [rules].on_exit_nonzero: si el comando falló y la regla está
        // armada, corré el comando declarado (típicamente un builtin como
        // `:jobs`). La guarda evita que el propio comando de la regla la
        // re-dispare. Sólo si no quedó otro corriendo de la cola.
        if !ok && !s.exit_rule_fired && s.running.is_none() {
            if let Some(cmd) = s.config.rules.on_exit_nonzero.clone() {
                let cmd = cmd.trim().to_string();
                if !cmd.is_empty() {
                    s.input.set_text(&cmd);
                    s = run_submitted(s);
                    s.exit_rule_fired = true;
                }
            }
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
    // Snapshot de los Arc: `push_output` toma `&mut s`, incompatible con
    // retener el borrow de `s.bg_jobs` durante el loop.
    let jobs = s.bg_jobs.clone();
    for arc in jobs.iter() {
        let mut keep = true;
        let mut finished: Option<RunEvent> = None;
        // Bloque propio del job — su salida vive en SU card, nunca en la
        // del foreground (era el bug del "output mezclado").
        let job_block;
        {
            let mut guard = match arc.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            job_block = guard.block;
            // Mismo límite que `drain_run`: ráfagas grandes de un job background
            // no pasman la pantalla; los eventos restantes se procesan en el
            // próximo Tick.
            for ev in guard.handle.try_events_limit(512) {
                match ev {
                    RunEvent::Stdout(line) => s.push_in_block(
                        job_block,
                        OutputLine::stdout(shuma_line::ansi::strip_ansi(&line)),
                    ),
                    RunEvent::StageStdout { stage, line } => s.push_in_block(
                        job_block,
                        OutputLine::stage_stdout(stage, shuma_line::ansi::strip_ansi(&line)),
                    ),
                    RunEvent::Stderr(line) => s.push_in_block(
                        job_block,
                        OutputLine::stderr(shuma_line::ansi::strip_ansi(&line)),
                    ),
                    RunEvent::Truncated => {
                        s.push_in_block(job_block, OutputLine::notice("… (truncada)"))
                    }
                    RunEvent::Spilled(path) => s.push_in_block(
                        job_block,
                        OutputLine::notice(format!("… (volcado a {path})")),
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
                RunEvent::Exited(0) => "✔ exit 0".to_string(),
                RunEvent::Exited(code) => format!("✘ exit {code}"),
                RunEvent::Failed(e) => format!("✘ failed: {e}"),
                _ => unreachable!(),
            };
            s.push_in_block(job_block, OutputLine::notice(notice));
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
    let mut run_block = s.current_block;
    if let Some(arc) = s.running.as_ref() {
        let guard = match arc.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        run_block = guard.block;
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
    s.push_in_block(run_block, OutputLine::notice("⏹ cancel (SIGKILL enviado)"));
    s
}

/// A6 — registra un comando largo terminado: si duró ≥
/// `[rules].on_long_command_secs` (`0` = apagado), suma una alerta para la badge
/// del diente (que el chasis pinta cuando la sesión no está activa) y deja un
/// rastro `⏲` en el bloque. Sin notificaciones del sistema: el chasis es la
/// superficie. Puro sobre el `State` — sin la maquinaria de spawn — para poder
/// testearlo directo. `ended` es el cierre en segundos unix.
pub(crate) fn register_long_command(s: &mut State, block: u64, ended: u64) {
    let umbral = s.config.rules.on_long_command_secs;
    if umbral == 0 {
        return;
    }
    let Some(&started) = s.block_started.get(&block) else {
        return;
    };
    let dur = ended.saturating_sub(started);
    if dur < umbral {
        return;
    }
    s.long_alerts += 1;
    s.push_in_block(
        block,
        OutputLine::notice(format!("⏲ comando largo — terminó tras {dur}s")),
    );
}

#[cfg(test)]
mod a6_long_command_tests {
    use super::*;

    /// State con `on_long_command_secs = umbral` y un bloque que arrancó hace
    /// `dur` segundos (ended − started = dur).
    fn state_con_bloque_durado(umbral: u64, dur: u64) -> (State, u64, u64) {
        let mut s = State::new(shuma_module::Source::Local);
        s.config.rules.on_long_command_secs = umbral;
        let block = 7;
        let started = 1_000_000;
        s.block_started.insert(block, started);
        (s, block, started + dur)
    }

    #[test]
    fn comando_largo_suma_alerta_y_rastro() {
        let (mut s, block, ended) = state_con_bloque_durado(30, 45);
        register_long_command(&mut s, block, ended);
        assert_eq!(s.long_alerts(), 1);
        // Dejó el rastro ⏲ en el bloque.
        assert!(s
            .output
            .iter()
            .any(|l| l.block == block && l.text.contains("⏲") && l.text.contains("45s")));
    }

    #[test]
    fn comando_corto_no_alerta() {
        let (mut s, block, ended) = state_con_bloque_durado(30, 5);
        register_long_command(&mut s, block, ended);
        assert_eq!(s.long_alerts(), 0);
    }

    #[test]
    fn umbral_cero_apaga_la_funcion() {
        let (mut s, block, ended) = state_con_bloque_durado(0, 9999);
        register_long_command(&mut s, block, ended);
        assert_eq!(s.long_alerts(), 0);
    }

    #[test]
    fn ack_limpia_la_badge() {
        let (mut s, block, ended) = state_con_bloque_durado(30, 60);
        register_long_command(&mut s, block, ended);
        assert_eq!(s.long_alerts(), 1);
        s.ack_long_alerts();
        assert_eq!(s.long_alerts(), 0);
    }
}

#[cfg(test)]
mod e2_inject_tests {
    use super::*;

    fn state_con_bloque(block: u64, lineas: &[&str]) -> State {
        let mut s = State::new(shuma_module::Source::Local);
        for t in lineas {
            let mut l = OutputLine::stdout(*t);
            l.block = block;
            s.output.push(l);
        }
        s
    }

    #[test]
    fn ref_como_fuente_alimenta_el_pipe() {
        let s = state_con_bloque(5, &["foo", "error bar", "baz"]);
        let (exec, stdin) = resolve_injects(&s, "%c5 | grep error");
        assert_eq!(exec, "grep error");
        assert_eq!(stdin.as_deref(), Some("foo\nerror bar\nbaz\n"));
    }

    #[test]
    fn ref_sola_se_remuestra_con_cat() {
        let s = state_con_bloque(12, &["línea uno", "línea dos"]);
        let (exec, stdin) = resolve_injects(&s, "%c12");
        assert_eq!(exec, "cat");
        assert_eq!(stdin.as_deref(), Some("línea uno\nlínea dos\n"));
    }

    #[test]
    fn pn_aliasa_al_stdout_del_bloque() {
        let s = state_con_bloque(3, &["a", "b"]);
        let (exec, stdin) = resolve_injects(&s, "%p3 | sort");
        assert_eq!(exec, "sort");
        assert_eq!(stdin.as_deref(), Some("a\nb\n"));
    }

    #[test]
    fn linea_sin_ref_pasa_intacta() {
        let s = State::new(shuma_module::Source::Local);
        let (exec, stdin) = resolve_injects(&s, "ls -la | grep foo");
        assert_eq!(exec, "ls -la | grep foo");
        assert!(stdin.is_none());
    }
}
