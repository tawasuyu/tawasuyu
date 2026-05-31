pub(crate) const DOCUMENT_EVENTS_BOOTSTRAP: &str = r#"
// Eventos a nivel `document` (`document.addEventListener('DOMContentLoaded',
// fn)`, `document.addEventListener('click', fn)` para delegación, etc.).
// Análogo a window_events (Fase 7.39) pero el target es el objeto `document`,
// que set_document RECREA en cada carga — por eso vive como installer
// re-montable (igual que create_event). El store de listeners es global
// (`__puriy_document_listeners`); el installer lo resetea, así un document
// nuevo arranca sin listeners de la página previa.
globalThis.__puriy_install_document_events = function(doc) {
    globalThis.__puriy_document_listeners = {};
    doc.addEventListener = function(type, fn, options) {
        var capture = options === true ||
                      (options && typeof options === 'object' && options.capture === true);
        var once = !!(options && typeof options === 'object' && options.once === true);
        var key = (capture ? 'c:' : '') + String(type);
        var store = globalThis.__puriy_document_listeners;
        if (!store[key]) store[key] = [];
        store[key].push({ fn: fn, once: once });
    };
    doc.removeEventListener = function(type, fn, options) {
        var capture = options === true ||
                      (options && typeof options === 'object' && options.capture === true);
        var key = (capture ? 'c:' : '') + String(type);
        var list = globalThis.__puriy_document_listeners[key];
        if (!list) return;
        for (var i = 0; i < list.length; i++) {
            if (list[i].fn === fn) { list.splice(i, 1); return; }
        }
    };
};
// __puriy_dispatch_document(type, init, target) — corre `document.on<type>`
// + los listeners registrados. `target` opcional: cuando un evento bubbleó
// desde un elemento (click/keydown), el chrome pasa el elemento original como
// `event.target`, mientras `currentTarget` queda en `document` (modelo de
// event delegation, el uso real de document.addEventListener('click', ...)).
// Devuelve "count,prevented" — mismo formato que __puriy_dispatch_window.
globalThis.__puriy_dispatch_document = function(type, init, target) {
    var doc = globalThis.document;
    if (!doc) return '0,0';
    var event = {
        type: type,
        target: target || doc,
        currentTarget: doc,
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
    var prop = doc['on' + type];
    if (typeof prop === 'function') {
        try { prop(event); count++; }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    var list = globalThis.__puriy_document_listeners
        ? globalThis.__puriy_document_listeners[String(type)] : null;
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
// Instalación eager sobre un document fresco (runtime headless / antes de la
// primera carga). set_document re-monta el installer sobre el document nuevo.
(function() {
    var doc = globalThis.document = globalThis.document || {};
    globalThis.__puriy_install_document_events(doc);
})();
"#;
