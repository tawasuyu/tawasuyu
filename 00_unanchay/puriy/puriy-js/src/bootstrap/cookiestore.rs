pub(crate) const COOKIESTORE_BOOTSTRAP: &str = r#"
// Fase 7.140 — Cookie Store API (`cookieStore`). Cara asíncrona y moderna de `document.cookie`
// (Fase 7.90): `get`/`getAll`/`set`/`delete` devuelven Promesas y el evento `change` notifica
// altas/bajas. Comparte el MISMO jar (`__puriy_cookie_jar`) que `document.cookie`, así que una
// escritura por cualquiera de las dos vías es visible por la otra. Mismas reglas: las cookies
// HttpOnly (sólo seteables desde la red) no se exponen a JS; setear por cookieStore fuerza
// httpOnly=false. El evento `change` sólo lo disparan las mutaciones vía cookieStore (no las de
// red ni las de document.cookie — divergencia menor del spec, suficiente para el 95% del uso).
(function() {
    if (globalThis.cookieStore != null) return;
    globalThis.__puriy_cookie_jar = globalThis.__puriy_cookie_jar || {};

    function jar() { return globalThis.__puriy_cookie_jar; }
    function alive(c) { return !(c.expiresAt != null && c.expiresAt <= Date.now()); }
    function toObj(c) {
        return { name: c.name, value: c.value, path: c.path || '/',
                 expires: c.expiresAt, secure: !!c.secure, sameSite: c.sameSite || 'lax' };
    }
    function readName(name) {
        var j = jar();
        var c = j[name];
        if (!c) return null;
        if (!alive(c)) { delete j[name]; return null; }
        if (c.httpOnly) return null;
        return toObj(c);
    }

    var listeners = [];
    var onchange = null;
    function fireChange(changed, deleted) {
        var ev = { type: 'change', changed: changed, deleted: deleted };
        var c = listeners.slice();
        for (var i = 0; i < c.length; i++) c[i].call(cookieStore, ev);
        if (typeof onchange === 'function') onchange.call(cookieStore, ev);
    }

    function normalizeSet(name, value) {
        if (name != null && typeof name === 'object') {
            var o = name;
            return { name: o.name, value: o.value,
                     path: o.path || '/', expiresAt: (o.expires != null ? o.expires : null),
                     secure: !!o.secure, sameSite: o.sameSite || 'lax', httpOnly: false };
        }
        return { name: name, value: value, path: '/', expiresAt: null,
                 secure: false, sameSite: 'lax', httpOnly: false };
    }

    var cookieStore = {
        get: function(name) {
            var n = (name != null && typeof name === 'object') ? name.name : name;
            return Promise.resolve(readName(n));
        },
        getAll: function(name) {
            var n = (name != null && typeof name === 'object') ? name.name : name;
            var j = jar(), out = [];
            for (var k in j) {
                if (!Object.prototype.hasOwnProperty.call(j, k)) continue;
                var c = j[k];
                if (!alive(c)) { delete j[k]; continue; }
                if (c.httpOnly) continue;
                if (n != null && c.name !== n) continue;
                out.push(toObj(c));
            }
            return Promise.resolve(out);
        },
        set: function(name, value) {
            var c = normalizeSet(name, value);
            if (c.name == null || c.name === '') {
                return Promise.reject(new TypeError('nombre de cookie inválido'));
            }
            if (c.expiresAt != null && c.expiresAt <= Date.now()) {
                delete jar()[c.name];
                fireChange([], [{ name: c.name, value: '' }]);
            } else {
                jar()[c.name] = c;
                fireChange([{ name: c.name, value: c.value }], []);
            }
            return Promise.resolve();
        },
        delete: function(name) {
            var n = (name != null && typeof name === 'object') ? name.name : name;
            delete jar()[n];
            fireChange([], [{ name: n, value: '' }]);
            return Promise.resolve();
        },
        addEventListener: function(type, fn) { if (type === 'change') listeners.push(fn); },
        removeEventListener: function(type, fn) {
            if (type !== 'change') return;
            var i = listeners.indexOf(fn); if (i >= 0) listeners.splice(i, 1);
        }
    };
    Object.defineProperty(cookieStore, 'onchange', {
        get: function() { return onchange; },
        set: function(fn) { onchange = fn; }
    });

    globalThis.cookieStore = cookieStore;
    void 0;
})();
"#;
