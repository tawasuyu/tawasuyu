pub(crate) const BROADCAST_CHANNEL_BOOTSTRAP: &str = r#"
// Fase 7.80 — BroadcastChannel. Mensajería entre contextos del MISMO origen.
// En puriy hay un solo runtime JS por pestaña, así que el "bus" es local al
// runtime: instancias con el mismo `name` forman un grupo; postMessage entrega
// un MessageEvent('message') a TODAS las OTRAS instancias del grupo (nunca al
// emisor — spec). Hereda de EventTarget (Fase 7.76). Los datos se clonan con
// structuredClone (Fase 7.65), como exige el structured clone del spec.
//
// Divergencia documentada: el spec entrega de forma ASÍNCRONA (encola una
// tarea); acá entregamos SÍNCRONO dentro del postMessage, igual que el modelo
// de dispatch del resto del runtime. Para el caso de un solo contexto el
// efecto observable es el mismo (el handler corre antes de devolver control al
// código que llamó postMessage, en vez de en un microtask posterior).
globalThis.__puriy_bc_registry = {};   // name -> [instancias vivas]
globalThis.BroadcastChannel = function(name) {
    if (arguments.length < 1) throw new TypeError('BroadcastChannel requiere un name');
    globalThis.EventTarget.call(this);
    this.name = String(name);
    this.onmessage = null;
    this.onmessageerror = null;
    this._closed = false;
    var reg = globalThis.__puriy_bc_registry;
    if (!reg[this.name]) reg[this.name] = [];
    reg[this.name].push(this);
};
globalThis.BroadcastChannel.prototype = Object.create(globalThis.EventTarget.prototype);
globalThis.BroadcastChannel.prototype.constructor = globalThis.BroadcastChannel;
// Corre el handler `on<tipo>` (message/messageerror) + los addEventListener.
globalThis.BroadcastChannel.prototype.__puriy_fire = function(event) {
    var on = this['on' + event.type];
    if (typeof on === 'function') {
        try { on.call(this, event); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    this.dispatchEvent(event);
};
globalThis.BroadcastChannel.prototype.postMessage = function(message) {
    if (this._closed) {
        throw new globalThis.DOMException('BroadcastChannel cerrado', 'InvalidStateError');
    }
    var peers = globalThis.__puriy_bc_registry[this.name] || [];
    var self = this;
    // snapshot: un peer podría cerrarse durante el reparto.
    var snapshot = peers.slice();
    for (var i = 0; i < snapshot.length; i++) {
        var peer = snapshot[i];
        if (peer === self || peer._closed) continue;
        // Cada peer recibe su propia copia independiente (sin aliasing).
        var copy;
        try {
            copy = globalThis.structuredClone(message);
        } catch (e) {
            // DataCloneError → messageerror en el peer (spec), no message.
            peer.__puriy_fire(new globalThis.MessageEvent('messageerror', { origin: '' }));
            continue;
        }
        peer.__puriy_fire(new globalThis.MessageEvent('message', { data: copy, origin: '' }));
    }
};
globalThis.BroadcastChannel.prototype.close = function() {
    if (this._closed) return;
    this._closed = true;
    var arr = globalThis.__puriy_bc_registry[this.name];
    if (arr) {
        var idx = arr.indexOf(this);
        if (idx >= 0) arr.splice(idx, 1);
        if (arr.length === 0) delete globalThis.__puriy_bc_registry[this.name];
    }
};
"#;
