pub(crate) const SCREEN_BOOTSTRAP: &str = r#"
// Fase 7.99 — `window.screen` + `screen.orientation` + `window.devicePixelRatio`.
// Geometría del display, que las apps leen para layout responsive en JS,
// detección de retina y orientación móvil. El motor no conoce el monitor real:
// los valores viven en `__puriy_screen_state` y el chrome los setea con
// `__puriy_set_screen({...})` / `__puriy_set_device_pixel_ratio(n)` /
// `__puriy_set_orientation(type, angle)` (mismo patrón host-driven que
// matchMedia 7.98). `screen.orientation` es un EventTarget (Fase 7.76) que
// dispara `change`. Defaults razonables hasta que el chrome setee: 1280x720,
// dpr 1, landscape-primary. `orientation.lock()` rechaza con NotSupportedError
// (no controlamos el compositor); `unlock()` es no-op.
(function() {
    var st = globalThis.__puriy_screen_state = globalThis.__puriy_screen_state || {
        width: 1280, height: 720, availWidth: 1280, availHeight: 720,
        colorDepth: 24, pixelDepth: 24, devicePixelRatio: 1,
        orientationType: 'landscape-primary', orientationAngle: 0
    };

    function ScreenOrientation() {
        globalThis.EventTarget.call(this);
        this.onchange = null;
    }
    ScreenOrientation.prototype = Object.create(globalThis.EventTarget.prototype);
    ScreenOrientation.prototype.constructor = ScreenOrientation;
    Object.defineProperty(ScreenOrientation.prototype, 'type', {
        get: function() { return globalThis.__puriy_screen_state.orientationType; }
    });
    Object.defineProperty(ScreenOrientation.prototype, 'angle', {
        get: function() { return globalThis.__puriy_screen_state.orientationAngle; }
    });
    ScreenOrientation.prototype.lock = function(orientation) {
        return Promise.reject(new globalThis.DOMException(
            'screen.orientation.lock() no soportado', 'NotSupportedError'));
    };
    ScreenOrientation.prototype.unlock = function() { /* no-op */ };
    globalThis.ScreenOrientation = ScreenOrientation;

    var orientation = new ScreenOrientation();

    function Screen() {}
    var screenProps = ['width', 'height', 'availWidth', 'availHeight', 'colorDepth', 'pixelDepth'];
    for (var i = 0; i < screenProps.length; i++) {
        (function(name) {
            Object.defineProperty(Screen.prototype, name, {
                get: function() { return globalThis.__puriy_screen_state[name]; }
            });
        })(screenProps[i]);
    }
    Object.defineProperty(Screen.prototype, 'orientation', {
        get: function() { return orientation; }
    });
    // availLeft/availTop son 0 en pantalla única.
    Object.defineProperty(Screen.prototype, 'availLeft', { get: function() { return 0; } });
    Object.defineProperty(Screen.prototype, 'availTop', { get: function() { return 0; } });
    globalThis.Screen = Screen;
    globalThis.screen = new Screen();

    Object.defineProperty(globalThis, 'devicePixelRatio', {
        configurable: true,
        get: function() { return globalThis.__puriy_screen_state.devicePixelRatio; }
    });

    globalThis.__puriy_set_screen = function(data) {
        if (data == null) return false;
        var keys = ['width', 'height', 'availWidth', 'availHeight', 'colorDepth', 'pixelDepth'];
        for (var i = 0; i < keys.length; i++) {
            if (data[keys[i]] != null) st[keys[i]] = Number(data[keys[i]]);
        }
        return true;
    };
    globalThis.__puriy_set_device_pixel_ratio = function(n) {
        st.devicePixelRatio = Number(n);
        return true;
    };
    globalThis.__puriy_set_orientation = function(type, angle) {
        st.orientationType = String(type);
        st.orientationAngle = (angle != null) ? Number(angle) : st.orientationAngle;
        var ev = new globalThis.Event('change');
        if (typeof orientation.onchange === 'function') {
            try { orientation.onchange.call(orientation, ev); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        orientation.dispatchEvent(ev);
        return true;
    };
})();
"#;
