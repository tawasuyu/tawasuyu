pub(crate) const POINTERLOCK_BOOTSTRAP: &str = r#"
// Fase 7.124 — Pointer Lock API (`element.requestPointerLock()` /
// `document.exitPointerLock()`). Juegos en primera persona y editores 3D la usan para
// capturar el cursor: el ratón deja de moverse en pantalla y los eventos llegan como deltas
// relativos (`movementX`/`movementY`). El motor no controla el cursor del compositor:
// `requestPointerLock()` publica `kind: 'pointerlock-request'` (value = id del elemento) y
// devuelve una Promise que el chrome resuelve con `__puriy_pointerlock_resolve(elementId)`
// (setea `document.pointerLockElement`, dispara `pointerlockchange`) o rechaza con
// `__puriy_pointerlock_reject(elementId)` (dispara `pointerlockerror`). `exitPointerLock()`
// limpia el estado, dispara `pointerlockchange` y publica `pointerlock-exit`. Los eventos
// viajan por `__puriy_dispatch_window` (igual que fullscreen 7.123 / visibility 7.42) y
// corren `document.onpointerlockchange` si es función.
(function() {
    var doc = globalThis.document = globalThis.document || {};
    if (doc.exitPointerLock != null) return;

    var state = globalThis.__puriy_pointerlock_state = globalThis.__puriy_pointerlock_state || {
        elementId: null
    };
    var pending = globalThis.__puriy_pointerlock_pending = globalThis.__puriy_pointerlock_pending || {};

    function fireDoc(type) {
        if (typeof doc['on' + type] === 'function') {
            try { doc['on' + type].call(doc, { type: type }); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        if (typeof globalThis.__puriy_dispatch_window === 'function') {
            globalThis.__puriy_dispatch_window(type, null);
        }
    }

    Object.defineProperty(doc, 'pointerLockElement', {
        configurable: true,
        get: function() {
            var id = globalThis.__puriy_pointerlock_state.elementId;
            if (id == null) return null;
            return (globalThis.__puriy_elements && globalThis.__puriy_elements[id]) || null;
        }
    });

    doc.exitPointerLock = function() {
        globalThis.__puriy_pointerlock_state.elementId = null;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'pointerlock-exit', value: '' });
        fireDoc('pointerlockchange');
    };

    // Llamado por `el.requestPointerLock()` (definido en __puriy_make_element).
    globalThis.__puriy_request_pointer_lock = function(elementId) {
        var id = String(elementId);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'pointerlock-request', value: id });
        return new Promise(function(resolve, reject) {
            pending[id] = { resolve: resolve, reject: reject };
        });
    };

    globalThis.__puriy_pointerlock_resolve = function(elementId) {
        var id = String(elementId);
        var p = pending[id];
        globalThis.__puriy_pointerlock_state.elementId = id;
        fireDoc('pointerlockchange');
        if (p) { delete pending[id]; p.resolve(); return true; }
        return false;
    };
    globalThis.__puriy_pointerlock_reject = function(elementId, message) {
        var id = String(elementId);
        var p = pending[id];
        fireDoc('pointerlockerror');
        if (p) {
            delete pending[id];
            p.reject(new globalThis.DOMException(
                (message != null) ? String(message) : 'requestPointerLock denegado', 'NotSupportedError'));
            return true;
        }
        return false;
    };
    void 0;
})();
"#;
