use super::*;

#[derive(Debug, Clone, Copy)]
pub(crate) enum JobSignal {
    /// SIGTERM — pedido cortés de terminar (`:term N`).
    Term,
    /// SIGKILL — fin inmediato e incondicional (`:kill N`).
    Kill,
    Stop,
    Cont,
}

/// Lista los bg_jobs con su índice y comando. Marca finalizados.
pub(crate) fn apply_jobs_list(mut s: State) -> State {
    if s.bg_jobs.is_empty() {
        s.push_output(OutputLine::notice("(sin jobs en background)"));
        return s;
    }
    // Snapshot de los Arc para no retener el borrow de `s.bg_jobs`
    // mientras `push_output` toma `&mut s`.
    let jobs = s.bg_jobs.clone();
    for (i, arc) in jobs.iter().enumerate() {
        let (cmd, status) = match arc.lock() {
            Ok(g) => (
                g.command.clone(),
                if g.handle.is_finished() {
                    "done"
                } else {
                    "running"
                },
            ),
            Err(p) => {
                let g = p.into_inner();
                (
                    g.command.clone(),
                    if g.handle.is_finished() {
                        "done"
                    } else {
                        "running"
                    },
                )
            }
        };
        s.push_output(OutputLine::notice(format!("[{i}] {status}  {cmd}")));
    }
    s
}

/// Aplica `:term N` / `:kill N` / `:stop N` / `:cont N` al job de índice `N`.
/// Stop/Cont son no-op en jobs sin `Killer` (remotos vía daemon); Term/Kill
/// caen a cerrar el stream del daemon.
pub(crate) fn apply_jobs_signal(mut s: State, rest: &str, sig: JobSignal) -> State {
    let idx: usize = match rest.trim().parse() {
        Ok(n) => n,
        Err(_) => {
            s.push_output(OutputLine::notice("uso: :term N | :kill N | :stop N | :cont N"));
            return s;
        }
    };
    let Some(arc) = s.bg_jobs.get(idx).cloned() else {
        s.push_output(OutputLine::notice(format!("no hay job [{idx}]")));
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
        JobSignal::Kill => match guard.killer.as_ref() {
            Some(k) => {
                k.kill();
                true
            }
            None => {
                // Remoto: el daemon no expone SIGKILL fino; cerrar el stream
                // es lo más fuerte que tenemos.
                guard.handle.kill();
                true
            }
        },
        JobSignal::Stop => guard.killer.as_ref().map(|k| k.stop()).unwrap_or(false),
        JobSignal::Cont => guard.killer.as_ref().map(|k| k.cont()).unwrap_or(false),
    };
    let label = match sig {
        JobSignal::Term => "TERM",
        JobSignal::Kill => "KILL",
        JobSignal::Stop => "STOP",
        JobSignal::Cont => "CONT",
    };
    drop(guard);
    s.push_output(OutputLine::notice(if acted {
        format!("[{idx}] SIG{label} enviado")
    } else {
        format!("[{idx}] no se pudo enviar SIG{label}")
    }));
    s
}

/// `:limit <MB>` — tope de captura de stdout por run. `0` = sin tope.
pub(crate) fn apply_capture_limit(mut s: State, rest: &str) -> State {
    match rest.trim().parse::<usize>() {
        Ok(mb) => {
            s.capture_limit_bytes = mb.saturating_mul(1024 * 1024);
            let msg = if mb == 0 {
                "captura sin tope".to_string()
            } else {
                format!("captura limitada a {mb} MB por comando")
            };
            s.push_output(OutputLine::notice(msg));
        }
        Err(_) => s.push_output(OutputLine::notice("uso: :limit <MB>  (0 = sin tope)")),
    }
    s
}

/// `:spill on|off` — volcar a disco la salida que excede el `:limit`.
pub(crate) fn apply_spill(mut s: State, rest: &str) -> State {
    let arg = rest.trim();
    let on = matches!(arg, "on" | "si" | "sí" | "1" | "true");
    let off = matches!(arg, "off" | "no" | "0" | "false");
    if !on && !off {
        s.push_output(OutputLine::notice("uso: :spill on|off"));
        return s;
    }
    s.spill = on;
    let note = match (on, s.capture_limit_bytes) {
        (true, 0) => "spill activado — pero sin `:limit <MB>` no tiene efecto",
        (true, _) => "spill activado — la salida excedente se vuelca a disco",
        (false, _) => "spill desactivado",
    };
    s.push_output(OutputLine::notice(note));
    s
}

