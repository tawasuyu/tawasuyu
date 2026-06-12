use super::*;

pub(crate) fn run_submitted(mut s: State) -> State {
    let line = s.input.text().to_string();
    let trimmed = line.trim().to_string();
    s.input.clear();
    if trimmed.is_empty() {
        return s;
    }
    // Si ya hay un comando vivo y el usuario NO terminó con `&` (que
    // fuerza bg), interpretamos el Enter como respuesta al stdin del
    // running: típico apt Y/n, sudo password, prompts custom. Escribimos
    // `<line>\n` al stdin. El usuario aún puede arrancar un bg paralelo
    // tipeando `cmd &`.
    if s.running.is_some() && !trimmed.ends_with('&') {
        let bytes = {
            let mut v = line.clone().into_bytes();
            v.push(b'\n');
            v
        };
        if let Some(active_arc) = s.running.clone() {
            if let Ok(guard) = active_arc.lock() {
                guard.handle.write_input(bytes);
            }
        }
        // Echo discreto de lo enviado para que el usuario vea qué tipeó.
        s.push_output(OutputLine::notice(format!("← {line}")));
        return s;
    }
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
            ":limit" => return apply_capture_limit(s, rest),
            ":spill" => return apply_spill(s, rest),
            ":scrollback" => return apply_scrollback(s, rest),
            ":save" => return save_group(s, rest),
            ":groups" => return apply_groups_list(s),
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
        return start_bg(s, exec_line);
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

pub(crate) fn start_run(mut s: State, line: String) -> State {
    let cwd_str = s.cwd.display().to_string();
    let (mut spec, tui) = build_spec(&line, &cwd_str);
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
        // Reprocess armado: el stdout del bloque fuente alimenta el stdin.
        if let Some(src) = s.reprocess_source.take() {
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
