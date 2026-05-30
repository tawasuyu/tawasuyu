pub(crate) const BADGING_BOOTSTRAP: &str = r#"
// Fase 7.111 — Badging API (`navigator.setAppBadge` / `clearAppBadge`). Las PWAs
// la usan para poner el contador de no-leídos sobre el ícono de la app (el "3"
// rojo). `setAppBadge(n)` muestra el número; `setAppBadge()` sin arg muestra un
// punto genérico ("flag"); `setAppBadge(0)` o `clearAppBadge()` lo quitan. El
// motor no pinta el ícono: guarda el estado en `__puriy_app_badge` y publica una
// mutación `kind: 'app-badge'` (value `'flag'`/`'clear'`/`<n>`, mismo canal que
// wakelock 7.103) que el chrome rutea al launcher/dock del host. Valor negativo
// o no finito → rechaza `TypeError` (como el spec).
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.setAppBadge != null) return;

    globalThis.__puriy_app_badge = globalThis.__puriy_app_badge || null;

    nav.setAppBadge = function(contents) {
        if (contents == null) {
            globalThis.__puriy_app_badge = 'flag';
            globalThis.__puriy_dirty.push({ id: '__window__', kind: 'app-badge', value: 'flag' });
            return Promise.resolve(undefined);
        }
        var n = Number(contents);
        if (!isFinite(n) || n < 0) {
            return Promise.reject(new TypeError('setAppBadge: valor inválido'));
        }
        n = Math.floor(n);
        // 0 limpia el badge (equivalente a clearAppBadge), igual que el spec.
        globalThis.__puriy_app_badge = (n === 0) ? null : n;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'app-badge', value: (n === 0) ? 'clear' : String(n)
        });
        return Promise.resolve(undefined);
    };
    nav.clearAppBadge = function() {
        globalThis.__puriy_app_badge = null;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'app-badge', value: 'clear' });
        return Promise.resolve(undefined);
    };
})();
"#;
