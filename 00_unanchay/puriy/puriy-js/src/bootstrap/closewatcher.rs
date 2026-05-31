pub(crate) const CLOSEWATCHER_BOOTSTRAP: &str = r#"
// Fase 7.167 — CloseWatcher API (`new CloseWatcher()`).
// Modela "peticiones de cierre" (Escape en desktop, botón Atrás en Android) de forma
// uniforme para overlays/diálogos/sidebars custom que no usan <dialog> ni popover. Es un
// EventTarget puro-JS: `requestClose()` dispara `cancel` (cancelable — si se previene, no
// cierra), si no se previene dispara `close` y destruye; `close()` cierra directo (sin
// cancel); `destroy()` desactiva sin disparar nada. Mantiene un stack global
// (`__puriy_close_watchers`); el chrome enchufa Escape/Atrás llamando
// `__puriy_close_watcher_request_close()`, que pide cerrar el watcher del tope (LIFO, como el
// spec). Soporta `options.signal` (AbortSignal — al abortar, destruye el watcher).
(function() {
    if (globalThis.CloseWatcher != null) return;
    if (typeof globalThis.EventTarget !== 'function') return;  // requiere Fase 7.76

    var stack = globalThis.__puriy_close_watchers = globalThis.__puriy_close_watchers || [];

    function CloseWatcher(options) {
        globalThis.EventTarget.call(this);
        this.oncancel = null;
        this.onclose = null;
        this._active = true;
        var signal = (options && options.signal) || null;
        var self = this;
        if (signal) {
            if (signal.aborted) {
                this._active = false;
            } else if (typeof signal.addEventListener === 'function') {
                signal.addEventListener('abort', function() { self.destroy(); });
            }
        }
        if (this._active) stack.push(this);
    }
    CloseWatcher.prototype = Object.create(globalThis.EventTarget.prototype);
    CloseWatcher.prototype.constructor = CloseWatcher;

    function fire(self, type, cancelable) {
        var ev = new globalThis.Event(type, { cancelable: !!cancelable });
        var on = self['on' + type];
        if (typeof on === 'function') {
            try { on.call(self, ev); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        self.dispatchEvent(ev);
        return ev;
    }

    function removeFromStack(self) {
        var idx = stack.indexOf(self);
        if (idx >= 0) stack.splice(idx, 1);
    }

    // requestClose(): respeta `cancel` (cancelable). Si no se previene, cierra.
    CloseWatcher.prototype.requestClose = function() {
        if (!this._active) return;
        var ev = fire(this, 'cancel', true);
        if (ev.defaultPrevented) return;
        this.close();
    };
    // close(): cierra directo (sin `cancel`). Dispara `close` y destruye.
    CloseWatcher.prototype.close = function() {
        if (!this._active) return;
        this._active = false;
        removeFromStack(this);
        fire(this, 'close', false);
    };
    // destroy(): desactiva sin disparar eventos.
    CloseWatcher.prototype.destroy = function() {
        if (!this._active) return;
        this._active = false;
        removeFromStack(this);
    };
    globalThis.CloseWatcher = CloseWatcher;

    // El chrome llama esto en Escape / botón Atrás: pide cerrar el watcher del tope.
    globalThis.__puriy_close_watcher_request_close = function() {
        if (stack.length === 0) return false;
        var top = stack[stack.length - 1];
        top.requestClose();
        return true;
    };
    void 0;
})();
"#;
