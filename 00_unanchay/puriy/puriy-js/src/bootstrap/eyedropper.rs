pub(crate) const EYEDROPPER_BOOTSTRAP: &str = r#"
// Fase 7.116 — EyeDropper API (`new EyeDropper().open()`). Los editores de color
// y herramientas de diseño la usan para que el usuario pinche cualquier pixel de
// la pantalla y obtener su color (cuentagotas). El motor no tiene captura de
// pantalla: `open()` publica una mutación `kind: 'eyedropper'` al chrome (mismo
// canal que share 7.97) y devuelve una Promise que resuelve con
// `{ sRGBHex }` cuando el chrome elige un color con
// `__puriy_eyedropper_resolve(id, hex)`, o rechaza con `AbortError` si el
// usuario cancela (Esc) vía `__puriy_eyedropper_reject(id)` — mismo patrón
// pending-id que share/geolocation. Soporta `AbortSignal` en las opciones
// (Fase 7.74): si la señal ya está abortada o se aborta luego, rechaza.
(function() {
    if (globalThis.EyeDropper != null) return;

    globalThis.__puriy_eyedropper_pending = globalThis.__puriy_eyedropper_pending || {};
    globalThis.__puriy_eyedropper_next_id = globalThis.__puriy_eyedropper_next_id || 1;

    function EyeDropper() {}
    EyeDropper.prototype.open = function(options) {
        var signal = options && options.signal;
        if (signal && signal.aborted) {
            return Promise.reject(new globalThis.DOMException(
                'EyeDropper abortado', 'AbortError'));
        }
        var id = globalThis.__puriy_eyedropper_next_id++;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'eyedropper', value: String(id)
        });
        return new Promise(function(resolve, reject) {
            globalThis.__puriy_eyedropper_pending[id] = { resolve: resolve, reject: reject };
            if (signal && typeof signal.addEventListener === 'function') {
                signal.addEventListener('abort', function() {
                    var p = globalThis.__puriy_eyedropper_pending[id];
                    if (!p) return;
                    delete globalThis.__puriy_eyedropper_pending[id];
                    p.reject(new globalThis.DOMException('EyeDropper abortado', 'AbortError'));
                });
            }
        });
    };
    globalThis.EyeDropper = EyeDropper;

    globalThis.__puriy_eyedropper_resolve = function(id, hex) {
        var p = globalThis.__puriy_eyedropper_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_eyedropper_pending[id];
        p.resolve({ sRGBHex: (hex != null) ? String(hex) : '#000000' });
        return true;
    };
    globalThis.__puriy_eyedropper_reject = function(id, name, message) {
        var p = globalThis.__puriy_eyedropper_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_eyedropper_pending[id];
        p.reject(new globalThis.DOMException(
            (message != null) ? String(message) : 'EyeDropper canceled',
            (name != null) ? String(name) : 'AbortError'));
        return true;
    };
})();
"#;