/// `:scrollback` (sin args): muestra el estado del scrollback persistente —
/// líneas en memoria + spilleadas + path del spill file. `:scrollback open`
/// abre el spill file con `$EDITOR` (o cae a `xdg-open`) para que el
/// usuario inspeccione el archive sin salir del shell.
pub(crate) fn apply_scrollback(mut s: State, rest: &str) -> State {
    let arg = rest.trim();
    let snap = match s.surf_history.lock() {
        Ok(g) => (g.len(), g.spilled_count(), g.spill_path()),
        Err(p) => {
            let g = p.into_inner();
            (g.len(), g.spilled_count(), g.spill_path())
        }
    };
    let (in_mem, in_spill, spill_path) = snap;
    match arg {
        "open" => {
            let Some(path) = spill_path else {
                s.push_output(OutputLine::notice(
                    ":scrollback open — el spill no está activo (ver [scrollback].spill = true en shumarc.toml)",
                ));
                return s;
            };
            let path_s = path.display().to_string();
            // Preferimos $EDITOR para inspección textual; si no, xdg-open.
            if let Ok(editor) = std::env::var("EDITOR") {
                spawn_detached(&editor, &[path_s.as_str()]);
            } else {
                spawn_detached("xdg-open", &[path_s.as_str()]);
            }
            s.push_output(OutputLine::notice(format!("abriendo {path_s}…")));
        }
        "" => {
            // Estado: líneas en memoria + en spill + path.
            s.push_output(OutputLine::notice(format!(
                "scrollback: {in_mem} líneas en memoria, {in_spill} archivadas"
            )));
            if let Some(p) = spill_path {
                s.push_output(OutputLine::notice(format!(
                    "spill: {} ({:?})",
                    p.display(),
                    p.metadata().ok().map(|m| m.len()).unwrap_or(0)
                )));
                s.push_output(OutputLine::notice(
                    "abrí con `:scrollback open` o `cat`-éalo desde otra shell",
                ));
            } else {
                s.push_output(OutputLine::notice(
                    "spill no activo — activá con [scrollback].spill = true en shumarc.toml",
                ));
            }
        }
        a if a.starts_with("grep ") => {
            let pattern = a[5..].trim();
            if pattern.is_empty() {
                s.push_output(OutputLine::notice("uso: :scrollback grep <patrón>"));
                return s;
            }
            s = apply_scrollback_grep(s, pattern);
        }
        _ => {
            s.push_output(OutputLine::notice("uso: :scrollback [open | grep <patrón>]"));
        }
    }
    s
}

/// Busca un substring literal en TODO el archive del scrollback — tanto
/// las líneas en memoria como las del spill file. Útil cuando el usuario
/// sabe que algo apareció hace mucho y ya está fuera del cache visible.
/// Reporta los hits como notices con su `global_id` 1-based.
/// Case-sensitive (literal); el usuario usa el `:scrollback open` + el
/// search de `$EDITOR` para casos más complejos.
fn apply_scrollback_grep(mut s: State, pattern: &str) -> State {
    let hist = match s.surf_history.lock() {
        Ok(g) => g.clone(),
        Err(p) => p.into_inner().clone(),
    };
    let mut hits: Vec<(u64, String)> = Vec::new();
    let total_spilled = hist.spilled_count();
    // Spilled: leer una por una. Lentos en archives enormes; el caller
    // que necesite más velocidad usa el editor con su grep.
    for id in 0..total_spilled as u64 {
        if let Ok(Some(text)) = hist.read_spilled(id) {
            if text.contains(pattern) {
                hits.push((id, text));
            }
        }
        // Cap defensivo: nunca más de 1000 hits para no saturar el output.
        if hits.len() >= 1000 {
            break;
        }
    }
    // In-memory: las líneas vigentes (índices 0..len → global ids
    // dropped+0..dropped+len).
    let in_mem = hist.len();
    let dropped = hist.dropped();
    for i in 0..in_mem {
        if hits.len() >= 1000 {
            break;
        }
        if let Some(text) = hist.line(i) {
            if text.contains(pattern) {
                hits.push((dropped + i as u64, text.to_string()));
            }
        }
    }
    if hits.is_empty() {
        s.push_output(OutputLine::notice(format!(
            "grep: sin hits para `{pattern}` ({} líneas revisadas)",
            total_spilled + in_mem
        )));
        return s;
    }
    s.push_output(OutputLine::notice(format!(
        "grep: {} hit{} para `{pattern}`",
        hits.len(),
        if hits.len() == 1 { "" } else { "s" }
    )));
    for (id, text) in hits.iter().take(50) {
        s.push_output(OutputLine::notice(format!("  [{}] {}", id + 1, text)));
    }
    if hits.len() > 50 {
        s.push_output(OutputLine::notice(format!(
            "  … y {} más (cap del builtin a 50 visibles)",
            hits.len() - 50
        )));
    }
    s
}

