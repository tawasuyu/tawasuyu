pub(crate) const STORAGEMANAGER_BOOTSTRAP: &str = r#"
// Fase 7.104 — `navigator.storage` (StorageManager API). Las apps offline la
// usan para saber cuánto espacio tienen (`estimate()`) y para pedir
// almacenamiento persistente que el navegador no desaloje (`persist()`). El
// motor no maneja cuota real: el estado vive en `__puriy_storage_state` y el
// chrome lo setea con `__puriy_set_storage_estimate({usage, quota})` /
// `__puriy_set_storage_persisted(bool)` (mismo patrón host-driven que
// screen 7.99). `getDirectory()` (OPFS) rechaza `SecurityError` — no hay sandbox
// de filesystem todavía. Defaults: 0 usado, 2 GB de cuota, no persistente.
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.storage != null) return;

    var state = globalThis.__puriy_storage_state = globalThis.__puriy_storage_state || {
        usage: 0, quota: 2 * 1024 * 1024 * 1024, persistido: false
    };

    function StorageManager() {}
    StorageManager.prototype.estimate = function() {
        var s = globalThis.__puriy_storage_state;
        return Promise.resolve({ usage: s.usage, quota: s.quota, usageDetails: {} });
    };
    StorageManager.prototype.persisted = function() {
        return Promise.resolve(globalThis.__puriy_storage_state.persistido);
    };
    StorageManager.prototype.persist = function() {
        // El chrome decide; acá devolvemos el estado actual (sin prompt).
        return Promise.resolve(globalThis.__puriy_storage_state.persistido);
    };
    StorageManager.prototype.getDirectory = function() {
        return Promise.reject(new globalThis.DOMException(
            'OPFS no soportado', 'SecurityError'));
    };
    globalThis.StorageManager = StorageManager;
    nav.storage = new StorageManager();

    globalThis.__puriy_set_storage_estimate = function(data) {
        if (data == null) return false;
        if (data.usage != null) state.usage = Number(data.usage);
        if (data.quota != null) state.quota = Number(data.quota);
        return true;
    };
    globalThis.__puriy_set_storage_persisted = function(persisted) {
        state.persistido = !!persisted;
        return true;
    };
})();
"#;
