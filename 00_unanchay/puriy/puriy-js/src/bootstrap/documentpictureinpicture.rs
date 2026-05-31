pub(crate) const DOCUMENTPICTUREINPICTURE_BOOTSTRAP: &str = r#"
// Fase 7.166 — Document Picture-in-Picture API (`documentPictureInPicture.requestWindow()`).
// Abre una ventana PiP siempre-encima con DOM arbitrario (no sólo un <video>): controles
// de reproducción custom, mini-chat, dashboards. `documentPictureInPicture` es un singleton
// EventTarget. `requestWindow(options)` publica `kind: 'document-pip-request'`
// (value `<id> GS <width> GS <height>`, GS = U+001D) y devuelve una Promise pendiente que el
// chrome resuelve con `__puriy_document_pip_resolve(id, win)` (setea
// `documentPictureInPicture.window`, dispara el evento `enter` con `.window`, resuelve con el
// Window) o rechaza con `__puriy_document_pip_reject(id, name)`. La ventana real (con su propio
// documento) es del chrome (PENDIENTE) — por defecto resolvemos un Window sintético mínimo.
// Patrón pending-id como webauthn 7.128 / webotp 7.164.
(function() {
    if (globalThis.documentPictureInPicture != null) return;
    if (typeof globalThis.EventTarget !== 'function') return;  // requiere Fase 7.76

    function DocumentPictureInPicture() {
        globalThis.EventTarget.call(this);
        this._window = null;
        this.onenter = null;
    }
    DocumentPictureInPicture.prototype = Object.create(globalThis.EventTarget.prototype);
    DocumentPictureInPicture.prototype.constructor = DocumentPictureInPicture;
    Object.defineProperty(DocumentPictureInPicture.prototype, 'window', {
        configurable: true,
        get: function() { return this._window; }
    });

    var pending = globalThis.__puriy_document_pip_pending = globalThis.__puriy_document_pip_pending || {};
    globalThis.__puriy_document_pip_next_id = globalThis.__puriy_document_pip_next_id || 1;
    var GS = String.fromCharCode(0x1D);

    DocumentPictureInPicture.prototype.requestWindow = function(options) {
        options = options || {};
        var w = (options.width != null) ? (options.width | 0) : 0;
        var h = (options.height != null) ? (options.height | 0) : 0;
        var id = globalThis.__puriy_document_pip_next_id++;
        globalThis.__puriy_dirty.push({
            id: '__window__',
            kind: 'document-pip-request',
            value: id + GS + w + GS + h
        });
        var self = this;
        return new Promise(function(resolve, reject) {
            pending[id] = { resolve: resolve, reject: reject, target: self };
        });
    };

    var dpip = new DocumentPictureInPicture();
    globalThis.documentPictureInPicture = dpip;

    // El chrome abre la ventana PiP-de-documento y entrega su Window.
    globalThis.__puriy_document_pip_resolve = function(id, win) {
        var p = pending[id];
        if (!p) return false;
        delete pending[id];
        // Window sintético mínimo si el chrome no provee uno propio.
        var w = win || {
            closed: false,
            document: {
                body: null,
                head: null,
                createElement: function() { return null; }
            },
            close: function() { this.closed = true; }
        };
        p.target._window = w;
        var ev = new globalThis.Event('enter');
        ev.window = w;
        if (typeof p.target.onenter === 'function') {
            try { p.target.onenter.call(p.target, ev); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        p.target.dispatchEvent(ev);
        p.resolve(w);
        return true;
    };
    globalThis.__puriy_document_pip_reject = function(id, name, message) {
        var p = pending[id];
        if (!p) return false;
        delete pending[id];
        p.reject(new globalThis.DOMException(
            (message != null) ? String(message) : 'requestWindow denegado',
            (name != null) ? String(name) : 'NotAllowedError'));
        return true;
    };
    void 0;
})();
"#;
