pub(crate) const NOTIFICATION_BOOTSTRAP: &str = r#"
// Fase 7.94 — Notification API. `Notification.permission` + `requestPermission()`
// + `new Notification(title, opts)` (EventTarget que dispara show/click/close/
// error) + `close()`. El permiso lo decide el chrome con
// `__puriy_set_notification_permission(state)`; independiente del de la
// Permissions API (Fase 7.93) — igual que en los browsers reales, donde
// `Notification.permission` y `permissions.query({name:'notifications'})` son
// dos superficies del mismo estado pero APIs separadas.
//
// El `show`/`error` se dispara en un microtask (Promise.resolve().then) para que
// el código pueda asignar `onshow`/`onerror` DESPUÉS del `new` y aún así
// recogerlo — replica que el spec encola la presentación de forma asíncrona.
(function() {
    globalThis.__puriy_notification_permission = globalThis.__puriy_notification_permission || 'default';

    function Notification(title, options) {
        if (arguments.length < 1) throw new TypeError('Notification requiere un title');
        globalThis.EventTarget.call(this);
        options = options || {};
        this.title = String(title);
        this.body = (options.body != null) ? String(options.body) : '';
        this.icon = (options.icon != null) ? String(options.icon) : '';
        this.tag = (options.tag != null) ? String(options.tag) : '';
        this.lang = (options.lang != null) ? String(options.lang) : '';
        this.dir = (options.dir != null) ? String(options.dir) : 'auto';
        this.data = (options.data !== undefined) ? options.data : null;
        this.onshow = null; this.onclick = null; this.onclose = null; this.onerror = null;
        var self = this;
        Promise.resolve().then(function() {
            if (globalThis.__puriy_notification_permission === 'granted') {
                // Publica al chrome para que la pinte (wiring nativo pendiente).
                globalThis.__puriy_dirty.push({ id: '__window__', kind: 'notification', value: self.title });
                self.__puriy_fire('show');
            } else {
                self.__puriy_fire('error');
            }
        });
    }
    Notification.prototype = Object.create(globalThis.EventTarget.prototype);
    Notification.prototype.constructor = Notification;
    Notification.prototype.__puriy_fire = function(type) {
        var ev = new globalThis.Event(type);
        var on = this['on' + type];
        if (typeof on === 'function') {
            try { on.call(this, ev); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        this.dispatchEvent(ev);
    };
    Notification.prototype.close = function() { this.__puriy_fire('close'); };

    Object.defineProperty(Notification, 'permission', {
        get: function() { return globalThis.__puriy_notification_permission; }
    });
    // requestPermission soporta tanto la firma legacy con callback como la
    // moderna que devuelve Promise. No abre prompt real: refleja el estado que
    // el chrome haya seteado (default si ninguno).
    Notification.requestPermission = function(deprecatedCallback) {
        var perm = globalThis.__puriy_notification_permission;
        if (typeof deprecatedCallback === 'function') {
            try { deprecatedCallback(perm); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        return Promise.resolve(perm);
    };
    Notification.maxActions = 2;

    globalThis.Notification = Notification;
    globalThis.__puriy_set_notification_permission = function(state) {
        globalThis.__puriy_notification_permission = String(state);
        return true;
    };
})();
"#;
