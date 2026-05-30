pub(crate) const WEBSOCKET_BOOTSTRAP: &str = r#"
// Fase 7.78 — WebSocket. Hereda del EventTarget genérico (Fase 7.76) vía cadena
// de prototipos real, así `addEventListener`/`dispatchEvent` vienen gratis y el
// `instanceof EventTarget` se cumple. El transporte real es del chrome (wiring
// nativo PENDIENTE): el constructor publica una mutación `kind: 'websocket'`
// con la apertura, `send`/`close` publican frames, y el chrome reinyecta los
// eventos del socket llamando `__puriy_ws_dispatch(id, kind, a, b, c)`. Los
// tests ejercitan ese hook directamente (igual que fetch/XHR resuelven por id).
//
// Formato de payload: campos separados por U+001D (Group Separator), mismo
// canal y convención que fetch()/sendBeacon. Apertura: id, 'open', url, protos.
globalThis.__puriy_ws_next_id = 1;
globalThis.__puriy_ws_registry = {};
globalThis.WebSocket = function(url, protocols) {
    globalThis.EventTarget.call(this);
    var base = (globalThis.location && globalThis.location.href) || '';
    this.url = globalThis.__puriy_resolve_url(String(url), base);
    this.readyState = 0;       // CONNECTING
    this.bufferedAmount = 0;
    this.extensions = '';
    this.protocol = '';
    this.binaryType = 'blob';
    this.onopen = null; this.onmessage = null; this.onerror = null; this.onclose = null;
    var protoList = '';
    if (protocols != null) {
        protoList = Array.isArray(protocols) ? protocols.join(',') : String(protocols);
    }
    var id = globalThis.__puriy_ws_next_id++;
    this._id = id;
    globalThis.__puriy_ws_registry[id] = this;
    globalThis.__puriy_dirty.push({
        id: '__window__', kind: 'websocket',
        value: String(id) + '' + 'open' + '' + this.url + '' + protoList
    });
};
globalThis.WebSocket.CONNECTING = 0;
globalThis.WebSocket.OPEN = 1;
globalThis.WebSocket.CLOSING = 2;
globalThis.WebSocket.CLOSED = 3;
globalThis.WebSocket.prototype = Object.create(globalThis.EventTarget.prototype);
globalThis.WebSocket.prototype.constructor = globalThis.WebSocket;
globalThis.WebSocket.prototype.CONNECTING = 0;
globalThis.WebSocket.prototype.OPEN = 1;
globalThis.WebSocket.prototype.CLOSING = 2;
globalThis.WebSocket.prototype.CLOSED = 3;
// Despacha a `on<tipo>` y a los addEventListener registrados (EventTarget).
globalThis.WebSocket.prototype.__puriy_fire = function(event) {
    var on = this['on' + event.type];
    if (typeof on === 'function') {
        try { on.call(this, event); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    this.dispatchEvent(event);
};
globalThis.WebSocket.prototype.send = function(data) {
    if (this.readyState === 0) {
        throw new Error('InvalidStateError: WebSocket aún CONNECTING');
    }
    if (this.readyState >= 2) return;   // CLOSING/CLOSED: el spec descarta en silencio
    var isBinary = false, text;
    if (typeof data === 'string') {
        text = data;
    } else if (data instanceof ArrayBuffer || ArrayBuffer.isView(data)) {
        isBinary = true;
        var bytes = (data instanceof ArrayBuffer) ? new Uint8Array(data)
                  : new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
        var s = '';
        for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
        text = s;
    } else if (globalThis.Blob && data instanceof globalThis.Blob) {
        isBinary = true;
        var b = data._bytes || new Uint8Array(0);
        var sb = '';
        for (var j = 0; j < b.length; j++) sb += String.fromCharCode(b[j]);
        text = sb;
    } else {
        text = String(data);
    }
    this.bufferedAmount += text.length;
    globalThis.__puriy_dirty.push({
        id: '__window__', kind: 'websocket',
        value: String(this._id) + '' + 'send' + '' + (isBinary ? '1' : '0') + '' + text
    });
};
globalThis.WebSocket.prototype.close = function(code, reason) {
    if (code !== undefined && code !== 1000 && !(code >= 3000 && code <= 4999)) {
        throw new Error('InvalidAccessError: código de cierre WebSocket inválido');
    }
    if (this.readyState === 2 || this.readyState === 3) return;
    this.readyState = 2;   // CLOSING (el chrome confirma con __puriy_ws_dispatch 'close')
    var c = (code === undefined) ? 1000 : (code | 0);
    var r = (reason === undefined) ? '' : String(reason);
    globalThis.__puriy_dirty.push({
        id: '__window__', kind: 'websocket',
        value: String(this._id) + '' + 'close' + '' + String(c) + '' + r
    });
};
// Reinyección desde el chrome (wiring nativo pendiente). Tipos:
//   'open'    a=protocol, b=extensions
//   'message' a=data (string o binary-string), b='1' si binario
//   'error'   (sin args)
//   'close'   a=code, b=reason, c=wasClean ('1'|true)
globalThis.__puriy_ws_dispatch = function(id, kind, a, b, c) {
    var ws = globalThis.__puriy_ws_registry[id];
    if (!ws) return;
    if (kind === 'open') {
        ws.readyState = 1;
        ws.protocol = a || '';
        ws.extensions = b || '';
        ws.__puriy_fire(new globalThis.Event('open'));
    } else if (kind === 'message') {
        var data = a;
        if (b === '1' || b === 1 || b === true) {
            // binary-string → según binaryType. 'arraybuffer' devuelve ArrayBuffer.
            if (ws.binaryType === 'arraybuffer') {
                var buf = new ArrayBuffer(String(a).length);
                var view = new Uint8Array(buf);
                for (var i = 0; i < view.length; i++) view[i] = String(a).charCodeAt(i) & 0xff;
                data = buf;
            } else {
                data = new globalThis.Blob([String(a)]);
            }
        }
        ws.__puriy_fire(new globalThis.MessageEvent('message', { data: data, origin: ws.url }));
    } else if (kind === 'error') {
        ws.__puriy_fire(new globalThis.Event('error'));
    } else if (kind === 'close') {
        ws.readyState = 3;
        delete globalThis.__puriy_ws_registry[id];
        ws.__puriy_fire(new globalThis.CloseEvent('close', {
            code: (a === undefined) ? 1006 : (a | 0),
            reason: b || '',
            wasClean: (c === '1' || c === 1 || c === true)
        }));
    }
};
"#;
