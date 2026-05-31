pub(crate) const PRESENTATION_BOOTSTRAP: &str = r#"
// Fase 7.134 — Presentation API (`PresentationRequest`/`PresentationConnection`/
// `PresentationAvailability`, `navigator.presentation`). Apps de "segunda pantalla"
// (slides, video) la usan para proyectar a un display remoto. El motor no descubre
// pantallas: `start()` es host-driven (publica kind: 'presentation-start'
// value `<id> GS <url>`) y el chrome resuelve con `__puriy_presentation_resolve(id, info)`
// (→ PresentationConnection 'connected') o cancela con
// `__puriy_presentation_reject(id, name)` (NotAllowedError/NotFoundError) — patrón
// pending-id como push/share. `getAvailability()` devuelve un objeto cuyo `.value`
// decide el chrome (`__puriy_set_presentation_availability(bool)`). El host empuja
// mensajes entrantes con `__puriy_presentation_message(connId, data)`.
(function() {
    if (globalThis.PresentationRequest != null) return;
    var GS = String.fromCharCode(0x1D);
    var pending = {};
    var nextId = 1;
    globalThis.__puriy_presentation_next_id = 1;
    var availabilityValue = false;
    var availabilityObjs = [];
    var connections = {};

    function mix(proto) {
        proto.addEventListener = function(type, fn) {
            (this._listeners[type] = this._listeners[type] || []).push(fn);
        };
        proto.removeEventListener = function(type, fn) {
            var a = this._listeners[type]; if (!a) return;
            var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
        };
        proto.dispatchEvent = function(ev) {
            var a = this._listeners[ev.type];
            if (a) { var c = a.slice(); for (var i = 0; i < c.length; i++) c[i].call(this, ev); }
            if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
            return true;
        };
    }

    function PresentationConnection(id, url) {
        this.id = id;
        this.url = url;
        this.state = 'connecting';
        this.binaryType = 'arraybuffer';
        this._listeners = {};
        connections[id] = this;
    }
    mix(PresentationConnection.prototype);
    PresentationConnection.prototype.send = function(data) {
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'presentation-send', value: this.id + GS + String(data)
        });
    };
    PresentationConnection.prototype.close = function() {
        if (this.state === 'closed' || this.state === 'terminated') return;
        this.state = 'closed';
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'presentation-close', value: this.id });
        this.dispatchEvent({ type: 'close', reason: 'closed', message: null });
    };
    PresentationConnection.prototype.terminate = function() {
        if (this.state === 'terminated') return;
        this.state = 'terminated';
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'presentation-terminate', value: this.id });
        this.dispatchEvent({ type: 'terminate' });
    };

    function PresentationAvailability() {
        this.value = availabilityValue;
        this._listeners = {};
        availabilityObjs.push(this);
    }
    mix(PresentationAvailability.prototype);

    function PresentationRequest(urls) {
        if (urls == null) throw new globalThis.DOMException('urls requerido', 'NotSupportedError');
        this.urls = (typeof urls === 'string') ? [urls] : urls.slice();
        if (this.urls.length === 0) throw new globalThis.DOMException('urls vacío', 'NotSupportedError');
        this._listeners = {};
    }
    mix(PresentationRequest.prototype);
    PresentationRequest.prototype.start = function() {
        var self = this;
        var id = nextId++; globalThis.__puriy_presentation_next_id = nextId;
        return new Promise(function(resolve, reject) {
            pending[id] = { resolve: resolve, reject: reject, url: self.urls[0] };
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'presentation-start', value: id + GS + self.urls[0]
            });
        });
    };
    PresentationRequest.prototype.reconnect = function(connId) {
        var self = this;
        var id = nextId++; globalThis.__puriy_presentation_next_id = nextId;
        return new Promise(function(resolve, reject) {
            pending[id] = { resolve: resolve, reject: reject, url: self.urls[0], reconnect: connId };
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'presentation-reconnect', value: id + GS + String(connId)
            });
        });
    };
    PresentationRequest.prototype.getAvailability = function() {
        return Promise.resolve(new PresentationAvailability());
    };

    // El host confirma la sesión tras el picker de pantallas.
    globalThis.__puriy_presentation_resolve = function(reqId, info) {
        var p = pending[reqId]; if (!p) return false; delete pending[reqId];
        info = info || {};
        var conn = new PresentationConnection(info.id || ('conn-' + reqId), info.url || p.url);
        conn.state = 'connected';
        p.resolve(conn);
        return true;
    };
    globalThis.__puriy_presentation_reject = function(reqId, name) {
        var p = pending[reqId]; if (!p) return false; delete pending[reqId];
        p.reject(new globalThis.DOMException('presentation cancelada', name || 'NotAllowedError'));
        return true;
    };
    globalThis.__puriy_presentation_message = function(connId, data) {
        var c = connections[connId]; if (!c) return false;
        c.dispatchEvent({ type: 'message', data: data });
        return true;
    };
    globalThis.__puriy_presentation_close = function(connId, reason) {
        var c = connections[connId]; if (!c) return false;
        c.state = 'closed';
        c.dispatchEvent({ type: 'close', reason: reason || 'closed', message: null });
        return true;
    };
    globalThis.__puriy_set_presentation_availability = function(v) {
        availabilityValue = !!v;
        for (var i = 0; i < availabilityObjs.length; i++) {
            var a = availabilityObjs[i];
            if (a.value !== availabilityValue) {
                a.value = availabilityValue;
                a.dispatchEvent({ type: 'change' });
            }
        }
        return true;
    };

    globalThis.PresentationRequest = PresentationRequest;
    globalThis.PresentationConnection = PresentationConnection;
    globalThis.PresentationAvailability = PresentationAvailability;
    if (globalThis.navigator == null) globalThis.navigator = {};
    if (globalThis.navigator.presentation == null) {
        globalThis.navigator.presentation = { defaultRequest: null, receiver: null };
    }
    void 0;
})();
"#;
