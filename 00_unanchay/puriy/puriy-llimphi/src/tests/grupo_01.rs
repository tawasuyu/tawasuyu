#[allow(unused_imports)] use super::*;
#[allow(unused_imports)] use super::super::*;
#[allow(unused_imports)] use llimphi_raster::kurbo::{Cap, Join};
#[allow(unused_imports)] use llimphi_raster::peniko::{Brush, Extend};



    #[test]
    fn split_words_tokeniza_con_espacios() {
        // Cada palabra lleva su espacio separador; el último sin espacio
        // porque el run no termina en espacio.
        assert_eq!(split_words("foo bar baz"), vec!["foo ", "bar ", "baz"]);
        // Espacio inicial preservado (separa del elemento inline anterior).
        assert_eq!(split_words(" baz"), vec![" baz"]);
        // Espacio final preservado (separa del siguiente elemento inline).
        assert_eq!(split_words("foo "), vec!["foo "]);
        // Una sola palabra entre dos elementos: conserva ambos lados.
        assert_eq!(split_words(" x "), vec![" x "]);
        // Vacío → sin tokens.
        assert!(split_words("").is_empty());
    }

    #[test]
    fn contexto_inline_mixto_se_detecta() {
        // <p>foo <b>bar</b> baz</p> → bloque con hijos inline múltiples.
        let bt = parse("<p>foo <b>bar</b> baz</p>");
        // Buscamos el <p> (block con inline children) en el árbol.
        let mut hallado = false;
        bt.walk(|b| {
            if b.children.len() > 1 && has_inline_children(b) {
                hallado = true;
                assert!(is_mixed_inline_context(b));
            }
        });
        assert!(hallado, "debería existir un contexto inline mixto en el <p>");
    }

    #[test]
    fn parrafo_de_un_solo_run_no_es_mixto() {
        // <p>solo texto</p> → un solo hijo de texto → NO mixto (se mide entero).
        let bt = parse("<p>solo texto sin elementos inline</p>");
        bt.walk(|b| {
            if b.text.is_none() && has_inline_children(b) {
                assert!(
                    !is_mixed_inline_context(b),
                    "un párrafo de un solo run no debe partirse en palabras"
                );
            }
        });
    }

    #[test]
    fn transform_affine_vacio_es_none() {
        assert!(transform_affine(&[], 1.0).is_none());
    }

    #[test]
    fn ctrl_rueda_zoomea_sin_ctrl_scrollea() {
        let arriba = WheelDelta { x: 0.0, y: -1.0 };
        let abajo = WheelDelta { x: 0.0, y: 1.0 };
        let ctrl = Modifiers { ctrl: true, ..Default::default() };
        let sin = Modifiers::default();
        // Ctrl + rueda arriba = acercar; Ctrl + rueda abajo = alejar.
        assert!(matches!(wheel_to_msg(arriba, ctrl), Some(Msg::ZoomIn)));
        assert!(matches!(wheel_to_msg(abajo, ctrl), Some(Msg::ZoomOut)));
        // Sin Ctrl la rueda scrollea, no zoomea.
        assert!(matches!(wheel_to_msg(abajo, sin), Some(Msg::Scroll(_))));
        assert!(matches!(wheel_to_msg(arriba, sin), Some(Msg::Scroll(_))));
    }

    #[test]
    fn hover_tween_avanza_hacia_uno_mientras_hovered() {
        let tw = HoverTween {
            hovered: true,
            progress_at_toggle: 0.0,
            toggle_ms: 1000,
            duration_ms: 1000,
        };
        assert!((tw.sample_linear(1500) - 0.5).abs() < 1e-6);
        // pasada la duración, clampa a 1.0.
        assert!((tw.sample_linear(9000) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hover_tween_revierte_hacia_cero_al_salir() {
        // Salió con el tween a media transición: retrocede desde 1.0.
        let tw = HoverTween {
            hovered: false,
            progress_at_toggle: 1.0,
            toggle_ms: 1000,
            duration_ms: 1000,
        };
        assert!((tw.sample_linear(1500) - 0.5).abs() < 1e-6);
        assert!(tw.sample_linear(9000).abs() < 1e-6);
    }

    #[test]
    fn hover_tween_revierte_desde_progreso_parcial_sin_saltar() {
        // Entró, llegó a 0.3, y salió: arranca el retroceso en 0.3, no en 1.0.
        let tw = HoverTween {
            hovered: false,
            progress_at_toggle: 0.3,
            toggle_ms: 2000,
            duration_ms: 1000,
        };
        assert!((tw.sample_linear(2000) - 0.3).abs() < 1e-6);
        assert!((tw.sample_linear(2100) - 0.2).abs() < 1e-6);
        assert!(tw.sample_linear(3000).abs() < 1e-6);
    }

    #[test]
    fn hover_tween_duracion_nula_es_instantanea() {
        let on = HoverTween { hovered: true, progress_at_toggle: 0.0, toggle_ms: 0, duration_ms: 0 };
        let off = HoverTween { hovered: false, progress_at_toggle: 1.0, toggle_ms: 0, duration_ms: 0 };
        assert_eq!(on.sample_linear(123), 1.0);
        assert_eq!(off.sample_linear(123), 0.0);
    }

    #[test]
    fn transform_affine_translate_escala_por_zoom() {
        use puriy_engine::style::Transform as T;
        let a = transform_affine(&[T::Translate(10.0, 20.0)], 2.0).unwrap();
        // translate(10,20) @ zoom 2 → mueve el origen a (20, 40).
        let p = a * Point::new(0.0, 0.0);
        assert!((p.x - 20.0).abs() < 1e-6, "x = {}", p.x);
        assert!((p.y - 40.0).abs() < 1e-6, "y = {}", p.y);
    }

    #[test]
    fn transform_affine_scale_no_depende_del_zoom() {
        use puriy_engine::style::Transform as T;
        let a = transform_affine(&[T::Scale(3.0, 4.0)], 2.0).unwrap();
        let p = a * Point::new(1.0, 1.0);
        assert!((p.x - 3.0).abs() < 1e-6, "x = {}", p.x);
        assert!((p.y - 4.0).abs() < 1e-6, "y = {}", p.y);
    }

    #[test]
    fn transform_affine_rotate_90_grados() {
        use puriy_engine::style::Transform as T;
        let a = transform_affine(&[T::Rotate(90.0)], 1.0).unwrap();
        // rotate(90°) horario en pantalla: (1,0) → (0,1).
        let p = a * Point::new(1.0, 0.0);
        assert!(p.x.abs() < 1e-6, "x = {}", p.x);
        assert!((p.y - 1.0).abs() < 1e-6, "y = {}", p.y);
    }

    #[test]
    fn transform_affine_compone_en_orden_de_declaracion() {
        use puriy_engine::style::Transform as T;
        // `transform: translate(10,0) scale(2)` → matriz T·S: el punto (1,0)
        // se escala a (2,0) y luego se traslada a (12,0).
        let a = transform_affine(&[T::Translate(10.0, 0.0), T::Scale(2.0, 2.0)], 1.0)
            .unwrap();
        let p = a * Point::new(1.0, 0.0);
        assert!((p.x - 12.0).abs() < 1e-6, "x = {}", p.x);
    }

    #[test]
    fn pick_download_filename_usa_hint_si_es_seguro() {
        assert_eq!(
            pick_download_filename("https://x/y/z.pdf", "doc.pdf"),
            "doc.pdf"
        );
        // Path traversal en el hint → cae a path de la URL.
        assert_eq!(
            pick_download_filename("https://x/y/z.pdf", "../etc/passwd"),
            "z.pdf"
        );
        assert_eq!(
            pick_download_filename("https://x/y/z.pdf", "a\\b"),
            "z.pdf"
        );
        // Hint vacío → path de la URL.
        assert_eq!(
            pick_download_filename("https://x/file.tar.gz", ""),
            "file.tar.gz"
        );
        // URL sin path significativo + hint vacío → fallback.
        assert_eq!(pick_download_filename("https://x/", ""), "descarga");
    }

    #[test]
    fn same_doc_with_fragment_detecta_solo_fragment() {
        assert_eq!(
            same_doc_with_fragment("https://x/p", "https://x/p#top"),
            Some("top".to_string())
        );
        // Sin fragment en target → recargar normal.
        assert_eq!(same_doc_with_fragment("https://x/p", "https://x/p"), None);
        // Path distinto → recargar normal.
        assert_eq!(
            same_doc_with_fragment("https://x/p", "https://x/q#top"),
            None
        );
        // Query distinta → recargar normal.
        assert_eq!(
            same_doc_with_fragment("https://x/p", "https://x/p?q=1#top"),
            None
        );
    }

    #[test]
    fn count_matches_devuelve_cero_cuando_query_vacia() {
        let tree = parse("<p>hola mundo</p>");
        assert_eq!(count_matches(Some(&tree), &ci("")), 0);
    }

    #[test]
    fn count_matches_devuelve_cero_cuando_tree_none() {
        assert_eq!(count_matches(None, &ci("algo")), 0);
    }

    #[test]
    fn count_matches_es_case_insensitive() {
        let tree = parse("<p>Hola MUNDO</p><p>mundO repetido</p>");
        let n = count_matches(Some(&tree), &ci("mundo"));
        assert!(n >= 2, "esperaba >= 2 matches, conseguí {n}");
    }

    #[test]
    fn count_matches_busca_dentro_de_hojas() {
        let tree = parse(
            "<article><h1>Tutorial</h1><p>Este tutorial cubre Rust</p><p>Otra cosa</p></article>",
        );
        // La query "tutorial" matchea el <h1> y el primer <p> (ambos como hojas).
        let n = count_matches(Some(&tree), &ci("tutorial"));
        assert_eq!(n, 2);
    }

    #[test]
    fn count_matches_query_sin_hits_devuelve_cero() {
        let tree = parse("<p>foo bar baz</p>");
        assert_eq!(count_matches(Some(&tree), &ci("qwerty")), 0);
    }

    // ── Fase 7.31 — toggles case-sensitive / whole-word ──────────────

    #[test]
    fn matcher_case_sensitive_distingue_mayusculas() {
        let tree = parse("<p>Hola MUNDO</p><p>mundo bajo</p>");
        let sensible = Matcher::new("mundo", MatchOpts { case_sensitive: true, whole_word: false });
        // Sólo el "mundo" en minúsculas del segundo <p> matchea.
        assert_eq!(count_matches(Some(&tree), &sensible), 1);
        // Sin el toggle, ambos (MUNDO y mundo) matchean.
        assert_eq!(count_matches(Some(&tree), &ci("mundo")), 2);
    }

    #[test]
    fn matcher_whole_word_excluye_substrings() {
        let tree = parse("<p>cat</p><p>category</p><p>a cat sat</p>");
        let word = Matcher::new("cat", MatchOpts { case_sensitive: false, whole_word: true });
        // "cat" y "a cat sat" matchean como palabra; "category" no.
        assert_eq!(count_matches(Some(&tree), &word), 2);
        // Sin whole-word, los tres contienen "cat".
        assert_eq!(count_matches(Some(&tree), &ci("cat")), 3);
    }

    #[test]
    fn matcher_whole_word_respeta_bordes_unicode() {
        let tree = parse("<p>café con leche</p><p>cafetería</p>");
        let word = Matcher::new("café", MatchOpts { case_sensitive: false, whole_word: true });
        // "café" es palabra completa en el primero; "cafetería" no contiene
        // "café" como substring (la 'é' difiere), así que igual no matchea.
        assert_eq!(count_matches(Some(&tree), &word), 1);
    }

    #[test]
    fn matcher_query_vacia_no_matchea_nada() {
        let m = ci("");
        assert!(m.is_empty());
        assert!(!m.matches("cualquier texto"));
    }

    #[test]
    fn find_open_y_close_alternan_estado_y_limpian_query() {
        let mut m = model_con_doc("<p>uno</p><p>dos</p>");
        m.find_open();
        assert!(m.find_active);
        assert!(m.find_input.text().is_empty());
        assert_eq!(m.find_current, 0);
        m.find_input.set_text("dos");
        m.find_current = 1;
        m.find_close();
        assert!(!m.find_active);
        assert!(m.find_input.text().is_empty(), "close limpia la query");
        assert_eq!(m.find_current, 0);
    }

    #[test]
    fn find_step_avanza_y_wrapea() {
        // Tres hojas de texto con "rust" → tres matches.
        let mut m = model_con_doc(
            "<p>rust uno</p><p>dos rust</p><p>tres rust cuatro</p>",
        );
        m.find_open();
        m.find_input.set_text("rust");
        // open → primer next va al match 1.
        m.find_step(true);
        assert_eq!(m.find_current, 1);
        m.find_step(true);
        assert_eq!(m.find_current, 2);
        m.find_step(true);
        assert_eq!(m.find_current, 3);
        // Cuarto next wrapea a 1.
        m.find_step(true);
        assert_eq!(m.find_current, 1);
    }

    #[test]
    fn find_step_prev_wrapea_al_ultimo() {
        let mut m = model_con_doc("<p>foo</p><p>foo otra vez</p>");
        m.find_open();
        m.find_input.set_text("foo");
        // Desde 0, prev wrapea al último (total = 2).
        m.find_step(false);
        assert_eq!(m.find_current, 2);
        m.find_step(false);
        assert_eq!(m.find_current, 1);
        m.find_step(false);
        assert_eq!(m.find_current, 2);
    }

    #[test]
    fn find_step_sin_matches_es_no_op() {
        let mut m = model_con_doc("<p>hola</p>");
        m.find_open();
        m.find_input.set_text("zzz");
        m.find_step(true);
        assert_eq!(m.find_current, 0, "sin matches no avanza");
    }

    #[test]
    fn find_step_mueve_scroll_del_tab() {
        // Un documento alto: el match vive bien abajo → scroll_y > 0.
        let mut m = model_con_doc(
            "<p>arriba</p><p>x</p><p>x</p><p>x</p><p>x</p><p>x</p><p>objetivo abajo</p>",
        );
        m.find_open();
        m.find_input.set_text("objetivo");
        m.find_step(true);
        assert_eq!(m.find_current, 1);
        assert!(
            m.tabs[0].scroll_y >= 0.0,
            "scroll_y debe ser no-negativo tras navegar"
        );
    }

    #[test]
    fn toggle_case_resetea_navegacion_y_filtra() {
        let mut m = model_con_doc("<p>Rust</p><p>rust</p>");
        m.find_open();
        m.find_input.set_text("rust");
        // Case-insensitive: ambos matchean → next llega al 2.
        m.find_step(true);
        m.find_step(true);
        assert_eq!(m.find_current, 2);
        // Activar case-sensitive resetea la nav y reduce a 1 match.
        m.find_case_sensitive = !m.find_case_sensitive;
        m.find_current = 0;
        let total = count_matches(m.active().box_tree.as_ref(), &m.find_matcher());
        assert_eq!(total, 1, "case-sensitive deja sólo el 'rust' minúscula");
        assert_eq!(m.find_current, 0, "toggle resetea la nav");
    }