pub(crate) const EVENT_TARGET_BOOTSTRAP: &str = r#"
// Fase 7.76 — EventTarget genérico. Clase base reutilizable: AbortSignal, XHR,
// FileReader, WebSocket (cuando llegue) son EventTargets. Acá una implementación
// standalone (no retrofiteamos los EventTarget ad-hoc existentes) para que user
// code pueda `new EventTarget()` o extenderla, y despachar Events propios.
//
// Soporta: addEventListener(type, cb, options) con `options.once` y dedup del
// mismo callback (spec); removeEventListener; dispatchEvent(event) que setea
// target/currentTarget, respeta `once`, llama listeners-función o el interfaz
// `handleEvent`, corta en stopImmediatePropagation, y devuelve
// `!event.defaultPrevented`. No hay árbol de propagación (single target).
globalThis.EventTarget = function() {
    this._et = {};
};
globalThis.EventTarget.prototype.addEventListener = function(type, callback, options) {
    if (callback == null) return;
    type = String(type);
    var once = (options && typeof options === 'object') ? !!options.once : false;
    if (!this._et[type]) this._et[type] = [];
    var arr = this._et[type];
    for (var i = 0; i < arr.length; i++) {
        if (arr[i].cb === callback) return;   // dedup: mismo callback no se agrega dos veces
    }
    arr.push({ cb: callback, once: once });
};
globalThis.EventTarget.prototype.removeEventListener = function(type, callback) {
    type = String(type);
    var arr = this._et[type];
    if (!arr) return;
    for (var i = arr.length - 1; i >= 0; i--) {
        if (arr[i].cb === callback) arr.splice(i, 1);
    }
};
globalThis.EventTarget.prototype.dispatchEvent = function(event) {
    if (!event || event.type == null) {
        throw new TypeError('dispatchEvent: se requiere un Event');
    }
    event.target = this;
    event.currentTarget = this;
    var arr = this._et[String(event.type)];
    if (arr) {
        var snap = arr.slice();   // un listener puede mutar la lista durante el dispatch
        for (var i = 0; i < snap.length; i++) {
            var entry = snap[i];
            if (entry.once) this.removeEventListener(event.type, entry.cb);
            try {
                if (typeof entry.cb === 'function') entry.cb.call(this, event);
                else if (entry.cb && typeof entry.cb.handleEvent === 'function') entry.cb.handleEvent(event);
            } catch (e) {
                globalThis.__puriy_stderr += String(e) + '\n';
            }
            if (event._stopImmediate) break;
        }
    }
    event.currentTarget = null;
    return !event.defaultPrevented;
};
"#;
