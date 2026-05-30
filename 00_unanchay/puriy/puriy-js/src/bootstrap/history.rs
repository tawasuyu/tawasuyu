pub(crate) const HISTORY_BOOTSTRAP: &str = r#"
// Fase 7.88 — History API (`history.pushState/replaceState/back/forward/go`) +
// evento `popstate`. Es el motor de las SPA: un router (React Router, Vue
// Router, etc.) hace `pushState` para cambiar la URL sin recargar y escucha
// `popstate` para reaccionar al back/forward del usuario.
//
// Cargado DESPUÉS de `location` (usa `location.__puriy_set_url_silent` para
// reflejar la URL del entry activo sin disparar navegación) y `url`
// (`__puriy_resolve_url`).
//
// Pila de sesión local al runtime: cada entry es `{ state, url }`. pushState
// trunca el forward y agrega; replaceState pisa el entry actual; back/forward/go
// mueven el cursor y disparan `popstate` con el state del destino (pushState y
// replaceState NO disparan popstate — spec). go(0) recarga el documento. go
// fuera de rango es no-op.
(function() {
    var h = globalThis.history = globalThis.history || {};
    var stack = [{ state: null, url: (globalThis.location && globalThis.location.href) || '' }];
    var idx = 0;

    function currentHref() {
        return (globalThis.location && globalThis.location.href) || stack[idx].url || '';
    }
    function applyEntry() {
        var e = stack[idx];
        if (e.url && globalThis.location && typeof globalThis.location.__puriy_set_url_silent === 'function') {
            globalThis.location.__puriy_set_url_silent(e.url);
        }
    }

    Object.defineProperty(h, 'length', {
        configurable: true, get: function() { return stack.length; }
    });
    Object.defineProperty(h, 'state', {
        configurable: true, get: function() { return stack[idx].state; }
    });
    if (h.scrollRestoration == null) h.scrollRestoration = 'auto';

    h.pushState = function(state, unused_title, url) {
        var resolved = (url != null)
            ? globalThis.__puriy_resolve_url(String(url), currentHref())
            : currentHref();
        stack.length = idx + 1;   // descarta cualquier forward
        stack.push({ state: (state === undefined ? null : state), url: resolved });
        idx = stack.length - 1;
        applyEntry();
    };
    h.replaceState = function(state, unused_title, url) {
        var resolved = (url != null)
            ? globalThis.__puriy_resolve_url(String(url), currentHref())
            : stack[idx].url;
        stack[idx] = { state: (state === undefined ? null : state), url: resolved };
        applyEntry();
    };
    h.go = function(delta) {
        delta = (delta == null) ? 0 : (delta | 0);
        if (delta === 0) {
            // go(0) recarga el documento actual (spec).
            if (globalThis.location && typeof globalThis.location.reload === 'function') {
                globalThis.location.reload();
            }
            return;
        }
        var target = idx + delta;
        if (target < 0 || target >= stack.length) return;   // fuera de rango: no-op
        idx = target;
        applyEntry();
        globalThis.__puriy_dispatch_window('popstate', { state: stack[idx].state });
    };
    h.back = function() { h.go(-1); };
    h.forward = function() { h.go(1); };
})();
"#;
