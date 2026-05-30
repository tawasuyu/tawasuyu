pub(crate) const MATCHMEDIA_BOOTSTRAP: &str = r#"
// Fase 7.98 — `window.matchMedia` + `MediaQueryList`. Las apps lo usan para JS
// responsive: `prefers-color-scheme`, `prefers-reduced-motion`, breakpoints de
// ancho, `print`. `matchMedia(query)` devuelve un `MediaQueryList` (EventTarget,
// Fase 7.76) con `.media` + `.matches` + `.onchange` + `addEventListener
// ('change')` y los legacy `addListener`/`removeListener`. El motor no evalúa
// media queries: el resultado lo decide el chrome con el hook
// `__puriy_set_media_match(query, matches)`, que flippea el estado y dispara
// `change` en los MediaQueryList vivos de esa query (mismo patrón que
// online/offline 7.86 y Permissions 7.93). Default `matches=false` hasta que el
// chrome lo setee. Divergencia: el evento es un Event con `matches`/`media`
// pegados, no un `MediaQueryListEvent` real.
(function() {
    globalThis.__puriy_media_state = globalThis.__puriy_media_state || {};   // query -> bool
    var live = {};   // query -> [MediaQueryList vivos]

    function MediaQueryList(query) {
        globalThis.EventTarget.call(this);
        this.media = query;
        this.onchange = null;
    }
    MediaQueryList.prototype = Object.create(globalThis.EventTarget.prototype);
    MediaQueryList.prototype.constructor = MediaQueryList;
    Object.defineProperty(MediaQueryList.prototype, 'matches', {
        get: function() { return globalThis.__puriy_media_state[this.media] === true; }
    });
    // Legacy addListener/removeListener (deprecados pero todavía muy usados).
    MediaQueryList.prototype.addListener = function(fn) {
        if (typeof fn === 'function') this.addEventListener('change', fn);
    };
    MediaQueryList.prototype.removeListener = function(fn) {
        if (typeof fn === 'function') this.removeEventListener('change', fn);
    };

    globalThis.MediaQueryList = MediaQueryList;
    globalThis.matchMedia = function(query) {
        query = String(query);
        var mql = new MediaQueryList(query);
        if (!live[query]) live[query] = [];
        live[query].push(mql);
        return mql;
    };

    globalThis.__puriy_set_media_match = function(query, matches) {
        query = String(query);
        globalThis.__puriy_media_state[query] = !!matches;
        var arr = live[query];
        if (!arr) return true;
        for (var i = 0; i < arr.length; i++) {
            var mql = arr[i];
            var ev = new globalThis.Event('change');
            ev.matches = mql.matches;
            ev.media = mql.media;
            if (typeof mql.onchange === 'function') {
                try { mql.onchange.call(mql, ev); }
                catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
            }
            mql.dispatchEvent(ev);
        }
        return true;
    };
})();
"#;
