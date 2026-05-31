pub(crate) const WAKELOCK_BOOTSTRAP: &str = r#"
// Fase 7.103 — `navigator.wakeLock` (Screen Wake Lock API). Reproductores de
// video y apps de lectura la piden para que la pantalla no se apague.
// `wakeLock.request('screen')` publica una mutación `kind: 'wakelock-request'`
// al chrome y devuelve una Promise<WakeLockSentinel>; resuelve o rechaza
// SÍNCRONAMENTE según `__puriy_wakelock_state.permitido` (default true — la
// página visible obtiene el lock), que el chrome flippea con
// `__puriy_set_wakelock_permission(bool)` (mismo patrón synchronous-permission
// que mediaDevices 7.101). Denegado → `NotAllowedError`. El `WakeLockSentinel`
// es un EventTarget (Fase 7.76) con `.type` + `.released` + `release()` (publica
// `wakelock-release` y dispara el evento `release`). Wiring del compositor
// pendiente.
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.wakeLock != null) return;

    var state = globalThis.__puriy_wakelock_state = globalThis.__puriy_wakelock_state || {
        permitido: true
    };
    globalThis.__puriy_wakelock_next_id = globalThis.__puriy_wakelock_next_id || 1;

    function WakeLockSentinel(type, id) {
        globalThis.EventTarget.call(this);
        this.type = type;
        this.released = false;
        this.onrelease = null;
        this.__id = id;
    }
    WakeLockSentinel.prototype = Object.create(globalThis.EventTarget.prototype);
    WakeLockSentinel.prototype.constructor = WakeLockSentinel;
    WakeLockSentinel.prototype.release = function() {
        if (!this.released) {
            this.released = true;
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'wakelock-release', value: String(this.__id)
            });
            var ev = new globalThis.Event('release');
            if (typeof this.onrelease === 'function') {
                try { this.onrelease.call(this, ev); }
                catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
            }
            this.dispatchEvent(ev);
        }
        return Promise.resolve(undefined);
    };
    globalThis.WakeLockSentinel = WakeLockSentinel;

    function WakeLock() {}
    WakeLock.prototype.request = function(type) {
        type = (type != null) ? String(type) : 'screen';
        if (type !== 'screen') {
            return Promise.reject(new globalThis.DOMException(
                "wakeLock: tipo '" + type + "' no soportado", 'NotSupportedError'));
        }
        if (!globalThis.__puriy_wakelock_state.permitido) {
            return Promise.reject(new globalThis.DOMException(
                'Wake Lock denegado', 'NotAllowedError'));
        }
        var id = globalThis.__puriy_wakelock_next_id++;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'wakelock-request', value: id + ':' + type
        });
        return Promise.resolve(new WakeLockSentinel(type, id));
    };
    globalThis.WakeLock = WakeLock;
    nav.wakeLock = new WakeLock();

    globalThis.__puriy_set_wakelock_permission = function(allowed) {
        state.permitido = !!allowed;
        return true;
    };
})();
"#;
