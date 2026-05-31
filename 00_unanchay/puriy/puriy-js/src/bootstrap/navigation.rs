pub(crate) const NAVIGATION_BOOTSTRAP: &str = r#"
// Fase 7.138 — Navigation API (`navigation` + `NavigationHistoryEntry` + `NavigateEvent`).
// Sucesor moderno de History API (Fase 7.88): un router de SPA escucha el evento `navigate`,
// llama `event.intercept({ handler })` para tomar control de la navegación y resolver su propio
// render, y lee `navigation.currentEntry`/`entries()` para el estado. Cargado después de
// `location`/`url`/`history`. Mantiene su propia pila de entries local al runtime (divergencia:
// NO está sincronizada con `history` — modela lo que el 90% de las apps observan). El commit
// real de la URL al chrome sigue el mismo canal silencioso que History (`__puriy_set_url_silent`).
(function() {
    if (globalThis.navigation != null) return;

    var seq = 0;
    function nextId() { return 'n' + (seq++); }
    function currentHref() {
        return (globalThis.location && globalThis.location.href) || 'about:blank';
    }
    function resolve(url) {
        try {
            if (typeof globalThis.__puriy_resolve_url === 'function') {
                return globalThis.__puriy_resolve_url(String(url), currentHref());
            }
        } catch (e) {}
        return String(url);
    }
    function applyUrl(url) {
        if (url && globalThis.location && typeof globalThis.location.__puriy_set_url_silent === 'function') {
            globalThis.location.__puriy_set_url_silent(url);
        }
    }

    function NavigationHistoryEntry(url, state, index) {
        this.url = url;
        this.key = nextId();
        this.id = nextId();
        this.index = index;
        this.sameDocument = true;
        this._state = (state === undefined ? null : state);
    }
    NavigationHistoryEntry.prototype.getState = function() { return this._state; };
    NavigationHistoryEntry.prototype.addEventListener = function() {};
    NavigationHistoryEntry.prototype.removeEventListener = function() {};

    var entries = [ new NavigationHistoryEntry(currentHref(), null, 0) ];
    var current = 0;
    var listeners = {};

    function dispatch(type, ev) {
        ev.type = type;
        var a = listeners[type];
        if (a) { var c = a.slice(); for (var i = 0; i < c.length; i++) c[i].call(navigation, ev); }
        var on = navigation['on' + type];
        if (typeof on === 'function') on.call(navigation, ev);
    }
    function settled(entry) {
        return { committed: Promise.resolve(entry), finished: Promise.resolve(entry) };
    }

    function navigate(url, options) {
        options = options || {};
        var resolved = resolve(url);
        var navType = options.history === 'replace' ? 'replace' : 'push';
        var intercepts = [];
        var prevented = false;
        var ev = {
            navigationType: navType,
            canIntercept: true,
            userInitiated: false,
            hashChange: false,
            signal: null,
            destination: {
                url: resolved,
                sameDocument: false,
                index: -1,
                key: '',
                getState: function() { return options.state; }
            },
            intercept: function(opts) { if (opts && typeof opts.handler === 'function') intercepts.push(opts.handler); },
            scroll: function() {},
            preventDefault: function() { prevented = true; }
        };
        dispatch('navigate', ev);

        if (prevented && intercepts.length === 0) {
            var err = new globalThis.DOMException('navegación cancelada', 'AbortError');
            var rej = Promise.reject(err); rej.catch(function() {});
            var rej2 = Promise.reject(err); rej2.catch(function() {});
            return { committed: rej, finished: rej2 };
        }

        var committedResolve, finishedResolve, finishedReject;
        var committed = new Promise(function(res) { committedResolve = res; });
        var finished = new Promise(function(res, rej) { finishedResolve = res; finishedReject = rej; });

        var entry;
        if (navType === 'replace') {
            entry = new NavigationHistoryEntry(resolved, options.state, current);
            entries[current] = entry;
        } else {
            entries.length = current + 1;   // descarta forward
            entry = new NavigationHistoryEntry(resolved, options.state, current + 1);
            entries.push(entry);
            current = entries.length - 1;
        }
        applyUrl(resolved);
        committedResolve(entry);
        dispatch('currententrychange', { navigationType: navType });

        var runs = intercepts.map(function(h) {
            try { return Promise.resolve(h()); } catch (e) { return Promise.reject(e); }
        });
        Promise.all(runs).then(
            function() { finishedResolve(entry); dispatch('navigatesuccess', {}); },
            function(e) { finishedReject(e); dispatch('navigateerror', { error: e }); }
        );
        return { committed: committed, finished: finished };
    }

    function traverse(targetIndex) {
        if (targetIndex < 0 || targetIndex >= entries.length || targetIndex === current) {
            return settled(entries[current]);
        }
        current = targetIndex;
        applyUrl(entries[current].url);
        dispatch('currententrychange', { navigationType: 'traverse' });
        return settled(entries[current]);
    }

    var navigation = {
        addEventListener: function(type, fn) { (listeners[type] = listeners[type] || []).push(fn); },
        removeEventListener: function(type, fn) {
            var a = listeners[type]; if (!a) return;
            var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
        },
        dispatchEvent: function(ev) { dispatch(ev.type, ev); return true; },
        navigate: navigate,
        reload: function(options) { applyUrl(entries[current].url); return settled(entries[current]); },
        back: function() { return traverse(current - 1); },
        forward: function() { return traverse(current + 1); },
        traverseTo: function(key) {
            for (var i = 0; i < entries.length; i++) if (entries[i].key === key) return traverse(i);
            return settled(entries[current]);
        },
        updateCurrentEntry: function(opts) {
            if (opts && 'state' in opts) entries[current]._state = opts.state;
            dispatch('currententrychange', { navigationType: 'replace' });
        },
        entries: function() { return entries.slice(); }
    };
    Object.defineProperty(navigation, 'currentEntry', { get: function() { return entries[current]; } });
    Object.defineProperty(navigation, 'canGoBack', { get: function() { return current > 0; } });
    Object.defineProperty(navigation, 'canGoForward', { get: function() { return current < entries.length - 1; } });
    Object.defineProperty(navigation, 'transition', { get: function() { return null; } });

    globalThis.navigation = navigation;
    globalThis.NavigationHistoryEntry = NavigationHistoryEntry;
    void 0;
})();
"#;
