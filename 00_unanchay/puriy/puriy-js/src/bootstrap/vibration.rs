pub(crate) const VIBRATION_BOOTSTRAP: &str = r#"
// Fase 7.108 — `navigator.vibrate()` (Vibration API). Las apps móviles la usan
// para feedback háptico (un buzz al tocar, un patrón al recibir un mensaje).
// Acepta un número (ms) o un array de números (pulsos/pausas alternados:
// [200, 100, 200] = vibra 200, pausa 100, vibra 200). El motor no maneja el
// motor de vibración: publica una mutación `kind: 'vibrate'` (value = JSON del
// patrón normalizado) que el chrome rutea al host (mismo canal que wakelock
// 7.103). `vibrate(0)` o `vibrate([])` cancela cualquier vibración en curso.
// Patrón inválido (negativo o no finito) → devuelve false sin publicar.
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.vibrate != null) return;

    nav.vibrate = function(pattern) {
        var arr;
        if (pattern == null) arr = [];
        else if (Array.isArray(pattern)) arr = pattern.map(Number);
        else arr = [Number(pattern)];
        for (var i = 0; i < arr.length; i++) {
            if (!isFinite(arr[i]) || arr[i] < 0) return false; // patrón inválido
        }
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'vibrate', value: JSON.stringify(arr)
        });
        return true;
    };
})();
"#;
