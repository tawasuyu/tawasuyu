pub(crate) const WORKERS_BOOTSTRAP: &str = r#"
// Fase 7.143 — Web Workers (`Worker`, `SharedWorker`). Hilos en segundo plano para
// trabajo pesado sin bloquear el hilo de UI. El motor no corre un realm JS aislado
// por worker (eso es del chrome): la creación y el paso de mensajes son host-driven.
//   · new Worker(url) publica kind: 'worker-spawn' (value `<id> GS <url>`).
//   · worker.postMessage(data) publica kind: 'worker-message' (value `<id> GS <json>`).
//   · worker.terminate() publica kind: 'worker-terminate'.
//   · El chrome empuja mensajes salientes del worker con `__puriy_worker_message(id, data)`
//     (→ MessageEvent 'message') y errores con `__puriy_worker_error(id, msg)` (→ 'error').
// SharedWorker expone un `port` (MessagePort de Fase 7.81) cuyo postMessage publica
// kind: 'sharedworker-message'; el host entrega entrantes con
// `__puriy_sharedworker_message(id, data)`.
(function() {
    if (globalThis.Worker != null) return;
    var GS = String.fromCharCode(0x1D);
    var nextId = 1;
    var workers = {};
    var sharedWorkers = {};
    globalThis.__puriy_worker_next_id = 1;

    function emit(target, type, ev) {
        var h = target['on' + type];
        if (typeof h === 'function') { try { h.call(target, ev); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; } }
        target.dispatchEvent(ev);
    }
    function serialize(message) {
        var cloned;
        try { cloned = globalThis.structuredClone(message); } catch (e) { cloned = message; }
        try { return JSON.stringify(cloned); } catch (e2) { return ''; }
    }

    // ---- Worker ----
    function Worker(url, opts) {
        globalThis.EventTarget.call(this);
        this._id = nextId++; globalThis.__puriy_worker_next_id = nextId;
        workers[this._id] = this;
        this.url = String(url);
        opts = opts || {};
        this.name = opts.name != null ? opts.name : '';
        this.onmessage = null; this.onmessageerror = null; this.onerror = null;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'worker-spawn', value: this._id + GS + this.url });
    }
    Worker.prototype = Object.create(globalThis.EventTarget.prototype);
    Worker.prototype.constructor = Worker;
    Worker.prototype.postMessage = function(message) {
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'worker-message', value: this._id + GS + serialize(message)
        });
    };
    Worker.prototype.terminate = function() {
        delete workers[this._id];
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'worker-terminate', value: String(this._id) });
    };

    // ---- SharedWorker ----
    function SharedWorker(url, opts) {
        globalThis.EventTarget.call(this);
        this._id = nextId++; globalThis.__puriy_worker_next_id = nextId;
        sharedWorkers[this._id] = this;
        this.url = String(url);
        if (typeof opts === 'string') opts = { name: opts };
        opts = opts || {};
        this.name = opts.name != null ? opts.name : '';
        this.onerror = null;
        var port = new globalThis.MessagePort();
        var sid = this._id;
        port.postMessage = function(message) {
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'sharedworker-message', value: sid + GS + serialize(message)
            });
        };
        port.start();
        this.port = port;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'sharedworker-spawn', value: this._id + GS + this.url });
    }
    SharedWorker.prototype = Object.create(globalThis.EventTarget.prototype);
    SharedWorker.prototype.constructor = SharedWorker;

    // ---- Hooks del host ----
    globalThis.__puriy_worker_message = function(id, data) {
        var w = workers[id]; if (!w) return false;
        emit(w, 'message', new globalThis.MessageEvent('message', { data: data, origin: '' }));
        return true;
    };
    globalThis.__puriy_worker_error = function(id, msg) {
        var w = workers[id]; if (!w) return false;
        var ev = new globalThis.Event('error');
        ev.message = msg != null ? String(msg) : '';
        emit(w, 'error', ev);
        return true;
    };
    globalThis.__puriy_sharedworker_message = function(id, data) {
        var w = sharedWorkers[id]; if (!w) return false;
        w.port._deliver(data);
        return true;
    };

    globalThis.Worker = Worker;
    globalThis.SharedWorker = SharedWorker;
    void 0;
})();
"#;
