pub(crate) const WINDOW_EVENTS_BOOTSTRAP: &str = r#"
// Fase 7.39 — window-level event listeners. `window === globalThis`, así
// que `window.addEventListener('scroll', fn)` cae acá. Aparte del store
// de elementos (Fase 7.10) porque el target es el propio globalThis y no
// pasa por __puriy_elements. El chrome llama `dispatch_window_event` cuando
// el user-scroll mueve scroll_y / resize cambia el viewport / la página
// termina de cargar (load).
globalThis.__puriy_window_listeners = {};
globalThis.addEventListener = function(type, fn, options) {
    var capture = options === true ||
                  (options && typeof options === 'object' && options.capture === true);
    var once = !!(options && typeof options === 'object' && options.once === true);
    var key = (capture ? 'c:' : '') + String(type);
    if (!globalThis.__puriy_window_listeners[key]) {
        globalThis.__puriy_window_listeners[key] = [];
    }
    globalThis.__puriy_window_listeners[key].push({ fn: fn, once: once });
};
globalThis.removeEventListener = function(type, fn, options) {
    var capture = options === true ||
                  (options && typeof options === 'object' && options.capture === true);
    var key = (capture ? 'c:' : '') + String(type);
    var list = globalThis.__puriy_window_listeners[key];
    if (!list) return;
    for (var i = 0; i < list.length; i++) {
        if (list[i].fn === fn) { list.splice(i, 1); return; }
    }
};
// onload / onscroll / onresize property handlers — apps modernas suelen
// usar addEventListener pero el shorthand `window.onload = fn` sigue vivo
// en código viejo. Los disparamos junto a los listeners (matchea spec).
globalThis.__puriy_dispatch_window = function(type, init) {
    var event = {
        type: type,
        target: globalThis,
        currentTarget: globalThis,
        defaultPrevented: false,
        _stopped: false,
        preventDefault: function() { this.defaultPrevented = true; },
        stopPropagation: function() { this._stopped = true; }
    };
    if (init) {
        for (var k in init) {
            if (Object.prototype.hasOwnProperty.call(init, k)) event[k] = init[k];
        }
    }
    var count = 0;
    var prop = globalThis['on' + type];
    if (typeof prop === 'function') {
        try { prop(event); count++; }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    var list = globalThis.__puriy_window_listeners[String(type)];
    if (list) {
        var snapshot = list.slice();
        for (var i = 0; i < snapshot.length; i++) {
            var entry = snapshot[i];
            try { entry.fn(event); count++; }
            catch (e2) { globalThis.__puriy_stderr += String(e2) + '\n'; }
            if (entry.once) {
                var idx = list.indexOf(entry);
                if (idx >= 0) list.splice(idx, 1);
            }
        }
    }
    return count + ',' + (event.defaultPrevented ? '1' : '0');
};
"#;
