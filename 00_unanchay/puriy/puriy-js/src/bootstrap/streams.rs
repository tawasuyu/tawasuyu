pub(crate) const STREAMS_BOOTSTRAP: &str = r#"
// Fase 7.45 — ReadableStream JS-puro. Cierra el ítem (1) de la lista de
// limitaciones de Fase 7.34: `response.body` ahora es un ReadableStream
// real con getReader()/read()/cancel()/tee() + async iteration.
//
// Diseño: como el body del fetch llega entero (no hay backpressure ni
// transferencia chunked desde el chrome todavía), el stream que respalda
// a `response.body` es de UN solo chunk (Uint8Array del body) seguido de
// {done:true}. Pero la clase soporta el caso general — un underlying
// source con start/pull/cancel puede encolar N chunks — así apps que
// construyen sus propios streams (`new ReadableStream({...})`) funcionan.
//
// Limitaciones documentadas:
//  (1) Sin backpressure real: highWaterMark/size de la queuing strategy se
//      ignoran; `desiredSize` se reporta heurístico (1 si abierto, 0 si
//      cerrado). pull() se llama eager hasta que el consumidor lee.
//  (2) BYOB readers (getReader({mode:'byob'})) no soportados — sólo el
//      default reader.
//  (3) pipeTo()/pipeThrough() no implementados (no hay WritableStream).
//  (4) tee() bufferea: el segundo branch guarda todos los chunks ya leídos
//      por el primero (no hay límite de memoria — razonable para bodies
//      acotados de fetch).

// Cola interna de un stream + maquinaria de pull/close/error compartida
// entre el constructor y el reader. Devuelve un objeto con métodos que el
// ReadableStream y el ReadableStreamDefaultReader comparten por closure.
globalThis.__puriy_make_stream_internals = function(underlyingSource) {
    var queue = [];            // chunks encolados pendientes de leer
    var closed = false;        // close() llamado y queue drenada
    var closeRequested = false;// close() llamado (puede quedar queue)
    var errored = false;
    var storedError = undefined;
    var pullScheduled = false;
    var startCalled = false;
    // Promises de read() esperando un chunk (cuando la queue está vacía).
    var readRequests = [];

    function resolveRead(req) {
        if (queue.length > 0) {
            req.resolve({ value: queue.shift(), done: false });
            return true;
        }
        if (closeRequested) {
            closed = true;
            req.resolve({ value: undefined, done: true });
            return true;
        }
        if (errored) {
            req.reject(storedError);
            return true;
        }
        return false;
    }

    function flushReadRequests() {
        while (readRequests.length > 0) {
            var req = readRequests[0];
            if (resolveRead(req)) {
                readRequests.shift();
            } else {
                break;
            }
        }
    }

    var controller = {
        enqueue: function(chunk) {
            if (closeRequested || errored) {
                throw new TypeError('Cannot enqueue on a closed or errored stream');
            }
            queue.push(chunk);
            flushReadRequests();
        },
        close: function() {
            closeRequested = true;
            flushReadRequests();
        },
        error: function(e) {
            errored = true;
            storedError = e;
            // Rechazar todos los reads pendientes.
            while (readRequests.length > 0) {
                readRequests.shift().reject(e);
            }
        },
        get desiredSize() {
            if (errored) return null;
            if (closeRequested) return 0;
            return 1;
        }
    };

    function maybePull() {
        if (pullScheduled || errored || closeRequested) return;
        if (!underlyingSource || typeof underlyingSource.pull !== 'function') return;
        // Sólo pull si hay un lector esperando o la queue está vacía.
        pullScheduled = true;
        Promise.resolve().then(function() {
            pullScheduled = false;
            if (errored || closeRequested) return;
            try {
                var r = underlyingSource.pull(controller);
                if (r && typeof r.then === 'function') {
                    r.catch(function(e) { controller.error(e); });
                }
            } catch (e) {
                controller.error(e);
            }
        });
    }

    function ensureStarted() {
        if (startCalled) return;
        startCalled = true;
        if (underlyingSource && typeof underlyingSource.start === 'function') {
            try {
                var r = underlyingSource.start(controller);
                if (r && typeof r.then === 'function') {
                    r.catch(function(e) { controller.error(e); });
                }
            } catch (e) {
                controller.error(e);
            }
        }
    }

    return {
        controller: controller,
        ensureStarted: ensureStarted,
        // Llamado por reader.read() — devuelve un Promise<{value,done}>.
        pullRead: function() {
            ensureStarted();
            return new Promise(function(resolve, reject) {
                var req = { resolve: resolve, reject: reject };
                if (!resolveRead(req)) {
                    readRequests.push(req);
                    maybePull();
                }
            });
        },
        cancel: function(reason) {
            if (errored) return Promise.reject(storedError);
            queue = [];
            closeRequested = true;
            closed = true;
            // Resolver cualquier read pendiente con done.
            while (readRequests.length > 0) {
                readRequests.shift().resolve({ value: undefined, done: true });
            }
            if (underlyingSource && typeof underlyingSource.cancel === 'function') {
                try {
                    var r = underlyingSource.cancel(reason);
                    if (r && typeof r.then === 'function') return r;
                } catch (e) {
                    return Promise.reject(e);
                }
            }
            return Promise.resolve(undefined);
        },
        isErrored: function() { return errored; },
        getError: function() { return storedError; },
        isClosed: function() { return closed; }
    };
};

