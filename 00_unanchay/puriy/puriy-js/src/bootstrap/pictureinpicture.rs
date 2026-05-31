pub(crate) const PICTUREINPICTURE_BOOTSTRAP: &str = r#"
// Fase 7.165 — Picture-in-Picture API (`video.requestPictureInPicture()`).
// Saca un <video> a una ventana flotante always-on-top mientras el usuario navega.
// El motor no controla el compositor: `requestPictureInPicture()` (definido en
// __puriy_make_element) publica `kind: 'pip-request'` (value = id del elemento) y
// devuelve una Promise pendiente que el chrome resuelve con
// `__puriy_pip_resolve(elementId, width, height)` (entra en PiP, setea
// `document.pictureInPictureElement`, dispara `enterpictureinpicture`, resuelve con
// una `PictureInPictureWindow`) o rechaza con `__puriy_pip_reject(elementId, name)`.
// `document.exitPictureInPicture()` limpia el estado, dispara `leavepictureinpicture`
// y publica `pip-exit`. El chrome notifica resize de la ventana con
// `__puriy_pip_resize(width, height)`. La ventana flotante real es del chrome (PENDIENTE).
(function() {
    var doc = globalThis.document = globalThis.document || {};
    if (doc.exitPictureInPicture != null) return;

    var state = globalThis.__puriy_pip_state = globalThis.__puriy_pip_state || {
        elementId: null,
        window: null
    };
    var pending = globalThis.__puriy_pip_pending = globalThis.__puriy_pip_pending || {};

    // PictureInPictureWindow — la ventana flotante. EventTarget con `resize`.
    function PictureInPictureWindow(width, height) {
        if (globalThis.EventTarget && globalThis.EventTarget.call) {
            try { globalThis.EventTarget.call(this); } catch (e) {}
        }
        this.width = (width != null) ? (width | 0) : 0;
        this.height = (height != null) ? (height | 0) : 0;
        this.onresize = null;
    }
    if (typeof globalThis.EventTarget === 'function') {
        PictureInPictureWindow.prototype = Object.create(globalThis.EventTarget.prototype);
        PictureInPictureWindow.prototype.constructor = PictureInPictureWindow;
    }
    globalThis.PictureInPictureWindow = PictureInPictureWindow;

    // El compositor host no nos limita: PiP siempre disponible.
    doc.pictureInPictureEnabled = true;
    Object.defineProperty(doc, 'pictureInPictureElement', {
        configurable: true,
        get: function() {
            var id = globalThis.__puriy_pip_state.elementId;
            if (id == null) return null;
            return (globalThis.__puriy_elements && globalThis.__puriy_elements[id]) || null;
        }
    });

    function fireEl(elementId, type) {
        if (elementId != null && typeof globalThis.__puriy_dispatch === 'function') {
            globalThis.__puriy_dispatch(String(elementId), type, null);
        }
    }

    doc.exitPictureInPicture = function() {
        var id = globalThis.__puriy_pip_state.elementId;
        if (id == null) {
            return Promise.reject(new globalThis.DOMException(
                'No hay elemento en picture-in-picture', 'InvalidStateError'));
        }
        globalThis.__puriy_pip_state.elementId = null;
        globalThis.__puriy_pip_state.window = null;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'pip-exit', value: String(id) });
        fireEl(id, 'leavepictureinpicture');
        return Promise.resolve();
    };

    // Llamado por el método `el.requestPictureInPicture()` (en __puriy_make_element).
    globalThis.__puriy_request_pip = function(elementId) {
        var id = String(elementId);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'pip-request', value: id });
        return new Promise(function(resolve, reject) {
            pending[id] = { resolve: resolve, reject: reject };
        });
    };

    // El chrome confirma la entrada en PiP con el tamaño de la ventana flotante.
    globalThis.__puriy_pip_resolve = function(elementId, width, height) {
        var id = String(elementId);
        var p = pending[id];
        var win = new PictureInPictureWindow(
            (width != null) ? width : 320, (height != null) ? height : 180);
        globalThis.__puriy_pip_state.elementId = id;
        globalThis.__puriy_pip_state.window = win;
        fireEl(id, 'enterpictureinpicture');
        if (p) { delete pending[id]; p.resolve(win); return true; }
        return false;
    };
    // El chrome niega la petición (sin gesto de usuario, política, etc.).
    globalThis.__puriy_pip_reject = function(elementId, name, message) {
        var id = String(elementId);
        var p = pending[id];
        if (p) {
            delete pending[id];
            p.reject(new globalThis.DOMException(
                (message != null) ? String(message) : 'requestPictureInPicture denegado',
                (name != null) ? String(name) : 'NotAllowedError'));
            return true;
        }
        return false;
    };
    // El chrome notifica un resize de la ventana PiP.
    globalThis.__puriy_pip_resize = function(width, height) {
        var win = globalThis.__puriy_pip_state.window;
        if (!win) return false;
        win.width = width | 0;
        win.height = height | 0;
        var ev = new globalThis.Event('resize');
        if (typeof win.onresize === 'function') {
            try { win.onresize.call(win, ev); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        if (typeof win.dispatchEvent === 'function') win.dispatchEvent(ev);
        return true;
    };
    void 0;
})();
"#;
