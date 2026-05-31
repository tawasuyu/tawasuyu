//! Fase 7.156-7.157 — `document.createEvent(interface)` legacy (DOM Level 2)
//! + los init methods legacy `initUIEvent`/`initMouseEvent`/`initKeyboardEvent`.
//!
//! Antes de los constructores tipados (`new MouseEvent(...)`, 7.105+), la forma
//! canónica de crear y despachar un evento sintético era el patrón de dos pasos
//! de DOM L2: `var e = document.createEvent('MouseEvents'); e.initMouseEvent(...)`.
//! Sigue vivo en librerías y polyfills viejos (jQuery pre-3, Modernizr, código de
//! testing). `createEvent` se instala vía hook desde `set_document` (mismo molde
//! que selection/cookies/tree_walker); los init methods se cuelgan de los
//! prototipos en bootstrap (corren después de ui_events/keyboard_events).

pub(crate) const CREATE_EVENT_BOOTSTRAP: &str = r#"
// Fase 7.156 — document.createEvent(interface): factory legacy. Mapea el nombre
// de interfaz (case-insensitive) al constructor tipado y devuelve un evento SIN
// inicializar (type=''); el init*Event lo configura. Interfaz desconocida →
// NotSupportedError (DOMException), igual que el chrome.
globalThis.__puriy_install_create_event = function(doc) {
    if (!doc || typeof doc.createEvent === 'function') return;
    // Tabla de los nombres legacy del spec (más los alias históricos de
    // navegadores). Apunta al NOMBRE del constructor global, resuelto en runtime
    // (los constructores tipados de 7.105+ ya existen post-bootstrap).
    var MAP = {
        'event': 'Event', 'events': 'Event', 'htmlevents': 'Event',
        'customevent': 'CustomEvent',
        'uievent': 'UIEvent', 'uievents': 'UIEvent',
        'mouseevent': 'MouseEvent', 'mouseevents': 'MouseEvent',
        'keyboardevent': 'KeyboardEvent', 'keyevents': 'KeyboardEvent',
        'compositionevent': 'CompositionEvent',
        'focusevent': 'FocusEvent',
        'wheelevent': 'WheelEvent',
        'inputevent': 'InputEvent',
        'messageevent': 'MessageEvent',
        'touchevent': 'TouchEvent',
        'hashchangeevent': 'HashChangeEvent',
        'popstateevent': 'PopStateEvent',
        'storageevent': 'StorageEvent',
        'progressevent': 'ProgressEvent',
        'dragevent': 'DragEvent'
    };
    doc.createEvent = function(iface) {
        var key = String(iface).toLowerCase();
        var ctorName = MAP[key];
        var Ctor = ctorName ? globalThis[ctorName] : null;
        if (typeof Ctor !== 'function') {
            throw new globalThis.DOMException(
                "Failed to execute 'createEvent' on 'Document': The provided event type ('"
                + iface + "') is invalid.", 'NotSupportedError');
        }
        var ev = new Ctor('');
        // Marca legacy: el evento nace sin inicializar (el spec lo dispara recién
        // tras initEvent). No lo gateamos duro — initEvent setea type/bubbles/etc.
        ev._initialized = false;
        return ev;
    };
};

// Fase 7.157 — init methods legacy de las interfaces tipadas. Análogos a
// Event.prototype.initEvent (7.112) y CustomEvent.initCustomEvent (7.134) pero
// seteando los campos extra de cada interfaz. Gateados por el dispatch flag
// (`_dispatch`): el spec los vuelve no-op mientras el evento está en vuelo (7.158).
(function() {
    if (globalThis.UIEvent && globalThis.UIEvent.prototype &&
        typeof globalThis.UIEvent.prototype.initUIEvent !== 'function') {
        globalThis.UIEvent.prototype.initUIEvent = function(type, bubbles, cancelable, view, detail) {
            if (this._dispatch) return;
            this.type = String(type);
            this.bubbles = !!bubbles;
            this.cancelable = !!cancelable;
            this.view = view !== undefined ? view : null;
            this.detail = detail !== undefined ? detail : 0;
        };
    }
    if (globalThis.MouseEvent && globalThis.MouseEvent.prototype &&
        typeof globalThis.MouseEvent.prototype.initMouseEvent !== 'function') {
        globalThis.MouseEvent.prototype.initMouseEvent = function(
            type, bubbles, cancelable, view, detail,
            screenX, screenY, clientX, clientY,
            ctrlKey, altKey, shiftKey, metaKey, button, relatedTarget) {
            if (this._dispatch) return;
            this.type = String(type);
            this.bubbles = !!bubbles;
            this.cancelable = !!cancelable;
            this.view = view !== undefined ? view : null;
            this.detail = detail !== undefined ? detail : 0;
            this.screenX = screenX || 0;
            this.screenY = screenY || 0;
            this.clientX = clientX || 0;
            this.clientY = clientY || 0;
            this.ctrlKey = !!ctrlKey;
            this.altKey = !!altKey;
            this.shiftKey = !!shiftKey;
            this.metaKey = !!metaKey;
            this.button = button || 0;
            this.relatedTarget = relatedTarget !== undefined ? relatedTarget : null;
        };
    }
    if (globalThis.KeyboardEvent && globalThis.KeyboardEvent.prototype &&
        typeof globalThis.KeyboardEvent.prototype.initKeyboardEvent !== 'function') {
        // El spec legacy de initKeyboardEvent tiene firmas divergentes entre
        // navegadores (WebKit usa `keyIdentifier`, Gecko `charArg`/`keyArg`).
        // Adoptamos la forma WebKit/moderna `(type, bubbles, cancelable, view,
        // key, location, ctrl, alt, shift, meta)` — la más común en el wild.
        globalThis.KeyboardEvent.prototype.initKeyboardEvent = function(
            type, bubbles, cancelable, view, key, location,
            ctrlKey, altKey, shiftKey, metaKey) {
            if (this._dispatch) return;
            this.type = String(type);
            this.bubbles = !!bubbles;
            this.cancelable = !!cancelable;
            this.view = view !== undefined ? view : null;
            this.key = key !== undefined ? String(key) : '';
            this.location = location || 0;
            this.ctrlKey = !!ctrlKey;
            this.altKey = !!altKey;
            this.shiftKey = !!shiftKey;
            this.metaKey = !!metaKey;
        };
    }

    // Instalación eager: events llamaba este hook desde set_document; net no.
    // En runtime fresco / headless aún no hay `document`, así que lo creamos
    // (igual que cssom/fontface/fullscreen) y montamos createEvent. Tras una
    // carga real, set_document re-monta el hook sobre el document nuevo.
    var __doc = globalThis.document = globalThis.document || {};
    globalThis.__puriy_install_create_event(__doc);
})();
"#;
