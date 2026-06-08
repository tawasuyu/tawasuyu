pub(crate) const NAVIGATOR_BOOTSTRAP: &str = r#"
// Fase 7.68 — `navigator` mínimo + `navigator.sendBeacon`. sendBeacon es la
// vía fire-and-forget para telemetría/analytics: encola un POST y devuelve
// inmediatamente (sin Promise ni response). Reutiliza el mismo canal de
// mutación `kind: 'fetch'` que fetch()/XHR (ver fetch.rs), pero NO registra
// pending — el `resolve_fetch` del chrome hace no-op si el id no tiene
// handler. El body se serializa con el mismo `__puriy_serialize_body`.
globalThis.navigator = globalThis.navigator || {};
if (globalThis.navigator.userAgent == null) {
    globalThis.navigator.userAgent = 'Puriy/0.1 (tawasuyu)';
}
if (globalThis.navigator.onLine == null) {
    globalThis.navigator.onLine = true;
}
// Fase 7.85 — props de feature-detection que las libs leen constantemente para
// decidir capacidades/locale. Sólo se setean si faltan (no pisan un valor que
// el host haya inyectado antes). Los tres legacy (appCodeName/appName/product)
// son CONSTANTES literales que el spec obliga a devolver en TODO browser
// ('Mozilla'/'Netscape'/'Gecko') — scripts viejos las chequean a ciegas.
var nav = globalThis.navigator;
if (nav.language == null) nav.language = 'es-ES';
if (nav.languages == null) nav.languages = ['es-ES', 'es', 'en'];
if (nav.platform == null) nav.platform = 'Linux x86_64';
if (nav.hardwareConcurrency == null) nav.hardwareConcurrency = 4;
if (nav.cookieEnabled == null) nav.cookieEnabled = true;
if (nav.maxTouchPoints == null) nav.maxTouchPoints = 0;
if (nav.vendor == null) nav.vendor = '';
if (nav.doNotTrack == null) nav.doNotTrack = null;
if (nav.appCodeName == null) nav.appCodeName = 'Mozilla';
if (nav.appName == null) nav.appName = 'Netscape';
if (nav.product == null) nav.product = 'Gecko';
if (nav.appVersion == null) nav.appVersion = '5.0 (Linux x86_64)';
globalThis.navigator.sendBeacon = function(url, data) {
    if (url == null) throw new TypeError('sendBeacon: url requerida');
    var ser = globalThis.__puriy_serialize_body((data != null) ? data : null);
    var hdr_pairs = [];
    globalThis.__puriy_apply_content_type(hdr_pairs, ser.contentType);
    var base = (globalThis.location && globalThis.location.href) || '';
    var resolved = globalThis.__puriy_resolve_url(String(url), base);
    var id = globalThis.__puriy_fetch_next_id++;
    // sendBeacon siempre es POST. Mismo formato de payload que fetch: campos
    // separados por U+001D (Group Separator) y los pares de header aplanados
    // y unidos por U+001F (Unit Separator). Sin pending → fire-and-forget.
    var GS = String.fromCharCode(0x1D);
    var US = String.fromCharCode(0x1F);
    var payload = String(id) + GS + 'POST' + GS + resolved
                + GS + (ser.hasBody ? '1' : '0')
                + GS + ser.text
                + GS + hdr_pairs.join(US);
    globalThis.__puriy_dirty.push({ id: '__window__', kind: 'fetch', value: payload });
    // El spec devuelve false si el user agent no pudo encolar (p. ej. cuota
    // de beacon excedida); acá siempre encolamos, así que true.
    return true;
};
// Fase 7.86 — eventos `online`/`offline`. El chrome llama a este hook cuando
// la conectividad cambia (p. ej. la red del host cae). Actualiza
// `navigator.onLine` y dispara el evento correspondiente sobre window — donde
// `window.ononline`/`onoffline` y `addEventListener('online'|'offline')` ya lo
// recogen vía el dispatch genérico (Fase 7.39). No-op si el estado no cambió.
globalThis.__puriy_set_online = function(online) {
    var next = !!online;
    if (globalThis.navigator.onLine === next) return false;
    globalThis.navigator.onLine = next;
    globalThis.__puriy_dispatch_window(next ? 'online' : 'offline', null);
    return true;
};
"#;
