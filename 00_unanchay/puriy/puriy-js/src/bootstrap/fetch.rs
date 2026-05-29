pub(crate) const FETCH_BOOTSTRAP: &str = r#"
// Fase 7.31 — fetch() async. Crea un Promise, registra los handlers
// resolve/reject en `__puriy_fetch_pending[id]`, publica una mutación
// `kind: 'fetch:<id>'` con el payload serializado, y devuelve el
// Promise. Cuando el chrome termina el HTTP request, llama
// `JsRuntime::resolve_fetch(id, response)` o `reject_fetch(id, msg)`
// que disparan los `.then()`/`.catch()` del JS user code.
globalThis.__puriy_fetch_next_id = 1;
globalThis.__puriy_fetch_pending = {};
globalThis.fetch = function(url, init) {
    var id = globalThis.__puriy_fetch_next_id++;
    var method = (init && init.method) ? String(init.method).toUpperCase() : 'GET';
    var body = (init && init.body != null) ? String(init.body) : '';
    var has_body = (init && init.body != null);
    // Headers serializados como "namevaluename2value2..."
    // — U+001F como Unit Separator (mismo sep que usa drain_dirty).
    var hdr_pairs = [];
    if (init && init.headers) {
        if (init.headers instanceof globalThis.Headers) {
            // Fase 7.33 — si es Headers class, iterar internal store.
            for (var hk in init.headers._store) {
                hdr_pairs.push(hk);
                hdr_pairs.push(init.headers._store[hk]);
            }
        } else if (typeof init.headers === 'object') {
            for (var k in init.headers) {
                if (Object.prototype.hasOwnProperty.call(init.headers, k)) {
                    hdr_pairs.push(k);
                    hdr_pairs.push(String(init.headers[k]));
                }
            }
        }
    }
    // Fase 7.34 — signal: si ya está aborted, reject inmediato.
    if (init && init.signal && init.signal.aborted) {
        return Promise.reject(new Error('AbortError: fetch aborted'));
    }
    var promise = new Promise(function(resolve, reject) {
        globalThis.__puriy_fetch_pending[id] = {resolve: resolve, reject: reject};
    });
    // Si hay signal, registrar callback que llama reject_fetch al abortar.
    if (init && init.signal) {
        init.signal.addEventListener('abort', function() {
            if (globalThis.__puriy_fetch_pending[id]) {
                globalThis.__puriy_fetch_pending[id].reject(new Error('AbortError: fetch aborted'));
                delete globalThis.__puriy_fetch_pending[id];
            }
        });
    }
    // Payload: campos separados por U+001D (Group Separator).
    // [0] id, [1] method, [2] url, [3] body, [4] headers...
    // Fase 7.37 — resolver URL relativa contra location.href.
    var base = (globalThis.location && globalThis.location.href) || '';
    var resolved = globalThis.__puriy_resolve_url(url, base);
    var payload = String(id) + '' + method + '' + resolved
                + '' + (has_body ? '1' : '0')
                + '' + body
                + '' + hdr_pairs.join('');
    globalThis.__puriy_dirty.push({
        id: '__window__',
        kind: 'fetch',
        value: payload
    });
    return promise;
};
// Fase 7.31 — Response class JS-puro. Construido por
// `__puriy_fetch_resolve(id, status, statusText, body, headers)` que el
// chrome llama cuando el HTTP termina. Methods: .text() y .json()
// devuelven Promises (matchea spec). .headers expone una Headers-like
// (Fase 7.33). Múltiples llamadas a .text()/.json() funcionan porque
// guardamos _body string crudo (sin "body locked" todavía).
globalThis.__puriy_make_response = function(status, statusText, body, hdrPairs) {
    var headers = new globalThis.Headers();
    if (hdrPairs) {
        for (var i = 0; i + 1 < hdrPairs.length; i += 2) {
            headers.set(hdrPairs[i], hdrPairs[i + 1]);
        }
    }
    return {
        status: status,
        statusText: statusText,
        ok: status >= 200 && status < 300,
        url: '',
        type: 'basic',
        bodyUsed: false,
        headers: headers,
        _body: body,
        // Fase 7.35 — bodyUsed enforcement. Spec: tras consumir el body
        // (.text()/.json()/.arrayBuffer()), bodyUsed pasa a true y un
        // segundo intento rechaza con TypeError("body stream already read").
        // Aplica también si el primer intento falló (JSON.parse roto).
        text: function() {
            if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
            this.bodyUsed = true;
            return Promise.resolve(this._body);
        },
        json: function() {
            if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
            this.bodyUsed = true;
            try { return Promise.resolve(JSON.parse(this._body)); }
            catch (e) { return Promise.reject(e); }
        },
        arrayBuffer: function() {
            // Stub: devuelve un objeto pseudo-arrayBuffer con _body bytes
            // mapeados a un Array de char codes. Apps que necesitan
            // ArrayBuffer real verán divergencia; suficiente para JSON.
            if (this.bodyUsed) return Promise.reject(new TypeError('body stream already read'));
            this.bodyUsed = true;
            var len = this._body.length;
            var buf = new ArrayBuffer(len);
            var view = new Uint8Array(buf);
            for (var i = 0; i < len; i++) view[i] = this._body.charCodeAt(i) & 0xff;
            return Promise.resolve(buf);
        }
    };
};
// Fase 7.31 — resolve y reject los Promises pending. El chrome los
// llama desde el handler de Msg::FetchComplete.
//
// Fase 7.38 — el mismo canal sirve para XHR (`__puriy_xhr_pending[id]`).
// Si el id está en el mapa XHR, ruteamos al handler XHR (que llena
// status/responseText/headers y dispara onreadystatechange + onload).
// Sino, va al Promise-based fetch como hasta ahora.
globalThis.__puriy_xhr_pending = {};
globalThis.__puriy_fetch_resolve = function(id, status, statusText, body, hdrPairs) {
    var xhr = globalThis.__puriy_xhr_pending[id];
    if (xhr) {
        delete globalThis.__puriy_xhr_pending[id];
        xhr.__puriy_complete(status, statusText, body, hdrPairs);
        return;
    }
    var pending = globalThis.__puriy_fetch_pending[id];
    if (!pending) return;
    delete globalThis.__puriy_fetch_pending[id];
    var resp = globalThis.__puriy_make_response(status, statusText, body, hdrPairs);
    pending.resolve(resp);
};
globalThis.__puriy_fetch_reject = function(id, msg) {
    var xhr = globalThis.__puriy_xhr_pending[id];
    if (xhr) {
        delete globalThis.__puriy_xhr_pending[id];
        xhr.__puriy_error(String(msg));
        return;
    }
    var pending = globalThis.__puriy_fetch_pending[id];
    if (!pending) return;
    delete globalThis.__puriy_fetch_pending[id];
    pending.reject(new Error(String(msg)));
};
"#;
