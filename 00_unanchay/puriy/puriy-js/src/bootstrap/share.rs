pub(crate) const SHARE_BOOTSTRAP: &str = r#"
// Fase 7.97 — Web Share API (`navigator.share` + `navigator.canShare`). Las
// PWAs y sitios móviles la usan para delegar el "compartir" al sistema (mail,
// mensajería, redes). El motor no tiene hoja de share del SO: `share(data)`
// serializa y publica una mutación `kind: 'share'` al chrome, y devuelve una
// Promise que resuelve/rechaza cuando el chrome resuelve la hoja de share con
// los hooks `__puriy_share_resolve(id)` / `__puriy_share_reject(id, name, msg)`
// (mismo patrón pending-id que geolocation/fetch; wiring nativo pendiente).
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    globalThis.__puriy_share_pending = globalThis.__puriy_share_pending || {};
    globalThis.__puriy_share_next_id = globalThis.__puriy_share_next_id || 1;

    function isShareData(data) {
        if (data == null) return false;
        return data.url != null || data.text != null || data.title != null ||
            (data.files != null && data.files.length > 0);
    }

    // canShare(data): true sólo si data es compartible. Sin args → false
    // (no hay nada que compartir), igual que Chrome.
    nav.canShare = function(data) {
        return isShareData(data);
    };
    nav.share = function(data) {
        if (!isShareData(data)) {
            return Promise.reject(new TypeError('navigator.share requiere title, text, url o files'));
        }
        var id = globalThis.__puriy_share_next_id++;
        var payload = {
            title: (data.title != null) ? String(data.title) : '',
            text: (data.text != null) ? String(data.text) : '',
            url: (data.url != null) ? String(data.url) : ''
        };
        // U+001D (Group Separator) separa el id del payload — mismo separador de
        // control que usa el canal fetch (Fase 7.34).
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'share',
            value: id + '' + JSON.stringify(payload)
        });
        return new Promise(function(resolve, reject) {
            globalThis.__puriy_share_pending[id] = { resolve: resolve, reject: reject };
        });
    };

    globalThis.__puriy_share_resolve = function(id) {
        var p = globalThis.__puriy_share_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_share_pending[id];
        p.resolve(undefined);
        return true;
    };
    globalThis.__puriy_share_reject = function(id, name, message) {
        var p = globalThis.__puriy_share_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_share_pending[id];
        p.reject(new globalThis.DOMException(
            (message != null) ? String(message) : 'Share canceled',
            (name != null) ? String(name) : 'AbortError'
        ));
        return true;
    };
})();
"#;
