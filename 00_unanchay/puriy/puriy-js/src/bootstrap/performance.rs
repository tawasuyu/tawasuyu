pub(crate) const PERFORMANCE_BOOTSTRAP: &str = r#"
// Fase 7.74 — performance.now() + performance.timeOrigin. Timing usado por
// librerías de red para medir latencia de requests. Se apoya en el reloj del
// host `__puriy_now_ms` (ms desde el arranque del runtime, el mismo que avanza
// setTimeout y que el chrome actualiza vía set_now_ms/tick). `timeOrigin` es 0
// porque ese reloj ya cuenta desde el inicio.
//
// Divergencia: `now()` devuelve ms enteros (el spec da un float de alta
// resolución sub-ms); suficiente para medir duraciones a escala de ms.
//
// QuickJS-ng trae un `performance.now` builtin atado al reloj de pared (y
// non-writable, así que `performance.now = ...` no lo pisaba). Lo reemplazamos
// por un objeto fresco para que el timing siga al reloj del host/chrome —
// determinístico y coherente con setTimeout — en vez del wall clock real.
globalThis.performance = { timeOrigin: 0 };
globalThis.performance.now = function() {
    return globalThis.__puriy_now_ms || 0;
};
"#;
