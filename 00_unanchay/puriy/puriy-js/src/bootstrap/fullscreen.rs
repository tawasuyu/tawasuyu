pub(crate) const FULLSCREEN_BOOTSTRAP: &str = r#"
// Fase 7.123 — Fullscreen API (`element.requestFullscreen()` / `document.exitFullscreen()`).
// Juegos, reproductores de video y editores la usan para ocupar toda la pantalla tras un
// gesto del usuario. El motor no controla el compositor: `requestFullscreen()` publica una
// mutación `kind: 'fullscreen-request'` (value = id del elemento) y devuelve una Promise
// pendiente que el chrome resuelve con `__puriy_fullscreen_resolve(elementId)` (entra en
// fullscreen, setea `document.fullscreenElement`, dispara `fullscreenchange`) o rechaza con
// `__puriy_fullscreen_reject(elementId)` (dispara `fullscreenerror`, rechaza `TypeError`).
// `exitFullscreen()` limpia el estado, dispara `fullscreenchange` y publica `fullscreen-exit`.
// Los eventos viajan por `__puriy_dispatch_window` (mismo criterio que visibility 7.42 — los
// listeners viven sobre window) y además corren `document.onfullscreenchange` si es función.
(function() {
    var doc = globalThis.document = globalThis.document || {};
    if (doc.exitFullscreen != null) return;

    var state = globalThis.__puriy_fullscreen_state = globalThis.__puriy_fullscreen_state || {
        elementId: null
    };
    var pending = globalThis.__puriy_fullscreen_pending = globalThis.__puriy_fullscreen_pending || {};

    function fireDoc(type) {
        if (typeof doc['on' + type] === 'function') {
            try { doc['on' + type].call(doc, { type: type }); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        if (typeof globalThis.__puriy_dispatch_window === 'function') {
            globalThis.__puriy_dispatch_window(type, null);
        }
    }

    // El compositor host no nos limita; el documento siempre puede pedir fullscreen.
    doc.fullscreenEnabled = true;
    Object.defineProperty(doc, 'fullscreenElement', {
        configurable: true,
        get: function() {
            var id = globalThis.__puriy_fullscreen_state.elementId;
            if (id == null) return null;
            return (globalThis.__puriy_elements && globalThis.__puriy_elements[id]) || null;
        }
    });

    doc.exitFullscreen = function() {
        globalThis.__puriy_fullscreen_state.elementId = null;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'fullscreen-exit', value: '' });
        fireDoc('fullscreenchange');
        return Promise.resolve();
    };

    // Llamado por el método `el.requestFullscreen()` (definido en __puriy_make_element).
    globalThis.__puriy_request_fullscreen = function(elementId) {
        var id = String(elementId);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'fullscreen-request', value: id });
        return new Promise(function(resolve, reject) {
            pending[id] = { resolve: resolve, reject: reject };
        });
    };

    // El chrome confirma la entrada en fullscreen.
    globalThis.__puriy_fullscreen_resolve = function(elementId) {
        var id = String(elementId);
        var p = pending[id];
        globalThis.__puriy_fullscreen_state.elementId = id;
        fireDoc('fullscreenchange');
        if (p) { delete pending[id]; p.resolve(); return true; }
        return false;
    };
    // El chrome niega la petición (sin gesto de usuario, política, etc.).
    globalThis.__puriy_fullscreen_reject = function(elementId, message) {
        var id = String(elementId);
        var p = pending[id];
        fireDoc('fullscreenerror');
        if (p) {
            delete pending[id];
            p.reject(new globalThis.DOMException(
                (message != null) ? String(message) : 'requestFullscreen denegado', 'TypeError'));
            return true;
        }
        return false;
    };
    void 0;
})();
"#;
