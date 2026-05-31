pub(crate) const REQUEST_BOOTSTRAP: &str = r#"
// Fase 7.56 — Request como constructor público (`new Request(input, init)`).
// `input` puede ser una URL string u otro Request (en cuyo caso se clona y
// el `init` pisa los campos que trae). `fetch()` acepta un Request como
// primer arg (ver fetch.rs). Body crudo en `_body` (string/Blob/USP/…),
// consumible una vez vía text()/json()/arrayBuffer().
globalThis.Request = function(input, init) {
    init = init || {};
    if (input instanceof globalThis.Request) {
        this.url = input.url;
        this.method = init.method ? String(init.method).toUpperCase() : input.method;
        this.headers = new globalThis.Headers(
            (init.headers != null) ? init.headers : input.headers);
        this._body = (init.body != null) ? init.body : input._body;
        this.credentials = init.credentials || input.credentials;
        this.mode = init.mode || input.mode;
        this.signal = (init.signal != null) ? init.signal : input.signal;
        // Fase 7.73 — campos de init pasados tal cual (el `init` pisa al input).
        this.cache = init.cache || input.cache;
        this.redirect = init.redirect || input.redirect;
        this.referrer = (init.referrer != null) ? String(init.referrer) : input.referrer;
        this.referrerPolicy = (init.referrerPolicy != null) ? String(init.referrerPolicy) : input.referrerPolicy;
        this.integrity = (init.integrity != null) ? String(init.integrity) : input.integrity;
        this.keepalive = (init.keepalive != null) ? !!init.keepalive : input.keepalive;
    } else {
        this.url = String(input);
        this.method = init.method ? String(init.method).toUpperCase() : 'GET';
        this.headers = new globalThis.Headers(init.headers);
        this._body = (init.body != null) ? init.body : null;
        this.credentials = init.credentials || 'same-origin';
        this.mode = init.mode || 'cors';
        this.signal = init.signal || null;
        // Fase 7.73 — defaults del spec para los campos de init.
        this.cache = init.cache || 'default';
        this.redirect = init.redirect || 'follow';
        this.referrer = (init.referrer != null) ? String(init.referrer) : 'about:client';
        this.referrerPolicy = (init.referrerPolicy != null) ? String(init.referrerPolicy) : '';
        this.integrity = (init.integrity != null) ? String(init.integrity) : '';
        this.keepalive = !!init.keepalive;
    }
    this.bodyUsed = false;
};
globalThis.Request.prototype.text = function() {
    if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
    this.bodyUsed = true;
    return Promise.resolve(globalThis.__puriy_body_to_string(this._body));
};
globalThis.Request.prototype.json = function() {
    if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
    this.bodyUsed = true;
    try { return Promise.resolve(JSON.parse(globalThis.__puriy_body_to_string(this._body))); }
    catch (e) { return Promise.reject(e); }
};
globalThis.Request.prototype.arrayBuffer = function() {
    if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
    this.bodyUsed = true;
    var s = globalThis.__puriy_body_to_string(this._body);
    var buf = new ArrayBuffer(s.length);
    var view = new Uint8Array(buf);
    for (var i = 0; i < s.length; i++) view[i] = s.charCodeAt(i) & 0xff;
    return Promise.resolve(buf);
};
globalThis.Request.prototype.bytes = function() {
    if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
    this.bodyUsed = true;
    var s = globalThis.__puriy_body_to_string(this._body);
    var view = new Uint8Array(s.length);
    for (var i = 0; i < s.length; i++) view[i] = s.charCodeAt(i) & 0xff;
    return Promise.resolve(view);
};
globalThis.Request.prototype.formData = function() {
    if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
    this.bodyUsed = true;
    try {
        var text = globalThis.__puriy_body_to_string(this._body);
        return Promise.resolve(
            globalThis.__puriy_parse_form_body(text, this.headers.get('content-type')));
    } catch (e) { return Promise.reject(e); }
};
globalThis.Request.prototype.clone = function() {
    if (this.bodyUsed) throw new TypeError('Request body is already used');
    return new globalThis.Request(this);
};
"#;
