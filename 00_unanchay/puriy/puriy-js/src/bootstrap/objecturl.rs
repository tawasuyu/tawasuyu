pub(crate) const OBJECT_URL_BOOTSTRAP: &str = r#"
// Fase 7.50 — `URL.createObjectURL` / `URL.revokeObjectURL`. Cierra la
// limitación de Fase 7.47 (Blob sin object URLs). Un registro global mapea
// `blob:puriy/<n>` → el objeto (Blob u otro) que se le pasó; `revoke`
// borra la entrada. El chrome resuelve estos URLs (p.ej. `<img src=blob:…>`)
// vía `__puriy_resolve_blob_url(url)` — cableado del lado nativo pendiente.
//
// `URL` se define como objeto sólo si no existe todavía, para no pisar un
// futuro constructor `new URL(href)` (que añadiría los métodos estáticos
// sobre el mismo binding).
globalThis.__puriy_blob_urls = {};
globalThis.__puriy_blob_url_next = 1;
if (typeof globalThis.URL === 'undefined') {
    globalThis.URL = {};
}
globalThis.URL.createObjectURL = function(obj) {
    var id = globalThis.__puriy_blob_url_next++;
    var url = 'blob:puriy/' + id;
    globalThis.__puriy_blob_urls[url] = obj;
    return url;
};
globalThis.URL.revokeObjectURL = function(url) {
    delete globalThis.__puriy_blob_urls[String(url)];
};
// Helper para el chrome: resolver un `blob:` URL al objeto registrado (o
// null si fue revocado / nunca existió).
globalThis.__puriy_resolve_blob_url = function(url) {
    var o = globalThis.__puriy_blob_urls[String(url)];
    return (o == null) ? null : o;
};
"#;
