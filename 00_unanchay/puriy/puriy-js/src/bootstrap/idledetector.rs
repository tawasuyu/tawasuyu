pub(crate) const IDLEDETECTOR_BOOTSTRAP: &str = r#"
// Fase 7.117 — Idle Detection API (`IdleDetector`). Apps de presencia/chat la usan
// para saber si el usuario está activo (`userState`: 'active'|'idle') y si la pantalla
// está bloqueada (`screenState`: 'locked'|'unlocked'). Permiso por defecto: denegado.
// `IdleDetector.requestPermission()` devuelve el estado de permiso vigente; el chrome
// lo concede vía `__puriy_idle_grant()`. `start({threshold, signal})` rechaza con
// `NotAllowedError` si no hay permiso, respeta `AbortSignal` (Fase 7.74), y se registra
// para recibir empujes de estado del host vía `__puriy_idle_set(userState, screenState)`,
// que dispara el evento `change` cuando algo cambia.
(function() {
    if (globalThis.IdleDetector != null) return;

    globalThis.__puriy_idle_permission = globalThis.__puriy_idle_permission || 'denied';
    globalThis.__puriy_idle_detectors = globalThis.__puriy_idle_detectors || [];

    function IdleDetector() {
        this.userState = null;
        this.screenState = null;
        this._listeners = {};
        this._started = false;
    }

    IdleDetector.requestPermission = function() {
        return Promise.resolve(globalThis.__puriy_idle_permission);
    };

    IdleDetector.prototype.start = function(options) {
        var self = this;
        var opts = options || {};
        var signal = opts.signal;
        if (globalThis.__puriy_idle_permission !== 'granted') {
            return Promise.reject(new globalThis.DOMException(
                'IdleDetector sin permiso', 'NotAllowedError'));
        }
        if (signal && signal.aborted) {
            return Promise.reject(new globalThis.DOMException(
                'IdleDetector abortado', 'AbortError'));
        }
        self.threshold = opts.threshold || 60000;
        self.userState = 'active';
        self.screenState = 'unlocked';
        self._started = true;
        if (signal && typeof signal.addEventListener === 'function') {
            signal.addEventListener('abort', function() { self._started = false; });
        }
        globalThis.__puriy_idle_detectors.push(self);
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'idle-start', value: String(self.threshold)
        });
        return Promise.resolve();
    };

    IdleDetector.prototype.addEventListener = function(type, fn) {
        (this._listeners[type] = this._listeners[type] || []).push(fn);
    };
    IdleDetector.prototype.removeEventListener = function(type, fn) {
        var a = this._listeners[type]; if (!a) return;
        var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    };
    IdleDetector.prototype.dispatchEvent = function(ev) {
        var a = this._listeners[ev.type];
        if (a) { for (var i = 0; i < a.length; i++) a[i].call(this, ev); }
        if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
        return true;
    };

    globalThis.IdleDetector = IdleDetector;

    // El chrome concede/deniega el permiso.
    globalThis.__puriy_idle_grant = function() {
        globalThis.__puriy_idle_permission = 'granted';
    };
    globalThis.__puriy_idle_deny = function() {
        globalThis.__puriy_idle_permission = 'denied';
    };

    // El host empuja un nuevo estado a todos los detectores activos.
    globalThis.__puriy_idle_set = function(userState, screenState) {
        var list = globalThis.__puriy_idle_detectors || [];
        for (var i = 0; i < list.length; i++) {
            var d = list[i];
            if (!d._started) continue;
            var changed = (d.userState !== userState) || (d.screenState !== screenState);
            d.userState = userState;
            d.screenState = screenState;
            if (changed) d.dispatchEvent({ type: 'change' });
        }
    };
    void 0;
})();
"#;
