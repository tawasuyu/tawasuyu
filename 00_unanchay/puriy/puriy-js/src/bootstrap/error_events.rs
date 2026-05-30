pub(crate) const ERROR_EVENTS_BOOTSTRAP: &str = r#"
// Fase 7.82 — manejo de errores global: `ErrorEvent` + `reportError` + el
// evento `'error'` sobre window. Cierra un gap que el módulo microtask dejaba
// anotado ("no tenemos onerror global"). `ErrorEvent` hereda de Event vía
// cadena de prototipos real (Fase 7.76/7.77), así `instanceof Event` se cumple.
globalThis.ErrorEvent = function(type, init) {
    globalThis.Event.call(this, type, init);
    init = init || {};
    this.message = (init.message !== undefined) ? String(init.message) : '';
    this.filename = (init.filename !== undefined) ? String(init.filename) : '';
    this.lineno = (init.lineno !== undefined) ? (init.lineno | 0) : 0;
    this.colno = (init.colno !== undefined) ? (init.colno | 0) : 0;
    this.error = (init.error !== undefined) ? init.error : null;
};
globalThis.ErrorEvent.prototype = Object.create(globalThis.Event.prototype);
globalThis.ErrorEvent.prototype.constructor = globalThis.ErrorEvent;

// Despacha un ErrorEvent('error') sobre window. A diferencia del dispatch
// genérico de window (Fase 7.39), `window.onerror` tiene la firma CLÁSICA de 5
// args `(message, filename, lineno, colno, error)` — no recibe el event. Si
// onerror devuelve `true`, el error se marca manejado (preventDefault). Los
// listeners de addEventListener('error', fn) sí reciben el event normal.
globalThis.__puriy_dispatch_error_event = function(event) {
    var on = globalThis.onerror;
    if (typeof on === 'function') {
        try {
            var r = on.call(globalThis, event.message, event.filename,
                            event.lineno, event.colno, event.error);
            if (r === true) event.defaultPrevented = true;
        } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    var list = globalThis.__puriy_window_listeners['error'];
    if (list) {
        var snapshot = list.slice();
        for (var i = 0; i < snapshot.length; i++) {
            var entry = snapshot[i];
            try { entry.fn.call(globalThis, event); }
            catch (e2) { globalThis.__puriy_stderr += String(e2) + '\n'; }
            if (entry.once) {
                var idx = list.indexOf(entry);
                if (idx >= 0) list.splice(idx, 1);
            }
        }
    }
    return event.defaultPrevented;
};

// reportError(e) — reporta una excepción al manejador de errores global como si
// hubiera sido un error no capturado, sin abortar el flujo. El message sale del
// `.message` del Error (o de String(e) si no es un Error).
globalThis.reportError = function(e) {
    var msg;
    if (e && typeof e === 'object' && e.message !== undefined) msg = String(e.message);
    else msg = String(e);
    var event = new globalThis.ErrorEvent('error', {
        message: msg, filename: '', lineno: 0, colno: 0, error: e, cancelable: true
    });
    globalThis.__puriy_dispatch_error_event(event);
};
"#;
