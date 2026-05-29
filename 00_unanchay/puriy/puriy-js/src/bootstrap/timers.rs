pub(crate) const TIMERS_BOOTSTRAP: &str = r#"
globalThis.__puriy_now_ms = 0;
globalThis.__puriy_timers = { next_id: 1, queue: {} };
globalThis.setTimeout = function(cb, ms) {
    if (typeof ms !== 'number' || ms < 0) ms = 0;
    var id = globalThis.__puriy_timers.next_id++;
    globalThis.__puriy_timers.queue[id] = {
        fire_at: (globalThis.__puriy_now_ms || 0) + ms,
        callback: cb,
        interval_ms: null
    };
    return id;
};
globalThis.setInterval = function(cb, ms) {
    if (typeof ms !== 'number' || ms < 1) ms = 1;
    var id = globalThis.__puriy_timers.next_id++;
    globalThis.__puriy_timers.queue[id] = {
        fire_at: (globalThis.__puriy_now_ms || 0) + ms,
        callback: cb,
        interval_ms: ms
    };
    return id;
};
globalThis.clearTimeout = function(id) {
    delete globalThis.__puriy_timers.queue[id];
};
globalThis.clearInterval = globalThis.clearTimeout;
// Fase 7.23 — requestAnimationFrame / cancelAnimationFrame. Mapeo a
// setTimeout 16ms (~60fps). El callback recibe `performance.now()`-ish
// timestamp (en ms desde el inicio del runtime, igual que __puriy_now_ms).
// Spec real: el browser sincroniza con el refresh rate del display y
// pasa un DOMHighResTimeStamp; acá aproximamos con el clock del reactor.
// id propio en `__puriy_raf` para no colisionar con setTimeout ids — así
// `cancelAnimationFrame(rafId)` no afecta a un timeout con el mismo numero.
globalThis.__puriy_raf = { next_id: 1, ids: {} };
globalThis.requestAnimationFrame = function(cb) {
    var raf_id = globalThis.__puriy_raf.next_id++;
    var timer_id = globalThis.setTimeout(function() {
        delete globalThis.__puriy_raf.ids[raf_id];
        if (typeof cb === 'function') {
            try { cb(globalThis.__puriy_now_ms || 0); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
    }, 16);
    globalThis.__puriy_raf.ids[raf_id] = timer_id;
    return raf_id;
};
globalThis.cancelAnimationFrame = function(raf_id) {
    var timer_id = globalThis.__puriy_raf.ids[raf_id];
    if (timer_id) {
        globalThis.clearTimeout(timer_id);
        delete globalThis.__puriy_raf.ids[raf_id];
    }
};
// Fase 7.43 — requestIdleCallback / cancelIdleCallback. Apps usan esto
// para diferir trabajo no-crítico (analytics flush, cache warming,
// prefetch) hasta que el main thread esté libre. **Spec real**: el
// browser corre el callback cuando detecta idle time; le pasa un
// `IdleDeadline { timeRemaining(), didTimeout }`. Si pasa `opts.timeout`
// ms sin idle, dispara igual con `didTimeout=true`.
// **Acá**: shim sobre setTimeout(cb, 0). El deadline simula 50ms libres
// siempre (apps no toman decisiones basadas en eso normalmente — sólo
// chequean `> 0`). Si hay timeout, lo respetamos como delay del
// setTimeout. id propio para que `cancelIdleCallback` no afecte timers.
globalThis.__puriy_ric = { next_id: 1, ids: {} };
globalThis.requestIdleCallback = function(cb, opts) {
    var ric_id = globalThis.__puriy_ric.next_id++;
    var delay = (opts && typeof opts.timeout === 'number' && opts.timeout > 0)
        ? Math.min(opts.timeout, 50)
        : 0;
    var didTimeout = !!(opts && typeof opts.timeout === 'number' && opts.timeout > 0);
    var timer_id = globalThis.setTimeout(function() {
        delete globalThis.__puriy_ric.ids[ric_id];
        if (typeof cb !== 'function') return;
        var deadline = {
            didTimeout: didTimeout,
            timeRemaining: function() { return 50; }
        };
        try { cb(deadline); }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }, delay);
    globalThis.__puriy_ric.ids[ric_id] = timer_id;
    return ric_id;
};
globalThis.cancelIdleCallback = function(ric_id) {
    var timer_id = globalThis.__puriy_ric.ids[ric_id];
    if (timer_id) {
        globalThis.clearTimeout(timer_id);
        delete globalThis.__puriy_ric.ids[ric_id];
    }
};
globalThis.__puriy_tick = function(now) {
    var q = globalThis.__puriy_timers.queue;
    var ids = Object.keys(q);
    ids.sort(function(a, b) { return q[a].fire_at - q[b].fire_at; });
    var fired = 0;
    for (var i = 0; i < ids.length; i++) {
        var id = ids[i];
        var t = q[id];
        if (!t) continue;
        if (t.fire_at > now) continue;
        try {
            if (typeof t.callback === 'function') {
                t.callback();
            } else if (typeof t.callback === 'string') {
                (1, eval)(t.callback);
            }
        } catch (e) {
            globalThis.__puriy_stderr += String(e) + '\n';
        }
        fired++;
        if (t.interval_ms !== null && q[id]) {
            q[id].fire_at = now + t.interval_ms;
        } else {
            delete q[id];
        }
    }
    return fired;
};
"#;
