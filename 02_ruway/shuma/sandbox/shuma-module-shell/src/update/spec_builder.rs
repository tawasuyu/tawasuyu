use super::*;

/// Si `line` es un pipe «simple» de ≥2 etapas —sólo `Command`/`Argument`/
/// `Flag`/`Pipe`/espacio, sin comillas, variables, redirecciones,
/// operadores, globs (`* ? [ ] { }`) ni `~`— devuelve sus etapas como
/// [`StageSpec`] para correrlo por `Exec::Direct`. Si no, `None` (cae a
/// `sh -c`, que sí absorbe esa sintaxis). Un único comando también cae a
/// `sh -c`: el modo directo sólo aporta cuando hay tubería que interceptar.
///
/// Conservador a propósito: `shuma_line::Stage` no recoge los `StringLit`
/// en `args`, así que un pipe con comillas debe ir al shell o perdería el
/// argumento citado.
pub(crate) fn simple_pipe_stages(line: &str) -> Option<Vec<StageSpec>> {
    use shuma_line::TokenKind::*;
    let tokens = shuma_line::tokenize(line, shuma_line::Dialect::Bash);
    let simple = !tokens.is_empty()
        && tokens.iter().all(|t| {
            matches!(t.kind, Command | Argument | Flag | Pipe | Whitespace)
                && !t.text.contains(['*', '?', '[', ']', '{', '}'])
                && !t.text.starts_with('~')
        });
    if !simple {
        return None;
    }
    let pipeline = shuma_line::split_pipeline(&tokens);
    if pipeline.stages.len() < 2 {
        return None;
    }
    let mut stages = Vec::with_capacity(pipeline.stages.len());
    for st in &pipeline.stages {
        // Una etapa sin comando (línea incompleta, p. ej. termina en `|`)
        // → al shell, que reporta el error de sintaxis como toca.
        let program = st.command.clone()?;
        stages.push(StageSpec {
            program,
            args: st.args.clone(),
        });
    }
    Some(stages)
}

/// Decide cómo lanzar `line`: si el primer token está en la allowlist
/// TUI (o el usuario lo prefijó con `:tui`), abre un PTY; si es un pipe
/// simple, lo corre directo con captura por etapa; si no, va por el shell
/// normal (streaming Stdout/Stderr).
/// Inserta `-A` después de `sudo` cuando el usuario no lo puso, para que
/// sudo dispare `SUDO_ASKPASS` (popup) en vez de quedar colgado leyendo
/// stdin del PTY. Respeta `-A`, `-S`, `--askpass`, `--stdin` ya presentes.
/// Sólo toca la primera ocurrencia al principio del line — pipes / `&&` /
/// `;` van por su cuenta (el shell del PTY los maneja).
pub(crate) fn build_spec(line: &str, cwd: &str) -> (CommandSpec, Option<TuiSession>) {
    // sudo sin `-A`/`-S` quedaría colgado pidiendo pass en stdin del PTY —
    // inyectamos `-A` para que use `SUDO_ASKPASS` (popup Llimphi).
    let line_owned = inject_askpass(line);
    let line = line_owned.as_str();
    // Prefijo explícito `:tui <comando>`.
    let (cmd_line, force_tui) = match line.strip_prefix(":tui ") {
        Some(rest) => (rest.trim(), true),
        None => (line, false),
    };
    let first_word = cmd_line.split_whitespace().next().unwrap_or("");
    let is_tui = force_tui || TUI_ALLOWLIST.contains(&first_word);
    if !is_tui {
        // Pipe «simple» (sólo comandos/args/flags y `|`, sin comillas,
        // variables, redirecciones, globs ni `~`): lo corremos directo
        // —conectando los procesos nosotros— y activamos la captura por
        // etapa (tee) para inspeccionar los intermedios en vivo. Cualquier
        // sintaxis que el modo directo no absorbe cae a `sh -c`.
        if let Some(stages) = simple_pipe_stages(line) {
            return (
                CommandSpec {
                    exec: Exec::Direct { stages },
                    cwd: cwd.to_string(),
                    capture_limit: 0,
                    spill_path: None,
                    stdin_data: None,
                    capture_stages: true,
                },
                None,
            );
        }
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
        capture_stages: false,
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