globalThis.ReadableStream = function(underlyingSource, strategy) {
    var internals = globalThis.__puriy_make_stream_internals(underlyingSource || {});
    this.__internals = internals;
    this.__locked = false;
    // El spec corre start() al construir.
    internals.ensureStarted();
};
Object.defineProperty(globalThis.ReadableStream.prototype, 'locked', {
    get: function() { return this.__locked; }
});
globalThis.ReadableStream.prototype.getReader = function(opts) {
    if (opts && opts.mode === 'byob') {
        throw new TypeError('BYOB readers no soportados en puriy');
    }
    if (this.__locked) {
        throw new TypeError('ReadableStream is already locked to a reader');
    }
    this.__locked = true;
    var stream = this;
    var internals = this.__internals;
    var released = false;
    return {
        read: function() {
            if (released) {
                return Promise.reject(new TypeError('Reader has been released'));
            }
            return internals.pullRead();
        },
        releaseLock: function() {
            released = true;
            stream.__locked = false;
        },
        cancel: function(reason) {
            if (released) {
                return Promise.reject(new TypeError('Reader has been released'));
            }
            return internals.cancel(reason);
        },
        get closed() {
            // Promise que resuelve cuando el stream cierra (simplificado:
            // resuelve ya si está cerrado/errored, sino queda pendiente sin
            // tracking fino — apps que await reader.closed tras drenar todo
            // funcionan porque drenar marca closed).
            if (internals.isErrored()) return Promise.reject(internals.getError());
            if (internals.isClosed()) return Promise.resolve(undefined);
            return new Promise(function() {});
        }
    };
};
globalThis.ReadableStream.prototype.cancel = function(reason) {
    if (this.__locked) {
        return Promise.reject(new TypeError('Cannot cancel a locked stream'));
    }
    return this.__internals.cancel(reason);
};
// tee() — dos branches que reciben los mismos chunks. Implementación
// simple: un reader del stream original alimenta dos colas independientes.
globalThis.ReadableStream.prototype.tee = function() {
    if (this.__locked) {
        throw new TypeError('Cannot tee a locked stream');
    }
    var reader = this.getReader();
    var c1, c2;
    var pumping = false;
    function pump() {
        if (pumping) return;
        pumping = true;
        reader.read().then(function(res) {
            pumping = false;
            if (res.done) {
                if (c1) c1.close();
                if (c2) c2.close();
                return;
            }
            if (c1) c1.enqueue(res.value);
            if (c2) c2.enqueue(res.value);
        }).catch(function(e) {
            pumping = false;
            if (c1) c1.error(e);
            if (c2) c2.error(e);
        });
    }
    var branch1 = new globalThis.ReadableStream({
        start: function(controller) { c1 = controller; },
        pull: function() { pump(); }
    });
    var branch2 = new globalThis.ReadableStream({
        start: function(controller) { c2 = controller; },
        pull: function() { pump(); }
    });
    return [branch1, branch2];
};
// Async iteration: `for await (const chunk of stream) { ... }`.
globalThis.ReadableStream.prototype[Symbol.asyncIterator] = function() {
    var reader = this.getReader();
    return {
        next: function() {
            return reader.read().then(function(res) {
                if (res.done) reader.releaseLock();
                return res;
            });
        },
        return: function(value) {
            reader.cancel(value);
            reader.releaseLock();
            return Promise.resolve({ value: value, done: true });
        },
        [Symbol.asyncIterator]: function() { return this; }
    };
};
// Helper que el Response usa para envolver un body string en un stream de
// un solo chunk (Uint8Array de los bytes). Lazy: sólo construye el stream
// la primera vez que se accede a `response.body`.
globalThis.__puriy_body_to_stream = function(bodyStr) {
    var emitted = false;
    return new globalThis.ReadableStream({
        pull: function(controller) {
            if (emitted) {
                controller.close();
                return;
            }
            emitted = true;
            var len = bodyStr.length;
            var view = new Uint8Array(len);
            for (var i = 0; i < len; i++) view[i] = bodyStr.charCodeAt(i) & 0xff;
            controller.enqueue(view);
        }
    });
};
"#;
