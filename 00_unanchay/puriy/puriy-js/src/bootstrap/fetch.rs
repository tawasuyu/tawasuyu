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
    // Fase 7.56 — aceptar un Request como primer arg. El `init` explícito
    // (segundo arg) pisa los campos que trae el Request.
    if (globalThis.Request && url instanceof globalThis.Request) {
        var req = url;
        var over = init || {};
        init = {
            method: over.method || req.method,
            headers: (over.headers != null) ? over.headers : req.headers,
            body: (over.body != null) ? over.body : req._body,
            signal: (over.signal != null) ? over.signal : req.signal
        };
        url = req.url;
    }
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
// Fase 7.31 — Response del lado net. Construido por
// `__puriy_fetch_resolve(id, status, statusText, body, headers)` que el
// chrome llama cuando el HTTP termina. Fase 7.55 — refactorizado para
// delegar en el constructor público `new Response(body, init)` (módulo
// `response`); acá sólo armamos los Headers desde los pares crudos y
// marcamos `type = 'basic'` (respuesta de red). Métodos (text/json/
// arrayBuffer/blob), `bodyUsed` enforcement y el getter `body`
// (ReadableStream lazy) viven en `Response.prototype`.
globalThis.__puriy_make_response = function(status, statusText, body, hdrPairs) {
    var headers = new globalThis.Headers();
    if (hdrPairs) {
        for (var i = 0; i + 1 < hdrPairs.length; i += 2) {
            headers.set(hdrPairs[i], hdrPairs[i + 1]);
        }
    }
    var resp = new globalThis.Response(body, {
        status: status, statusText: statusText, headers: headers
    });
    resp.type = 'basic';
    return resp;
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
