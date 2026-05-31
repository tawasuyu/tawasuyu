pub(crate) const LOCATION_BOOTSTRAP: &str = r#"
// Fase 7.87 — `location` como objeto Location real. Hasta acá `location` era
// un objeto plano `{ href, toString }` que set_document inyectaba (ver lib.rs),
// sin pathname/search/hash/origin — los routers de SPA (que leen
// `location.pathname` para decidir qué vista pintar) se rompían. Ahora
// `__puriy_make_location(href)` parsea el href con `new URL` (Fase 7.58) y
// expone los componentes WHATWG vivos, con setters que disparan navegación.
//
// Cargado DESPUÉS de `urlclass` (provee `new URL`), `url` (`__puriy_resolve_url`),
// `window_events` (`__puriy_dispatch_window`) y `dom_events` (`__puriy_dirty`).
//
// Navegación: cambiar `href`/`pathname`/`search`/`protocol`/`host`/etc. o llamar
// `assign`/`replace`/`reload` publica un mutation `kind: 'navigate'` al canal
// `__puriy_dirty` (value = `mode U+001D url`, mode ∈ push/replace/reload) que el
// chrome consume para cargar otra página — wiring nativo PENDIENTE, hoy es
// no-op si el chrome no lo rutea, igual que websocket/eventsource. Cambiar SÓLO
// `hash` es same-document: NO navega, dispara `hashchange` sobre window.
globalThis.__puriy_make_location = function(initialHref) {
    var u = new globalThis.URL(initialHref);
    var loc = {};
    var GS = String.fromCharCode(0x1D);
    function navigate(href, mode) {
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'navigate', value: mode + GS + href });
    }
    // Hook interno: history.pushState/replaceState/popstate (Fase 7.88) cambian
    // la URL mostrada SIN recargar el documento. Actualiza `u` en silencio.
    loc.__puriy_set_url_silent = function(href) { u.href = String(href); };

    // Componentes settables: asignar muta `u` y dispara navegación push. `origin`
    // es de sólo lectura (spec). `href`/`hash`/`assign`/`replace`/`reload` aparte.
    ['protocol', 'host', 'hostname', 'port', 'pathname', 'search'].forEach(function(name) {
        Object.defineProperty(loc, name, {
            enumerable: true, configurable: true,
            get: function() { return u[name]; },
            set: function(v) { u[name] = String(v); navigate(u.href, 'push'); }
        });
    });
    Object.defineProperty(loc, 'origin', {
        enumerable: true, configurable: true,
        get: function() { return u.origin; }
    });
    Object.defineProperty(loc, 'href', {
        enumerable: true, configurable: true,
        get: function() { return u.href; },
        set: function(v) {
            var resolved = globalThis.__puriy_resolve_url(String(v), u.href);
            u.href = resolved;        // el spec actualiza location de inmediato y luego navega
            navigate(resolved, 'push');
        }
    });
    Object.defineProperty(loc, 'hash', {
        enumerable: true, configurable: true,
        get: function() { return u.hash; },
        set: function(v) {
            var nv = String(v);
            if (nv.length && nv.charAt(0) !== '#') nv = '#' + nv;
            var oldURL = u.href;
            u.hash = nv;
            var newURL = u.href;
            if (oldURL !== newURL) {
                // same-document: no recarga, dispara hashchange (Fase 7.39 dispatch).
                globalThis.__puriy_dispatch_window('hashchange', { oldURL: oldURL, newURL: newURL });
            }
        }
    });
    loc.assign = function(url) {
        var resolved = globalThis.__puriy_resolve_url(String(url), u.href);
        u.href = resolved;
        navigate(resolved, 'push');
    };
    loc.replace = function(url) {
        var resolved = globalThis.__puriy_resolve_url(String(url), u.href);
        u.href = resolved;
        navigate(resolved, 'replace');
    };
    loc.reload = function() { navigate(u.href, 'reload'); };
    loc.toString = function() { return u.href; };
    // ancestorOrigins: sin iframes, lista vacía (spec — DOMStringList).
    loc.ancestorOrigins = {
        length: 0,
        item: function() { return null; },
        contains: function() { return false; }
    };
    return loc;
};
// Default: hasta que el chrome llame set_document, `location` apunta a
// about:blank — así `location.pathname` no tira en contexto sin documento.
if (globalThis.location == null || typeof globalThis.location.assign !== 'function') {
    globalThis.location = globalThis.__puriy_make_location('about:blank');
}
"#;
