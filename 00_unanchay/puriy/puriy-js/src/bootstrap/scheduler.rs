pub(crate) const SCHEDULER_BOOTSTRAP: &str = r#"
// Fase 7.155 — Prioritized Task Scheduling API (`scheduler.postTask`/`yield`/
// `isInputPending` + `TaskController`/`TaskSignal`/`TaskPriorityChangeEvent`).
// Permite a una app encolar trabajo con prioridad (`user-blocking` >
// `user-visible` > `background`), cancelarlo vía signal y reaccionar a cambios
// de prioridad. Cuelga de AbortController/AbortSignal (Fase 7.34), queueMicrotask
// (microtask) y setTimeout (timers). JS-puro: la cola se drena por microtask en
// orden de prioridad; `isInputPending` siempre es false (no rastreamos input
// pendiente del chrome).
(function() {
    if (globalThis.scheduler != null) return;

    var PRIORITIES = ['user-blocking', 'user-visible', 'background'];
    function normPriority(p) { return PRIORITIES.indexOf(p) >= 0 ? p : 'user-visible'; }

    // ---------- TaskPriorityChangeEvent ----------
    function TaskPriorityChangeEvent(type, init) {
        init = init || {};
        var ev = (typeof globalThis.Event === 'function') ? new globalThis.Event(type, init) : { type: type };
        ev.previousPriority = init.previousPriority != null ? init.previousPriority : 'user-visible';
        return ev;
    }
    globalThis.TaskPriorityChangeEvent = TaskPriorityChangeEvent;

    // ---------- cola por prioridad ----------
    var queues = { 'user-blocking': [], 'user-visible': [], 'background': [] };
    var flushScheduled = false;
    function scheduleFlush() {
        if (flushScheduled) return;
        flushScheduled = true;
        globalThis.queueMicrotask(flush);
    }
    function flush() {
        flushScheduled = false;
        for (;;) {
            var task = null;
            for (var i = 0; i < PRIORITIES.length; i++) {
                var q = queues[PRIORITIES[i]];
                if (q.length) { task = q.shift(); break; }
            }
            if (!task) break;
            task.run();
        }
    }
    function enqueue(task) { queues[task.priority].push(task); scheduleFlush(); }

    // ---------- TaskSignal (AbortSignal + priority + prioritychange) ----------
    function makeTaskSignal(priority) {
        var pair = globalThis.__puriy_make_abort_signal();
        var sig = pair.signal;
        sig._priority = normPriority(priority);
        sig.onprioritychange = null;
        var prioListeners = [];
        Object.defineProperty(sig, 'priority', {
            get: function() { return this._priority; }, configurable: true });
        var origAdd = sig.addEventListener;
        sig.addEventListener = function(type, fn) {
            if (type === 'prioritychange') { prioListeners.push(fn); return; }
            origAdd.call(this, type, fn);
        };
        var origRemove = sig.removeEventListener;
        sig.removeEventListener = function(type, fn) {
            if (type === 'prioritychange') {
                var i = prioListeners.indexOf(fn); if (i >= 0) prioListeners.splice(i, 1); return;
            }
            origRemove.call(this, type, fn);
        };
        sig._emitPriorityChange = function(prev) {
            var ev = new TaskPriorityChangeEvent('prioritychange', { previousPriority: prev });
            ev.target = sig;
            if (typeof sig.onprioritychange === 'function') {
                try { sig.onprioritychange(ev); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
            }
            for (var i = 0; i < prioListeners.length; i++) {
                try { prioListeners[i](ev); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
            }
        };
        return { signal: sig, abort: pair.abort };
    }

    // ---------- TaskController ----------
    function TaskController(init) {
        init = init || {};
        var ts = makeTaskSignal(init.priority);
        this.signal = ts.signal;
        this._abort = ts.abort;
    }
    TaskController.prototype.abort = function(reason) { this._abort(reason); };
    TaskController.prototype.setPriority = function(priority) {
        priority = normPriority(priority);
        var sig = this.signal;
        if (sig._priority === priority) return;
        var prev = sig._priority;
        sig._priority = priority;
        sig._emitPriorityChange(prev);
    };
    globalThis.TaskController = TaskController;

    // ---------- scheduler ----------
    function abortReason(signal) {
        return signal.reason !== undefined ? signal.reason : new Error('AbortError');
    }
    var scheduler = {
        postTask: function(callback, options) {
            options = options || {};
            var signal = options.signal || null;
            var priority = options.priority != null ? normPriority(options.priority)
                         : (signal && signal.priority ? signal.priority : 'user-visible');
            var delay = options.delay > 0 ? options.delay : 0;
            return new Promise(function(resolve, reject) {
                if (signal && signal.aborted) { reject(abortReason(signal)); return; }
                var done = false;
                var task = {
                    priority: priority,
                    run: function() {
                        if (done) return; done = true;
                        if (signal && signal.aborted) { reject(abortReason(signal)); return; }
                        try { resolve(callback()); } catch (e) { reject(e); }
                    }
                };
                if (signal && typeof signal.addEventListener === 'function') {
                    signal.addEventListener('abort', function() {
                        if (done) return; done = true; reject(abortReason(signal));
                    });
                    // Reubicar la tarea encolada si cambia la prioridad del TaskSignal.
                    if (signal.priority) {
                        signal.addEventListener('prioritychange', function() {
                            if (done) return;
                            var oldQ = queues[task.priority], i = oldQ.indexOf(task);
                            task.priority = signal.priority;
                            if (i >= 0) { oldQ.splice(i, 1); enqueue(task); }
                        });
                    }
                }
                if (delay > 0) { globalThis.setTimeout(function() { if (!done) enqueue(task); }, delay); }
                else { enqueue(task); }
            });
        },
        // Cede el control: resuelve en una vuelta posterior de la cola.
        yield: function() {
            return new Promise(function(resolve) {
                enqueue({ priority: 'user-visible', run: function() { resolve(undefined); } });
            });
        },
        isInputPending: function() { return false; }
    };
    globalThis.scheduler = scheduler;
    void 0;
})();
"#;
