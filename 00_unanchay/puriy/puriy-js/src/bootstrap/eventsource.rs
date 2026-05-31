pub(crate) const EVENTSOURCE_BOOTSTRAP: &str = r#"
// Fase 7.79 — EventSource (Server-Sent Events). Hereda del EventTarget genérico
// (Fase 7.76). El stream real lo maneja el chrome (wiring nativo PENDIENTE): el
// constructor publica `kind: 'eventsource'` con la apertura, y el chrome
// reinyecta eventos YA PARSEADOS del wire-format SSE (campos data:/event:/id:/
// retry:) llamando `__puriy_es_dispatch(id, kind, a, b, c)`. Mantener el parseo
// del lado nativo deja este módulo delgado y testeable por el hook (igual que
// WebSocket). Campos separados por U+001D (GS), mismo canal que fetch/WebSocket.
//
// Distinción SSE vs WebSocket: los eventos con `event:` nombrado NO disparan
// `onmessage` — sólo addEventListener(nombre). `onmessage` es exclusivo del
// tipo por defecto 'message'. Eso cae solo del despacho por `event.type`.
globalThis.__puriy_es_next_id = 1;
globalThis.__puriy_es_registry = {};
globalThis.__puriy_es_GS = String.fromCharCode(0x1D);
globalThis.EventSource = function(url, init) {
    globalThis.EventTarget.call(this);
    var base = (globalThis.location && globalThis.location.href) || '';
    this.url = globalThis.__puriy_resolve_url(String(url), base);
    this.readyState = 0;   // CONNECTING
    this.withCredentials = !!(init && init.withCredentials);
    this.onopen = null; this.onmessage = null; this.onerror = null;
    var id = globalThis.__puriy_es_next_id++;
    this._id = id;
    globalThis.__puriy_es_registry[id] = this;
    var GS = globalThis.__puriy_es_GS;
    globalThis.__puriy_dirty.push({
        id: '__window__', kind: 'eventsource',
        value: String(id) + GS + 'open' + GS + this.url + GS + (this.withCredentials ? '1' : '0')
    });
};
globalThis.EventSource.CONNECTING = 0;
globalThis.EventSource.OPEN = 1;
globalThis.EventSource.CLOSED = 2;
globalThis.EventSource.prototype = Object.create(globalThis.EventTarget.prototype);
globalThis.EventSource.prototype.constructor = globalThis.EventSource;
globalThis.EventSource.prototype.CONNECTING = 0;
globalThis.EventSource.prototype.OPEN = 1;
globalThis.EventSource.prototype.CLOSED = 2;
// Despacha a `on<tipo>` (sólo open/message/error son estándar) + listeners.
globalThis.EventSource.prototype.__puriy_fire = function(event) {
    var on = this['on' + event.type];
    if (typeof on === 'function') {
        try { on.call(this, event); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    this.dispatchEvent(event);
};
globalThis.EventSource.prototype.close = function() {
    if (this.readyState === 2) return;
    this.readyState = 2;   // CLOSED — definitivo, no reconecta
    delete globalThis.__puriy_es_registry[this._id];
    globalThis.__puriy_dirty.push({
        id: '__window__', kind: 'eventsource',
        value: String(this._id) + globalThis.__puriy_es_GS + 'close'
    });
};
// Reinyección desde el chrome (wiring nativo pendiente). Tipos:
//   'open'    (sin args)
//   'message' a=event-type (default 'message'), b=data, c=lastEventId
//   'error'   (sin args) — el chrome decide reconexión; acá sólo volvemos a
//             CONNECTING salvo que ya estemos CLOSED.
globalThis.__puriy_es_dispatch = function(id, kind, a, b, c) {
    var es = globalThis.__puriy_es_registry[id];
    if (!es) return;
    if (kind === 'open') {
        es.readyState = 1;
        es.__puriy_fire(new globalThis.Event('open'));
    } else if (kind === 'message') {
        var type = (a === undefined || a === null || a === '') ? 'message' : String(a);
        es.__puriy_fire(new globalThis.MessageEvent(type, {
            data: (b === undefined) ? '' : b,
            origin: es.url,
            lastEventId: (c === undefined) ? '' : String(c)
        }));
    } else if (kind === 'error') {
        if (es.readyState !== 2) es.readyState = 0;   // reconectando
        es.__puriy_fire(new globalThis.Event('error'));
    }
};
"#;
