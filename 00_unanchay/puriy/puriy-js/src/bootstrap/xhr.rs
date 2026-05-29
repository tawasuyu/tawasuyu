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
    this.onload = null;
    this.onerror = null;
    this.onabort = null;
    this.ontimeout = null;
    this._method = 'GET';
    this._url = '';
    this._headers = [];
    this._response_headers = {};
    this._aborted = false;
    this._sent = false;
};
globalThis.XMLHttpRequest.UNSENT = 0;
globalThis.XMLHttpRequest.OPENED = 1;
globalThis.XMLHttpRequest.HEADERS_RECEIVED = 2;
globalThis.XMLHttpRequest.LOADING = 3;
globalThis.XMLHttpRequest.DONE = 4;
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
    if (typeof this.onreadystatechange === 'function') {
        try { this.onreadystatechange(); }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
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
    var has_body = body != null;
    var body_str = has_body ? String(body) : '';
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
    // Transición readyState 1→2: spec lo hace cuando llegan headers; acá
    // no hay streaming, así que disparamos al lanzar send() para que apps
    // que escuchan ese estado funcionen.
    this.readyState = 2;
    if (typeof this.onreadystatechange === 'function') {
        try { this.onreadystatechange(); }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
};
globalThis.XMLHttpRequest.prototype.abort = function() {
    if (this.readyState === 0 || this.readyState === 4) return;
    this._aborted = true;
    this.readyState = 4;
    this.status = 0;
    this.statusText = '';
    if (typeof this.onreadystatechange === 'function') {
        try { this.onreadystatechange(); }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    if (typeof this.onabort === 'function') {
        try { this.onabort(); }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
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
    this.readyState = 4;
    if (typeof this.onreadystatechange === 'function') {
        try { this.onreadystatechange(); }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    if (typeof this.onload === 'function') {
        try { this.onload(); }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
};
globalThis.XMLHttpRequest.prototype.__puriy_error = function(msg) {
    if (this._aborted) return;
    this.status = 0;
    this.statusText = '';
    this.readyState = 4;
    if (typeof this.onreadystatechange === 'function') {
        try { this.onreadystatechange(); }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    if (typeof this.onerror === 'function') {
        try { this.onerror(new Error(msg)); }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
};
"#;
