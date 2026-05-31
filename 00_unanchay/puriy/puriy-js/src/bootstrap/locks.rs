pub(crate) const LOCKS_BOOTSTRAP: &str = r#"
// Fase 7.105 — `navigator.locks` (Web Locks API). Coordina acceso a recursos
// entre múltiples contextos: `locks.request(name, cb)` retiene el lock mientras
// la Promise que devuelve `cb` siga pendiente, y lo libera al settle. A diferencia
// de battery/wakeLock/storage NO es host-driven: la coordinación es 100% in-process
// (QuickJS es single-thread con microtasks), así que el LockManager vive entero en JS.
// Modos: 'exclusive' (default, uno a la vez) y 'shared' (varios lectores concurrentes,
// bloqueados sólo por un exclusive). Cola FIFO sin starvation: un exclusive pendiente
// al frente bloquea a los shared que llegan después. Opciones soportadas:
// `mode`, `ifAvailable` (no espera: corre cb(null) si no se puede otorgar ya),
// `steal` (libera los held actuales y otorga inmediato), `signal` (AbortSignal:
// abortar saca de la cola y rechaza con AbortError). `query()` reporta held/pending.
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.locks != null) return;

    // held: { name: [ {mode, id} ] }   pending: [ {name, mode, id, grant, reject} ] (FIFO)
    var held = Object.create(null);
    var pending = [];
    var nextId = 1;

    function Lock(name, mode) { this.name = name; this.mode = mode; }
    globalThis.Lock = Lock;

    function grantable(name, mode) {
        var cur = held[name];
        if (!cur || cur.length === 0) return true;
        if (mode === 'shared') {
            for (var i = 0; i < cur.length; i++) if (cur[i].mode === 'exclusive') return false;
            return true;
        }
        return false; // exclusive necesita el recurso libre
    }

    function pump() {
        // Otorga desde el frente mientras el head sea otorgable (FIFO estricto).
        while (pending.length > 0 && grantable(pending[0].name, pending[0].mode)) {
            pending.shift().grant();
        }
    }

    function LockManager() {}
    LockManager.prototype.request = function(name, optsOrCb, maybeCb) {
        var opts, cb;
        if (typeof optsOrCb === 'function') { opts = {}; cb = optsOrCb; }
        else { opts = optsOrCb || {}; cb = maybeCb; }
        if (typeof cb !== 'function') {
            return Promise.reject(new TypeError('locks.request: callback requerido'));
        }
        name = String(name);
        var mode = (opts.mode === 'shared') ? 'shared' : 'exclusive';
        var ifAvailable = !!opts.ifAvailable;
        var steal = !!opts.steal;
        var signal = opts.signal;

        if (signal && signal.aborted) {
            return Promise.reject(signal.reason || new globalThis.DOMException('Abortado', 'AbortError'));
        }

        return new Promise(function(resolve, reject) {
            var id = nextId++;
            function runWithLock() {
                var rec = { mode: mode, id: id };
                (held[name] = held[name] || []).push(rec);
                function release() {
                    var arr = held[name];
                    if (arr) {
                        for (var i = 0; i < arr.length; i++) {
                            if (arr[i].id === id) { arr.splice(i, 1); break; }
                        }
                        if (arr.length === 0) delete held[name];
                    }
                    pump();
                }
                var result;
                try { result = cb(new Lock(name, mode)); }
                catch (e) { release(); reject(e); return; }
                Promise.resolve(result).then(
                    function(v) { release(); resolve(v); },
                    function(e) { release(); reject(e); }
                );
            }

            if (steal) { delete held[name]; runWithLock(); return; }
            if (grantable(name, mode)) { runWithLock(); return; }
            if (ifAvailable) {
                var r;
                try { r = cb(null); } catch (e) { reject(e); return; }
                Promise.resolve(r).then(resolve, reject);
                return;
            }
            var entry = { name: name, mode: mode, id: id, grant: runWithLock };
            if (signal) {
                signal.addEventListener('abort', function() {
                    var idx = pending.indexOf(entry);
                    if (idx >= 0) {
                        pending.splice(idx, 1);
                        reject(signal.reason || new globalThis.DOMException('Abortado', 'AbortError'));
                    }
                });
            }
            pending.push(entry);
        });
    };

    LockManager.prototype.query = function() {
        var h = [], p = [];
        for (var name in held) {
            var arr = held[name];
            for (var i = 0; i < arr.length; i++) h.push({ name: name, mode: arr[i].mode, clientId: '' });
        }
        for (var j = 0; j < pending.length; j++) {
            p.push({ name: pending[j].name, mode: pending[j].mode, clientId: '' });
        }
        return Promise.resolve({ held: h, pending: p });
    };
    globalThis.LockManager = LockManager;
    nav.locks = new LockManager();
})();
"#;
