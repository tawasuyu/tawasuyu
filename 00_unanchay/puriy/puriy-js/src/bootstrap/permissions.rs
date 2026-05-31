pub(crate) const PERMISSIONS_BOOTSTRAP: &str = r#"
// Fase 7.93 — Permissions API (`navigator.permissions.query`). Las libs la
// usan para saber si pueden usar una capacidad sin disparar el prompt
// (geolocation, notifications, camera, microphone, clipboard-read, ...).
// `query({name})` devuelve una Promise<PermissionStatus>; `PermissionStatus`
// hereda de EventTarget (Fase 7.76) y expone `state` ('granted'|'denied'|
// 'prompt') + `onchange`. El estado lo decide el chrome con el hook
// `__puriy_set_permission(name, state)`, que dispara `change` en los
// PermissionStatus vivos de ese nombre (mismo patrón que online/offline 7.86).
(function() {
    globalThis.__puriy_permission_state = globalThis.__puriy_permission_state || {};
    var live = {};   // name -> [PermissionStatus vivos]

    function PermissionStatus(name) {
        globalThis.EventTarget.call(this);
        this.name = name;
        this.onchange = null;
    }
    PermissionStatus.prototype = Object.create(globalThis.EventTarget.prototype);
    PermissionStatus.prototype.constructor = PermissionStatus;
    Object.defineProperty(PermissionStatus.prototype, 'state', {
        get: function() {
            var s = globalThis.__puriy_permission_state[this.name];
            return (s != null) ? s : 'prompt';   // sin decisión del usuario aún
        }
    });

    function Permissions() {}
    Permissions.prototype.query = function(desc) {
        if (desc == null || desc.name == null) {
            return Promise.reject(new TypeError('permissions.query requiere un objeto { name }'));
        }
        var name = String(desc.name);
        var status = new PermissionStatus(name);
        if (!live[name]) live[name] = [];
        live[name].push(status);
        return Promise.resolve(status);
    };

    globalThis.PermissionStatus = PermissionStatus;
    globalThis.Permissions = Permissions;
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.permissions == null) nav.permissions = new Permissions();

    globalThis.__puriy_set_permission = function(name, state) {
        name = String(name);
        globalThis.__puriy_permission_state[name] = String(state);
        var arr = live[name];
        if (arr) {
            for (var i = 0; i < arr.length; i++) {
                var st = arr[i];
                var ev = new globalThis.Event('change');
                if (typeof st.onchange === 'function') {
                    try { st.onchange.call(st, ev); }
                    catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
                }
                st.dispatchEvent(ev);
            }
        }
        return true;
    };
})();
"#;
