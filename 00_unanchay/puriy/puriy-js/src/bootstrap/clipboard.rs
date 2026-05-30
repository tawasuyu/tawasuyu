pub(crate) const CLIPBOARD_BOOTSTRAP: &str = r#"
// Fase 7.96 — Clipboard API (`navigator.clipboard`). `writeText`/`readText` +
// `write`/`read` (con `ClipboardItem`). El portapapeles del sistema lo posee el
// chrome; acá llevamos un buffer local (`__puriy_clipboard_text`) que el chrome
// sincroniza con el hook `__puriy_set_clipboard(text)` cuando el usuario copió
// algo afuera, y al que `writeText`/`write` publican una mutación
// `kind: 'clipboard'` para que el chrome lo empuje al portapapeles real
// (wiring nativo pendiente). Mismo patrón buffer-local + hook que cookies/
// location. Divergencia: el spec gatea read/write por la Permissions API
// ('clipboard-read'/'clipboard-write') y exige user gesture — acá no se gatea
// (resuelve siempre); cuando aparezca caso real se cruza con Fase 7.93.
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    globalThis.__puriy_clipboard_text = globalThis.__puriy_clipboard_text || '';

    function ClipboardItem(items, options) {
        this._items = {};   // type -> Blob | string | Promise
        this.presentationStyle = (options && options.presentationStyle) || 'unspecified';
        items = items || {};
        for (var k in items) {
            if (Object.prototype.hasOwnProperty.call(items, k)) {
                this._items[k] = items[k];
            }
        }
    }
    Object.defineProperty(ClipboardItem.prototype, 'types', {
        get: function() { return Object.keys(this._items); }
    });
    // getType resuelve perezosamente: el valor puede venir como Blob, string o
    // una Promise de cualquiera de los dos (el spec lo permite). Siempre devuelve
    // Promise<Blob>.
    ClipboardItem.prototype.getType = function(type) {
        var v = this._items[type];
        if (v == null) {
            return Promise.reject(new globalThis.DOMException("The type '" + type + "' was not found", 'NotFoundError'));
        }
        return Promise.resolve(v).then(function(resolved) {
            if (resolved instanceof globalThis.Blob) return resolved;
            return new globalThis.Blob([String(resolved)], { type: type });
        });
    };
    ClipboardItem.supports = function(type) {
        return type === 'text/plain' || type === 'text/html' || type === 'image/png';
    };

    function Clipboard() { globalThis.EventTarget.call(this); }
    Clipboard.prototype = Object.create(globalThis.EventTarget.prototype);
    Clipboard.prototype.constructor = Clipboard;
    Clipboard.prototype.writeText = function(text) {
        text = String(text);
        globalThis.__puriy_clipboard_text = text;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'clipboard', value: 'writeText:' + text });
        return Promise.resolve();
    };
    Clipboard.prototype.readText = function() {
        return Promise.resolve(globalThis.__puriy_clipboard_text);
    };
    // write(items): por cada ClipboardItem con text/plain, vuelca su texto al
    // buffer y publica la mutación. Tipos no-texto se ignoran (sin portapapeles
    // de imágenes todavía). Resuelve cuando todos los text/plain se leyeron.
    Clipboard.prototype.write = function(items) {
        items = items || [];
        var pending = [];
        for (var i = 0; i < items.length; i++) {
            var item = items[i];
            if (item && item._items && item._items['text/plain'] != null) {
                pending.push(item.getType('text/plain').then(function(blob) {
                    return blob.text().then(function(t) {
                        globalThis.__puriy_clipboard_text = t;
                        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'clipboard', value: 'write:' + t });
                    });
                }));
            }
        }
        return Promise.all(pending).then(function() { return undefined; });
    };
    Clipboard.prototype.read = function() {
        var text = globalThis.__puriy_clipboard_text;
        var item = new ClipboardItem({ 'text/plain': new globalThis.Blob([text], { type: 'text/plain' }) });
        return Promise.resolve([item]);
    };

    globalThis.ClipboardItem = ClipboardItem;
    globalThis.Clipboard = Clipboard;
    if (nav.clipboard == null) nav.clipboard = new Clipboard();

    // El chrome sincroniza el portapapeles del sistema (un copy externo) acá.
    globalThis.__puriy_set_clipboard = function(text) {
        globalThis.__puriy_clipboard_text = String(text);
        return true;
    };
})();
"#;
