pub(crate) const COOKIES_BOOTSTRAP: &str = r#"
// Fase 7.90 — `document.cookie`. Jar de cookies local al runtime (clave =
// nombre; ignoramos el scoping fino por path/domain — suficiente para el 95%
// del uso: leer/escribir cookies de sesión y preferencias). El getter/setter
// real cuelga de `document` en set_document (lib.rs) y delega en estos hooks.
//
// Distinción HttpOnly: una cookie marcada HttpOnly (sólo seteable desde la red
// vía `Set-Cookie`) NO es visible a JS — `document.cookie` la omite. El path
// de JS (`document.cookie = ...`) fuerza httpOnly=false aunque el string lo
// pida (spec). El chrome inyecta las cookies de respuesta con
// `__puriy_set_cookie_from_network(setCookieHeaderValue)`.
(function() {
    globalThis.__puriy_cookie_jar = globalThis.__puriy_cookie_jar || {};

    function parseCookie(str, fromNetwork) {
        var parts = String(str).split(';');
        var first = parts[0] || '';
        var eq = first.indexOf('=');
        var name, value;
        if (eq < 0) { name = first.trim(); value = ''; }
        else { name = first.slice(0, eq).trim(); value = first.slice(eq + 1).trim(); }
        if (name === '') return null;
        var expiresAt = null;   // ms epoch; null = cookie de sesión
        var httpOnly = false;
        var path = '/';
        for (var i = 1; i < parts.length; i++) {
            var seg = parts[i].trim();
            var se = seg.indexOf('=');
            var attr = (se < 0 ? seg : seg.slice(0, se)).trim().toLowerCase();
            var aval = (se < 0 ? '' : seg.slice(se + 1).trim());
            if (attr === 'max-age') {
                var ma = parseInt(aval, 10);
                if (!isNaN(ma)) expiresAt = (ma <= 0) ? 0 : (Date.now() + ma * 1000);
            } else if (attr === 'expires') {
                var t = Date.parse(aval);
                if (!isNaN(t)) expiresAt = t;
            } else if (attr === 'path') {
                path = aval || '/';
            } else if (attr === 'httponly') {
                httpOnly = true;
            }
            // secure / samesite / domain: parseadas pero no aplicadas todavía.
        }
        if (!fromNetwork) httpOnly = false;   // JS no puede setear HttpOnly
        return { name: name, value: value, expiresAt: expiresAt, httpOnly: httpOnly, path: path };
    }

    function store(c) {
        if (!c) return;
        // max-age<=0 / expires en el pasado ⇒ borrado (es como expira el spec).
        if (c.expiresAt != null && c.expiresAt <= Date.now()) {
            delete globalThis.__puriy_cookie_jar[c.name];
            return;
        }
        globalThis.__puriy_cookie_jar[c.name] = c;
    }

    globalThis.__puriy_cookie_set = function(str) { store(parseCookie(str, false)); };
    globalThis.__puriy_set_cookie_from_network = function(str) { store(parseCookie(str, true)); };
    globalThis.__puriy_cookie_get = function() {
        var jar = globalThis.__puriy_cookie_jar;
        var now = Date.now();
        var out = [];
        for (var k in jar) {
            if (!Object.prototype.hasOwnProperty.call(jar, k)) continue;
            var c = jar[k];
            if (c.expiresAt != null && c.expiresAt <= now) { delete jar[k]; continue; }
            if (c.httpOnly) continue;   // HttpOnly no se expone a JS
            out.push(c.name + '=' + c.value);
        }
        return out.join('; ');
    };
})();
"#;
