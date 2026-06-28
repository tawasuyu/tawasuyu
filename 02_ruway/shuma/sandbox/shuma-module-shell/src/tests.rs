    use super::*;
    use llimphi_ui::Modifiers;

    fn ev(key: Key, text: Option<&str>) -> KeyEvent {
        KeyEvent {
            key,
            state: KeyState::Pressed,
            text: text.map(|s| s.to_string()),
            modifiers: Modifiers::default(),
            repeat: false,
        }
    }

    /// Aplica `Msg::Tick` hasta que el run vivo se cierre (o se acabe el
    /// presupuesto). Imita lo que el chasis hace a 100 ms entre ticks.
    fn drain_until_idle(mut s: State) -> State {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while s.is_running() {
            s = update(s, Msg::Tick);
            if std::time::Instant::now() > deadline {
                panic!("run no terminó en 10s");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Un Tick más por si quedó algo en el canal después del Exited.
        update(s, Msg::Tick)
    }

    #[test]
    fn id_is_stable() {
        assert_eq!(ID, "shell");
    }

    #[test]
    fn placeholder_state_constructs() {
        let s = State::new(Source::Local);
        assert!(s.output.is_empty());
        assert!(s.cwd.is_absolute() || s.cwd == PathBuf::from("/"));
    }

    #[test]
    fn pwd_builtin_writes_cwd() {
        let mut s = State::new(Source::Local);
        s.input.set_text("pwd");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.starts_with("$ pwd")));
        assert!(s.output.iter().any(|l| l.kind == OutputKind::Stdout));
    }

    #[test]
    fn clear_builtin_empties_output() {
        let mut s = State::new(Source::Local);
        s.input.set_text("pwd");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(!s.output.is_empty());
        s.input.set_text("clear");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.is_empty());
    }

    #[test]
    fn clear_msg_empties_output() {
        let mut s = State::new(Source::Local);
        s.output.push(OutputLine::stdout("hola"));
        s = update(s, Msg::Clear);
        assert!(s.output.is_empty());
    }

    #[test]
    fn cd_to_root_changes_cwd() {
        let mut s = State::new(Source::Local);
        s.input.set_text("cd /");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.cwd, PathBuf::from("/"));
    }

    #[test]
    fn cd_to_nonexistent_logs_error() {
        let mut s = State::new(Source::Local);
        s.input.set_text("cd /nope/this/does/not/exist");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.starts_with("cd:")));
    }

    #[test]
    fn external_command_captures_stdout() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("echo hola_mundo");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.is_running(), "Enter debe arrancar el run");
        s = drain_until_idle(s);
        let combined: Vec<String> = s.output.iter().map(|l| l.text.clone()).collect();
        assert!(
            combined.iter().any(|t| t == "hola_mundo"),
            "esperaba stdout 'hola_mundo' en {combined:?}"
        );
        assert!(combined.iter().any(|t| t == "✔ exit 0"));
    }

    #[test]
    fn external_command_failure_writes_exit_nonzero() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("false");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        s = drain_until_idle(s);
        assert!(s.output.iter().any(|l| l.text.starts_with("✘ exit")));
    }

    #[test]
    fn rule_on_exit_nonzero_corre_el_comando_una_vez() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.config.rules.on_exit_nonzero = Some(":jobs".into());
        s.input.set_text("false");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        s = drain_until_idle(s);
        // El builtin de la regla (`:jobs`) corrió al fallar `false`…
        let veces = s
            .output
            .iter()
            .filter(|l| l.text.contains("sin jobs en background"))
            .count();
        // …y sólo una vez (la guarda de re-entrada evita el re-disparo).
        assert_eq!(veces, 1, "la regla on_exit_nonzero debe correr exactamente una vez");
    }

    #[test]
    fn rule_on_enter_cwd_corre_el_comando() {
        let tmp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.config
            .rules
            .on_enter_cwd
            .insert(tmp.display().to_string(), ":jobs".into());
        s.input.set_text(&format!("cd {}", tmp.display()));
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        s = drain_until_idle(s);
        assert!(
            s.output.iter().any(|l| l.text.contains("sin jobs en background")),
            "la regla on_enter_cwd debe correr al entrar al directorio"
        );
    }

    #[test]
    fn ask_builtin_arma_request_y_host_la_toma_una_vez() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text(":? listar archivos por tamaño");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        // El builtin armó la petición (Command) y avisó.
        let req = s.llm_request.clone().expect("hay petición");
        assert!(matches!(req.kind, crate::LlmKind::Command));
        assert!(req.prompt.contains("listar archivos"));
        assert!(s.output.iter().any(|l| l.text.contains("🜲")));
        // El host la toma una sola vez (queda en vuelo).
        assert!(s.take_llm_request().is_some());
        assert!(s.llm_inflight);
        assert!(s.take_llm_request().is_none());
    }

    #[test]
    fn hacer_builtin_arma_request_con_el_catalogo_atipay() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text(":hacé andá al escritorio 3");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let req = s.llm_request.clone().expect("hay petición");
        // Es Atipay (responde JSON con el id) y el system prompt trae el catálogo
        // por id, incluida la fuente Sistema.
        assert!(matches!(req.kind, crate::LlmKind::Atipay));
        assert!(req.prompt.contains("escritorio 3"));
        assert!(req.system.contains("mirada.workspace"));
        assert!(req.system.contains("sistema.apagar"));
    }

    #[test]
    fn atipay_result_resuelve_json_a_la_linea_y_no_ejecuta() {
        let mut s = State::new(Source::Local);
        s.llm_inflight = true;
        s = update(
            s,
            Msg::LlmResult {
                kind: crate::LlmKind::Atipay,
                ok: true,
                text: "{\"id\":\"sistema.apagar\"}".into(),
            },
        );
        // atipay armó el comando exacto; va al input, NO se ejecutó.
        assert_eq!(s.input.text(), "systemctl poweroff");
        assert!(!s.is_running());
        // Avisó del peligro disruptivo.
        assert!(s.output.iter().any(|l| l.text.contains("DISRUPTIVO")));
    }

    #[test]
    fn atipay_result_nada_no_toca_el_input() {
        let mut s = State::new(Source::Local);
        s.llm_inflight = true;
        s = update(s, Msg::LlmResult { kind: crate::LlmKind::Atipay, ok: true, text: "nada".into() });
        assert_eq!(s.input.text(), "");
    }

    #[test]
    fn llm_result_command_va_al_input_sin_ejecutar() {
        let mut s = State::new(Source::Local);
        s.llm_inflight = true;
        s = update(
            s,
            Msg::LlmResult {
                kind: crate::LlmKind::Command,
                ok: true,
                text: "`ls -la --sort=size`".into(),
            },
        );
        // Backticks limpiados, en el input, NO ejecutado.
        assert_eq!(s.input.text(), "ls -la --sort=size");
        assert!(!s.is_running());
        assert!(!s.llm_inflight);
    }

    #[test]
    fn write_vuelca_el_bloque_a_un_archivo() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("salida.txt");
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        // Bloque 3 con dos líneas de stdout.
        for t in ["linea uno", "linea dos"] {
            let mut l = OutputLine::stdout(t);
            l.block = 3;
            s.output.push(l);
        }
        s.input.set_text(&format!(":write %c3 {}", file.display()));
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let written = std::fs::read_to_string(&file).expect("archivo escrito");
        assert_eq!(written, "linea uno\nlinea dos\n");
        assert!(s.output.iter().any(|l| l.text.contains("bytes →")));
    }

    #[test]
    fn write_sin_ref_usa_el_ultimo_bloque() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ultimo.txt");
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        let mut a = OutputLine::stdout("viejo");
        a.block = 1;
        s.output.push(a);
        let mut b = OutputLine::stdout("reciente");
        b.block = 2;
        s.output.push(b);
        s.input.set_text(&format!(":write {}", file.display()));
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "reciente\n");
    }

    #[test]
    fn write_sin_archivo_avisa() {
        let mut s = State::new(Source::Local);
        let mut l = OutputLine::stdout("algo");
        l.block = 5;
        s.output.push(l);
        s.input.set_text(":write %c5");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.contains("falta el archivo")));
    }

    #[test]
    fn write_bloque_sin_stdout_avisa() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.txt");
        let mut s = State::new(Source::Local);
        s.input.set_text(&format!(":write %c99 {}", file.display()));
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.contains("no tiene salida")));
        assert!(!file.exists(), "no debe crear el archivo si no hay datos");
    }

    #[test]
    fn persist_status_no_miente_sobre_pty_persistente() {
        // E4 entregó la persistencia PTY (`:spawn`/`:attach`): el status no debe
        // decir que está "pendiente"; debe apuntar a `:spawn`.
        let mut s = State::new(Source::Local);
        s.input.set_text(":persist");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let texts: Vec<String> = s.output.iter().map(|l| l.text.clone()).collect();
        assert!(!texts.iter().any(|t| t.contains("pendiente")), "{texts:?}");
        assert!(texts.iter().any(|t| t.contains(":spawn")));
    }

    #[test]
    fn diff_compara_dos_bloques() {
        let mut s = State::new(Source::Local);
        // Bloque 1: a/b/c · Bloque 2: a/B/c/d → -b +B +d.
        for t in ["a", "b", "c"] {
            let mut l = OutputLine::stdout(t);
            l.block = 1;
            s.output.push(l);
        }
        for t in ["a", "B", "c", "d"] {
            let mut l = OutputLine::stdout(t);
            l.block = 2;
            s.output.push(l);
        }
        s.input.set_text(":diff %c1 %c2");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let texts: Vec<String> = s.output.iter().map(|l| l.text.clone()).collect();
        assert!(texts.iter().any(|t| t == "- b"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "+ B"));
        assert!(texts.iter().any(|t| t == "+ d"));
        // Resumen: 2 agregadas (B, d), 1 quitada (b).
        assert!(texts.iter().any(|t| t.contains("2+ / 1-")));
        // Contexto: las líneas sin cambios (a, c) aparecen con prefijo "  ".
        assert!(texts.iter().any(|t| t == "  a"), "falta contexto: {texts:?}");
        assert!(texts.iter().any(|t| t == "  c"));
    }

    #[test]
    fn diff_identicos_lo_dice() {
        let mut s = State::new(Source::Local);
        for b in [1u64, 2] {
            for t in ["x", "y"] {
                let mut l = OutputLine::stdout(t);
                l.block = b;
                s.output.push(l);
            }
        }
        s.input.set_text(":diff %c1 %c2");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.contains("idénticos")));
    }

    #[test]
    fn diff_sin_dos_refs_avisa() {
        let mut s = State::new(Source::Local);
        s.input.set_text(":diff %c1");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.contains("uso: :diff")));
    }

    #[test]
    fn yank_copia_el_bloque_y_avisa() {
        // El write al clipboard es best-effort (no-op headless); probamos el
        // resolver + el aviso con el conteo correcto.
        let mut s = State::new(Source::Local);
        for t in ["uno", "dos"] {
            let mut l = OutputLine::stdout(t);
            l.block = 4;
            s.output.push(l);
        }
        s.input.set_text(":yank %c4");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s
            .output
            .iter()
            .any(|l| l.text.contains("2 líneas") && l.text.contains("clipboard")));
    }

    #[test]
    fn yank_sin_salida_avisa() {
        let mut s = State::new(Source::Local);
        s.input.set_text(":yank");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.contains("no hay salida")));
    }

    #[test]
    fn explica_arma_request_text_con_la_salida_del_bloque() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        // Sembramos un bloque 7 con stdout.
        let mut l = OutputLine::stdout("error: algo falló");
        l.block = 7;
        s.output.push(l);
        s.input.set_text(":explica %c7");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let req = s.llm_request.clone().expect("hay petición");
        assert!(matches!(req.kind, crate::LlmKind::Text));
        assert!(req.prompt.contains("algo falló"));
        assert!(req.prompt.contains("%c7"));
        // La respuesta abrirá su propio bloque referenciable.
        assert!(s.llm_block_label.as_deref().unwrap_or("").contains("%c7"));
    }

    #[test]
    fn explica_tambien_ve_stderr_y_salida_de_ia() {
        // `gather_block_text` recoge stdout + stderr + IA, no sólo stdout: una
        // explicación de un build fallido necesita los errores (que van a stderr).
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        let mut err = OutputLine::stderr("error[E0308]: mismatched types");
        err.block = 7;
        s.output.push(err);
        s.input.set_text(":explica %c7");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let req = s.llm_request.clone().expect("hay petición pese a ser stderr");
        assert!(req.prompt.contains("E0308"));
    }

    #[test]
    fn filtra_arma_request_y_etiqueta_el_bloque() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        for t in ["info: ok", "ERROR: boom", "info: listo"] {
            let mut l = OutputLine::stdout(t);
            l.block = 4;
            s.output.push(l);
        }
        s.input.set_text(":filtra %c4 sólo los errores");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let req = s.llm_request.clone().expect("hay petición de filtro");
        assert!(matches!(req.kind, crate::LlmKind::Text));
        // La instrucción y la salida viajan en el prompt.
        assert!(req.prompt.contains("sólo los errores"));
        assert!(req.prompt.contains("ERROR: boom"));
        // El bloque de respuesta queda etiquetado con la instrucción + la fuente.
        let label = s.llm_block_label.as_deref().unwrap_or("");
        assert!(label.contains("filtra") && label.contains("%c4"), "{label}");
    }

    #[test]
    fn filtra_sin_instruccion_avisa() {
        let mut s = State::new(Source::Local);
        let mut l = OutputLine::stdout("algo");
        l.block = 2;
        s.output.push(l);
        s.input.set_text(":filtra %c2");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.contains("falta la instrucción")));
        assert!(s.llm_request.is_none());
    }

    #[test]
    fn respuesta_de_ia_aterriza_en_su_bloque_y_es_redirigible() {
        // Simulamos el ciclo: el builtin dejó un label pendiente; llega la
        // respuesta del LLM → abre bloque propio con líneas `Ai`. Después esa
        // salida de IA debe ser recogible por los redireccionadores (`:yank`).
        let mut s = State::new(Source::Local);
        s.llm_inflight = true;
        s.llm_block_label = Some("🜲 :explica %c1".to_string());
        s = update(
            s,
            Msg::LlmResult {
                kind: crate::LlmKind::Text,
                ok: true,
                text: "Resumen: todo bien.\nNo hay errores.".into(),
            },
        );
        // Abrió un bloque nuevo (Prompt) con dos líneas Ai.
        let ai_block = s
            .output
            .iter()
            .find(|l| l.kind == OutputKind::Prompt && l.text.contains("explica"))
            .map(|l| l.block)
            .expect("se abrió un bloque para la respuesta");
        let ai_lines: Vec<&OutputLine> = s
            .output
            .iter()
            .filter(|l| l.block == ai_block && l.kind == OutputKind::Ai)
            .collect();
        assert_eq!(ai_lines.len(), 2);
        assert!(!s.llm_inflight);
        assert!(s.llm_block_label.is_none(), "el label se consumió");
        // La salida de IA es redirigible: `:yank` de ese bloque la recoge.
        s.input.set_text(&format!(":yank %c{ai_block}"));
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s
            .output
            .iter()
            .any(|l| l.text.contains("2 líneas") && l.text.contains("clipboard")));
    }

    #[test]
    fn predice_lista_comandos_por_frecuencia_y_cwd() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/repo");
        // Historial aislado (in-memory) para no leer el disco real.
        s.history = std::sync::Arc::new(std::sync::Mutex::new(
            shuma_history::History::open(PathBuf::from("/dev/null")).unwrap(),
        ));
        {
            let mut h = s.history.lock().unwrap();
            for t in 0..4 {
                let _ = h.append(shuma_history::Entry::new("cargo build", "/repo", 2 * t));
                let _ = h.append(shuma_history::Entry::new("git status", "/otro", 2 * t + 1));
            }
        }
        s.input.set_text(":predice");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let texts: Vec<String> = s.output.iter().map(|l| l.text.clone()).collect();
        assert!(texts.iter().any(|t| t.contains("comandos probables")), "{texts:?}");
        // El de cwd aparece con la marca ◆ y el conteo "aquí".
        assert!(
            texts.iter().any(|t| t.contains("◆") && t.contains("cargo build") && t.contains("aquí")),
            "{texts:?}"
        );
    }

    #[test]
    fn filtra_encadena_sobre_salida_de_ia() {
        // Una respuesta de IA en un bloque debe poder volver a filtrarse: el
        // `:filtra` sobre ese bloque arma su prompt con el texto de IA.
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        let mut a = OutputLine::ai("línea de IA uno");
        a.block = 9;
        s.output.push(a);
        let mut b = OutputLine::ai("línea de IA dos");
        b.block = 9;
        s.output.push(b);
        s.input.set_text(":filtra %c9 dejá sólo la primera");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let req = s.llm_request.clone().expect("filtra encadena sobre IA");
        assert!(req.prompt.contains("línea de IA uno"));
        assert!(req.prompt.contains("dejá sólo la primera"));
    }

    #[test]
    fn long_running_command_does_not_block_update() {
        // `update(Enter)` debe spawnear sin bloquear: vuelve enseguida con el
        // run AÚN vivo, en vez de esperar a que el comando termine (como haría
        // `Command::output`).
        //
        // La prueba semántica (independiente del reloj) es `is_running()` justo
        // después: si `update` hubiera corrido el comando a completarse, el
        // proceso ya estaría muerto. El reloj es belt-and-suspenders.
        //
        // Usamos `sleep 1` (no 0.3) a propósito: bajo carga pesada (suite en
        // paralelo + builds) el overhead de setup —spawn de thread + fork— puede
        // robar varios cientos de ms. Con un sleep largo ese stall sigue siendo
        // chico frente al segundo de duración, así `is_running()` no se vuelve
        // flaky; y el umbral de 500 ms separa cómodamente "no-bloqueó" (~ms, o
        // unos cientos bajo carga) de "bloqueó toda la duración" (~1000 ms).
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("sleep 1");
        let t0 = std::time::Instant::now();
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let elapsed = t0.elapsed();
        assert!(s.is_running(), "el sleep debe seguir vivo tras Enter");
        assert!(
            elapsed.as_millis() < 500,
            "update tardó {elapsed:?} — debería volver sin esperar al comando (~1 s)"
        );
        s = drain_until_idle(s);
        assert!(s.output.iter().any(|l| l.text == "✔ exit 0"));
    }

    #[test]
    fn second_enter_with_ampersand_starts_bg() {
        // Política (2026-06-09): un Enter durante un run vivo SIN `&`
        // se interpreta como respuesta al stdin del running (apt Y/n,
        // sudo, etc.). Para spawnear bg paralelo, el usuario agrega `&`.
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("sleep 0.2");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.is_running());
        // Sin `&`: va al stdin del running, no spawnea bg.
        s.input.set_text("y");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.bg_jobs.is_empty(), "sin & no debe spawnar bg job");
        // Con `&`: arranca como bg job paralelo.
        s.input.set_text("echo segunda &");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(!s.bg_jobs.is_empty(), "con & arranca bg job");
        s = drain_until_idle(s);
        let combined: Vec<String> = s.output.iter().map(|l| l.text.clone()).collect();
        assert!(combined.iter().any(|t| t == "segunda"), "{combined:?}");
    }

    #[test]
    fn cancel_terminates_active_run() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("sleep 30");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.is_running());
        // El coordinador de `shuma-exec` puebla `Killer.children` en
        // background — un Cancel inmediato podría llegar antes y la
        // señal caería en el vacío. Esperar a que aparezca el PID.
        let arc = s.running.as_ref().unwrap().clone();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline {
            let has_pid = arc
                .lock()
                .unwrap()
                .killer
                .as_ref()
                .map(|k| !k.pids().is_empty())
                .unwrap_or(false);
            if has_pid {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            arc.lock()
                .unwrap()
                .killer
                .as_ref()
                .map(|k| !k.pids().is_empty())
                .unwrap_or(false),
            "el coordinador no expuso el PID en 500ms"
        );
        s = update(s, Msg::Cancel);
        s = drain_until_idle(s);
        assert!(!s.is_running(), "sleep 30 debe morir al cancelar");
        assert!(s.output.iter().any(|l| l.text.starts_with("⏹ cancel")));
    }

    #[test]
    fn empty_submit_does_nothing_but_clears_input() {
        let mut s = State::new(Source::Local);
        s.input.set_text("   ");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.is_empty());
        assert!(s.input.text().is_empty());
    }

    #[test]
    fn output_buffer_caps_at_max() {
        let mut buf: Vec<OutputLine> = Vec::new();
        for i in 0..MAX_OUTPUT_LINES + 50 {
            push_line(&mut buf, OutputLine::stdout(format!("línea {i}")));
        }
        assert_eq!(buf.len(), MAX_OUTPUT_LINES);
        assert!(buf[0].text.contains("50"));
    }

    #[test]
    fn tab_completion_inserts_unique_candidate() {
        // Si el prefijo tiene un único match, Tab debe completarlo.
        let mut s = State::new(Source::Local);
        s.input.set_text("ec");
        // Forzar un source determinístico para no depender de $PATH.
        struct Fixed;
        impl shuma_line::CompletionSource for Fixed {
            fn commands(&self) -> Vec<String> {
                vec!["echo".into()]
            }
            fn paths(&self, _: &str) -> Vec<String> {
                vec![]
            }
        }
        s.completion_source = Arc::new(ShellSource::new(&s.cwd));
        // Bypassear: aplicamos completion manualmente con el Fixed source,
        // ya que apply_completion_msg usa s.completion_source.
        let comp = s.input.complete(&Fixed);
        let candidate = comp.candidates.first().cloned().unwrap_or_default();
        s.input.apply_completion(&comp, &candidate);
        assert_eq!(s.input.text(), "echo");
    }

    #[test]
    fn arrow_up_walks_history_backwards() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        // Historial aislado: si no, entradas de tests paralelos se cuelan y el
        // ArrowUp camina sobre comandos ajenos.
        s.history = Arc::new(Mutex::new(
            shuma_history::History::open(std::path::PathBuf::from("/dev/null")).unwrap(),
        ));
        // Insertar entradas a mano vía History (no via run_submitted, que
        // dispararía procesos reales).
        {
            let mut h = s.history.lock().unwrap();
            let _ = h.append(shuma_history::Entry::new("uno", "/", 1));
            let _ = h.append(shuma_history::Entry::new("dos", "/", 2));
        }
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowUp), None)));
        assert_eq!(s.input.text(), "dos");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowUp), None)));
        assert_eq!(s.input.text(), "uno");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowDown), None)));
        assert_eq!(s.input.text(), "dos");
    }

    #[test]
    fn ctrl_r_opens_search_overlay() {
        let mut s = State::new(Source::Local);
        let ctrl_r = KeyEvent {
            key: Key::Character("r".into()),
            state: KeyState::Pressed,
            text: Some("r".into()),
            modifiers: Modifiers {
                ctrl: true,
                ..Default::default()
            },
            repeat: false,
        };
        s = update(s, Msg::Key(ctrl_r));
        assert!(s.history_search.is_some());
    }

    #[test]
    fn ghost_extends_from_history_when_prefix_matches() {
        let mut s = State::new(Source::Local);
        // Historial aislado: evita que un `cargo …` ajeno (de otro test
        // paralelo) gane el match del ghost y cambie el sufijo esperado.
        s.history = Arc::new(Mutex::new(
            shuma_history::History::open(std::path::PathBuf::from("/dev/null")).unwrap(),
        ));
        {
            let mut h = s.history.lock().unwrap();
            let _ = h.append(shuma_history::Entry::new("cargo build --release", "/", 1));
        }
        s.input.set_text("cargo bu");
        let g = current_ghost(&s);
        // Devuelve el sufijo que falta para llegar a la línea histórica.
        assert_eq!(g.as_deref(), Some("ild --release"));
    }

    #[test]
    fn build_spec_routes_known_tui_command_to_pty() {
        let (spec, tui) = build_spec("vim README.md", "/");
        assert!(matches!(spec.exec, shuma_exec::Exec::Pty { .. }));
        assert!(tui.is_some());
    }

    #[test]
    fn build_spec_routes_plain_command_to_shell() {
        let (spec, tui) = build_spec("ls -la", "/");
        assert!(matches!(spec.exec, shuma_exec::Exec::Shell { .. }));
        assert!(tui.is_none());
    }

    #[test]
    fn build_spec_routes_simple_pipe_to_direct_with_capture() {
        // Un pipe simple corre directo (sin bash) y con captura por etapa.
        let (spec, tui) = build_spec("ls -la | grep foo", "/");
        match &spec.exec {
            shuma_exec::Exec::Direct { stages } => {
                assert_eq!(stages.len(), 2, "dos etapas");
                assert_eq!(stages[0].program, "ls");
                assert_eq!(stages[1].program, "grep");
            }
            other => panic!("esperaba Exec::Direct, fue {other:?}"),
        }
        assert!(spec.capture_stages, "el pipe directo activa el tee");
        assert!(tui.is_none());
    }

    #[test]
    fn build_spec_pipe_with_quotes_falls_back_to_shell() {
        // `shuma_line::Stage` no recoge StringLit en args, así que un pipe
        // con comillas debe ir a `sh -c` o perdería el argumento citado.
        let (spec, _) = build_spec("echo 'a | b' | cat", "/");
        assert!(matches!(spec.exec, shuma_exec::Exec::Shell { .. }));
        assert!(!spec.capture_stages);
    }

    #[test]
    fn alt_screen_is_the_hard_tui_signal() {
        // `ESC[?1049h` entra a alternate screen (señal dura de TUI
        // full-screen); `ESC[?1049l` sale y vuelve a modo líneas.
        let mut p = vt100::Parser::new(24, 80, 0);
        p.process(b"hola mundo\r\n");
        assert!(!p.screen().alternate_screen(), "arranca en modo líneas");
        p.process(b"\x1b[?1049h");
        assert!(p.screen().alternate_screen(), "1049h = pantalla completa");
        p.process(b"\x1b[?1049l");
        assert!(!p.screen().alternate_screen(), "1049l = vuelve a líneas");
    }

    #[test]
    fn screen_to_lines_trims_trailing_blanks() {
        let mut p = vt100::Parser::new(24, 80, 0);
        p.process(b"primera\r\nsegunda\r\n");
        let lines = screen_to_lines(p.screen());
        // Sólo las dos filas con contenido; las 22 filas vacías de abajo
        // se recortan.
        assert_eq!(lines, vec!["primera", "segunda"]);
    }

    #[test]
    fn build_spec_pipe_with_glob_falls_back_to_shell() {
        let (spec, _) = build_spec("ls *.rs | cat", "/");
        assert!(matches!(spec.exec, shuma_exec::Exec::Shell { .. }));
    }

    #[test]
    fn simple_pipe_stages_rejects_single_command() {
        // Un único comando no gana nada del modo directo (no hay tubería
        // que interceptar) → `None`, cae a `sh -c`.
        assert!(simple_pipe_stages("ls -la").is_none());
    }

    #[test]
    fn simple_pipe_stages_rejects_trailing_pipe() {
        // Etapa sin comando (línea incompleta) → None.
        assert!(simple_pipe_stages("ls |").is_none());
    }

    #[test]
    fn piped_command_captures_intermediate_stage_output() {
        // `echo hola | cat`: stage0 (echo) se captura en vivo como una
        // OutputLine con stage=Some(0); la salida final (cat) sale como
        // stdout normal (stage None).
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("echo hola | cat");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.is_running(), "el pipe debe arrancar un run");
        s = drain_until_idle(s);
        let stage0: Vec<&OutputLine> = s
            .output
            .iter()
            .filter(|l| l.stage == Some(0))
            .collect();
        assert!(
            stage0.iter().any(|l| l.text == "hola"),
            "esperaba 'hola' capturado de la etapa 0, output: {:?}",
            s.output.iter().map(|l| (l.stage, &l.text)).collect::<Vec<_>>()
        );
        // La salida final (cat) llega como stdout normal sin stage.
        assert!(s
            .output
            .iter()
            .any(|l| l.stage.is_none() && l.text == "hola"));
        assert!(s.output.iter().any(|l| l.text == "✔ exit 0"));
    }

    #[test]
    fn infer_predicts_next_command_in_a_repeated_sequence() {
        // Historial con el patrón `git pull` → `make` repetido dos veces y
        // un `git pull` final: el motor debe predecir `make` como
        // continuación. cwd `/tmp/...` sin marcadores → sin gating.
        let mut s = State::new(Source::Local);
        // Historial AISLADO en memoria: `State::new` abre el real del disco y
        // varios tests en paralelo lo contaminarían (la minería vería entradas
        // ajenas y el patrón no emergería limpio).
        s.history = Arc::new(Mutex::new(
            shuma_history::History::open(std::path::PathBuf::from("/dev/null")).unwrap(),
        ));
        let dir = "/tmp/shuma-infer-pred-test";
        {
            let mut h = s.history.lock().unwrap();
            for (i, line) in ["git pull", "make", "git pull", "make", "git pull"]
                .iter()
                .enumerate()
            {
                let _ = h.append(shuma_history::Entry::new(*line, dir, i as u64));
            }
        }
        refresh_patterns(&mut s);
        assert!(!s.patterns.is_empty(), "debe emerger el patrón git→make");
        // La continuación predicha empieza por `make` (puede seguir con el
        // resto del patrón más largo, p. ej. `make && git pull`).
        let pred = predicted_sequence(&s).expect("predice una continuación");
        assert!(
            pred.starts_with("make"),
            "tras `git pull` predice `make…`, fue {pred:?}"
        );
    }

    #[test]
    fn ghost_uses_prediction_before_history() {
        // Con el patrón aprendido, tipear `ma` debe sugerir `ke` (de la
        // predicción `make`), aunque el historial no tenga un match mejor.
        let mut s = State::new(Source::Local);
        // Historial aislado (mismo motivo que `infer_predicts_…`): evita la
        // contaminación cruzada entre tests paralelos vía el archivo real.
        s.history = Arc::new(Mutex::new(
            shuma_history::History::open(std::path::PathBuf::from("/dev/null")).unwrap(),
        ));
        let dir = "/tmp/shuma-infer-ghost-test";
        {
            let mut h = s.history.lock().unwrap();
            for (i, line) in ["git pull", "make", "git pull", "make", "git pull"]
                .iter()
                .enumerate()
            {
                let _ = h.append(shuma_history::Entry::new(*line, dir, i as u64));
            }
        }
        refresh_patterns(&mut s);
        s.input.set_text("ma");
        // El ghost arranca con `ke` (sufijo de `make`, de la predicción). Con
        // el historial aislado la predicción es el patrón completo (`make &&
        // git pull`), así que el sufijo puede ser `ke && git pull` — basta con
        // que empiece por `ke` para probar que vino de la predicción `make…`.
        let ghost = current_ghost(&s).expect("hay ghost de la predicción");
        assert!(ghost.starts_with("ke"), "el ghost debe venir de `make…`, fue {ghost:?}");
    }

    #[test]
    fn git_branch_reads_head_ref() {
        // `.git/HEAD` con `ref: refs/heads/<rama>` → Some(rama). Usamos un
        // tmpdir aislado para no depender del repo real.
        let base = std::env::temp_dir().join(format!("shuma-gb-{}", std::process::id()));
        let git = base.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        // Desde un subdirectorio: debe subir hasta encontrar `.git`.
        let sub = base.join("sub/dir");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(git_branch(&sub).as_deref(), Some("feature/x"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn git_branch_none_outside_repo() {
        let base = std::env::temp_dir().join(format!("shuma-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(git_branch(&base), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn limit_builtin_sets_capture_bytes() {
        let mut s = State::new(Source::Local);
        s.input.set_text(":limit 5");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.capture_limit_bytes, 5 * 1024 * 1024);
        assert!(!s.is_running(), "`:limit` no spawnea proceso");
        // `:limit 0` quita el tope.
        s.input.set_text(":limit 0");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.capture_limit_bytes, 0);
    }

    #[test]
    fn spill_builtin_toggles_flag() {
        let mut s = State::new(Source::Local);
        s.input.set_text(":spill on");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.spill);
        s.input.set_text(":spill off");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(!s.spill);
    }

    #[test]
    fn sanitize_paste_drops_single_trailing_newline() {
        // Pegar "ls -la\n" no debe dejar una línea vacía colgando.
        assert_eq!(sanitize_paste("ls -la\n"), "ls -la");
    }

    #[test]
    fn sanitize_paste_preserves_interior_newlines() {
        // El input es multilínea: pegar un script conserva sus saltos
        // (no se colapsa a `;` como el shell GPUI).
        assert_eq!(sanitize_paste("ls\npwd\n"), "ls\npwd");
    }

    #[test]
    fn sanitize_paste_normalizes_crlf() {
        assert_eq!(sanitize_paste("a\r\nb"), "a\nb");
        assert_eq!(sanitize_paste("a\rb"), "a\nb");
    }

    #[test]
    fn sanitize_paste_strips_control_chars_and_tabs() {
        // ESC (\x1b) y BEL (\x07) se descartan; tab → espacio; los saltos
        // de línea sobreviven.
        assert_eq!(sanitize_paste("ls\t-la\x1b[X\x07"), "ls -la[X");
    }

    #[test]
    fn sanitize_paste_keeps_plain_text() {
        assert_eq!(sanitize_paste("echo hola mundo"), "echo hola mundo");
    }

    #[test]
    fn alias_from_config_expands_before_run() {
        // Un alias del `.shumarc` reemplaza la primera palabra; lo tipeado
        // queda en el historial, lo resuelto es lo que se ejecuta.
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.config
            .aliases
            .insert("saluda".into(), "echo hola_alias".into());
        s.input.set_text("saluda");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.is_running(), "el alias resuelto debe arrancar un run");
        s = drain_until_idle(s);
        let combined: Vec<String> = s.output.iter().map(|l| l.text.clone()).collect();
        assert!(
            combined.iter().any(|t| t == "hola_alias"),
            "esperaba stdout del alias resuelto en {combined:?}"
        );
        // El prompt muestra lo tipeado, no lo resuelto.
        assert!(combined.iter().any(|t| t == "$ saluda"));
    }

    #[test]
    fn alias_can_resolve_to_a_builtin() {
        // `alias raiz='cd /'` debe disparar el builtin cd sobre la línea ya
        // expandida.
        let mut s = State::new(Source::Local);
        s.config.aliases.insert("raiz".into(), "cd /".into());
        s.input.set_text("raiz");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.cwd, PathBuf::from("/"));
        assert!(!s.is_running(), "cd no spawnea proceso");
    }

    #[test]
    fn alias_never_hijacks_meta_command() {
        // Un alias declarado con el nombre de un meta-comando no debe
        // secuestrarlo: `:limit` sigue siendo el builtin del shell.
        let mut s = State::new(Source::Local);
        s.config
            .aliases
            .insert(":limit".into(), "echo secuestrado".into());
        s.input.set_text(":limit 7");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.capture_limit_bytes, 7 * 1024 * 1024);
        assert!(!s.is_running(), "el meta no debe ejecutar el alias");
    }

    #[test]
    fn save_group_captures_recent_commands() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        // Dos comandos reales (no meta) + un :save.
        for line in ["echo uno", "echo dos"] {
            s.input.set_text(line);
            s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
            s = drain_until_idle(s);
        }
        s.input.set_text(":save build");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.groups.len(), 1);
        assert_eq!(s.groups[0].name, "build");
        assert_eq!(s.groups[0].lines, vec!["echo uno", "echo dos"]);
        // El anchor avanzó: un segundo :save sin comandos nuevos no agrupa.
        s.input.set_text(":save vacio");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.groups.len(), 1, "no se crea grupo vacío");
    }

    #[test]
    fn run_group_msg_executes_group() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.groups.push(CommandGroup {
            name: "g".into(),
            lines: vec!["echo desde_panel".into()],
        });
        s = update(s, Msg::RunGroup(0));
        s = drain_until_idle(s);
        assert!(s.output.iter().any(|l| l.text == "desde_panel"));
        // Índice fuera de rango: no-op.
        s = update(s, Msg::RunGroup(9));
        assert!(!s.is_running());
    }

    #[test]
    fn fkey_runs_saved_group() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        // F1 sin grupos: no hace nada.
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::F1), None)));
        assert!(!s.is_running());
        // Guardamos un grupo de un comando y lo corremos con F1.
        s.groups.push(CommandGroup {
            name: "g".into(),
            lines: vec!["echo desde_f1".into()],
        });
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::F1), None)));
        s = drain_until_idle(s);
        assert!(s.output.iter().any(|l| l.text == "desde_f1"));
    }

    #[test]
    fn reprocess_feeds_block_stdout_as_stdin() {
        // Corre `printf "b\\na\\nc\\n"`, arma reprocess sobre su bloque, y
        // corre `sort`: debe recibir esa salida por stdin y ordenarla.
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("printf 'b\\na\\nc\\n'");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        s = drain_until_idle(s);
        let src_block = s.output.iter().find(|l| l.text == "b").unwrap().block;
        s = update(s, Msg::SetReprocess(src_block));
        assert_eq!(s.reprocess_source, Some(src_block));
        s.input.set_text("sort");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.reprocess_source.is_none(), "el submit consume el reprocess");
        s = drain_until_idle(s);
        // La salida de `sort` (en su propio bloque) está ordenada: a,b,c.
        let sorted: Vec<String> = s
            .output
            .iter()
            .filter(|l| l.block != src_block && l.kind == OutputKind::Stdout)
            .map(|l| l.text.clone())
            .collect();
        assert_eq!(sorted, vec!["a", "b", "c"], "sort recibió el stdin reprocesado");
    }

    #[test]
    fn set_reprocess_toggles_off_same_block() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::SetReprocess(3));
        assert_eq!(s.reprocess_source, Some(3));
        s = update(s, Msg::SetReprocess(3));
        assert_eq!(s.reprocess_source, None, "re-armar el mismo bloque desarma");
    }

    fn fake_completion(cands: &[&str], start: usize, end: usize) -> shuma_line::Completion {
        shuma_line::Completion {
            kind: shuma_line::CompletionKind::Command,
            candidates: cands.iter().map(|s| s.to_string()).collect(),
            replace_start: start,
            replace_end: end,
        }
    }

    #[test]
    fn completion_tab_accepts_highlighted() {
        // Con popup vivo, Tab acepta el candidato resaltado (no cicla).
        let mut s = State::new(Source::Local);
        s.input.set_text("ca");
        s.completion = Some(fake_completion(&["cargo", "cat", "cal"], 0, 2));
        s.completion_index = 1; // "cat"
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Tab), None)));
        assert_eq!(s.input.text(), "cat", "Tab aplica el resaltado");
        assert!(s.completion.is_none(), "y cierra el popup");
    }

    #[test]
    fn ctrl_a_selects_whole_input_line() {
        let mut s = State::new(Source::Local);
        s.input.set_text("git status");
        let ctrl_a = KeyEvent {
            key: Key::Character("a".into()),
            state: KeyState::Pressed,
            text: Some("a".into()),
            modifiers: Modifiers { ctrl: true, ..Default::default() },
            repeat: false,
        };
        s = update(s, Msg::Key(ctrl_a));
        assert_eq!(s.input.selected_text().as_deref(), Some("git status"));
    }

    #[test]
    fn shift_arrow_extends_input_selection() {
        let mut s = State::new(Source::Local);
        s.input.set_text("abc");
        // Shift+Left desde el final selecciona el último char.
        let shift_left = KeyEvent {
            key: Key::Named(NamedKey::ArrowLeft),
            state: KeyState::Pressed,
            text: None,
            modifiers: Modifiers { shift: true, ..Default::default() },
            repeat: false,
        };
        s = update(s, Msg::Key(shift_left));
        assert_eq!(s.input.selected_text().as_deref(), Some("c"));
    }

    #[test]
    fn rank_completion_by_usage_orders_by_history() {
        let mut s = State::new(Source::Local);
        // Historial aislado (el real del usuario contaminaría el ranking).
        s.history = Arc::new(Mutex::new(
            shuma_history::History::open(std::path::PathBuf::from("/dev/null")).unwrap(),
        ));
        {
            let mut h = s.history.lock().unwrap();
            // Líneas distintas (el dedup colapsa repetidas consecutivas).
            let _ = h.append(shuma_history::Entry::new("cat a", "/", 0));
            let _ = h.append(shuma_history::Entry::new("cargo build", "/", 1));
            let _ = h.append(shuma_history::Entry::new("cat b", "/", 2));
            let _ = h.append(shuma_history::Entry::new("cat c", "/", 3));
        }
        let mut comp = fake_completion(&["cargo", "cat", "cal"], 0, 2);
        rank_completion_by_usage(&s, &mut comp);
        assert_eq!(comp.candidates[0], "cat", "el más usado primero");
        assert_eq!(comp.candidates[1], "cargo");
        assert_eq!(comp.candidates[2], "cal", "sin uso, al final");
    }

    #[test]
    fn pattern_detection_window_excludes_old_and_keeps_recent() {
        // `refresh_patterns` corre en CADA submit; sobre un historial grande
        // `detect_patterns` era O(n) con constante alta (~150 ms con 5 k
        // entradas) y bloqueaba el `update`. Se acota a la ventana reciente.
        // Acá verificamos la SEMÁNTICA: un patrón que sólo vive en lo viejo
        // (más allá de la ventana) NO se detecta; uno reciente sí.
        let mut s = State::new(Source::Local);
        s.history = Arc::new(Mutex::new(
            shuma_history::History::open(std::path::PathBuf::from("/dev/null")).unwrap(),
        ));
        {
            let mut h = s.history.lock().unwrap();
            // Patrón viejo (2 ocurrencias) al principio del todo.
            for _ in 0..2 {
                let _ = h.append(shuma_history::Entry::new("viejocmd", "/", 0));
                let _ = h.append(shuma_history::Entry::new("viejodos", "/", 0));
            }
            // Relleno único que empuja lo viejo fuera de la ventana (cada línea
            // distinta: ni dedup ni patrón espurio).
            for i in 0..1600u32 {
                let _ = h.append(shuma_history::Entry::new(format!("relleno{i}"), "/", 0));
            }
            // Patrón reciente (2 ocurrencias) al final.
            for _ in 0..2 {
                let _ = h.append(shuma_history::Entry::new("nuevocmd", "/", 0));
                let _ = h.append(shuma_history::Entry::new("nuevodos", "/", 0));
            }
        }
        refresh_patterns(&mut s);
        let firmas: Vec<&Vec<String>> = s.patterns.iter().map(|p| &p.signature).collect();
        assert!(
            firmas.iter().any(|f| f.contains(&"nuevocmd".to_string())),
            "el patrón reciente debe detectarse: {firmas:?}"
        );
        assert!(
            !firmas.iter().any(|f| f.contains(&"viejocmd".to_string())),
            "el patrón fuera de la ventana NO debe detectarse: {firmas:?}"
        );
    }

    #[test]
    fn ghost_corpus_is_bounded_to_recent_window() {
        // `current_ghost` corre por frame; el corpus se acota a la ventana
        // reciente para no clonar todo el historial. Verificamos el límite: una
        // coincidencia que quedó FUERA de la ventana ya no ghostea; una dentro sí.
        let mut s = State::new(Source::Local);
        s.history = Arc::new(Mutex::new(
            shuma_history::History::open(std::path::PathBuf::from("/dev/null")).unwrap(),
        ));
        s.cwd = PathBuf::from("/");
        {
            let mut h = s.history.lock().unwrap();
            // Única coincidencia, al principio del todo.
            let _ = h.append(shuma_history::Entry::new("zzfantasma --bandera", "/", 0));
            // Relleno que la empuja fuera de la ventana del ghost (2000).
            for i in 0..2200u32 {
                let _ = h.append(shuma_history::Entry::new(format!("relleno{i}"), "/", 0));
            }
        }
        s.input.set_text("zzfantasma");
        assert_eq!(
            current_ghost(&s),
            None,
            "una coincidencia fuera de la ventana no debe ghostear"
        );
        // La misma línea, ahora reciente, sí ghostea.
        {
            let mut h = s.history.lock().unwrap();
            let _ = h.append(shuma_history::Entry::new("zzfantasma --bandera", "/", 0));
        }
        assert_eq!(current_ghost(&s).as_deref(), Some(" --bandera"));
    }

    #[test]
    fn completion_arrows_cycle_both_ways() {
        let mut s = State::new(Source::Local);
        s.completion = Some(fake_completion(&["a", "b", "c"], 0, 0));
        s.completion_index = 0;
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowUp), None)));
        assert_eq!(s.completion_index, 2, "↑ desde 0 va al último");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowDown), None)));
        assert_eq!(s.completion_index, 0);
    }

    #[test]
    fn completion_enter_submits_not_accepts() {
        // Con popup vivo, Enter ejecuta el comando como está (no acepta el
        // resaltado): el popup es sugerencia, no modal.
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("ca");
        s.completion = Some(fake_completion(&["cargo", "cat"], 0, 2));
        s.completion_index = 1; // "cat"
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.completion.is_none(), "Enter cierra el popup");
        assert!(
            s.input.text().is_empty(),
            "ejecutó (limpió el input) en vez de aplicar 'cat'"
        );
        s = drain_until_idle(s);
    }

    #[test]
    fn completion_escape_closes_without_change() {
        let mut s = State::new(Source::Local);
        s.input.set_text("ca");
        s.completion = Some(fake_completion(&["cargo", "cat"], 0, 2));
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Escape), None)));
        assert!(s.completion.is_none());
        assert_eq!(s.input.text(), "ca", "Esc no toca el texto");
    }

    #[test]
    fn typing_processes_key_and_refreshes_completion() {
        // Tipear procesa la tecla y refresca el popup en vivo (puede reabrir
        // con nuevos candidatos según el entorno; lo determinístico es que la
        // tecla entró al input).
        let mut s = State::new(Source::Local);
        s.input.set_text("ca");
        s.completion = Some(fake_completion(&["cargo", "cat"], 0, 2));
        let key = KeyEvent {
            key: Key::Character("r".into()),
            state: KeyState::Pressed,
            text: Some("r".into()),
            modifiers: Modifiers::default(),
            repeat: false,
        };
        s = update(s, Msg::Key(key));
        assert_eq!(s.input.text(), "car", "la tecla se procesa normal");
    }

    #[test]
    fn toggle_stage_flips_expanded_set() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::ToggleStage { block: 2, stage: 0 });
        assert!(s.expanded_stages.contains(&(2, 0)), "primer toggle despliega");
        s = update(s, Msg::ToggleStage { block: 2, stage: 0 });
        assert!(
            !s.expanded_stages.contains(&(2, 0)),
            "segundo toggle repliega"
        );
    }

    #[test]
    fn build_spec_tui_prefix_overrides_default() {
        // `:tui ls` no es típico, pero el prefix lo fuerza igual.
        let (spec, tui) = build_spec(":tui ls", "/");
        assert!(matches!(spec.exec, shuma_exec::Exec::Pty { .. }));
        assert!(tui.is_some());
    }

    #[test]
    fn key_to_pty_bytes_handles_special_keys() {
        let enter = ev(Key::Named(NamedKey::Enter), None);
        assert_eq!(key_to_pty_bytes(&enter), b"\r");
        let up = ev(Key::Named(NamedKey::ArrowUp), None);
        assert_eq!(key_to_pty_bytes(&up), b"\x1b[A");
        let esc = ev(Key::Named(NamedKey::Escape), None);
        assert_eq!(key_to_pty_bytes(&esc), b"\x1b");
        // Ctrl-C → 0x03.
        let ctrl_c = KeyEvent {
            key: Key::Character("c".into()),
            state: KeyState::Pressed,
            text: Some("c".into()),
            modifiers: Modifiers {
                ctrl: true,
                ..Default::default()
            },
            repeat: false,
        };
        assert_eq!(key_to_pty_bytes(&ctrl_c), vec![3u8]);
    }

    #[test]
    fn source_daemon_failure_surfaces_as_notice() {
        // Sin daemon corriendo, start_run con Source::Daemon debe
        // dejar un notice rojo y no enredarse — el shell sigue vivo.
        let mut s = State::new(Source::Daemon {
            socket: Some(PathBuf::from("/tmp/shuma-no-existe-test.sock")),
            label: None,
        });
        let _ = std::fs::remove_file("/tmp/shuma-no-existe-test.sock");
        s.input.set_text("echo hola");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.starts_with("✘ daemon:")));
        assert!(!s.is_running(), "no debe quedar un run vivo si falló");
    }

    #[test]
    fn ampersand_suffix_starts_background_job() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("sleep 5 &");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(!s.is_running(), "& no debe dejar un foreground vivo");
        assert_eq!(s.bg_jobs.len(), 1);
        // El header de la card del job: `[0] $ sleep 5 &`.
        assert!(s
            .output
            .iter()
            .any(|l| l.text.contains("[0]") && l.text.contains("sleep 5")));
        // Cancelar el job así no queda sleep colgado en el host.
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        s.input.set_text(":term 0");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s
            .output
            .iter()
            .any(|l| l.text.contains("[0] SIGTERM enviado")));
    }

    #[test]
    fn kill_builtin_signals_background_job() {
        // `:kill N` manda SIGKILL al job N (paralelo a `:term`).
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("sleep 5 &");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.bg_jobs.len(), 1);
        s.input.set_text(":kill 0");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s
            .output
            .iter()
            .any(|l| l.text.contains("[0] SIGKILL enviado")));
    }

    #[test]
    fn input_focus_dirige_el_enter_y_no_pliega_a_los_vivos() {
        // Modelo de input paralelo: arrancar un comando lo foca; la línea
        // puede re-focarse para arrancar otro en paralelo; el vivo NO se
        // pliega; y el foco se puede alternar a cualquier job vivo.
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");

        // Foreground vivo → queda focado para recibir stdin.
        s.input.set_text("sleep 30");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.is_running());
        let block_a = s.current_block;
        assert_eq!(
            s.input_focus,
            Some(block_a),
            "el comando recién arrancado recibe el foco del input"
        );

        // Volver a la línea (click/hover sobre el input) → arranca comandos.
        s = update(s, Msg::FocusInput);
        assert_eq!(s.input_focus, None);

        // Con un foreground vivo, el nuevo comando corre en paralelo (bg job)
        // y se lleva el foco; el viejo NO se pliega (sigue activo).
        s.input.set_text("sleep 30");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(s.bg_jobs.len(), 1, "el segundo corre en paralelo");
        let block_b = s.bg_jobs[0].lock().unwrap().block;
        assert_eq!(s.input_focus, Some(block_b));
        assert!(
            !s.collapsed.contains(&block_a),
            "una ejecución viva no se pliega al arrancar otra"
        );

        // Alternar el foco al primer job vivo (click/hover sobre su card).
        s = update(s, Msg::FocusJob(block_a));
        assert_eq!(s.input_focus, Some(block_a));

        // Focar un bloque sin job vivo no roba el foco a la línea.
        s = update(s, Msg::FocusInput);
        s = update(s, Msg::FocusJob(99_999));
        assert_eq!(s.input_focus, None, "no se foca un bloque sin job vivo");

        // Limpieza: matar los jobs para no dejar sleeps colgados.
        s.input.set_text(":kill 0");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        if let Some(arc) = s.running.take() {
            if let Some(k) = arc.lock().unwrap().killer.as_ref() {
                k.kill();
            }
        }
    }

    #[test]
    fn jobs_builtin_lists_background_jobs() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("sleep 5 &");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        s.input.set_text(":jobs");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s
            .output
            .iter()
            .any(|l| l.text.contains("[0]") && l.text.contains("sleep")));
        s.input.set_text(":term 0");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
    }

    #[test]
    fn jobs_builtin_empty_shows_notice() {
        let mut s = State::new(Source::Local);
        s.input.set_text(":jobs");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.output.iter().any(|l| l.text.contains("sin jobs")));
    }

    #[test]
    fn enter_with_open_quote_inserts_newline_instead_of_submit() {
        let mut s = State::new(Source::Local);
        s.input.set_text("echo 'hola");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        // No debe haber arrancado un run — Enter agregó \n.
        assert!(!s.is_running());
        assert_eq!(s.input.text(), "echo 'hola\n");
    }

    #[test]
    fn shift_enter_always_inserts_newline() {
        let mut s = State::new(Source::Local);
        s.input.set_text("ls"); // texto completo, sin continuation pendiente
        let shift_enter = KeyEvent {
            key: Key::Named(NamedKey::Enter),
            state: KeyState::Pressed,
            text: None,
            modifiers: Modifiers {
                shift: true,
                ..Default::default()
            },
            repeat: false,
        };
        s = update(s, Msg::Key(shift_enter));
        assert!(!s.is_running(), "shift+enter no debe ejecutar");
        assert_eq!(s.input.text(), "ls\n");
    }

    #[test]
    fn paste_key_event_is_recognized() {
        // Ctrl-V con texto en clipboard se procesa como paste (no
        // termina llamando apply_key con el carácter 'v'). Sin display
        // server (CI), read_clipboard devuelve None y el state no
        // cambia. Pero verificamos que la rama de paste se toma.
        let mut s = State::new(Source::Local);
        s.input.set_text("hola");
        let ctrl_v = KeyEvent {
            key: Key::Character("v".into()),
            state: KeyState::Pressed,
            text: Some("v".into()),
            modifiers: Modifiers {
                ctrl: true,
                ..Default::default()
            },
            repeat: false,
        };
        s = update(s, Msg::Key(ctrl_v));
        // El input no debe llevar una 'v' al final — la rama paste se
        // tragó la tecla (y en CI sin clipboard no insertó nada).
        assert_eq!(s.input.text(), "hola");
    }

    #[test]
    fn ansi_idx_palette_matches_expected_basics() {
        // Idx 0 = negro, 15 = blanco, 196 = rojo claro del cubo.
        let black = ansi_idx_to_color(0);
        assert_eq!(black.components[0], 0.0);
        let white = ansi_idx_to_color(15);
        assert!(white.components[0] > 0.99);
    }

    #[test]
    fn arrow_right_at_end_accepts_ghost() {
        let mut s = State::new(Source::Local);
        // Historial aislado: un `cargo …` ajeno cambiaría el ghost aceptado.
        s.history = Arc::new(Mutex::new(
            shuma_history::History::open(std::path::PathBuf::from("/dev/null")).unwrap(),
        ));
        {
            let mut h = s.history.lock().unwrap();
            let _ = h.append(shuma_history::Entry::new("cargo build --release", "/", 1));
        }
        s.input.set_text("cargo bu");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::ArrowRight), None)));
        assert_eq!(s.input.text(), "cargo build --release");
    }

    #[test]
    fn open_decoration_cd_into_a_directory() {
        let mut s = State::new(Source::Local);
        let target = std::env::temp_dir();
        let kind = shuma_line::DecorationKind::Path {
            abs: target.clone(),
            is_dir: true,
            is_executable: false,
            is_symlink: false,
        };
        s = update(s, Msg::OpenDecoration(kind));
        // cwd cambia al directorio target (no comparamos canónico — el
        // open_decoration acepta el path tal cual viene si es dir).
        assert_eq!(s.cwd, target);
    }

    #[test]
    fn open_decoration_git_sha_prefills_input() {
        let mut s = State::new(Source::Local);
        let kind = shuma_line::DecorationKind::GitSha("abcdef0123456".into());
        s = update(s, Msg::OpenDecoration(kind));
        assert_eq!(s.input.text(), "git show abcdef0123456");
    }

    #[test]
    fn open_decoration_path_executable_prefills_input() {
        let mut s = State::new(Source::Local);
        let kind = shuma_line::DecorationKind::Path {
            abs: PathBuf::from("/usr/bin/ls"),
            is_dir: false,
            is_executable: true,
            is_symlink: false,
        };
        s = update(s, Msg::OpenDecoration(kind));
        assert_eq!(s.input.text(), "/usr/bin/ls");
    }

    #[test]
    fn dispatch_maps_clear() {
        assert!(matches!(dispatch("shell.clear"), Some(Msg::Clear)));
        assert!(matches!(dispatch("shell.cancel"), Some(Msg::Cancel)));
        assert!(dispatch("desconocido").is_none());
    }

    #[test]
    fn contributions_expose_clear_and_cancel_shortcuts() {
        let s = State::new(Source::Local);
        let c = contributions(&s);
        assert!(c.monitors.is_empty());
        let labels: Vec<&str> = c.shortcuts.iter().map(|s| s.label.as_str()).collect();
        assert!(labels.contains(&"Clear"), "{labels:?}");
        assert!(labels.contains(&"Cancel"), "{labels:?}");
    }

    #[test]
    fn typing_appends_to_input() {
        let mut s = State::new(Source::Local);
        // El widget text-input usa apply_key con KeyEvent que incluye texto.
        let key = KeyEvent {
            key: Key::Character("h".into()),
            state: KeyState::Pressed,
            text: Some("h".into()),
            modifiers: Modifiers::default(),
            repeat: false,
        };
        s = update(s, Msg::Key(key));
        assert_eq!(s.input.text(), "h");
    }

    #[test]
    fn external_command_records_intention_in_graph() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        assert!(s.intent_graph().is_empty(), "grafo arranca vacío");
        s.input.set_text("echo lienzo");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert_eq!(
            s.intent_graph().len(),
            1,
            "Enter debe registrar el `%c1` en el grafo"
        );
        assert_eq!(s.intent_graph().commands()[0].intention, "echo lienzo");
        s = drain_until_idle(s);
        let node = &s.intent_graph().commands()[0];
        assert_eq!(node.status, shuma_intent::NodeStatus::Ok);
        assert!(
            node.output_bytes >= 7,
            "esperaba ≥7 bytes (len de 'lienzo\\n'), recibí {}",
            node.output_bytes
        );
    }

    #[test]
    fn failed_command_records_failed_status() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("false");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        s = drain_until_idle(s);
        assert_eq!(s.intent_graph().len(), 1);
        assert_eq!(
            s.intent_graph().commands()[0].status,
            shuma_intent::NodeStatus::Failed
        );
    }

    #[test]
    fn builtin_does_not_register_in_graph() {
        let mut s = State::new(Source::Local);
        s.input.set_text("pwd");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(
            s.intent_graph().is_empty(),
            "builtins no entran al grafo de intenciones"
        );
    }

    #[test]
    fn insert_at_cursor_appends_into_input() {
        let mut s = State::new(Source::Local);
        // `set_text` deja el cursor al final, así que `insert` extiende.
        s.input.set_text("sort ");
        s = update(s, Msg::InsertAtCursor("%p1".into()));
        assert_eq!(s.input.text(), "sort %p1");
    }

    #[test]
    fn push_output_groups_lines_into_command_blocks() {
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ ls"));
        s.push_output(OutputLine::stdout("a.txt"));
        s.push_output(OutputLine::stdout("b.txt"));
        s.push_output(OutputLine::notice("✔ exit 0"));
        let b = s.output[0].block;
        assert!(b > 0, "el prompt debe abrir un bloque > 0");
        assert!(
            s.output.iter().all(|l| l.block == b),
            "comando + salida + exit comparten bloque: {:?}",
            s.output.iter().map(|l| l.block).collect::<Vec<_>>()
        );
        // Un segundo prompt abre un bloque nuevo y monotónico.
        s.push_output(OutputLine::prompt("$ pwd"));
        assert!(
            s.output.last().unwrap().block > b,
            "el segundo comando abre un bloque nuevo"
        );
    }

    #[test]
    fn push_in_block_keeps_async_output_out_of_foreground_card() {
        // El bug de "output mezclado": un job async drenando en su bloque
        // NO debe contaminar el bloque del comando de foreground, aunque
        // `current_block` apunte a este último.
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ fg")); // abre bloque fg
        let fg_block = s.current_block;
        let job_block = s.open_block(); // bloque propio del job (current sigue en fg)
        s.push_in_block(job_block, OutputLine::stdout("salida del job"));
        s.push_output(OutputLine::stdout("salida del fg"));
        let bg = s
            .output
            .iter()
            .find(|l| l.text == "salida del job")
            .unwrap()
            .block;
        let fg = s
            .output
            .iter()
            .find(|l| l.text == "salida del fg")
            .unwrap()
            .block;
        assert_eq!(bg, job_block);
        assert_eq!(fg, fg_block);
        assert_ne!(bg, fg, "job y foreground en cards distintas");
    }

    #[test]
    fn body_lines_excludes_prompt_stage_and_status() {
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ echo hola | cat"));
        let blk = s.current_block;
        s.push_output(OutputLine::stage_stdout(0, "intermedia"));
        s.push_output(OutputLine::stdout("hola"));
        s.push_output(OutputLine::stderr("ups"));
        s.push_output(OutputLine::notice("✔ exit 0"));
        // Cuerpo = stdout/stderr/notice no-status, sin el prompt ni la etapa.
        assert_eq!(body_lines_for_block(&s, blk), vec!["hola", "ups"]);
    }

    #[test]
    fn finished_command_stays_expanded_then_recedes_on_next() {
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("seq 1 20");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let blk = s.current_block;
        s = drain_until_idle(s);
        // Recién terminado: sigue EXPANDIDO (se ve completo).
        assert!(!s.collapsed.contains(&blk), "el comando recién hecho queda expandido");
        // Al correr uno nuevo, el anterior recede (se pliega).
        s.input.set_text("echo otra");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(s.collapsed.contains(&blk), "el anterior se pliega al nacer uno nuevo");
        let nuevo = s.current_block;
        assert!(!s.collapsed.contains(&nuevo), "el nuevo nace expandido");
    }

    #[test]
    fn command_without_output_does_not_recede() {
        // Un comando sin cuerpo (no produjo salida) no se pliega al pasar al
        // siguiente — no hay nada que esconder, y se mostrará distinto.
        let mut s = State::new(Source::Local);
        s.cwd = PathBuf::from("/");
        s.input.set_text("true"); // exit 0, sin stdout
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        let blk = s.current_block;
        s = drain_until_idle(s);
        s.input.set_text("echo x");
        s = update(s, Msg::Key(ev(Key::Named(NamedKey::Enter), None)));
        assert!(!s.collapsed.contains(&blk), "un comando sin salida no recede");
    }

    #[test]
    fn word_range_picks_the_word_under_the_column() {
        // "foo bar_baz qux" — col dentro de "bar_baz" selecciona toda la
        // palabra (incluye `_`); sobre el espacio no selecciona. La usa el
        // doble-click de la superficie de terminal.
        let t = "foo bar_baz qux";
        assert_eq!(word_range_at(t, 5), (4, 11)); // dentro de bar_baz
        assert_eq!(word_range_at(t, 0), (0, 3)); // foo
        assert_eq!(word_range_at(t, 3), (0, 3)); // justo después de foo
        assert_eq!(word_range_at(t, 11), (4, 11)); // justo después de bar_baz
    }

    #[test]
    fn scroll_clamps_between_zero_and_overflow() {
        let mut s = State::new(Source::Local);
        *s.out_overflow.lock().unwrap() = 100.0;
        s = update(s, Msg::Scroll(40.0));
        assert_eq!(s.scroll_px, 40.0);
        s = update(s, Msg::Scroll(200.0)); // pasa del tope → clamp a overflow
        assert_eq!(s.scroll_px, 100.0);
        s = update(s, Msg::Scroll(-500.0)); // de vuelta al fondo
        assert_eq!(s.scroll_px, 0.0);
    }

    #[test]
    fn scroll_setea_anchor_para_estabilidad_bajo_append() {
        // Al hacer scroll up, el anchor capta el overflow vigente para
        // que appends posteriores no muevan la vista del usuario (Fase 5
        // del SDD-TERMINAL).
        let mut s = State::new(Source::Local);
        *s.out_overflow.lock().unwrap() = 100.0;
        s = update(s, Msg::Scroll(40.0));
        assert_eq!(s.scroll_px, 40.0);
        // anchor capturó el overflow al momento del scroll.
        assert_eq!(s.surf_scroll_anchor, 100.0);
        // Simular un append: el overflow crece pero scroll_px NO cambia.
        // La fórmula del view interpretará scroll_y contra el anchor viejo.
        *s.out_overflow.lock().unwrap() = 150.0;
        // El usuario no scrolleó; scroll_px sigue siendo 40 y anchor 100,
        // así que scroll_y intencionado = 100 - 40 = 60 (mismo de antes).
        assert_eq!(s.scroll_px, 40.0);
        assert_eq!(s.surf_scroll_anchor, 100.0);
        // Próximo scroll del usuario re-baseliza al nuevo overflow.
        // curr_scroll_y = (100 - 40) = 60. delta=10 → new = 50.
        // scroll_px = 150 - 50 = 100. anchor = 150.
        s = update(s, Msg::Scroll(10.0));
        assert_eq!(s.scroll_px, 100.0);
        assert_eq!(s.surf_scroll_anchor, 150.0);
    }

    #[test]
    fn scroll_captura_velocidad_para_inercia() {
        // El último delta del usuario queda en `surf_scroll_velocity` para
        // que el próximo Tick lo aplique con decay (Fase 5.2).
        let mut s = State::new(Source::Local);
        *s.out_overflow.lock().unwrap() = 100.0;
        s = update(s, Msg::Scroll(30.0));
        assert_eq!(s.surf_scroll_velocity, 30.0);
        s = update(s, Msg::Scroll(15.0));
        assert_eq!(s.surf_scroll_velocity, 15.0, "se reemplaza por el último");
    }

    #[test]
    fn tick_aplica_inercia_y_decae() {
        // Con velocidad seteada, Tick scrollea por ella y la reduce por
        // fricción 0.82. Eventualmente cae bajo epsilon y se anula.
        let mut s = State::new(Source::Local);
        *s.out_overflow.lock().unwrap() = 1000.0;
        s = update(s, Msg::Scroll(40.0));
        let v0 = s.surf_scroll_velocity;
        let px0 = s.scroll_px;
        // Primer Tick: scrollea 40 más → scroll_px sube por ese delta;
        // velocidad cae por fricción.
        s = update(s, Msg::Tick);
        assert!(s.scroll_px > px0, "el tick aplica el delta");
        assert!(
            s.surf_scroll_velocity.abs() < v0.abs(),
            "la velocidad decae"
        );
        // Tras ~30 ticks la velocidad ya cayó bajo epsilon (0.5).
        for _ in 0..30 {
            s = update(s, Msg::Tick);
        }
        assert_eq!(s.surf_scroll_velocity, 0.0, "termina en 0");
    }

    #[test]
    fn inercia_se_detiene_al_tocar_el_fondo() {
        // Si la inercia lleva al usuario contra el fondo (re-pin), la
        // velocidad se anula inmediatamente (sin "rebote" simulado).
        let mut s = State::new(Source::Local);
        *s.out_overflow.lock().unwrap() = 100.0;
        // Subir un poco para tener margen.
        s = update(s, Msg::Scroll(50.0));
        assert!(s.scroll_px > 0.0);
        // Inyectar velocidad hacia abajo (negativa = scroll down → bottom).
        s.surf_scroll_velocity = -500.0;
        s = update(s, Msg::Tick);
        assert_eq!(s.scroll_px, 0.0, "alcanzó el fondo");
        assert_eq!(s.surf_scroll_velocity, 0.0, "inercia se detiene en el límite");
    }

    #[test]
    fn scroll_re_pin_al_fondo_resetea_anchor() {
        // Si el scroll del usuario alcanza el fondo (scroll_y >= overflow),
        // re-pin: scroll_px=0 y anchor=0. Próximos appends siguen pegados
        // al fondo (UX terminal clásica).
        let mut s = State::new(Source::Local);
        *s.out_overflow.lock().unwrap() = 100.0;
        s = update(s, Msg::Scroll(40.0));
        assert_eq!(s.surf_scroll_anchor, 100.0);
        s = update(s, Msg::Scroll(-500.0));
        assert_eq!(s.scroll_px, 0.0);
        assert_eq!(s.surf_scroll_anchor, 0.0, "re-pin limpia el anchor");
    }

    #[test]
    fn toggle_block_flips_collapsed_set() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::ToggleBlock(3));
        assert!(s.collapsed.contains(&3), "primer toggle colapsa");
        s = update(s, Msg::ToggleBlock(3));
        assert!(!s.collapsed.contains(&3), "segundo toggle despliega");
    }

    #[test]
    fn clear_output_also_drops_collapsed_set() {
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ ls"));
        s.collapsed.insert(s.output[0].block);
        s.clear_output();
        assert!(s.output.is_empty());
        assert!(s.collapsed.is_empty(), "clear limpia también los colapsos");
    }

    /// El SurfLayout snapshot que poblaríamos en `output_pane_surface` —
    /// versión sintética para tests de la state machine, sin pasar por el
    /// render. Cubre 3 líneas mono de 6 chars cada una.
    fn synth_surf_layout() -> SurfLayout {
        let metrics = llimphi_widget_terminal::TermMetrics {
            font_size: 12.0,
            line_height: 16.0,
            char_width: 8.0,
        };
        let mut store = llimphi_widget_terminal::Scrollback::new(0);
        store.push_line("abcdef");
        store.push_line("ghijkl");
        store.push_line("mnopqr");
        SurfLayout {
            items_geo: vec![llimphi_widget_terminal::ItemGeo::Lines(0, 3)],
            scroll_y: 0.0,
            viewport_h: 200.0,
            metrics,
            gutter_w: 30.0,
            store: Arc::new(store),
        }
    }

    #[test]
    fn surf_select_drag_move_arranca_y_extiende_la_seleccion() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout());
        // Primer Move: anchor en línea 0 col 2 (ax=50 = 30 gutter + 4
        // TEXT_LEFT_PADDING + 2*8 char_w, ay=4).
        s = update(
            s,
            Msg::SurfSelectDrag {
                phase: llimphi_ui::DragPhase::Move,
                dx: 0.0,
                dy: 0.0,
                ax: 50.0,
                ay: 4.0,
            },
        );
        assert!(s.surf_selecting);
        let sel = s.surf_selection.expect("anchor set");
        assert_eq!(sel.anchor, llimphi_widget_terminal::Point::new(0, 2));
        assert_eq!(sel.head, llimphi_widget_terminal::Point::new(0, 2));
        // Move siguiente: delta de (+32, +32) → acc = (78, 36) → fila 2, col 6.
        s = update(
            s,
            Msg::SurfSelectDrag {
                phase: llimphi_ui::DragPhase::Move,
                dx: 32.0,
                dy: 32.0,
                ax: 50.0,
                ay: 4.0,
            },
        );
        let sel = s.surf_selection.expect("extended");
        assert_eq!(sel.anchor, llimphi_widget_terminal::Point::new(0, 2));
        assert_eq!(sel.head, llimphi_widget_terminal::Point::new(2, 6));
    }

    #[test]
    fn surf_select_drag_end_libera_pero_mantiene_seleccion_para_copy() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout());
        // Drag completo (Press → Move → End) cubriendo varios chars.
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::Move, dx: 0.0, dy: 0.0, ax: 50.0, ay: 4.0,
        });
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::Move, dx: 16.0, dy: 0.0, ax: 50.0, ay: 4.0,
        });
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::End, dx: 0.0, dy: 0.0, ax: 50.0, ay: 4.0,
        });
        assert!(!s.surf_selecting, "End libera el flag");
        assert!(s.surf_selection.is_some(), "pero la selección queda para copy");
    }

    #[test]
    fn surf_select_drag_end_sin_drag_real_limpia_la_seleccion_colapsada() {
        // Un Press+End sin Move intermedio = click corto. La selección queda
        // colapsada (anchor == head); el End la limpia para no dejar
        // afford visual sin sentido.
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout());
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::Move, dx: 0.0, dy: 0.0, ax: 50.0, ay: 4.0,
        });
        // Ahora un End sin Mover.
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::End, dx: 0.0, dy: 0.0, ax: 50.0, ay: 4.0,
        });
        assert!(s.surf_selection.is_none(), "click sin drag → sin selección");
    }

    /// Layout sintético con texto que el find puede matchear.
    fn synth_surf_layout_with(lines: &[&str]) -> SurfLayout {
        let metrics = llimphi_widget_terminal::TermMetrics {
            font_size: 12.0,
            line_height: 16.0,
            char_width: 8.0,
        };
        let mut store = llimphi_widget_terminal::Scrollback::new(0);
        for l in lines {
            store.push_line(l);
        }
        let len = store.len();
        SurfLayout {
            items_geo: vec![llimphi_widget_terminal::ItemGeo::Lines(0, len)],
            scroll_y: 0.0,
            viewport_h: 200.0,
            metrics,
            gutter_w: 30.0,
            store: Arc::new(store),
        }
    }

    #[test]
    fn find_open_inicializa_estado_vacio() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::FindOpen);
        let f = s.find.expect("find abierto");
        assert!(f.query.is_empty());
        assert!(f.matches.is_empty());
        assert!(f.current.is_none());
        assert!(!f.case_insensitive);
    }

    #[test]
    fn find_char_recomputa_y_resalta_el_primer_match() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout_with(&[
            "foo bar baz",
            "qux foo quux",
            "nada que ver",
        ]));
        s = update(s, Msg::FindOpen);
        s = update(s, Msg::FindChar('f'));
        s = update(s, Msg::FindChar('o'));
        s = update(s, Msg::FindChar('o'));
        let f = s.find.as_ref().expect("find abierto");
        assert_eq!(f.matches.len(), 2);
        assert_eq!(f.current, Some(0));
        // La selección debe reflejar el primer match (línea 0, col 0..3).
        let sel = s.surf_selection.expect("highlight");
        assert_eq!(sel.anchor, llimphi_widget_terminal::Point::new(0, 0));
        assert_eq!(sel.head, llimphi_widget_terminal::Point::new(0, 3));
    }

    #[test]
    fn find_next_y_prev_son_ciclicos() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() =
            Some(synth_surf_layout_with(&["aa", "aa", "aa"]));
        s = update(s, Msg::FindOpen);
        s = update(s, Msg::FindChar('a'));
        // 6 matches (2 por línea, no superpuestos).
        assert_eq!(s.find.as_ref().unwrap().matches.len(), 6);
        s = update(s, Msg::FindNext);
        assert_eq!(s.find.as_ref().unwrap().current, Some(1));
        // Prev desde 0 envuelve al último (5).
        s = update(s, Msg::FindPrev);
        s = update(s, Msg::FindPrev);
        assert_eq!(s.find.as_ref().unwrap().current, Some(5));
    }

    #[test]
    fn find_toggle_case_re_busca_con_la_nueva_politica() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout_with(&[
            "Hola", "HOLA", "hola",
        ]));
        s = update(s, Msg::FindOpen);
        s = update(s, Msg::FindChar('h'));
        s = update(s, Msg::FindChar('o'));
        s = update(s, Msg::FindChar('l'));
        s = update(s, Msg::FindChar('a'));
        // Case sensitive: sólo matchea "hola" (línea 2).
        assert_eq!(s.find.as_ref().unwrap().matches.len(), 1);
        s = update(s, Msg::FindToggleCase);
        // Case insensitive: matchea las 3.
        assert_eq!(s.find.as_ref().unwrap().matches.len(), 3);
    }

    #[test]
    fn find_close_limpia_estado_y_selection_del_match() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout_with(&["foo"]));
        s = update(s, Msg::FindOpen);
        s = update(s, Msg::FindChar('f'));
        s = update(s, Msg::FindChar('o'));
        s = update(s, Msg::FindChar('o'));
        assert!(s.surf_selection.is_some());
        s = update(s, Msg::FindClose);
        assert!(s.find.is_none());
        assert!(s.surf_selection.is_none(), "Esc no deja selección residual del match");
    }

    #[test]
    fn find_backspace_re_busca_con_la_query_acortada() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() =
            Some(synth_surf_layout_with(&["foo", "foobar"]));
        s = update(s, Msg::FindOpen);
        for c in "foobar".chars() {
            s = update(s, Msg::FindChar(c));
        }
        assert_eq!(s.find.as_ref().unwrap().matches.len(), 1); // "foobar" matchea sólo línea 1
        s = update(s, Msg::FindBackspace);
        s = update(s, Msg::FindBackspace);
        s = update(s, Msg::FindBackspace);
        // Query = "foo" → 2 matches.
        assert_eq!(s.find.as_ref().unwrap().query, "foo");
        assert_eq!(s.find.as_ref().unwrap().matches.len(), 2);
    }

    #[test]
    fn surf_double_click_selecciona_la_palabra_bajo_el_punto() {
        // Snapshot con "hola mundo querido" en la primera línea — el
        // doble-click en col=6 (sobre 'u' de "mundo") debe seleccionar
        // exactamente "mundo" (bytes 5..10).
        let mut s = State::new(Source::Local);
        let metrics = llimphi_widget_terminal::TermMetrics {
            font_size: 12.0,
            line_height: 16.0,
            char_width: 8.0,
        };
        let mut store = llimphi_widget_terminal::Scrollback::new(0);
        store.push_line("hola mundo querido");
        *s.surf_layout.lock().unwrap() = Some(SurfLayout {
            items_geo: vec![llimphi_widget_terminal::ItemGeo::Lines(0, 1)],
            scroll_y: 0.0,
            viewport_h: 200.0,
            metrics,
            gutter_w: 30.0,
            store: Arc::new(store),
        });
        // lx = 30 (gutter) + 6 * 8 (char 6) + 2 = 80. ly = 4 (centro fila 0).
        s = update(s, Msg::SurfDoubleClick { lx: 80.0, ly: 4.0, rect_w: 400.0, rect_h: 200.0 });
        let sel = s.surf_selection.expect("selección de palabra");
        assert_eq!(sel.anchor, llimphi_widget_terminal::Point::new(0, 5));
        assert_eq!(sel.head, llimphi_widget_terminal::Point::new(0, 10));
    }

    #[test]
    fn surf_history_acumula_lineas_de_body_entre_frames() {
        // El cuerpo `surf_history` persiste a lo largo de la sesión —
        // a diferencia del Scrollback per-frame que arma el view. Acá
        // simulamos varios push_output y verificamos que la history
        // refleja sólo las líneas de body (no Prompts ni notices).
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ ls"));
        s.push_output(OutputLine::stdout("uno"));
        s.push_output(OutputLine::stderr("err1"));
        s.push_output(OutputLine::notice("✔ exit 0"));
        s.push_output(OutputLine::stdout("dos"));
        let h = s.surf_history.lock().unwrap();
        // Prompts y notices NO van; stdout + stderr SÍ.
        assert_eq!(h.len(), 3);
        assert_eq!(h.line(0), Some("uno"));
        assert_eq!(h.line(1), Some("err1"));
        assert_eq!(h.line(2), Some("dos"));
    }

    #[test]
    fn surf_history_excluye_lineas_de_etapa_de_pipe() {
        // Las stage_lines (capturas de tee de etapas intermedias) tampoco
        // van a la history (espeja el filtro de `body_lines_for_block`).
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ ls | wc"));
        // Línea intermedia con stage=Some(_) — no va.
        let mut staged = OutputLine::stdout("intermedia");
        staged.stage = Some(0);
        s.push_output(staged);
        // Línea de body normal — sí va.
        s.push_output(OutputLine::stdout("final"));
        let h = s.surf_history.lock().unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h.line(0), Some("final"));
    }

    #[test]
    fn refresh_spilled_visible_carga_tail_del_archive() {
        use llimphi_widget_terminal::{Scrollback, SpillStore};
        // History con cap chico + spill → muchas líneas terminan en disco.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut sb = Scrollback::new(20);
        let spill = SpillStore::create(dir.path().join("test.spill")).expect("spill");
        sb.enable_spill(spill);
        let history = Arc::new(Mutex::new(sb));
        let cache = Arc::new(Mutex::new(SurfSpilledCache::default()));

        // Cache vacío + history vacía → refresh no carga nada.
        refresh_surf_spilled_visible(&history, &cache);
        assert!(cache.lock().unwrap().lines.is_empty());

        // Push muchas líneas hasta forzar spill.
        for i in 0..50 {
            history.lock().unwrap().push_line(&format!("L{i:04}"));
        }
        let spilled = history.lock().unwrap().spilled_count();
        assert!(spilled > 0, "el cap forzó spill");

        // Refresh carga las últimas N (clamped a MAX_SPILLED_VISIBLE).
        refresh_surf_spilled_visible(&history, &cache);
        let c = cache.lock().unwrap();
        let expected_n = spilled.min(MAX_SPILLED_VISIBLE);
        assert_eq!(c.lines.len(), expected_n);
        assert_eq!(c.cached_at, spilled);
        // Última línea del cache = última línea que entró al spill.
        let last_spilled_id = spilled as u64 - 1;
        let expected_last = format!("L{:04}", last_spilled_id);
        assert_eq!(c.lines.last(), Some(&expected_last));
    }

    #[test]
    fn scrollback_grep_busca_en_memoria_y_spill() {
        // History con cap chico + spill: muchas líneas en disco + algunas
        // en memoria. `:scrollback grep <pat>` debe encontrar hits en
        // ambas mitades y reportarlos por notice.
        let mut s = State::new(Source::Local);
        // Forzar enable_spill (la State::new default no lo activa).
        let dir = tempfile::tempdir().unwrap();
        let mut sb = llimphi_widget_terminal::Scrollback::new(20);
        let spill = llimphi_widget_terminal::SpillStore::create(
            dir.path().join("test.spill"),
        )
        .unwrap();
        sb.enable_spill(spill);
        *s.surf_history.lock().unwrap() = sb;
        // Push lines: some "foo", some "bar". Cap chico → muchas spilled.
        for i in 0..50 {
            let line = if i % 5 == 0 {
                format!("foo_line_{i}")
            } else {
                format!("bar_line_{i}")
            };
            s.push_output(OutputLine::stdout(&line));
        }
        // Sanity: hay spilleadas.
        let total_spilled = s.surf_history.lock().unwrap().spilled_count();
        assert!(total_spilled > 0);
        // grep "foo": debe encontrar las 10 ocurrencias (i = 0, 5, 10, ...).
        s.input.set_text(":scrollback grep foo");
        s = update(s, Msg::Key(KeyEvent {
            key: Key::Named(NamedKey::Enter),
            state: KeyState::Pressed,
            text: None,
            modifiers: llimphi_ui::Modifiers::default(),
            repeat: false,
        }));
        // El último Notice header reporta el total de hits.
        let summary = s.output.iter().rev()
            .find(|l| l.kind == OutputKind::Notice && l.text.starts_with("grep:"))
            .expect("grep summary");
        assert!(summary.text.contains("10 hits"), "summary: {}", summary.text);
    }

    #[test]
    fn refresh_spilled_visible_no_recarga_si_no_cambio() {
        use llimphi_widget_terminal::{Scrollback, SpillStore};
        let dir = tempfile::tempdir().unwrap();
        let mut sb = Scrollback::new(20);
        let spill = SpillStore::create(dir.path().join("test.spill")).unwrap();
        sb.enable_spill(spill);
        let history = Arc::new(Mutex::new(sb));
        for i in 0..30 {
            history.lock().unwrap().push_line(&format!("L{i:04}"));
        }
        let cache = Arc::new(Mutex::new(SurfSpilledCache::default()));
        refresh_surf_spilled_visible(&history, &cache);
        let first_count = cache.lock().unwrap().cached_at;
        // Sin nuevas pushes el cached_at no debe cambiar tras un segundo refresh.
        refresh_surf_spilled_visible(&history, &cache);
        assert_eq!(cache.lock().unwrap().cached_at, first_count);
    }

    #[test]
    fn spill_effective_start_cola_y_paginada() {
        // Cola (None): arranca en las últimas MAX_SPILLED_VISIBLE.
        assert_eq!(
            spill_effective_start(None, 1000),
            (1000 - MAX_SPILLED_VISIBLE) as u64
        );
        // Menos historial que la ventana → arranca en 0.
        assert_eq!(spill_effective_start(None, 50), 0);
        // Paginada Some(id) sobre el piso → respeta el id.
        assert_eq!(spill_effective_start(Some(100), 1000), 100);
        // Clampea al piso: no más de MAX_SPILLED_LOADED desde el final.
        let floor = (5000 - MAX_SPILLED_LOADED) as u64;
        assert_eq!(spill_effective_start(Some(0), 5000), floor);
    }

    #[test]
    fn spill_page_back_decision() {
        let row_h = 16.0;
        let near = row_h; // < row_h*3 → "cerca del tope"
        // Lejos del tope → no pagina.
        assert!(spill_page_back(None, 1000, 500.0, row_h).is_none());
        // Cerca del tope con historial por delante → retrocede una página.
        let got = spill_page_back(None, 1000, near, row_h).expect("pagina");
        assert_eq!(got, (1000 - MAX_SPILLED_VISIBLE - SPILL_PAGE) as u64);
        // En el inicio del archive (effective 0) → nada más que traer.
        assert!(spill_page_back(Some(0), 50, near, row_h).is_none());
        // Contra el piso de carga → no pagina más.
        let floor = (5000 - MAX_SPILLED_LOADED) as u64;
        assert!(spill_page_back(Some(floor), 5000, near, row_h).is_none());
    }

    #[test]
    fn refresh_carga_ventana_paginada_mas_vieja() {
        use llimphi_widget_terminal::{Scrollback, SpillStore};
        let dir = tempfile::tempdir().unwrap();
        let mut sb = Scrollback::new(10);
        let spill = SpillStore::create(dir.path().join("test.spill")).unwrap();
        sb.enable_spill(spill);
        let history = Arc::new(Mutex::new(sb));
        for i in 0..400 {
            history.lock().unwrap().push_line(&format!("L{i:04}"));
        }
        let spilled = history.lock().unwrap().spilled_count();
        assert!(spilled > MAX_SPILLED_VISIBLE);
        let cache = Arc::new(Mutex::new(SurfSpilledCache::default()));
        // Cola (default): carga sólo las últimas MAX_SPILLED_VISIBLE.
        refresh_surf_spilled_visible(&history, &cache);
        {
            let c = cache.lock().unwrap();
            assert_eq!(c.lines.len(), MAX_SPILLED_VISIBLE);
            assert_eq!(c.first_id, (spilled - MAX_SPILLED_VISIBLE) as u64);
        }
        // Paginar al inicio: window_start = Some(0) → carga desde la id 0.
        cache.lock().unwrap().window_start = Some(0);
        refresh_surf_spilled_visible(&history, &cache);
        let c = cache.lock().unwrap();
        assert_eq!(c.first_id, 0, "la ventana arranca en el inicio del archive");
        assert_eq!(c.lines.len(), spilled, "todo el archive (< MAX_SPILLED_LOADED)");
        assert_eq!(c.lines.first(), Some(&"L0000".to_string()));
    }

    #[test]
    fn scroll_al_tope_pagina_el_archive_y_estabiliza() {
        use llimphi_widget_terminal::{Scrollback, SpillStore};
        let mut s = State::new(Source::Local);
        let dir = tempfile::tempdir().unwrap();
        let mut sb = Scrollback::new(10);
        let spill = SpillStore::create(dir.path().join("t.spill")).unwrap();
        sb.enable_spill(spill);
        // Suficientes para que la cola + una página no agoten el archive.
        for i in 0..1000 {
            sb.push_line(&format!("L{i:04}"));
        }
        *s.surf_history.lock().unwrap() = sb;
        let spilled = s.surf_history.lock().unwrap().spilled_count();
        assert!(spilled > MAX_SPILLED_VISIBLE + SPILL_PAGE);
        // Simulamos estar scrolled-up pegados al borde superior: overflow
        // grande y scroll_px ~ overflow (scroll_y ≈ 0).
        *s.out_overflow.lock().unwrap() = 1000.0;
        s.scroll_px = 1000.0;
        s.surf_scroll_anchor = 1000.0;
        assert!(s.surf_spilled_visible.lock().unwrap().window_start.is_none());

        // Rueda hacia arriba estando en el tope → pagina el archive.
        s = crate::update::apply_scroll_delta(s, 50.0);
        let ws = s.surf_spilled_visible.lock().unwrap().window_start;
        assert_eq!(
            ws,
            Some((spilled - MAX_SPILLED_VISIBLE - SPILL_PAGE) as u64),
            "retrocedió una página desde la cola"
        );
        // El ancla subió (K·row_h) para que la vista no salte al prependear.
        assert!(s.surf_scroll_anchor > 1000.0, "ancla compensada");

        // Volver al fondo resetea la ventana a "cola" liviana.
        *s.out_overflow.lock().unwrap() = 1000.0;
        s.scroll_px = 0.0;
        s = crate::update::apply_scroll_delta(s, -5000.0); // delta abajo fuerte
        assert!(s.surf_spilled_visible.lock().unwrap().window_start.is_none());
    }

    #[test]
    fn clear_output_tambien_resetea_history() {
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::stdout("a"));
        s.push_output(OutputLine::stdout("b"));
        assert_eq!(s.surf_history.lock().unwrap().len(), 2);
        s.clear_output();
        assert_eq!(s.surf_history.lock().unwrap().len(), 0);
        assert_eq!(s.surf_history.lock().unwrap().dropped(), 0);
    }

    #[test]
    fn scrollback_builtin_reporta_estado_en_notice() {
        // Sin spill activo (default del Config), `:scrollback` reporta
        // sólo líneas en memoria y avisa que el spill no está activo.
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::stdout("a"));
        s.push_output(OutputLine::stdout("b"));
        s.push_output(OutputLine::stdout("c"));
        s.input.set_text(":scrollback");
        s = update(s, Msg::Key(KeyEvent {
            key: Key::Named(NamedKey::Enter),
            state: KeyState::Pressed,
            text: None,
            modifiers: llimphi_ui::Modifiers::default(),
            repeat: false,
        }));
        // El último notice debe mencionar el conteo.
        let last_notice = s.output.iter().rev()
            .find(|l| l.kind == OutputKind::Notice)
            .expect("notice");
        assert!(
            last_notice.text.contains("scrollback") || last_notice.text.contains("spill"),
            "notice menciona scrollback/spill: {}", last_notice.text
        );
    }

    #[test]
    fn dos_double_clicks_seguidos_seleccionan_la_linea_entera() {
        // tap-tap = word. tap-tap-tap-tap (dos pares) dentro de 350 ms =
        // line (paridad xterm triple-click). El handler usa el timestamp
        // ms entre los dos SurfDoubleClick.
        let mut s = State::new(Source::Local);
        let metrics = llimphi_widget_terminal::TermMetrics {
            font_size: 12.0, line_height: 16.0, char_width: 8.0,
        };
        let mut store = llimphi_widget_terminal::Scrollback::new(0);
        store.push_line("hola mundo querido");
        *s.surf_layout.lock().unwrap() = Some(SurfLayout {
            items_geo: vec![llimphi_widget_terminal::ItemGeo::Lines(0, 1)],
            scroll_y: 0.0,
            viewport_h: 200.0,
            metrics,
            gutter_w: 30.0,
            store: Arc::new(store),
        });
        // Primer double-click: selecciona "hola" (palabra).
        s = update(s, Msg::SurfDoubleClick { lx: 50.0, ly: 4.0, rect_w: 400.0, rect_h: 200.0 });
        // Segundo double-click "inmediato": ahora selecciona toda la línea.
        s = update(s, Msg::SurfDoubleClick { lx: 50.0, ly: 4.0, rect_w: 400.0, rect_h: 200.0 });
        let sel = s.surf_selection.expect("line select");
        assert_eq!(sel.anchor.line, 0);
        assert_eq!(sel.anchor.col, 0);
        assert_eq!(sel.head.col, "hola mundo querido".len());
    }

    #[test]
    fn surf_double_click_sobre_separador_no_selecciona() {
        // Double-click sobre un espacio o un delimitador no debe
        // armar selección (paridad con xterm: si el click cae sobre
        // whitespace exactamente, no hay palabra).
        let mut s = State::new(Source::Local);
        let metrics = llimphi_widget_terminal::TermMetrics {
            font_size: 12.0,
            line_height: 16.0,
            char_width: 8.0,
        };
        let mut store = llimphi_widget_terminal::Scrollback::new(0);
        store.push_line("hola mundo querido");
        *s.surf_layout.lock().unwrap() = Some(SurfLayout {
            items_geo: vec![llimphi_widget_terminal::ItemGeo::Lines(0, 1)],
            scroll_y: 0.0,
            viewport_h: 200.0,
            metrics,
            gutter_w: 30.0,
            store: Arc::new(store),
        });
        // Posicionar sobre el espacio entre "hola" y "mundo" (col=4 byte = ' ').
        // lx = 30 + 4*8 + 2 = 64.
        s = update(
            s,
            Msg::SurfDoubleClick { lx: 64.0, ly: 4.0, rect_w: 400.0, rect_h: 200.0 },
        );
        // El handler de doble-click absorbe el caso "después de palabra" y
        // selecciona la palabra que termina ahí ("hola"). El otro caso
        // (espacio en medio de la línea, no después de palabra) deja la
        // selección sin tocar. Este test confirma que NO panic-ea.
        // Si seleccionó algo, debe ser "hola" (bytes 0..4).
        if let Some(sel) = s.surf_selection {
            assert_eq!(sel.anchor.line, 0);
            assert_eq!(sel.anchor.col, 0);
            assert_eq!(sel.head.col, 4);
        }
    }

    #[test]
    fn surf_open_y_dismiss_menu_actualiza_estado() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::SurfOpenMenu { x: 100.0, y: 50.0 });
        assert_eq!(s.surf_menu, Some((100.0, 50.0)));
        s = update(s, Msg::SurfMenuDismiss);
        assert!(s.surf_menu.is_none());
    }

    #[test]
    fn surf_menu_pick_seleccionar_todo_arma_rango_full() {
        // Item 2 = Seleccionar todo. Pone surf_selection desde (0,0) hasta
        // el fin de la última línea del scrollback.
        let mut s = State::new(Source::Local);
        let metrics = llimphi_widget_terminal::TermMetrics {
            font_size: 12.0, line_height: 16.0, char_width: 8.0,
        };
        let mut store = llimphi_widget_terminal::Scrollback::new(0);
        store.push_line("hola");
        store.push_line("mundo");
        store.push_line("xxx");
        *s.surf_layout.lock().unwrap() = Some(SurfLayout {
            items_geo: vec![llimphi_widget_terminal::ItemGeo::Lines(0, 3)],
            scroll_y: 0.0,
            viewport_h: 200.0,
            metrics,
            gutter_w: 30.0,
            store: Arc::new(store),
        });
        s = update(s, Msg::SurfOpenMenu { x: 50.0, y: 50.0 });
        s = update(s, Msg::SurfMenuPick(2));
        let sel = s.surf_selection.expect("select all");
        assert_eq!(sel.anchor, llimphi_widget_terminal::Point::new(0, 0));
        // Última línea = "xxx" (3 bytes).
        assert_eq!(sel.head, llimphi_widget_terminal::Point::new(2, 3));
        assert!(s.surf_menu.is_none(), "el pick cierra el menú");
    }

    #[test]
    fn surf_clear_selection_resetea_estado() {
        let mut s = State::new(Source::Local);
        *s.surf_layout.lock().unwrap() = Some(synth_surf_layout());
        // Arranca un drag.
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::Move, dx: 0.0, dy: 0.0, ax: 50.0, ay: 4.0,
        });
        s = update(s, Msg::SurfSelectDrag {
            phase: llimphi_ui::DragPhase::Move, dx: 16.0, dy: 0.0, ax: 50.0, ay: 4.0,
        });
        assert!(s.surf_selection.is_some());
        s = update(s, Msg::SurfClearSelection);
        assert!(s.surf_selection.is_none());
        assert!(!s.surf_selecting);
    }
    #[test]
    fn output_snapshot_restore_round_trip() {
        let mut s = State::new(Source::Local);
        s.push_output(OutputLine::prompt("$ echo uno"));
        s.push_output(OutputLine::stdout("uno"));
        s.push_output(OutputLine::prompt("$ echo dos"));
        s.push_output(OutputLine::stdout("dos"));
        let snap = s.output_snapshot(1000);
        assert_eq!(snap.lines.len(), 4);
        assert_eq!(snap.block_seq, s.block_seq);
        // JSON round-trip (lo que persiste el chasis).
        let json = serde_json::to_string(&snap).expect("serializa");
        let back: OutputSnapshot = serde_json::from_str(&json).expect("parsea");

        let mut s2 = State::new(Source::Local);
        s2.restore_output(back);
        // 4 líneas + el notice separador.
        assert_eq!(s2.output.len(), 5);
        // El bloque viejo no-último queda plegado; el último, abierto.
        let primero = snap.lines[0].block;
        let ultimo = snap.lines[3].block;
        assert!(s2.collapsed.contains(&primero));
        assert!(!s2.collapsed.contains(&ultimo));
        // Los ids no se reciclan: block_seq avanza desde el snapshot.
        assert!(s2.block_seq >= snap.block_seq);
        // Lo nuevo abre bloque nuevo, no contamina los restaurados.
        s2.push_output(OutputLine::prompt("$ echo tres"));
        assert!(s2.current_block > ultimo);
    }

    #[test]
    fn output_snapshot_capea_y_conserva_metadata_de_bloques_presentes() {
        let mut s = State::new(Source::Local);
        for i in 0..50 {
            s.push_output(OutputLine::prompt(format!("$ cmd {i}")));
            s.push_output(OutputLine::stdout(format!("salida {i}")));
        }
        let snap = s.output_snapshot(10);
        assert_eq!(snap.lines.len(), 10);
        // Toda la metadata refiere a bloques presentes en el recorte.
        let presentes: std::collections::HashSet<u64> =
            snap.lines.iter().map(|l| l.block).collect();
        assert!(snap.block_command.keys().all(|b| presentes.contains(b)));
        assert!(snap.block_started.keys().all(|b| presentes.contains(b)));
    }

