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

/// `:env` — variables de ambiente **aprendibles**, organizadas en grupos
/// (el panel «Environment» del sidebar muestra y activa/desactiva los
/// grupos; este builtin es la vía de teclado).
///
/// - `:env` lista los grupos con sus variables.
/// - `:env NAME=VALOR [@grupo]` exporta al proceso Y la aprende al grupo
///   (default «general») en `env.json` — sobrevive reinicios.
/// - `:env -NAME` la olvida (proceso + todos los grupos).
pub(crate) fn apply_env(mut s: State, rest: &str) -> State {
    let arg = rest.trim();
    if arg.is_empty() {
        let groups = shuma_config::load_env_groups();
        if groups.iter().all(|g| g.vars.is_empty()) {
            s.push_output(OutputLine::notice(
                "(sin variables — `:env NAME=valor` exporta y aprende al grupo «general»)",
            ));
        } else {
            for g in &groups {
                s.push_output(OutputLine::notice(format!(
                    "[{}] {} — {} variable{}",
                    if g.active { "on " } else { "off" },
                    g.name,
                    g.vars.len(),
                    if g.vars.len() == 1 { "" } else { "s" },
                )));
                for (k, v) in &g.vars {
                    s.push_output(OutputLine::notice(format!("    {k}={v}")));
                }
            }
        }
        return s;
    }
    // `:env -NAME` — olvidar de todos los grupos.
    if let Some(name) = arg.strip_prefix('-') {
        let name = name.trim();
        if !es_nombre_env(name) {
            s.push_output(OutputLine::notice("uso: :env [-NAME | NAME=valor [@grupo]]"));
            return s;
        }
        std::env::remove_var(name);
        let mut groups = shuma_config::load_env_groups();
        let mut hits = 0;
        for g in &mut groups {
            if g.remove(name) {
                hits += 1;
            }
        }
        if hits > 0 {
            let _ = shuma_config::save_env_groups(&groups);
            s.push_output(OutputLine::notice(format!(
                "✔ {name} olvidada ({hits} grupo{})",
                if hits == 1 { "" } else { "s" }
            )));
        } else {
            s.push_output(OutputLine::notice(format!(
                "{name} no estaba en ningún grupo — igual se removió del proceso"
            )));
        }
        return s;
    }
    // `:env NAME=VALOR [@grupo]` — exportar + aprender.
    let (asign, grupo) = match arg.rsplit_once('@') {
        Some((a, g)) if !g.trim().is_empty() && !g.contains('=') => {
            (a.trim(), g.trim().to_string())
        }
        _ => (arg, "general".to_string()),
    };
    let Some((name, value)) = asign.split_once('=') else {
        s.push_output(OutputLine::notice("uso: :env                      (listar grupos)"));
        s.push_output(OutputLine::notice("     :env NAME=valor [@grupo]  (exportar + aprender)"));
        s.push_output(OutputLine::notice("     :env -NAME                (olvidar)"));
        return s;
    };
    let (name, value) = (name.trim(), value.trim());
    if !es_nombre_env(name) {
        s.push_output(OutputLine::notice(format!(
            "✘ `{name}` no es un nombre de variable válido ([A-Za-z_][A-Za-z0-9_]*)"
        )));
        return s;
    }
    // Valor con expansión de `$VAR` contra el ambiente vigente — permite
    // `:env PATH=$PATH:/opt/bin`.
    let value = shuma_config::expand_env(value);
    let mut groups = shuma_config::load_env_groups();
    let g = match groups.iter_mut().find(|g| g.name == grupo) {
        Some(g) => g,
        None => {
            groups.push(shuma_config::EnvGroup::new(grupo.clone()));
            groups.last_mut().expect("recién pusheado")
        }
    };
    g.upsert(name, &value);
    let activo = g.active;
    if activo {
        std::env::set_var(name, &value);
    }
    match shuma_config::save_env_groups(&groups) {
        Ok(()) => s.push_output(OutputLine::notice(format!(
            "✔ {name}={value} aprendida al grupo «{grupo}»{}",
            if activo { " y exportada" } else { " (grupo inactivo — no exporta)" }
        ))),
        Err(e) => s.push_output(OutputLine::notice(format!(
            "{name}={value} exportada — pero no se pudo guardar env.json: {e}"
        ))),
    }
    s
}

