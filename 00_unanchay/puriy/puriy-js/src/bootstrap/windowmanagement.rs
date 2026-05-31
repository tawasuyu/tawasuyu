pub(crate) const WINDOWMANAGEMENT_BOOTSTRAP: &str = r#"
// Fase 7.162 — Window Management API (`window.getScreenDetails()` + `ScreenDetails`/
// `ScreenDetailed` + `screen.isExtended`). Permite a una app multi-ventana saber
// cuántos monitores hay y dónde, para abrir ventanas en pantallas concretas
// (presentaciones, dashboards). Cuelga de `screen` (Fase 7.x) + `EventTarget`.
// Host-driven: la topología real de monitores la pone el chrome con
// `__puriy_set_screen_details([...])`; por defecto hay un único monitor derivado
// de `window.screen`. Gated-by-permission (`window-management`): por defecto
// concedido, el chrome lo flippea con `__puriy_set_window_management_permission`.
(function() {
    if (typeof globalThis.getScreenDetails === 'function') return;
    if (globalThis.__puriy_window_management_permission == null) {
        globalThis.__puriy_window_management_permission = true;
    }

    function ScreenDetailed(info) {
        info = info || {};
        var sc = globalThis.screen || {};
        function pick(k, dflt) { return info[k] != null ? info[k] : dflt; }
        this.availWidth = pick('availWidth', sc.availWidth != null ? sc.availWidth : 1920);
        this.availHeight = pick('availHeight', sc.availHeight != null ? sc.availHeight : 1080);
        this.width = pick('width', sc.width != null ? sc.width : 1920);
        this.height = pick('height', sc.height != null ? sc.height : 1080);
        this.availLeft = pick('availLeft', 0);
        this.availTop = pick('availTop', 0);
        this.left = pick('left', 0);
        this.top = pick('top', 0);
        this.colorDepth = pick('colorDepth', sc.colorDepth != null ? sc.colorDepth : 24);
        this.pixelDepth = pick('pixelDepth', this.colorDepth);
        this.isPrimary = !!pick('isPrimary', true);
        this.isInternal = !!pick('isInternal', true);
        this.devicePixelRatio = pick('devicePixelRatio', globalThis.devicePixelRatio != null ? globalThis.devicePixelRatio : 1);
        this.label = String(pick('label', ''));
    }
    globalThis.ScreenDetailed = ScreenDetailed;

    function buildList() {
        var raw = globalThis.__puriy_screen_details_list;
        if (Array.isArray(raw) && raw.length) {
            return raw.map(function(i) { return new ScreenDetailed(i); });
        }
        return [new ScreenDetailed({})];
    }

    function ScreenDetails(list) {
        var et = new globalThis.EventTarget();
        for (var m in et) { if (typeof et[m] === 'function') this[m] = et[m].bind(et); }
        this.screens = list;
        this.currentScreen = list[0] || null;
        this.oncurrentscreenchange = null;
        this.onscreenschange = null;
    }
    globalThis.ScreenDetails = ScreenDetails;

    globalThis.getScreenDetails = function() {
        if (!globalThis.__puriy_window_management_permission) {
            return Promise.reject(new globalThis.DOMException('Permission denied', 'NotAllowedError'));
        }
        return Promise.resolve(new ScreenDetails(buildList()));
    };
    if (typeof globalThis.window === 'object' && globalThis.window) {
        globalThis.window.getScreenDetails = globalThis.getScreenDetails;
    }

    // `screen.isExtended` — true cuando hay más de un monitor.
    if (globalThis.screen != null) {
        Object.defineProperty(globalThis.screen, 'isExtended', {
            configurable: true,
            get: function() {
                var raw = globalThis.__puriy_screen_details_list;
                return Array.isArray(raw) && raw.length > 1;
            }
        });
    }

    globalThis.__puriy_set_screen_details = function(list) {
        globalThis.__puriy_screen_details_list = Array.isArray(list) ? list.slice() : [];
        return true;
    };
    globalThis.__puriy_set_window_management_permission = function(ok) {
        globalThis.__puriy_window_management_permission = !!ok;
        return true;
    };
    void 0;
})();
"#;
