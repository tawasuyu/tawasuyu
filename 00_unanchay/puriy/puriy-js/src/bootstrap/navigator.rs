pub(crate) const NAVIGATOR_BOOTSTRAP: &str = r#"
// Fase 7.68 — `navigator` mínimo + `navigator.sendBeacon`. sendBeacon es la
// vía fire-and-forget para telemetría/analytics: encola un POST y devuelve
// inmediatamente (sin Promise ni response). Reutiliza el mismo canal de
// mutación `kind: 'fetch'` que fetch()/XHR (ver fetch.rs), pero NO registra
// pending — el `resolve_fetch` del chrome hace no-op si el id no tiene
// handler. El body se serializa con el mismo `__puriy_serialize_body`.
globalThis.navigator = globalThis.navigator || {};
if (globalThis.navigator.userAgent == null) {
    globalThis.navigator.userAgent = 'Puriy/0.1 (gioser)';
}
if (globalThis.navigator.onLine == null) {
    globalThis.navigator.onLine = true;
}
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
"#;