fn es_nombre_env(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

/// `:persist` — asegura que la sesión persista lo máximo posible hoy.
///
/// - `:persist` muestra el estado de cada capa de persistencia.
/// - `:persist on` enciende captura con spill (límite default 64 MB si no
///   había) y lo aprende al rc (`[capture]` + `[scrollback]`).
/// - `:persist off` apaga el spill (el resto de las capas no se tocan).
pub(crate) fn apply_persist(mut s: State, rest: &str) -> State {
    let arg = rest.trim();
    match arg {
        "" => {
            let limit_mb = s.capture_limit_bytes / (1024 * 1024);
            let spill_estado = match (s.spill, s.capture_limit_bytes) {
                (true, 0) => "spill on, pero sin :limit (sin efecto)".to_string(),
                (true, _) => format!("spill on, límite {limit_mb} MB"),
                (false, _) => "off — `:persist on` lo enciende".to_string(),
            };
            let scrollback = match s.surf_history.lock() {
                Ok(h) if h.spill_path().is_some() => "✔ spillea a disco".to_string(),
                _ => "sólo en memoria ([scrollback].spill = true para disco)".to_string(),
            };
            let daemon = daemon_socket_path();
            let daemon_estado = match &daemon {
                Some(p) if p.exists() => format!("✔ corriendo ({})", p.display()),
                Some(p) => format!("no corre (socket esperado: {})", p.display()),
                None => "sin XDG_RUNTIME_DIR".to_string(),
            };
            s.push_output(OutputLine::notice("persistencia de la sesión:"));
            s.push_output(OutputLine::notice("  ✔ historial de comandos — durable siempre"));
            s.push_output(OutputLine::notice(
                "  ✔ sesiones del chasis — sessions.json (se rearman al abrir)",
            ));
            s.push_output(OutputLine::notice(format!("  · captura por comando — {spill_estado}")));
            s.push_output(OutputLine::notice(format!("  · scrollback — {scrollback}")));
            s.push_output(OutputLine::notice(format!("  · shuma-daemon — {daemon_estado}")));
            s.push_output(OutputLine::notice(
                "  · output de la sesión — flag «Persistir sesión» en el panel izquierdo \
                 (guarda y restaura el historial visible al reabrir)",
            ));
            s.push_output(OutputLine::notice(
                "  ⚠ comandos vivos mueren con la app — PTY persistente en el daemon: pendiente",
            ));
        }
        "on" => {
            s.spill = true;
            if s.capture_limit_bytes == 0 {
                s.capture_limit_bytes = 64 * 1024 * 1024;
            }
            let mb = s.capture_limit_bytes / (1024 * 1024);
            if let Some(rc) = shuma_config::Config::default_path() {
                let _ = shuma_config::upsert_key(&rc, "capture", "limit_mb", &mb.to_string());
                let _ = shuma_config::upsert_key(&rc, "capture", "spill", "true");
                let _ = shuma_config::upsert_key(&rc, "scrollback", "spill", "true");
            }
            s.push_output(OutputLine::notice(format!(
                "✔ persistencia on: captura {mb} MB + spill a disco, aprendido al shumarc \
                 (el scrollback spillea desde la próxima sesión)"
            )));
        }
        "off" => {
            s.spill = false;
            if let Some(rc) = shuma_config::Config::default_path() {
                let _ = shuma_config::upsert_key(&rc, "capture", "spill", "false");
                let _ = shuma_config::upsert_key(&rc, "scrollback", "spill", "false");
            }
            s.push_output(OutputLine::notice("persistencia off (spill apagado)"));
        }
        _ => s.push_output(OutputLine::notice("uso: :persist [on|off]")),
    }
    s
}

/// Socket admin esperado del shuma-daemon local.
fn daemon_socket_path() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| std::path::PathBuf::from(d).join("shuma.sock"))
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

// ─────────────────────────── E1 · Macros parametrizables ───────────────────