/// `:save <nombre>` — guarda como grupo los comandos del historial desde el
/// último `:save` (excluyendo los meta-comandos `:`). Ejecutables por F1..F8.
pub(crate) fn save_group(mut s: State, rest: &str) -> State {
    let name = rest.trim().to_string();
    if name.is_empty() {
        s.push_output(OutputLine::notice(
            "uso: :save <nombre>  (agrupa los comandos desde el último :save)",
        ));
        return s;
    }
    let (lines, hist_len) = {
        let Ok(h) = s.history.lock() else {
            return s;
        };
        let entries = h.entries();
        // El propio `:save` ya entró al historial: lo excluimos junto con el
        // resto de meta-comandos `:`.
        let upto = entries.len().saturating_sub(1);
        let lines: Vec<String> = entries
            .get(s.group_anchor..upto)
            .unwrap_or(&[])
            .iter()
            .map(|e| e.line.clone())
            .filter(|l| !l.trim_start().starts_with(':'))
            .collect();
        (lines, entries.len())
    };
    if lines.is_empty() {
        s.push_output(OutputLine::notice(
            "nada que guardar — corré algún comando antes de `:save`",
        ));
        return s;
    }
    // El próximo grupo arranca desde acá.
    s.group_anchor = hist_len;
    // Reemplaza un grupo homónimo, si existe.
    let n = lines.len();
    if let Some(g) = s.groups.iter_mut().find(|g| g.name == name) {
        g.lines = lines;
    } else {
        s.groups.push(CommandGroup { name: name.clone(), lines });
    }
    let fkey = s
        .groups
        .iter()
        .position(|g| g.name == name)
        .map(|i| i + 1)
        .unwrap_or(0);
    s.push_output(OutputLine::notice(format!(
        "grupo «{name}» guardado ({n} comandos) — F{fkey} lo ejecuta"
    )));
    s
}

/// `:groups` — lista los grupos guardados con su tecla de función.
pub(crate) fn apply_groups_list(mut s: State) -> State {
    if s.groups.is_empty() {
        s.push_output(OutputLine::notice(
            "(sin grupos — `:save <nombre>` guarda los últimos comandos)",
        ));
        return s;
    }
    let rows: Vec<String> = s
        .groups
        .iter()
        .enumerate()
        .map(|(i, g)| format!("F{}  {}  ({} cmds)", i + 1, g.name, g.lines.len()))
        .collect();
    for r in rows {
        s.push_output(OutputLine::notice(r));
    }
    s
}

/// Reconstruye el stdout de un bloque (su card) uniendo las líneas
/// `Stdout` sin etapa — para alimentarlo como stdin de un reprocess.
pub(crate) fn gather_block_stdout(s: &State, block: u64) -> String {
    let mut out = String::new();
    for l in &s.output {
        if l.block == block && l.kind == OutputKind::Stdout && l.stage.is_none() {
            out.push_str(&l.text);
            out.push('\n');
        }
    }
    out
}

/// Índice de grupo (0-based) para F1..F8; `None` para cualquier otra tecla.
pub(crate) fn fkey_index(key: &Key) -> Option<usize> {
    match key {
        Key::Named(NamedKey::F1) => Some(0),
        Key::Named(NamedKey::F2) => Some(1),
        Key::Named(NamedKey::F3) => Some(2),
        Key::Named(NamedKey::F4) => Some(3),
        Key::Named(NamedKey::F5) => Some(4),
        Key::Named(NamedKey::F6) => Some(5),
        Key::Named(NamedKey::F7) => Some(6),
        Key::Named(NamedKey::F8) => Some(7),
        _ => None,
    }
}

/// Ejecuta el grupo de índice `idx` (0-based) como una sola línea
/// (`l1 && l2 && …`). No-op si no existe ese grupo.
pub(crate) fn run_group(s: State, idx: usize) -> State {
    let Some(joined) = s
        .groups
        .get(idx)
        .map(|g| g.lines.join(" && "))
        .filter(|j| !j.is_empty())
    else {
        return s;
    };
    let mut s = s;
    s.input.set_text(joined);
    run_submitted(s)
}
