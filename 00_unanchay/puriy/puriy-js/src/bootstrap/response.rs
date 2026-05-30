pub(crate) const RESPONSE_BOOTSTRAP: &str = r#"
// Fase 7.55 — Response como constructor público (`new Response(body, init)`).
// Unifica con `__puriy_make_response` (refactorizado en fetch.rs para
// delegar acá): antes el factory armaba un objeto plano con los métodos
// inline; ahora todo cuelga del prototype y la respuesta de red es un
// `new Response(...)` con `type = 'basic'`.
//
// Body crudo en `_body` (string de bytes). `__puriy_body_to_string`
// normaliza string / ArrayBuffer / TypedArray / Blob / otros a esa forma.
// `bodyUsed` se respeta entre text()/json()/arrayBuffer()/blob() y el
// getter `body` (ReadableStream lazy, un solo chunk).
globalThis.__puriy_body_to_string = function(body) {
    if (body == null) return '';
    if (typeof body === 'string') return body;
    if (body instanceof ArrayBuffer) {
        var v = new Uint8Array(body); var s = '';
        for (var i = 0; i < v.length; i++) s += String.fromCharCode(v[i]);
        return s;
    }
    if (body instanceof globalThis.Blob) {
        var s2 = '';
        for (var j = 0; j < body._bytes.length; j++) s2 += String.fromCharCode(body._bytes[j]);
        return s2;
    }
    if (body && body.buffer instanceof ArrayBuffer && typeof body.length === 'number') {
        var s3 = '';
        for (var k = 0; k < body.length; k++) s3 += String.fromCharCode(body[k] & 0xff);
        return s3;
    }
    return String(body);
};
globalThis.Response = function(body, init) {
    init = init || {};
    this.status = (init.status != null) ? init.status : 200;
    this.statusText = (init.statusText != null) ? String(init.statusText) : '';
    this.ok = this.status >= 200 && this.status < 300;
    this.url = '';
    this.type = 'default';
    this.bodyUsed = false;
    this.headers = (init.headers instanceof globalThis.Headers)
        ? init.headers : new globalThis.Headers(init.headers);
    this._body = globalThis.__puriy_body_to_string(body);
    this._bodyStream = null;
};
globalThis.Response.prototype.text = function() {
    if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
    this.bodyUsed = true;
    return Promise.resolve(this._body);
};
globalThis.Response.prototype.json = function() {
    if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
    this.bodyUsed = true;
    try { return Promise.resolve(JSON.parse(this._body)); }
    catch (e) { return Promise.reject(e); }
};
globalThis.Response.prototype.arrayBuffer = function() {
    if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
    this.bodyUsed = true;
    var len = this._body.length;
    var buf = new ArrayBuffer(len);
    var view = new Uint8Array(buf);
    for (var i = 0; i < len; i++) view[i] = this._body.charCodeAt(i) & 0xff;
    return Promise.resolve(buf);
};
globalThis.Response.prototype.blob = function() {
    if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
    this.bodyUsed = true;
    return Promise.resolve(new globalThis.Blob([this._body], {
        type: this.headers.get('content-type') || ''
    }));
};
globalThis.Response.prototype.clone = function() {
    if (this.bodyUsed) throw new TypeError('Response body is already used');
    var r = new globalThis.Response(this._body, {
        status: this.status, statusText: this.statusText,
        headers: new globalThis.Headers(this.headers)
    });
    r.url = this.url; r.type = this.type;
    return r;
};
Object.defineProperty(globalThis.Response.prototype, 'body', {
    get: function() {
        if (this._bodyStream) return this._bodyStream;
        var self = this;
        var emitted = false;
        this._bodyStream = new globalThis.ReadableStream({
            pull: function(controller) {
                if (emitted || self.bodyUsed) { controller.close(); return; }
                emitted = true;
                self.bodyUsed = true;
                var len = self._body.length;
                var view = new Uint8Array(len);
                for (var i = 0; i < len; i++) view[i] = self._body.charCodeAt(i) & 0xff;
                controller.enqueue(view);
            }
        });
        return this._bodyStream;
    }
});
// Estáticos: Response.json(data, init) y Response.error().
globalThis.Response.json = function(data, init) {
    init = init || {};
    var headers = (init.headers instanceof globalThis.Headers)
        ? init.headers : new globalThis.Headers(init.headers);
    if (!headers.has('content-type')) headers.set('content-type', 'application/json');
    return new globalThis.Response(JSON.stringify(data), {
        status: init.status, statusText: init.statusText, headers: headers
    });
};
globalThis.Response.error = function() {
    var r = new globalThis.Response(null, { status: 0 });
    r.type = 'error';
    r.ok = false;
    return r;
};
// Fase 7.62 — Response.redirect(url, status). status default 302; sólo se
// aceptan los códigos de redirect del spec (301/302/303/307/308), si no
// tira RangeError. El header `Location` lleva la URL tal cual.
// Divergencia: el spec parsea/valida la URL (TypeError si es inválida) y la
// serializa absoluta; acá la guardamos cruda — suficiente para construir la
// respuesta, sin resolución contra base.
globalThis.Response.redirect = function(url, status) {
    status = (status == null) ? 302 : status;
    if (status !== 301 && status !== 302 && status !== 303 && status !== 307 && status !== 308) {
        throw new RangeError('Invalid status code for redirect: ' + status);
    }
    var r = new globalThis.Response(null, { status: status });
    r.headers.set('Location', String(url));
    return r;
};
"#;