/// Carga el libro de macros de `~/.config/shuma/macros.toml`. Ausente o
/// corrupto → libro vacío (config de conveniencia, el shell arranca igual).
pub(crate) fn load_macro_book() -> shuma_intent::MacroBook {
    shuma_config::macros_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persiste el libro de macros (atómico: tmp + rename).
pub(crate) fn save_macro_book(book: &shuma_intent::MacroBook) {
    let Some(path) = shuma_config::macros_path() else {
        return;
    };
    let Ok(text) = toml::to_string_pretty(book) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("toml.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Sustituye los huecos `%1..%9` de una plantilla de macro por los argumentos
/// posicionales, y `%*` por todos unidos por espacio. Un `%` sin dígito válido
/// detrás se deja literal. Un hueco sin argumento se reemplaza por vacío.
pub(crate) fn substitute_macro_params(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('*') => {
                chars.next();
                out.push_str(&args.join(" "));
            }
            Some(d) if d.is_ascii_digit() && *d != '0' => {
                let idx = (*d as u8 - b'1') as usize;
                chars.next();
                if let Some(a) = args.get(idx) {
                    out.push_str(a);
                }
            }
            // `%` solo o seguido de algo que no es hueco: literal.
            _ => out.push('%'),
        }
    }
    out
}

/// `:macro [save <nombre> <plantilla> | run <nombre> args… | rm <nombre> |
/// list]` — el plano de control de las macros parametrizables. Sin subcomando
/// (o `list`) las lista.
pub(crate) fn apply_macro(s: State, rest: &str) -> State {
    let mut parts = rest.trim().splitn(2, char::is_whitespace);
    let sub = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();
    match sub {
        "" | "list" | "ls" => list_macros(s),
        "save" | "set" => macro_save(s, arg),
        "run" => macro_run(s, arg),
        "rm" | "del" | "delete" => macro_rm(s, arg),
        other => {
            let mut s = s;
            s.push_output(OutputLine::notice(format!(
                "macro: subcomando «{other}» desconocido — usá save | run | rm | list"
            )));
            s
        }
    }
}

/// `:macro save <nombre> <plantilla>` — guarda (o reemplaza) una macro. La
/// plantilla es todo lo que sigue al nombre y puede tener huecos `%1..%9`.
fn macro_save(mut s: State, arg: &str) -> State {
    let mut it = arg.splitn(2, char::is_whitespace);
    let name = it.next().unwrap_or("").trim();
    let template = it.next().unwrap_or("").trim();
    if name.is_empty() || template.is_empty() {
        s.push_output(OutputLine::notice(
            "uso: :macro save <nombre> <plantilla con %1 %2…>",
        ));
        return s;
    }
    s.macro_book
        .insert(shuma_intent::Macro::new(name).step(template));
    save_macro_book(&s.macro_book);
    s.push_output(OutputLine::notice(format!(
        "✔ macro «{name}» guardada — `:macro run {name} …` la corre"
    )));
    s
}

/// `:macro run <nombre> arg1 arg2 …` — instancia la macro sustituyendo
/// `%1..%9` por los argumentos y la ejecuta (varios pasos → `a && b && …`).
fn macro_run(mut s: State, arg: &str) -> State {
    let mut it = arg.split_whitespace();
    let Some(name) = it.next() else {
        s.push_output(OutputLine::notice("uso: :macro run <nombre> [args…]"));
        return s;
    };
    let args: Vec<&str> = it.collect();
    let Some(m) = s.macro_book.by_name(name) else {
        s.push_output(OutputLine::notice(format!(
            "macro «{name}» no existe — `:macros` las lista"
        )));
        return s;
    };
    let joined = instantiate_macro(m, &args);
    if joined.trim().is_empty() {
        return s;
    }
    s.input.set_text(&joined);
    run_submitted(s)
}

/// Instancia una macro: sustituye `%1..%9`/`%*` en cada paso por `args` y une
/// los pasos con `&&` (una sola línea ejecutable). Puro — sin tocar el State
/// ni disco; el corazón testeable de `:macro run`.
pub(crate) fn instantiate_macro(m: &shuma_intent::Macro, args: &[&str]) -> String {
    m.intentions
        .iter()
        .map(|t| substitute_macro_params(t, args))
        .collect::<Vec<_>>()
        .join(" && ")
}

/// `:macro rm <nombre>` — borra una macro del libro.
fn macro_rm(mut s: State, arg: &str) -> State {
    let name = arg.trim();
    if name.is_empty() {
        s.push_output(OutputLine::notice("uso: :macro rm <nombre>"));
        return s;
    }
    let mut book = shuma_intent::MacroBook::new();
    let mut removed = false;
    for m in s.macro_book.all() {
        if m.name == name {
            removed = true;
        } else {
            book.insert(m.clone());
        }
    }
    if removed {
        s.macro_book = book;
        save_macro_book(&s.macro_book);
        s.push_output(OutputLine::notice(format!("✔ macro «{name}» borrada")));
    } else {
        s.push_output(OutputLine::notice(format!("macro «{name}» no existe")));
    }
    s
}

/// `:macros` / `:macro list` — lista las macros guardadas con su plantilla.
pub(crate) fn list_macros(mut s: State) -> State {
    if s.macro_book.is_empty() {
        s.push_output(OutputLine::notice(
            "(sin macros — `:macro save <nombre> <plantilla %1 %2>` guarda una)",
        ));
        return s;
    }
    let rows: Vec<String> = s
        .macro_book
        .all()
        .iter()
        .map(|m| format!("• {}  →  {}", m.name, m.intentions.join(" && ")))
        .collect();
    for r in rows {
        s.push_output(OutputLine::notice(r));
    }
    s
}

// ─────────────────────────── E6 · :stats (telemetría local) ────────────────

/// Agregado por binario para el reporte de `:stats`.
struct StatRow {
    binario: String,
    veces: u64,
    fallos: u64,
    durs: Vec<u64>,
    ultimo_started: u64,
}

impl StatRow {
    /// Percentil `p` (0.0..=1.0) de las duraciones registradas, en ms.
    /// `None` si ningún run del binario reportó duración.
    fn percentil(&self, p: f64) -> Option<u64> {
        if self.durs.is_empty() {
            return None;
        }
        let mut v = self.durs.clone();
        v.sort_unstable();
        let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
        v.get(idx).copied()
    }
}

/// `:stats [filtro]` — telemetría propia, local, consultable (E6). Lee el
/// historial durable (`line`/`exit`/`started`/`duration_ms`) y arma una tabla
/// por binario: veces, fallos, %fallo, p50/p95 de duración, último uso. La
/// tabla se renderiza con el mismo widget ordenable que `ls -l` (el detector
/// de `sections.rs` reconoce `:stats` y parsea las filas tab-separadas). Cero
/// red: los datos nunca salen de la máquina. Alimenta los rankings de A3/A4.
///
/// `:stats foo` filtra a los binarios cuyo nombre contiene `foo`.
pub(crate) fn apply_stats(mut s: State, rest: &str) -> State {
    let filtro = rest.trim();
    // Snapshot del historial para no retener el lock mientras pusheamos.
    let entries: Vec<shuma_history::Entry> = match s.history.lock() {
        Ok(h) => h.entries().to_vec(),
        Err(p) => p.into_inner().entries().to_vec(),
    };
    if entries.is_empty() {
        s.push_output(OutputLine::notice(
            "(historial vacío — corré algunos comandos y volvé a `:stats`)",
        ));
        return s;
    }
    let now_s = now_unix_millis() / 1000;
    match compute_stats(&entries, filtro, now_s) {
        Some(lines) => {
            for l in lines {
                s.push_output(OutputLine::stdout(l));
            }
        }
        None => s.push_output(OutputLine::notice(if filtro.is_empty() {
            "(sin binarios medibles en el historial)".to_string()
        } else {
            format!("(ningún binario contiene «{filtro}»)")
        })),
    }
    s
}

/// Corazón puro de `:stats`: de un slice de entradas del historial deriva las
/// líneas de salida (1 de resumen sin tab + header + filas tab-separadas que
/// `sections::detect_stats` parsea como tabla ordenable). `None` si no hay
/// ningún binario medible (todo builtins o nada matchea el filtro). `now_s` se
/// inyecta para que el «hace cuánto» sea determinista en tests.
pub(crate) fn compute_stats(
    entries: &[shuma_history::Entry],
    filtro: &str,
    now_s: u64,
) -> Option<Vec<String>> {
    // Agregación por binario (primera palabra de la línea). Los meta-comandos
    // del shell (`:save`, `:stats`…) se omiten: no son procesos medibles.
    let mut por_bin: std::collections::HashMap<String, StatRow> = std::collections::HashMap::new();
    let total_lineas = entries.len();
    let mut con_exit = 0u64;
    let mut horas = [0u64; 24];
    for e in entries {
        let Some(bin) = e.line.split_whitespace().next() else {
            continue;
        };
        if bin.starts_with(':') {
            continue;
        }
        if !filtro.is_empty() && !bin.contains(filtro) {
            continue;
        }
        horas[((e.started / 3600) % 24) as usize] += 1;
        let row = por_bin.entry(bin.to_string()).or_insert_with(|| StatRow {
            binario: bin.to_string(),
            veces: 0,
            fallos: 0,
            durs: Vec::new(),
            ultimo_started: 0,
        });
        row.veces += 1;
        if let Some(code) = e.exit {
            con_exit += 1;
            if code != 0 {
                row.fallos += 1;
            }
        }
        if let Some(d) = e.duration_ms {
            row.durs.push(d);
        }
        row.ultimo_started = row.ultimo_started.max(e.started);
    }
    if por_bin.is_empty() {
        return None;
    }
    // Orden por defecto: más usados primero (la columna es re-ordenable en UI).
    let mut filas: Vec<StatRow> = por_bin.into_values().collect();
    filas.sort_by(|a, b| b.veces.cmp(&a.veces).then(a.binario.cmp(&b.binario)));
    let distintos = filas.len();
    let pico = horas
        .iter()
        .enumerate()
        .max_by_key(|(_, n)| **n)
        .filter(|(_, n)| **n > 0)
        .map(|(h, _)| h);

    let pico_txt = pico
        .map(|h| format!(" · pico {h:02}–{:02}h UTC", (h + 1) % 24))
        .unwrap_or_default();
    let alcance = if filtro.is_empty() {
        String::new()
    } else {
        format!(" (filtro «{filtro}»)")
    };
    let mut out = Vec::with_capacity(filas.len() + 2);
    // Resumen (sin tab → el detector lo deja como sección «resumen»).
    out.push(format!(
        "{total_lineas} comandos en historial · {distintos} binarios distintos · \
         {con_exit} con código de salida{pico_txt}{alcance}"
    ));
    // Tabla tab-separada: header + una fila por binario.
    out.push("comando\tveces\tfallos\t%fallo\tp50ms\tp95ms\túltimo".to_string());
    for f in filas.iter().take(200) {
        let pct = if f.veces > 0 { (f.fallos * 100) / f.veces } else { 0 };
        let p50 = f.percentil(0.50).map(|v| v.to_string()).unwrap_or_else(|| "-".into());
        let p95 = f.percentil(0.95).map(|v| v.to_string()).unwrap_or_else(|| "-".into());
        let ultimo = humanizar_hace(now_s.saturating_sub(f.ultimo_started));
        out.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            f.binario, f.veces, f.fallos, pct, p50, p95, ultimo
        ));
    }
    Some(out)
}

