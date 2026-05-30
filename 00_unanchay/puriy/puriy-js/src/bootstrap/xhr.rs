pub(crate) const XHR_BOOTSTRAP: &str = r#"
// Fase 7.38 — XMLHttpRequest sobre el pipeline fetch. Reusa el mismo
// canal de mutación `kind: 'fetch'` y los handlers `__puriy_fetch_resolve`/
// `__puriy_fetch_reject` — registrarse en `__puriy_xhr_pending[id]` los
// rutea al método del XHR en vez de un Promise.
//
// readyState semántica: 0 UNSENT, 1 OPENED, 2 HEADERS_RECEIVED, 3 LOADING,
// 4 DONE. Acá saltamos directo de 2 a 4 al completar — no hay streaming.
// Sólo async=true soportado; abrir con async=false tira porque el chrome
// no expone HTTP síncrono al JS (estaría bloqueando el UI thread y
// nuestro fetch_full no se llama desde el JS thread).
globalThis.XMLHttpRequest = function() {
    this.readyState = 0;
    this.status = 0;
    this.statusText = '';
    this.responseText = '';
    this.response = '';
    this.responseType = '';
    this.responseURL = '';
    this.onreadystatechange = null;
    this.onloadstart = null;
    this.onprogress = null;
    this.onload = null;
    this.onerror = null;
    this.onabort = null;
    this.onloadend = null;
    this.ontimeout = null;
    this._method = 'GET';
    this._url = '';
    this._headers = [];
    this._response_headers = {};
    // Fase 7.48 — listeners por tipo de evento (addEventListener).
    this._listeners = {};
    this._aborted = false;
    this._sent = false;
};
globalThis.XMLHttpRequest.UNSENT = 0;
globalThis.XMLHttpRequest.OPENED = 1;
globalThis.XMLHttpRequest.HEADERS_RECEIVED = 2;
globalThis.XMLHttpRequest.LOADING = 3;
globalThis.XMLHttpRequest.DONE = 4;
// Fase 7.48 — EventTarget mínimo. `addEventListener`/`removeEventListener`
// conviven con los handlers `on<tipo>`; `__puriy_fire` dispara ambos con un
// objeto-evento `{ type, target, lengthComputable, loaded, total }`.
globalThis.XMLHttpRequest.prototype.addEventListener = function(type, fn) {
    if (typeof fn !== 'function') return;
    type = String(type);
    if (!this._listeners[type]) this._listeners[type] = [];
    this._listeners[type].push(fn);
};
globalThis.XMLHttpRequest.prototype.removeEventListener = function(type, fn) {
    type = String(type);
    var arr = this._listeners[type];
    if (!arr) return;
    for (var i = arr.length - 1; i >= 0; i--) {
        if (arr[i] === fn) arr.splice(i, 1);
    }
};
globalThis.XMLHttpRequest.prototype.__puriy_fire = function(type, evt) {
    evt = evt || {};
    evt.type = type;
    evt.target = this;
    var on = this['on' + type];
    if (typeof on === 'function') {
        try { on.call(this, evt); }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    var arr = this._listeners[type];
    if (arr) {
        // Snapshot: un listener puede remover otro durante el dispatch.
        var snapshot = arr.slice();
        for (var i = 0; i < snapshot.length; i++) {
            try { snapshot[i].call(this, evt); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
    }
};
globalThis.XMLHttpRequest.prototype.open = function(method, url, async_) {
    if (async_ === false) {
        throw new Error('XMLHttpRequest síncrono no soportado');
    }
    this._method = String(method || 'GET').toUpperCase();
    this._url = String(url || '');
    this._headers = [];
    this._response_headers = {};
    this.readyState = 1;
    this.status = 0;
    this.statusText = '';
    this.responseText = '';
    this.response = '';
    this._sent = false;
    this._aborted = false;
    this.__puriy_fire('readystatechange');
};
globalThis.XMLHttpRequest.prototype.setRequestHeader = function(name, value) {
    this._headers.push(String(name));
    this._headers.push(String(value));
};
globalThis.XMLHttpRequest.prototype.send = function(body) {
    if (this.readyState !== 1) {
        throw new Error('InvalidStateError: open() no llamado');
    }
    if (this._sent) {
        throw new Error('InvalidStateError: send() ya llamado');
    }
    this._sent = true;
    var id = globalThis.__puriy_fetch_next_id++;
    globalThis.__puriy_xhr_pending[id] = this;
    // Fase 7.57 — serializa el body igual que fetch() (FormData → multipart,
    // URLSearchParams/Blob → su Content-Type implícito) y agrega el
    // Content-Type a `_headers` si el user no llamó setRequestHeader.
    var ser = globalThis.__puriy_serialize_body(body);
    var has_body = ser.hasBody;
    var body_str = ser.text;
    globalThis.__puriy_apply_content_type(this._headers, ser.contentType);
    var base = (globalThis.location && globalThis.location.href) || '';
    var resolved = globalThis.__puriy_resolve_url(this._url, base);
    this.responseURL = resolved;
    // Payload: mismo formato que fetch() (Fase 7.34).
    var payload = String(id) + '' + this._method + '' + resolved
                + '' + (has_body ? '1' : '0')
                + '' + body_str
                + '' + this._headers.join('');
    globalThis.__puriy_dirty.push({
        id: '__window__',
        kind: 'fetch',
        value: payload
    });
    // Fase 7.48 — `loadstart` apenas se lanza la petición.
    this.__puriy_fire('loadstart', { lengthComputable: false, loaded: 0, total: 0 });
    // Transición readyState 1→2: spec lo hace cuando llegan headers; acá
    // no hay streaming, así que disparamos al lanzar send() para que apps
    // que escuchan ese estado funcionen.
    this.readyState = 2;
    this.__puriy_fire('readystatechange');
};
globalThis.XMLHttpRequest.prototype.abort = function() {
    if (this.readyState === 0 || this.readyState === 4) return;
    this._aborted = true;
    this.readyState = 4;
    this.status = 0;
    this.statusText = '';
    this.__puriy_fire('readystatechange');
    this.__puriy_fire('abort', { lengthComputable: false, loaded: 0, total: 0 });
    this.__puriy_fire('loadend', { lengthComputable: false, loaded: 0, total: 0 });
};
globalThis.XMLHttpRequest.prototype.getResponseHeader = function(name) {
    var k = String(name).toLowerCase();
    return Object.prototype.hasOwnProperty.call(this._response_headers, k)
         ? this._response_headers[k] : null;
};
globalThis.XMLHttpRequest.prototype.getAllResponseHeaders = function() {
    var out = '';
    for (var k in this._response_headers) {
        if (Object.prototype.hasOwnProperty.call(this._response_headers, k)) {
            out += k + ': ' + this._response_headers[k] + '\r\n';
        }
    }
    return out;
};
// Helpers llamados desde __puriy_fetch_resolve/reject.
globalThis.XMLHttpRequest.prototype.__puriy_complete = function(status, statusText, body, hdrPairs) {
    if (this._aborted) return;
    this.status = status;
    this.statusText = statusText;
    // `responseText` sólo es válido para responseType '' o 'text' (spec lo
    // hace tirar para otros tipos); acá lo dejamos poblado siempre por
    // lenidad — divergencia documentada. `response` sí respeta el tipo.
    this.responseText = body;
    if (hdrPairs) {
        for (var i = 0; i + 1 < hdrPairs.length; i += 2) {
            this._response_headers[String(hdrPairs[i]).toLowerCase()] = hdrPairs[i + 1];
        }
    }
    // Fase 7.47 — `response` según responseType.
    var rtype = this.responseType || '';
    if (rtype === '' || rtype === 'text') {
        this.response = body;
    } else if (rtype === 'json') {
        // Spec: JSON inválido → response null (no tira).
        try { this.response = JSON.parse(body); } catch (e) { this.response = null; }
    } else if (rtype === 'arraybuffer') {
        var len = body.length;
        var buf = new ArrayBuffer(len);
        var view = new Uint8Array(buf);
        for (var i = 0; i < len; i++) view[i] = body.charCodeAt(i) & 0xff;
        this.response = buf;
    } else if (rtype === 'blob') {
        this.response = new globalThis.Blob([body], { type: this.getResponseHeader('content-type') || '' });
    } else {
        // 'document' no soportado (no parseamos el response a DOM) → null.
        this.response = null;
    }
    // Fase 7.48 — el body llega entero (sin transferencia chunked), así que
    // emitimos un único `progress` con loaded == total antes de pasar a DONE.
    var total = body ? body.length : 0;
    this.__puriy_fire('progress', { lengthComputable: true, loaded: total, total: total });
    this.readyState = 4;
    this.__puriy_fire('readystatechange');
    this.__puriy_fire('load', { lengthComputable: true, loaded: total, total: total });
    this.__puriy_fire('loadend', { lengthComputable: true, loaded: total, total: total });
};
globalThis.XMLHttpRequest.prototype.__puriy_error = function(msg) {
    if (this._aborted) return;
    this.status = 0;
    this.statusText = '';
    this.readyState = 4;
    this.__puriy_fire('readystatechange');
    // El handler `onerror` históricamente recibía un Error; lo preservamos
    // adjuntándolo al evento (`evt` también lleva type/target/loaded/total).
    var evt = { lengthComputable: false, loaded: 0, total: 0, error: new Error(msg) };
    this.__puriy_fire('error', evt);
    this.__puriy_fire('loadend', { lengthComputable: false, loaded: 0, total: 0 });
};
"#;
