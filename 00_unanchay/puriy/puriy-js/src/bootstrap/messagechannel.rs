pub(crate) const MESSAGE_CHANNEL_BOOTSTRAP: &str = r#"
// Fase 7.81 — MessageChannel + MessagePort. Canal bidireccional de mensajes
// entre dos puertos enlazados. new MessageChannel() da {port1, port2}; lo que
// port1.postMessage() entrega dispara 'message' en port2 y viceversa. Cada
// puerto es un EventTarget (Fase 7.76). El spec arranca los puertos "sin
// shippear": no entregan hasta start() o hasta asignar onmessage (que llama
// start() implícito). Mensajes previos al start quedan encolados y se entregan
// al arrancar, en orden.
//
// Divergencia documentada: el spec entrega de forma ASÍNCRONA (encola una
// tarea en el otro puerto); acá entregamos SÍNCRONO dentro del postMessage /
// start, igual que el modelo de dispatch del resto del runtime (BroadcastChannel,
// EventTarget). Para código de un solo contexto el efecto observable es el
// mismo. Datos clonados con structuredClone (Fase 7.65); sin transferables
// reales (el segundo arg `transfer` se ignora).
globalThis.MessagePort = function() {
    globalThis.EventTarget.call(this);
    this._other = null;
    this._started = false;
    this._closed = false;
    this._queue = [];
    this._onmessage = null;
    this.onmessageerror = null;
    var port = this;
    // Asignar onmessage arranca el puerto (spec) — por eso es un accessor.
    Object.defineProperty(this, 'onmessage', {
        configurable: true,
        get: function() { return port._onmessage; },
        set: function(fn) { port._onmessage = fn; port.start(); }
    });
};
globalThis.MessagePort.prototype = Object.create(globalThis.EventTarget.prototype);
globalThis.MessagePort.prototype.constructor = globalThis.MessagePort;
globalThis.MessagePort.prototype.__puriy_fire = function(event) {
    if (typeof this._onmessage === 'function' && event.type === 'message') {
        try { this._onmessage.call(this, event); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    this.dispatchEvent(event);
};
// El otro puerto llama esto para entregarnos `data`. Si no arrancamos todavía,
// encola; si ya arrancamos, dispara 'message' de inmediato.
globalThis.MessagePort.prototype._deliver = function(data) {
    if (this._closed) return;
    if (!this._started) { this._queue.push(data); return; }
    this.__puriy_fire(new globalThis.MessageEvent('message', { data: data, origin: '' }));
};
globalThis.MessagePort.prototype.postMessage = function(message) {
    if (!this._other || this._other._closed) return;
    var cloned;
    try { cloned = globalThis.structuredClone(message); } catch (e) { cloned = message; }
    this._other._deliver(cloned);
};
globalThis.MessagePort.prototype.start = function() {
    if (this._started) return;
    this._started = true;
    var q = this._queue;
    this._queue = [];
    for (var i = 0; i < q.length; i++) {
        if (this._closed) break;
        this.__puriy_fire(new globalThis.MessageEvent('message', { data: q[i], origin: '' }));
    }
};
globalThis.MessagePort.prototype.close = function() {
    this._closed = true;
};
globalThis.MessageChannel = function() {
    var p1 = new globalThis.MessagePort();
    var p2 = new globalThis.MessagePort();
    p1._other = p2;
    p2._other = p1;
    this.port1 = p1;
    this.port2 = p2;
};
"#;
