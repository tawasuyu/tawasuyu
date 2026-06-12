//! Tests de event subtypes UI, pointer/touch, lifecycle/form, storage event, drag/clipboard, base events, CustomElements, DOM interaction APIs, createEvent legacy, ES2024 conformance.
    use super::*;

    // ─────────────────────────────────────────────────────────────────
    // Sistema de eventos DOM reunido desde el frente `events`.
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn evt_subtipos_ui_construyen_y_heredan() {
        // KeyboardEvent: key + keyCode derivado (US layout: 'a' → 65).
        assert_eq!(
            eval("new KeyboardEvent('keydown', {key:'a'}).keyCode"),
            JsValue::Number(65.0)
        );
        assert_eq!(
            eval("new KeyboardEvent('keydown', {key:'Enter'}).keyCode"),
            JsValue::Number(13.0)
        );
        // MouseEvent: clientX + cadena de herencia UIEvent → Event.
        assert_eq!(eval("new MouseEvent('click', {clientX:42}).clientX"), JsValue::Number(42.0));
        assert_eq!(eval("(new MouseEvent('click')) instanceof UIEvent"), JsValue::Bool(true));
        assert_eq!(eval("(new MouseEvent('click')) instanceof Event"), JsValue::Bool(true));
        // UIEvent.detail, FocusEvent, InputEvent.data, WheelEvent.deltaY.
        assert_eq!(eval("new UIEvent('x', {detail:3}).detail"), JsValue::Number(3.0));
        assert_eq!(eval("(new FocusEvent('focus')) instanceof UIEvent"), JsValue::Bool(true));
        match eval("new InputEvent('input', {data:'z'}).data") {
            JsValue::String(s) => assert_eq!(s, "z"),
            o => panic!("InputEvent.data: {o:?}"),
        }
        assert_eq!(eval("new WheelEvent('wheel', {deltaY:7}).deltaY"), JsValue::Number(7.0));
    }

    #[test]
    fn evt_pointer_y_touch() {
        assert_eq!(eval("new PointerEvent('pointerdown', {pointerId:5}).pointerId"), JsValue::Number(5.0));
        match eval("new PointerEvent('pointerdown', {pointerType:'pen'}).pointerType") {
            JsValue::String(s) => assert_eq!(s, "pen"),
            o => panic!("pointerType: {o:?}"),
        }
        // TouchEvent con lista de toques.
        assert_eq!(
            eval("new TouchEvent('touchstart', {touches:[{clientX:1},{clientX:2}]}).touches.length"),
            JsValue::Number(2.0)
        );
    }

    #[test]
    fn evt_lifecycle_y_form() {
        match eval("new HashChangeEvent('hashchange', {newURL:'http://x/#a'}).newURL") {
            JsValue::String(s) => assert_eq!(s, "http://x/#a"),
            o => panic!("newURL: {o:?}"),
        }
        assert_eq!(eval("new PopStateEvent('popstate', {state:{n:1}}).state.n"), JsValue::Number(1.0));
        match eval("new AnimationEvent('animationend', {animationName:'spin'}).animationName") {
            JsValue::String(s) => assert_eq!(s, "spin"),
            o => panic!("animationName: {o:?}"),
        }
        match eval("new TransitionEvent('transitionend', {propertyName:'opacity'}).propertyName") {
            JsValue::String(s) => assert_eq!(s, "opacity"),
            o => panic!("propertyName: {o:?}"),
        }
        // SubmitEvent.submitter (verbatim del init).
        assert_eq!(eval("new SubmitEvent('submit', {submitter:{tag:'button'}}).submitter.tag === 'button'"), JsValue::Bool(true));
        // FormDataEvent (form_events) construye.
        assert_eq!(eval("typeof FormDataEvent === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof BeforeUnloadEvent === 'function'"), JsValue::Bool(true));
    }

    #[test]
    fn evt_storage_event_de_net_sigue_intacto() {
        // No debe ser pisado por el lifecycle_events portado (le quitamos su
        // StorageEvent justamente para preservar el de net + su dispatch).
        match eval("new StorageEvent('storage', {key:'k', newValue:'v'}).key") {
            JsValue::String(s) => assert_eq!(s, "k"),
            o => panic!("StorageEvent.key: {o:?}"),
        }
    }

    #[test]
    fn evt_transfer_drag_y_clipboard() {
        assert_eq!(eval("typeof DragEvent === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof ClipboardEvent === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof DataTransfer === 'function'"), JsValue::Bool(true));
        // DragEvent hereda de MouseEvent (coordenadas).
        assert_eq!(eval("(new DragEvent('drop')) instanceof MouseEvent"), JsValue::Bool(true));
    }

    #[test]
    fn evt_base_completa_event() {
        // Constantes de fase.
        assert_eq!(eval("Event.AT_TARGET"), JsValue::Number(2.0));
        assert_eq!(eval("Event.BUBBLING_PHASE"), JsValue::Number(3.0));
        // isTrusted siempre false en eventos sintéticos.
        assert_eq!(eval("new Event('x').isTrusted"), JsValue::Bool(false));
        // composedPath() existe y devuelve array.
        assert_eq!(eval("Array.isArray(new Event('x').composedPath())"), JsValue::Bool(true));
        // initEvent legacy.
        assert_eq!(eval("var e = new Event(''); e.initEvent('go', true, false); e.type === 'go' && e.bubbles"), JsValue::Bool(true));
        // cancelBubble ↔ _stopped.
        assert_eq!(eval("var e = new Event('x'); e.cancelBubble = true; e.cancelBubble"), JsValue::Bool(true));
    }

    #[test]
    fn evt_custom_elements_registry() {
        assert_eq!(
            eval("function C(){}; customElements.define('mi-tag', C); customElements.get('mi-tag') === C"),
            JsValue::Bool(true)
        );
        // Nombre inválido (sin guion) debe tirar.
        assert_eq!(
            eval("var ok=false; try{ customElements.define('notag', function(){}); }catch(e){ ok=true; } ok"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn evt_apis_dom_de_interaccion_presentes() {
        assert_eq!(eval("typeof document.createTreeWalker === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof NodeFilter === 'object' || typeof NodeFilter === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof XMLSerializer === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof getSelection === 'function'"), JsValue::Bool(true));
        assert_eq!(eval("typeof visualViewport === 'object' && visualViewport.width > 0"), JsValue::Bool(true));
    }

    #[test]
    fn evt_create_event_legacy() {
        assert_eq!(
            eval("var e = document.createEvent('Event'); e.initEvent('boom', true, true); e.type === 'boom' && e.bubbles && e.cancelable"),
            JsValue::Bool(true)
        );
    }

    // --- Fase 7.172 — conformance ES2024 del blob QuickJS embebido -------
    // El blob es quickjs-ng con stdlib ES2024 COMPLETA: estos cuatro builtins
    // ya son nativos (se verificó `.toString()` → "[native code]"), así que no
    // hay polyfill — son tests de conformance/regresión que fallarían si un
    // futuro cambio de blob degradara el engine.

    #[test]
    fn lang_promise_with_resolvers_forma() {
        // Devuelve el trío { promise, resolve, reject }.
        assert_eq!(
            eval(
                "var d = Promise.withResolvers(); \
                 typeof d.promise === 'object' && d.promise instanceof Promise && \
                 typeof d.resolve === 'function' && typeof d.reject === 'function'"
            ),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn lang_promise_with_resolvers_resuelve_externamente() {
        // El resolve externo settlea la promise; el .then corre en el drain.
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "globalThis.__wr = null; \
             var d = Promise.withResolvers(); \
             d.promise.then(function(v){ globalThis.__wr = v; }); \
             d.resolve(42);",
        )
        .expect("eval");
        assert_eq!(rt.eval("globalThis.__wr === 42").expect("read"), JsValue::Bool(true));
    }

    #[test]
    fn lang_array_from_async_iterable_con_map() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "globalThis.__fa = null; \
             Array.fromAsync([1,2,3], function(x){ return x * 2; }) \
                 .then(function(a){ globalThis.__fa = a.join(','); });",
        )
        .expect("eval");
        assert_eq!(rt.eval("globalThis.__fa === '2,4,6'").expect("read"), JsValue::Bool(true));
    }

    #[test]
    fn lang_array_from_async_awaitea_promesas() {
        // sync-iterable de promesas: fromAsync debe resolver cada valor.
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "globalThis.__fp = null; \
             Array.fromAsync([Promise.resolve(7), Promise.resolve(8)]) \
                 .then(function(a){ globalThis.__fp = a.join(','); });",
        )
        .expect("eval");
        assert_eq!(rt.eval("globalThis.__fp === '7,8'").expect("read"), JsValue::Bool(true));
    }

    #[test]
    fn lang_array_from_async_array_like() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "globalThis.__al = null; \
             Array.fromAsync({ length: 3, 0: 'a', 1: 'b', 2: 'c' }) \
                 .then(function(a){ globalThis.__al = a.join('-'); });",
        )
        .expect("eval");
        assert_eq!(rt.eval("globalThis.__al === 'a-b-c'").expect("read"), JsValue::Bool(true));
    }

    #[test]
    fn lang_object_group_by_particiona_con_proto_nulo() {
        assert_eq!(
            eval(
                "var g = Object.groupBy([1,2,3,4,5], function(n){ return n % 2 === 0 ? 'par' : 'impar'; }); \
                 g.par.join(',') === '2,4' && g.impar.join(',') === '1,3,5' && \
                 Object.getPrototypeOf(g) === null"
            ),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn lang_object_group_by_callback_invalido_tira_typeerror() {
        assert_eq!(
            eval("try { Object.groupBy([1], 42); false; } catch (e) { e instanceof TypeError; }"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn lang_map_group_by_agrupa_por_identidad_de_objeto() {
        assert_eq!(
            eval(
                "var a = {}, b = {}; \
                 var items = [{k:a,v:1},{k:b,v:2},{k:a,v:3}]; \
                 var m = Map.groupBy(items, function(it){ return it.k; }); \
                 m instanceof Map && m.get(a).length === 2 && m.get(b).length === 1 && m.get(a)[1].v === 3"
            ),
            JsValue::Bool(true)
        );
    }
