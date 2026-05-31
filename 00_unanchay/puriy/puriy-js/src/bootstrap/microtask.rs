pub(crate) const MICROTASK_BOOTSTRAP: &str = r#"
// Fase 7.70 — queueMicrotask(callback). Encola una microtask que corre al
// final del turno actual (antes que cualquier macrotask de timers). Lo usan
// librerías que necesitan diferir trabajo sin la latencia de setTimeout(0).
// Se mapea sobre `Promise.resolve().then` — el runtime ya drena el microtask
// queue de QuickJS al cerrar cada eval (ver drain_pending_jobs), así que la
// FIFO de microtasks se preserva entre queueMicrotask y Promise callbacks.
//
// Divergencia: si el callback tira, el spec reporta un error global no
// capturable; acá lo mandamos a __puriy_stderr (mismo criterio que los demás
// dispatchers de eventos), porque no tenemos onerror global.
globalThis.queueMicrotask = function(callback) {
    if (typeof callback !== 'function') {
        throw new TypeError('queueMicrotask: el argumento no es una función');
    }
    Promise.resolve().then(function() {
        try { callback(); }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    });
};
"#;
