pub(crate) const TRANSPORT_BOOTSTRAP: &str = r#"
// Fase 7.129 — WebTransport. Transporte bidireccional de baja latencia sobre
// HTTP/3 (QUIC). Vecino de WebSocket/EventSource en el frente net. El transporte
// real es del chrome (wiring nativo PENDIENTE): el constructor publica
// kind: 'webtransport' (value `<id> GS 'connect' GS url`) y el chrome confirma la
// sesión reinyectando eventos vía __puriy_wt_dispatch(id, kind, a, b, c):
//   'ready'    → resuelve la promesa `.ready`
//   'close'    → resuelve `.closed` (a=closeCode, b=reason); si c es '1' la sesión
//                murió sin gracia → rechaza `.ready` y `.closed` con NetworkError
//   'datagram' → empuja un datagrama entrante (a=data) a la cola de lectura
// `close()` y los writers de datagramas/streams publican al mismo canal. Modelo
// mínimo de streams (readable = ReadableStream de Fase streams; writable = writer
// que publica al host) — cubre lo que el 90% del JS observa (ready/closed/datagrams).
(function() {
    if (globalThis.WebTransport != null) return;
    globalThis.__puriy_wt_registry = globalThis.__puriy_wt_registry || {};
    globalThis.__puriy_wt_next_id = globalThis.__puriy_wt_next_id || 1;
    var GS = String.fromCharCode(0x1D);

    function publishWriter(id, kind) {
        return {
            getWriter: function() {
                return {
                    write: function(data) {
                        globalThis.__puriy_dirty.push({
                            id: '__window__', kind: 'webtransport',
                            value: String(id) + GS + kind + GS + String(data)
                        });
                        return Promise.resolve();
                    },
                    close: function() { return Promise.resolve(); },
                    abort: function() { return Promise.resolve(); },
                    releaseLock: function() {}
                };
            }
        };
    }

    function WebTransport(url, options) {
        var base = (globalThis.location && globalThis.location.href) || '';
        this.url = (typeof globalThis.__puriy_resolve_url === 'function')
            ? globalThis.__puriy_resolve_url(String(url), base) : String(url);
        var id = globalThis.__puriy_wt_next_id++;
        this._id = id;
        this._closed = false;
        var self = this;
        this.ready = new Promise(function(res, rej) { self.__readyRes = res; self.__readyRej = rej; });
        this.closed = new Promise(function(res, rej) { self.__closedRes = res; self.__closedRej = rej; });
        var incoming = [];
        this.datagrams = {
            incomingMaxAge: null, outgoingMaxAge: null,
            incomingHighWaterMark: 1, outgoingHighWaterMark: 1,
            maxDatagramSize: 1024,
            _incoming: incoming,
            readable: (typeof globalThis.ReadableStream === 'function')
                ? new globalThis.ReadableStream({
                    pull: function(c) { if (incoming.length) c.enqueue(incoming.shift()); }
                  })
                : { _incoming: incoming },
            writable: publishWriter(id, 'datagram')
        };
        globalThis.__puriy_wt_registry[id] = this;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'webtransport',
            value: String(id) + GS + 'connect' + GS + this.url
        });
    }
    WebTransport.prototype.close = function(info) {
        if (this._closed) return;
        this._closed = true;
        info = info || {};
        var code = (info.closeCode != null) ? (info.closeCode | 0) : 0;
        var reason = (info.reason != null) ? String(info.reason) : '';
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'webtransport',
            value: String(this._id) + GS + 'close' + GS + String(code) + GS + reason
        });
        delete globalThis.__puriy_wt_registry[this._id];
        this.__closedRes({ closeCode: code, reason: reason });
    };
    WebTransport.prototype.createBidirectionalStream = function() {
        var rs = (typeof globalThis.ReadableStream === 'function')
            ? new globalThis.ReadableStream({}) : {};
        return Promise.resolve({ readable: rs, writable: publishWriter(this._id, 'stream-write') });
    };
    WebTransport.prototype.createUnidirectionalStream = function() {
        return Promise.resolve(publishWriter(this._id, 'stream-write'));
    };
    globalThis.WebTransport = WebTransport;

    globalThis.__puriy_wt_dispatch = function(id, kind, a, b, c) {
        var wt = globalThis.__puriy_wt_registry[id];
        if (!wt) return false;
        if (kind === 'ready') {
            wt.__readyRes(undefined);
        } else if (kind === 'close') {
            wt._closed = true;
            delete globalThis.__puriy_wt_registry[id];
            if (c === '1' || c === 1 || c === true) {
                var err = new globalThis.DOMException('WebTransport falló', 'NetworkError');
                wt.__readyRej(err);
                wt.__closedRej(err);
            } else {
                wt.__closedRes({
                    closeCode: (a != null) ? (a | 0) : 0,
                    reason: (b != null) ? String(b) : ''
                });
            }
        } else if (kind === 'datagram') {
            wt.datagrams._incoming.push(a);
        }
        return true;
    };
    void 0;
})();
"#;