/// «hace cuánto», compacto, desde segundos transcurridos: `ahora` / `Nm` /
/// `Nh` / `Nd`. Determinista, sin formato de fecha (la celda es angosta).
fn humanizar_hace(secs: u64) -> String {
    match secs {
        0..=59 => "ahora".to_string(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

// ─────────────────────────── E5 · LLM como instrumento ─────────────────────

/// `:?  <pregunta>` — lenguaje natural → línea de comando propuesta. El
/// módulo sólo arma la petición (`State::llm_request`); el host corre el LLM
/// y devuelve `Msg::LlmResult`. La respuesta va al **input** para revisar y
/// Enter — NUNCA se auto-ejecuta. Rotulado `🜲 llm`, opt-in por invocación.
pub(crate) fn apply_ask(mut s: State, rest: &str) -> State {
    let q = rest.trim();
    if q.is_empty() {
        s.push_output(OutputLine::notice(
            "uso: :? <qué querés hacer> — el LLM propone una línea de comando (no la ejecuta)",
        ));
        return s;
    }
    let system = "Sos un asistente de shell en Linux. El usuario describe lo que quiere lograr. \
        Respondé EXCLUSIVAMENTE con UNA sola línea de comando de shell que lo cumpla — sin \
        explicación, sin markdown, sin backticks, sin comentarios. Una sola línea."
        .to_string();
    s.llm_request = Some(LlmRequest {
        kind: LlmKind::Command,
        system,
        prompt: q.to_string(),
        max_tokens: 200,
    });
    s.push_output(OutputLine::notice(format!("🜲 llm · pensando una línea para: {q}")));
    s
}

/// `:explica [%cN]` / `:resume [%cN]` — explica o resume la salida de un
/// bloque (la del más reciente si no se da ref). El resultado va al output,
/// rotulado `🜲`. `summarize=true` → `:resume`.
pub(crate) fn apply_explain(mut s: State, rest: &str, summarize: bool) -> State {
    let Some(block) = parse_block_ref(&s, rest) else {
        s.push_output(OutputLine::notice(
            "uso: :explica %cN  (o sin ref, sobre el bloque más reciente con salida)",
        ));
        return s;
    };
    let body = gather_block_stdout(&s, block);
    if body.trim().is_empty() {
        s.push_output(OutputLine::notice(format!(
            "el bloque %c{block} no tiene salida para {}",
            if summarize { "resumir" } else { "explicar" }
        )));
        return s;
    }
    // Cap del cuerpo para no inflar el prompt (los logs gigantes se truncan).
    let body = cap_prompt_body(&body, 8000);
    let (system, verbo) = if summarize {
        (
            "Sos un asistente de shell. Resumí en español, en pocas líneas y conciso, la salida \
             de un comando. Destacá lo importante; nada de relleno.",
            "resumiendo",
        )
    } else {
        (
            "Sos un asistente de shell. Explicá en español, claro y breve, qué dice la salida de \
             un comando y si hay algo que atender (errores, avisos). Sin relleno.",
            "explicando",
        )
    };
    s.llm_request = Some(LlmRequest {
        kind: LlmKind::Text,
        system: system.to_string(),
        prompt: format!("Salida del bloque %c{block}:\n\n{body}"),
        max_tokens: 600,
    });
    s.push_output(OutputLine::notice(format!("🜲 llm · {verbo} el bloque %c{block}…")));
    s
}

/// Resuelve el bloque objetivo de `:explica`/`:resume`: un `%cN`/`%pN`, un
/// número pelado, o —sin ref— el bloque más reciente con stdout.
fn parse_block_ref(s: &State, rest: &str) -> Option<u64> {
    let t = rest.trim();
    if !t.is_empty() {
        let t = t
            .strip_prefix("%c")
            .or_else(|| t.strip_prefix("%p"))
            .unwrap_or(t);
        return t.trim().parse::<u64>().ok();
    }
    // Sin ref: el bloque del stdout más reciente.
    s.output
        .iter()
        .rev()
        .find(|l| l.kind == OutputKind::Stdout && l.stage.is_none())
        .map(|l| l.block)
}

/// Trunca un cuerpo a `max` chars por el medio (cabeza + cola), preservando
/// el principio y el final — lo más útil de un log para resumir/explicar.
fn cap_prompt_body(body: &str, max: usize) -> String {
    if body.len() <= max {
        return body.to_string();
    }
    let head = max * 2 / 3;
    let tail = max - head;
    let start: String = body.chars().take(head).collect();
    let end: String = body.chars().rev().take(tail).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{start}\n…[recortado]…\n{end}")
}

// ─────────── E4 · sesiones PTY persistentes del daemon (sobreviven la app) ──

/// Socket del daemon a usar: el de `Source::Daemon` si la sesión corre contra
/// uno, o el default (`$XDG_RUNTIME_DIR/shuma.sock`) — así `:spawn`/`:sessions`
/// funcionan también en modo Local si hay un daemon corriendo.
fn daemon_socket_for(s: &State) -> std::path::PathBuf {
    match &s.source {
        Source::Daemon { socket, .. } => {
            socket.clone().unwrap_or_else(shuma_protocol::default_socket_path)
        }
        _ => shuma_protocol::default_socket_path(),
    }
}

/// Monta un asa remota + TUI como el run de foreground activo, en un bloque
/// nuevo. Compartido por `:spawn` y `:attach` — ambos rinden una sesión del
/// daemon como si fuera un comando TUI local, pero sobreviven a cerrar shuma.
fn mount_session_run(
    mut s: State,
    handle: shuma_remote_exec::RemoteRunHandle,
    program: &str,
    prompt: String,
    rows: u16,
    cols: u16,
) -> State {
    // El comando previo con cuerpo recede (se pliega), como en run_submitted.
    let prev = s.current_block;
    if prev != 0 && !body_lines_for_block(&s, prev).is_empty() {
        s.collapsed.insert(prev);
    }
    let block = s.open_block();
    s.push_in_block(block, OutputLine::prompt(prompt));
    let tui = TuiSession::new(program, rows, cols);
    s.running = Some(std::sync::Arc::new(std::sync::Mutex::new(ActiveRun {
        handle: BackendHandle::Remote(handle),
        killer: None,
        command: program.to_string(),
        tui: Some(tui),
        block,
    })));
    s.current_block = block;
    s.focused = true;
    s
}

/// `:spawn <cmd>` — corre `<cmd>` como **sesión PTY persistente** del daemon:
/// vive en el daemon, **sobrevive a cerrar shuma**. Se adjunta y la renderiza
/// como un TUI; cerrar shuma (o Ctrl-C) la desadjunta sin matarla
/// (`:sessions` / `shuma pty attach <id>` para reconectar).
pub(crate) fn apply_spawn_session(mut s: State, rest: &str) -> State {
    let cmd = rest.trim();
    if cmd.is_empty() {
        s.push_output(OutputLine::notice(
            "uso: :spawn <comando> — corre en el daemon, sobrevive a cerrar shuma",
        ));
        return s;
    }
    let sock = daemon_socket_for(&s);
    let (rows, cols) = (40u16, 120u16);
    let spec = shuma_exec::CommandSpec {
        exec: shuma_exec::Exec::Pty {
            program: "bash".into(),
            args: vec!["-lc".into(), cmd.to_string()],
            cols,
            rows,
        },
        cwd: s.cwd.display().to_string(),
        capture_limit: 0,
        spill_path: None,
        stdin_data: None,
        capture_stages: false,
    };
    match shuma_remote_exec::spawn_session(&spec, &sock, cmd) {
        Ok((session, handle)) => {
            s = mount_session_run(
                s,
                handle,
                "bash",
                format!("$ :spawn {cmd}  ·  sesión {session} (sobrevive a cerrar shuma)"),
                rows,
                cols,
            );
        }
        Err(e) => {
            s.push_output(OutputLine::notice(format!(
                "✘ :spawn — ¿hay un shuma-daemon corriendo? ({e})"
            )));
        }
    }
    s
}

/// `:sessions` — lista las sesiones PTY persistentes del daemon.
pub(crate) fn apply_sessions(mut s: State, _rest: &str) -> State {
    let sock = daemon_socket_for(&s);
    match shuma_remote_exec::list_sessions(&sock) {
        Ok(sessions) => {
            if sessions.is_empty() {
                s.push_output(OutputLine::notice(
                    "(sin sesiones persistentes — `:spawn <cmd>` crea una)",
                ));
                return s;
            }
            for ss in &sessions {
                let estado = if ss.alive {
                    format!("viva · {} adj", ss.attached)
                } else {
                    format!("muerta · exit {}", ss.exit_code.unwrap_or(-1))
                };
                s.push_output(OutputLine::notice(format!(
                    "{}  {:<20}  [{estado}]  {}",
                    ss.session, ss.label, ss.program
                )));
            }
            s.push_output(OutputLine::notice(
                ":attach <id> para verla · :kill-session <id> para matarla",
            ));
        }
        Err(e) => s.push_output(OutputLine::notice(format!(
            "✘ :sessions — ¿hay un shuma-daemon corriendo? ({e})"
        ))),
    }
    s
}

/// `:attach <id>` — se re-adjunta a una sesión persistente y la renderiza.
pub(crate) fn apply_attach_session(mut s: State, rest: &str) -> State {
    let Ok(id) = ulid::Ulid::from_string(rest.trim()) else {
        s.push_output(OutputLine::notice("uso: :attach <ULID>  (`:sessions` las lista)"));
        return s;
    };
    let sock = daemon_socket_for(&s);
    let (rows, cols) = (40u16, 120u16);
    // Programa para el skin del TUI: lo sacamos de la lista si está.
    let program = shuma_remote_exec::list_sessions(&sock)
        .ok()
        .and_then(|v| v.into_iter().find(|x| x.session == id))
        .map(|x| x.program)
        .unwrap_or_else(|| "bash".into());
    match shuma_remote_exec::attach_session(&sock, id, rows, cols) {
        Ok(handle) => {
            s = mount_session_run(s, handle, &program, format!("$ :attach {id}"), rows, cols);
        }
        Err(e) => s.push_output(OutputLine::notice(format!("✘ :attach — {e}"))),
    }
    s
}

/// `:kill-session <id>` — mata (o reapea) una sesión persistente.
pub(crate) fn apply_kill_session(mut s: State, rest: &str) -> State {
    let Ok(id) = ulid::Ulid::from_string(rest.trim()) else {
        s.push_output(OutputLine::notice("uso: :kill-session <ULID>"));
        return s;
    };
    let sock = daemon_socket_for(&s);
    match shuma_remote_exec::kill_session(&sock, id) {
        Ok(true) => s.push_output(OutputLine::notice(format!("✔ sesión {id} matada"))),
        Ok(false) => s.push_output(OutputLine::notice(format!("no existía: {id}"))),
        Err(e) => s.push_output(OutputLine::notice(format!("✘ {e}"))),
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

#[cfg(test)]
mod e1_macro_tests {
    use super::*;

    #[test]
    fn sustituye_huecos_posicionales() {
        assert_eq!(
            substitute_macro_params("deploy %1 to %2", &["app", "prod"]),
            "deploy app to prod"
        );
        // %* = todos los args.
        assert_eq!(
            substitute_macro_params("run %*", &["a", "b", "c"]),
            "run a b c"
        );
        // Hueco sin argumento → vacío.
        assert_eq!(substitute_macro_params("x %1 %2", &["uno"]), "x uno ");
        // `%` literal (sin dígito válido detrás) se conserva.
        assert_eq!(substitute_macro_params("50%% done", &[]), "50%% done");
        assert_eq!(substitute_macro_params("%0 no es hueco", &["z"]), "%0 no es hueco");
    }

    #[test]
    fn instancia_macro_multipaso() {
        let m = shuma_intent::Macro::new("deploy")
            .step("cargo build --release --bin %1")
            .step("scp target/release/%1 %2:/srv");
        let line = instantiate_macro(&m, &["app", "host"]);
        assert_eq!(
            line,
            "cargo build --release --bin app && scp target/release/app host:/srv"
        );
    }

    #[test]
    fn run_de_macro_inexistente_avisa_y_no_corre() {
        let mut s = State::new(shuma_module::Source::Local);
        s = apply_macro(s, "run no_existe foo");
        assert!(s.output.iter().any(|l| l.text.contains("no existe")));
        assert!(!s.is_running());
    }
}

#[cfg(test)]
mod e6_stats_tests {
    use super::*;
    use shuma_history::Entry;

    /// Entry con exit + duración (el `finalize` real las setea aparte).
    fn ent(line: &str, started: u64, exit: i32, dur: u64) -> Entry {
        let mut e = Entry::new(line, "/tmp", started);
        e.exit = Some(exit);
        e.duration_ms = Some(dur);
        e
    }

    #[test]
    fn agrega_por_binario_con_fallos_y_percentiles() {
        let entries = vec![
            ent("cargo build", 100, 0, 1000),
            ent("cargo test", 200, 1, 3000),
            ent("cargo build", 300, 0, 2000),
            ent("git status", 400, 0, 50),
            // builtin: se ignora.
            Entry::new(":stats", "/tmp", 500),
        ];
        let out = compute_stats(&entries, "", 1_000).expect("hay binarios");
        // Resumen + header + 2 filas (cargo, git).
        assert!(out[0].contains("5 comandos en historial"));
        assert!(out[0].contains("2 binarios distintos"));
        assert_eq!(out[1], "comando\tveces\tfallos\t%fallo\tp50ms\tp95ms\túltimo");
        // cargo va primero (3 usos > 1 de git).
        let cargo = &out[2];
        let cols: Vec<&str> = cargo.split('\t').collect();
        assert_eq!(cols[0], "cargo");
        assert_eq!(cols[1], "3"); // veces
        assert_eq!(cols[2], "1"); // fallos
        assert_eq!(cols[3], "33"); // %fallo (1/3)
    }

    #[test]
    fn filtro_restringe_a_binarios_que_contienen() {
        let entries = vec![ent("cargo build", 100, 0, 10), ent("git log", 200, 0, 10)];
        let out = compute_stats(&entries, "car", 1_000).expect("matchea cargo");
        // Sólo cargo: resumen + header + 1 fila.
        assert_eq!(out.len(), 3);
        assert!(out[2].starts_with("cargo\t"));
        // Filtro que no matchea nada → None.
        assert!(compute_stats(&entries, "zzz", 1_000).is_none());
    }

    #[test]
    fn humaniza_hace_en_tramos() {
        assert_eq!(humanizar_hace(0), "ahora");
        assert_eq!(humanizar_hace(59), "ahora");
        assert_eq!(humanizar_hace(120), "2m");
        assert_eq!(humanizar_hace(7200), "2h");
        assert_eq!(humanizar_hace(172_800), "2d");
    }
}
