pub(crate) const BATTERY_BOOTSTRAP: &str = r#"
// Fase 7.102 — `navigator.getBattery()` + `BatteryManager` (Battery Status API).
// Sitios de ahorro de energía y PWAs la leen para degradar animaciones/polling.
// El motor no toca el hardware: el estado vive en `__puriy_battery_state` y el
// chrome lo setea con `__puriy_set_battery({...})`, que flippea los campos y
// dispara los eventos `chargingchange` / `levelchange` / `chargingtimechange` /
// `dischargingtimechange` que correspondan (mismo patrón host-driven que
// online/offline 7.86 y matchMedia 7.98). `BatteryManager` es un EventTarget
// (Fase 7.76); `getBattery()` devuelve siempre la MISMA instancia (singleton,
// como el spec). Defaults: cargando, full, sin tiempos de descarga.
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    var st = globalThis.__puriy_battery_state = globalThis.__puriy_battery_state || {
        charging: true, chargingTime: 0, dischargingTime: Infinity, level: 1.0
    };

    function BatteryManager() {
        globalThis.EventTarget.call(this);
        this.onchargingchange = null;
        this.onchargingtimechange = null;
        this.ondischargingtimechange = null;
        this.onlevelchange = null;
    }
    BatteryManager.prototype = Object.create(globalThis.EventTarget.prototype);
    BatteryManager.prototype.constructor = BatteryManager;
    var fields = ['charging', 'chargingTime', 'dischargingTime', 'level'];
    for (var i = 0; i < fields.length; i++) {
        (function(name) {
            Object.defineProperty(BatteryManager.prototype, name, {
                get: function() { return globalThis.__puriy_battery_state[name]; }
            });
        })(fields[i]);
    }
    globalThis.BatteryManager = BatteryManager;

    var battery = new BatteryManager();

    function fire(mgr, type, onprop) {
        var ev = new globalThis.Event(type);
        if (typeof mgr[onprop] === 'function') {
            try { mgr[onprop].call(mgr, ev); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        mgr.dispatchEvent(ev);
    }

    nav.getBattery = function() { return Promise.resolve(battery); };

    globalThis.__puriy_set_battery = function(data) {
        if (data == null) return false;
        if (data.charging != null && !!data.charging !== st.charging) {
            st.charging = !!data.charging;
            fire(battery, 'chargingchange', 'onchargingchange');
        }
        if (data.chargingTime != null && Number(data.chargingTime) !== st.chargingTime) {
            st.chargingTime = Number(data.chargingTime);
            fire(battery, 'chargingtimechange', 'onchargingtimechange');
        }
        if (data.dischargingTime != null && Number(data.dischargingTime) !== st.dischargingTime) {
            st.dischargingTime = Number(data.dischargingTime);
            fire(battery, 'dischargingtimechange', 'ondischargingtimechange');
        }
        if (data.level != null && Number(data.level) !== st.level) {
            st.level = Number(data.level);
            fire(battery, 'levelchange', 'onlevelchange');
        }
        return true;
    };
})();
"#;
