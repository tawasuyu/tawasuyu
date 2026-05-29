pub(crate) const ABORT_BOOTSTRAP: &str = r#"
// Fase 7.34 — AbortController + AbortSignal. Minimal: el signal lleva
// `aborted: bool` y un addEventListener('abort', fn) que dispara cuando
// .abort() se llama. fetch() chequea signal.aborted al inicio y registra
// un listener si el signal está presente.
//
// Fase 7.36 — el harness interno `__puriy_make_abort_signal()` extrae la
// construcción del signal en un helper reusado por AbortController y por
// los estáticos `AbortSignal.timeout(ms)` / `AbortSignal.any(signals)`.
// Devuelve `{signal, abort}` — el closure de abort dispara el signal y
// marca aborted=true idempotente.
globalThis.__puriy_make_abort_signal = function() {
    var listeners = [];
    var aborted = false;
    var sig = {
        aborted: false,
        reason: undefined,
        addEventListener: function(type, fn) {
            if (type === 'abort') listeners.push(fn);
        },
        removeEventListener: function(type, fn) {
            if (type === 'abort') {
                var i = listeners.indexOf(fn);
                if (i >= 0) listeners.splice(i, 1);
            }
        },
        throwIfAborted: function() {
            if (this.aborted) throw new Error('AbortError');
        }
    };
    var abort = function(reason) {
        if (aborted) return;
        aborted = true;
        sig.aborted = true;
        sig.reason = reason;
        for (var i = 0; i < listeners.length; i++) {
            try { listeners[i](); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
    };
    return {signal: sig, abort: abort};
};
globalThis.AbortController = function() {
    var pair = globalThis.__puriy_make_abort_signal();
    this.signal = pair.signal;
    this.abort = pair.abort;
};
// Fase 7.36 — AbortSignal estáticos. Spec real expone `AbortSignal` como
// clase; acá la usamos como namespace para los dos helpers que faltaban.
// `new AbortSignal()` no está soportado (spec real también lo prohíbe).
globalThis.AbortSignal = {
    // AbortSignal.timeout(ms) — devuelve un signal que aborta solo tras
    // `ms` ms. Reusa setTimeout del harness de timers (Fase 4.x): el
    // host hace tick del reloj y dispara el closure. La razón del abort
    // es un Error('TimeoutError') — el spec real usa DOMException pero
    // acá no la tenemos.
    timeout: function(ms) {
        var pair = globalThis.__puriy_make_abort_signal();
        globalThis.setTimeout(function() {
            pair.abort(new Error('TimeoutError'));
        }, ms);
        return pair.signal;
    },
    // AbortSignal.any(signals) — devuelve un signal que aborta cuando
    // CUALQUIERA de los signals input aborta. Si alguno ya está aborted
    // al construir, el resultante nace aborted con el mismo reason.
    any: function(signals) {
        var pair = globalThis.__puriy_make_abort_signal();
        if (!signals) return pair.signal;
        for (var i = 0; i < signals.length; i++) {
            var s = signals[i];
            if (!s) continue;
            if (s.aborted) {
                pair.abort(s.reason);
                return pair.signal;
            }
            (function(s2) {
                s2.addEventListener('abort', function() {
                    pair.abort(s2.reason);
                });
            })(s);
        }
        return pair.signal;
    }
};
"#;
