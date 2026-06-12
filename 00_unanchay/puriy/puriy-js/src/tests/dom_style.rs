//! Tests de getComputedStyle, getBoundingClientRect, scroll, console avanzado, window scroll, Event/CustomEvent, replaceChildren, rAF, localStorage/sessionStorage.
    use super::*;

    // ============= Fase 7.30 — getComputedStyle stub =============

    #[test]
    fn get_computed_style_lee_lo_que_el_style_seteo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval("document.getElementById('x').style.color = 'red'").expect("e");
        rt.eval("document.getElementById('x').style.fontSize = '14px'").expect("e");
        // getPropertyValue con kebab name.
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).getPropertyValue('color')")
            .expect("e");
        assert_eq!(v, JsValue::String("red".into()));
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).getPropertyValue('font-size')")
            .expect("e");
        assert_eq!(v, JsValue::String("14px".into()));
        // Property access camelCase para propiedades comunes.
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).color")
            .expect("e");
        assert_eq!(v, JsValue::String("red".into()));
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).fontSize")
            .expect("e");
        assert_eq!(v, JsValue::String("14px".into()));
    }

    #[test]
    fn get_computed_style_prop_no_seteada_devuelve_empty_string() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        // Sin style.X seteado, getPropertyValue devuelve ''.
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).getPropertyValue('color')")
            .expect("e");
        assert_eq!(v, JsValue::String("".into()));
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).color")
            .expect("e");
        assert_eq!(v, JsValue::String("".into()));
    }

    #[test]
    fn get_computed_style_null_no_crash() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt
            .eval("getComputedStyle(null).getPropertyValue('color')")
            .expect("e");
        assert_eq!(v, JsValue::String("".into()));
    }

    #[test]
    fn get_computed_style_length_cuenta_propiedades_seteadas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).length")
            .expect("e");
        assert_eq!(v, JsValue::Number(0.0));
        rt.eval(
            "var s = document.getElementById('x').style; \
             s.color = 'red'; s.fontWeight = 'bold'; s.padding = '8px';",
        )
        .expect("e");
        let v = rt
            .eval("getComputedStyle(document.getElementById('x')).length")
            .expect("e");
        assert_eq!(v, JsValue::Number(3.0));
    }

    // ============= Fase 7.29 — getBoundingClientRect heurístico =============

    fn snap_with_dfs(id: &str, tag: &str, dfs: u32) -> ElementSnapshot {
        ElementSnapshot {
            id: id.into(),
            tag_name: tag.into(),
            text_content: String::new(),
            class_list: Vec::new(),
            value: None,
            parent_id: None,
            dataset: Vec::new(),
            attributes: Vec::new(),
            dfs_index: dfs,
        }
    }

    #[test]
    fn get_bounding_client_rect_devuelve_top_left_width_height() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dfs("x", "div", 3)]).expect("e");
        let v = rt
            .eval("var r = document.getElementById('x').getBoundingClientRect(); r.top")
            .expect("e");
        // top = (3 - 1) * 30 - scrollY(0) = 60
        assert_eq!(v, JsValue::Number(60.0));
        let v = rt.eval("r.height").expect("e");
        assert_eq!(v, JsValue::Number(30.0));
        let v = rt.eval("r.left").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
        // width = innerWidth para tag block.
        let v = rt.eval("r.width").expect("e");
        assert_eq!(v, JsValue::Number(1024.0));
        let v = rt.eval("r.right").expect("e");
        assert_eq!(v, JsValue::Number(1024.0));
        let v = rt.eval("r.bottom").expect("e");
        assert_eq!(v, JsValue::Number(90.0));
    }

    #[test]
    fn get_bounding_client_rect_descuenta_scroll_y() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dfs("x", "div", 5)]).expect("e");
        rt.set_scroll(0.0, 100.0).expect("scroll");
        // top = (5-1) * 30 - 100 = 20
        let v = rt
            .eval("document.getElementById('x').getBoundingClientRect().top")
            .expect("e");
        assert_eq!(v, JsValue::Number(20.0));
    }

    #[test]
    fn get_bounding_client_rect_inline_tag_es_100_wide() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dfs("s", "span", 1)]).expect("e");
        let v = rt
            .eval("document.getElementById('s').getBoundingClientRect().width")
            .expect("e");
        assert_eq!(v, JsValue::Number(100.0));
    }

    #[test]
    fn collect_element_snapshots_pobla_dfs_index() {
        // Verificado vía set_elements + chequear que dfs_index llega al JS.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap_with_dfs("x", "div", 42)]).expect("e");
        let v = rt.eval("document.getElementById('x')._dfs_index").expect("e");
        assert_eq!(v, JsValue::Number(42.0));
    }

    // ============= Fase 7.28 — sync chrome→JS scroll + innerWidth/Height =============

    #[test]
    fn set_scroll_actualiza_scroll_y_global() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_scroll(10.0, 200.0).expect("set_scroll");
        let v = rt.eval("scrollY").expect("e");
        assert_eq!(v, JsValue::Number(200.0));
        let v = rt.eval("scrollX").expect("e");
        assert_eq!(v, JsValue::Number(10.0));
        // pageYOffset es alias.
        let v = rt.eval("pageYOffset").expect("e");
        assert_eq!(v, JsValue::Number(200.0));
    }

    #[test]
    fn set_viewport_actualiza_inner_width_height() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        // Default es 1024×768.
        let v = rt.eval("innerWidth").expect("e");
        assert_eq!(v, JsValue::Number(1024.0));
        rt.set_viewport(1920.0, 1080.0).expect("set_vp");
        let v = rt.eval("innerWidth").expect("e");
        assert_eq!(v, JsValue::Number(1920.0));
        let v = rt.eval("innerHeight").expect("e");
        assert_eq!(v, JsValue::Number(1080.0));
        // outer* son alias en headless (no hay UI chrome).
        let v = rt.eval("outerWidth").expect("e");
        assert_eq!(v, JsValue::Number(1920.0));
    }

    #[test]
    fn set_scroll_no_publica_mutaciones_dirty() {
        // El chrome llama set_scroll para informar al JS, no para
        // pedirle al JS que aplique algo. La sincronización es read-only
        // desde la perspectiva del JS — no debe rebotar como mutación.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.drain_dom_mutations();
        rt.set_scroll(0.0, 500.0).expect("set");
        assert!(rt.drain_dom_mutations().is_empty());
    }

    // ============= Fase 7.27 — console.group/assert/count/time/dir/table =============

    #[test]
    fn console_group_indenta_subsiguientes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.group('outer'); console.log('inside'); console.groupEnd(); console.log('after')")
            .expect("e");
        // group label sin indent; inside con 2-space indent; after sin indent.
        let out = rt.stdout();
        assert!(out.contains("outer\n"), "out: {out:?}");
        assert!(out.contains("  inside\n"), "out: {out:?}");
        assert!(out.contains("after\n") && !out.contains("  after\n"), "out: {out:?}");
    }

    #[test]
    fn console_group_es_nesteable() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "console.group('a'); console.group('b'); console.log('x'); \
             console.groupEnd(); console.log('y'); console.groupEnd();",
        )
        .expect("e");
        let out = rt.stdout();
        // a sin indent, b con 2 spaces (dentro de a), x con 4 (dentro de a+b),
        // y con 2 (sólo dentro de a).
        assert!(out.contains("a\n"), "out: {out:?}");
        assert!(out.contains("  b\n"), "out: {out:?}");
        assert!(out.contains("    x\n"), "out: {out:?}");
        assert!(out.contains("  y\n"), "out: {out:?}");
    }

    #[test]
    fn console_assert_falsy_emite_stderr_truthy_no_op() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.assert(false, 'algo', 'mal')").expect("e");
        assert!(rt.stderr().contains("Assertion failed: algo mal"), "stderr: {:?}", rt.stderr());
        rt.eval("console.assert(true, 'no aparece')").expect("e");
        // stderr no debe sumar (assert con cond truthy es no-op).
        assert!(!rt.stderr().contains("no aparece"));
    }

    #[test]
    fn console_count_incrementa_por_label() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.count('a'); console.count('a'); console.count('b'); console.count('a')")
            .expect("e");
        let out = rt.stdout();
        assert!(out.contains("a: 1\n"), "out: {out:?}");
        assert!(out.contains("a: 2\n"), "out: {out:?}");
        assert!(out.contains("b: 1\n"), "out: {out:?}");
        assert!(out.contains("a: 3\n"), "out: {out:?}");
    }

    #[test]
    fn console_count_reset_vuelve_a_cero() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.count('x'); console.countReset('x'); console.count('x')")
            .expect("e");
        let out = rt.stdout();
        assert!(out.contains("x: 1\n"));
        // Post-reset el siguiente count debería ser 1 (no 2).
        assert_eq!(out.matches("x: 1\n").count(), 2);
    }

    #[test]
    fn console_time_end_calcula_delta_via_now_ms() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_now_ms(100);
        rt.eval("console.time('t1')").expect("e");
        rt.set_now_ms(250);
        rt.eval("console.timeEnd('t1')").expect("e");
        let out = rt.stdout();
        assert!(out.contains("t1: 150ms"), "out: {out:?}");
    }

    #[test]
    fn console_time_end_sin_time_emite_warning_a_stderr() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.timeEnd('inexistente')").expect("e");
        assert!(rt.stderr().contains("Timer 'inexistente' does not exist"), "stderr: {:?}", rt.stderr());
    }

    #[test]
    fn console_table_array_de_objetos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.table([{name:'a', n:1}, {name:'b', n:2}])").expect("e");
        let out = rt.stdout();
        assert!(out.contains("[0]"), "out: {out:?}");
        assert!(out.contains("[1]"), "out: {out:?}");
        assert!(out.contains("\"name\":\"a\""), "out: {out:?}");
    }

    #[test]
    fn console_dir_serializa_objeto_con_json() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("console.dir({a: 1, b: [2, 3]})").expect("e");
        let out = rt.stdout();
        assert!(out.contains("\"a\""), "out: {out:?}");
        assert!(out.contains("1"), "out: {out:?}");
    }

    // ============= Fase 7.26 — Window/Element scroll APIs =============

    #[test]
    fn window_scroll_to_actualiza_scroll_x_y_y_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.drain_dom_mutations();
        rt.eval("scrollTo(50, 200)").expect("e");
        let v = rt.eval("scrollY").expect("e");
        assert_eq!(v, JsValue::Number(200.0));
        let v = rt.eval("scrollX").expect("e");
        assert_eq!(v, JsValue::Number(50.0));
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "scroll");
        assert_eq!(muts[0].value, "50,200");
    }

    #[test]
    fn window_scroll_to_acepta_object_top_left() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("scrollTo({top: 100, left: 30})").expect("e");
        let v = rt.eval("scrollY").expect("e");
        assert_eq!(v, JsValue::Number(100.0));
        let v = rt.eval("scrollX").expect("e");
        assert_eq!(v, JsValue::Number(30.0));
    }

    #[test]
    fn window_scroll_by_suma_al_actual() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("scrollTo(10, 20); scrollBy(5, 30)").expect("e");
        let v = rt.eval("scrollX").expect("e");
        assert_eq!(v, JsValue::Number(15.0));
        let v = rt.eval("scrollY").expect("e");
        assert_eq!(v, JsValue::Number(50.0));
    }

    #[test]
    fn page_y_offset_es_alias_de_scroll_y() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("scrollTo(0, 99)").expect("e");
        let v = rt.eval("pageYOffset").expect("e");
        assert_eq!(v, JsValue::Number(99.0));
    }

    #[test]
    fn element_scroll_top_get_set_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        // Get inicial es 0.
        let v = rt.eval("document.getElementById('x').scrollTop").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('x').scrollTop = 42").expect("e");
        // Mirror local actualizado.
        let v = rt.eval("document.getElementById('x').scrollTop").expect("e");
        assert_eq!(v, JsValue::Number(42.0));
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "scrollTop");
        assert_eq!(muts[0].value, "42");
    }

    // ============= Fase 7.25 — Event/CustomEvent + dispatchEvent =============

    #[test]
    fn event_constructor_construye_objeto_con_type_y_flags() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt
            .eval("var e = new Event('foo', {bubbles: true, cancelable: true}); e.type")
            .expect("e");
        assert_eq!(v, JsValue::String("foo".into()));
        let v = rt.eval("e.bubbles").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("e.cancelable").expect("e");
        assert_eq!(v, JsValue::Bool(true));
        let v = rt.eval("e.defaultPrevented").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

    #[test]
    fn custom_event_lleva_detail_arbitrario() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        let v = rt
            .eval("var e = new CustomEvent('save', {detail: {file: 'a.txt', size: 42}}); e.detail.file")
            .expect("e");
        assert_eq!(v, JsValue::String("a.txt".into()));
        let v = rt.eval("e.detail.size").expect("e");
        assert_eq!(v, JsValue::Number(42.0));
        let v = rt.eval("e.type").expect("e");
        assert_eq!(v, JsValue::String("save".into()));
    }

    #[test]
    fn dispatch_event_corre_handler_con_event_original() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval(
            "document.getElementById('x').addEventListener('save', function(e) { \
                console.log('detail:' + e.detail.file); \
             });",
        )
        .expect("e");
        let v = rt
            .eval(
                "document.getElementById('x').dispatchEvent(\
                    new CustomEvent('save', {detail: {file: 'a.txt'}}))",
            )
            .expect("e");
        // dispatchEvent devuelve true (no cancelado).
        assert_eq!(v, JsValue::Bool(true));
        assert_eq!(rt.stdout(), "detail:a.txt\n");
    }

    #[test]
    fn dispatch_event_bubbleable_sube_por_ancestros() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("parent", "div", ""),
            snap_with_parent("child", "span", "parent"),
        ])
        .expect("e");
        rt.eval(
            "document.getElementById('parent').addEventListener('foo', function() { console.log('p'); }); \
             document.getElementById('child').addEventListener('foo', function() { console.log('c'); });",
        )
        .expect("e");
        // Con bubbles=true: handler de parent también corre.
        rt.eval(
            "document.getElementById('child').dispatchEvent(new Event('foo', {bubbles: true}))",
        )
        .expect("e");
        assert_eq!(rt.stdout(), "c\np\n");
        // Sin bubbles: sólo target.
        rt.clear_io();
        rt.eval("document.getElementById('child').dispatchEvent(new Event('foo'))").expect("e");
        assert_eq!(rt.stdout(), "c\n");
    }

    #[test]
    fn dispatch_event_prevent_default_devuelve_false_si_cancelable() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        rt.eval(
            "document.getElementById('x').addEventListener('foo', function(e) { e.preventDefault(); });",
        )
        .expect("e");
        // cancelable: true → preventDefault() afecta defaultPrevented → returns false.
        let v = rt
            .eval("document.getElementById('x').dispatchEvent(new Event('foo', {cancelable: true}))")
            .expect("e");
        assert_eq!(v, JsValue::Bool(false));
        // cancelable: false → preventDefault() es no-op → returns true.
        let v = rt
            .eval("document.getElementById('x').dispatchEvent(new Event('foo'))")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
    }

    #[test]
    fn dispatch_event_falla_sin_event_valido() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("x", "div", "")]).expect("e");
        let res = rt.eval("document.getElementById('x').dispatchEvent(null)");
        assert!(res.is_err(), "dispatchEvent(null) debe lanzar");
        let res = rt.eval("document.getElementById('x').dispatchEvent({})");
        assert!(res.is_err(), "dispatchEvent({{}}) sin type debe lanzar");
    }

    // ============= Fase 7.24 — replaceChildren + scrollIntoView =============

    #[test]
    fn replace_children_borra_existentes_y_agrega_nuevos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("p", "ul", ""),
            snap_with_parent("a", "li", "p"),
            snap_with_parent("b", "li", "p"),
        ])
        .expect("e");
        rt.drain_dom_mutations();
        rt.eval(
            "var p = document.getElementById('p'); \
             p.replaceChildren(document.createElement('li'), document.createElement('li'));",
        )
        .expect("e");
        let muts = rt.drain_dom_mutations();
        let removes: Vec<_> = muts.iter().filter(|m| m.kind == "removeChild").collect();
        let appends: Vec<_> = muts.iter().filter(|m| m.kind == "appendChild").collect();
        assert_eq!(removes.len(), 2, "removeChild para a y b");
        assert_eq!(appends.len(), 2, "appendChild para los dos nuevos");
    }

    #[test]
    fn replace_children_vacio_solo_borra() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("p", "ul", ""), snap_with_parent("a", "li", "p")])
            .expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('p').replaceChildren()").expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.iter().filter(|m| m.kind == "removeChild").count(), 1);
        assert_eq!(muts.iter().filter(|m| m.kind == "appendChild").count(), 0);
    }

    #[test]
    fn scroll_into_view_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[snap("target", "div", "")]).expect("e");
        rt.drain_dom_mutations();
        rt.eval("document.getElementById('target').scrollIntoView()").expect("e");
        let muts = rt.drain_dom_mutations();
        assert_eq!(muts.len(), 1);
        assert_eq!(muts[0].kind, "scrollIntoView");
        assert_eq!(muts[0].id, "target");
    }

    // ============= Fase 7.23 — requestAnimationFrame =============

    #[test]
    fn request_animation_frame_dispara_al_proximo_tick_de_16ms() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("requestAnimationFrame(function(ts) { console.log('raf:' + ts) })")
            .expect("e");
        // Antes de tick: no se dispara.
        assert_eq!(rt.stdout(), "");
        // Tick a 15ms: no llega.
        rt.tick(15).expect("tick15");
        assert_eq!(rt.stdout(), "");
        // Tick a 16ms: dispara.
        rt.tick(16).expect("tick16");
        assert_eq!(rt.stdout(), "raf:16\n");
    }

    #[test]
    fn cancel_animation_frame_evita_el_disparo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "var id = requestAnimationFrame(function() { console.log('NO') }); \
             cancelAnimationFrame(id);",
        )
        .expect("e");
        rt.tick(100).expect("tick");
        assert_eq!(rt.stdout(), "");
    }

    #[test]
    fn raf_dispatch_dispara_callback_con_timestamp() {
        // El callback recibe el now_ms como argumento — patrón típico de
        // animation loop: `requestAnimationFrame(function(ts) { ... })`.
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval(
            "requestAnimationFrame(function(ts) { console.log('ts:' + ts) });",
        )
        .expect("e");
        rt.tick(50).expect("tick");
        // El timestamp coincide con el now_ms del tick (50).
        assert!(rt.stdout().contains("ts:50"), "stdout: {:?}", rt.stdout());
    }

    // ============= Fase 7.22 — localStorage + sessionStorage =============

    #[test]
    fn local_storage_set_get_remove_y_clear() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("localStorage.setItem('user', 'sergio'); localStorage.setItem('lang', 'es')")
            .expect("e");
        let v = rt.eval("localStorage.getItem('user')").expect("e");
        assert_eq!(v, JsValue::String("sergio".into()));
        let v = rt.eval("localStorage.length").expect("e");
        assert_eq!(v, JsValue::Number(2.0));
        // getItem de key inexistente devuelve null.
        let v = rt.eval("localStorage.getItem('nada')").expect("e");
        assert_eq!(v, JsValue::Null);
        // removeItem borra.
        rt.eval("localStorage.removeItem('user')").expect("e");
        let v = rt.eval("localStorage.getItem('user')").expect("e");
        assert_eq!(v, JsValue::Null);
        let v = rt.eval("localStorage.length").expect("e");
        assert_eq!(v, JsValue::Number(1.0));
        // clear vacía todo.
        rt.eval("localStorage.clear()").expect("e");
        let v = rt.eval("localStorage.length").expect("e");
        assert_eq!(v, JsValue::Number(0.0));
    }

    #[test]
    fn local_storage_setitem_coerciona_a_string() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("localStorage.setItem('n', 42); localStorage.setItem('b', true)").expect("e");
        let v = rt.eval("localStorage.getItem('n')").expect("e");
        assert_eq!(v, JsValue::String("42".into()));
        let v = rt.eval("localStorage.getItem('b')").expect("e");
        assert_eq!(v, JsValue::String("true".into()));
    }

    #[test]
    fn local_storage_key_devuelve_key_por_indice() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("localStorage.setItem('a', '1'); localStorage.setItem('b', '2')").expect("e");
        let v = rt.eval("localStorage.key(0)").expect("e");
        assert_eq!(v, JsValue::String("a".into()));
        let v = rt.eval("localStorage.key(1)").expect("e");
        assert_eq!(v, JsValue::String("b".into()));
        // Fuera de rango devuelve null.
        let v = rt.eval("localStorage.key(99)").expect("e");
        assert_eq!(v, JsValue::Null);
    }

    #[test]
    fn session_storage_es_independiente_de_local_storage() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.eval("localStorage.setItem('x', 'L'); sessionStorage.setItem('x', 'S')").expect("e");
        let v = rt.eval("localStorage.getItem('x')").expect("e");
        assert_eq!(v, JsValue::String("L".into()));
        let v = rt.eval("sessionStorage.getItem('x')").expect("e");
        assert_eq!(v, JsValue::String("S".into()));
    }

    #[test]
    fn contains_descendiente_directo_y_anidado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "u", "b").expect("d");
        rt.set_elements(&[
            snap("root", "div", ""),
            snap_with_parent("mid", "section", "root"),
            snap_with_parent("leaf", "span", "mid"),
        ])
        .expect("e");
        // Hijo directo.
        let v = rt
            .eval("document.getElementById('root').contains(document.getElementById('mid'))")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // Nieto.
        let v = rt
            .eval("document.getElementById('root').contains(document.getElementById('leaf'))")
            .expect("e");
        assert_eq!(v, JsValue::Bool(true));
        // Reverso — leaf NO contiene a root.
        let v = rt
            .eval("document.getElementById('leaf').contains(document.getElementById('root'))")
            .expect("e");
        assert_eq!(v, JsValue::Bool(false));
        // null arg → false.
        let v = rt.eval("document.getElementById('root').contains(null)").expect("e");
        assert_eq!(v, JsValue::Bool(false));
    }

