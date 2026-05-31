pub(crate) const STORAGE_EVENT_BOOTSTRAP: &str = r#"
// Fase 7.92 — `StorageEvent` + evento `storage`. El spec dispara `storage`
// sobre los OTROS documentos del mismo origen cuando uno cambia
// localStorage/sessionStorage — es como dos pestañas se sincronizan. Con un
// solo runtime por pestaña no hay "otros documentos" acá, así que el evento
// nunca cae solo: el chrome lo entrega vía el hook `__puriy_dispatch_storage`
// cuando otra pestaña del mismo origen escribió. Mismo patrón que online/
// offline (Fase 7.86) y set_connection (Fase 7.89).
//
// `StorageEvent` hereda de `Event` (Fase 7.34) vía cadena de prototipos real.
globalThis.StorageEvent = function(type, init) {
    init = init || {};
    globalThis.Event.call(this, type, init);
    this.key = (init.key != null) ? String(init.key) : null;
    this.oldValue = (init.oldValue != null) ? String(init.oldValue) : null;
    this.newValue = (init.newValue != null) ? String(init.newValue) : null;
    this.url = (init.url != null) ? String(init.url) : '';
    this.storageArea = (init.storageArea != null) ? init.storageArea : null;
};
globalThis.StorageEvent.prototype = Object.create(globalThis.Event.prototype);
globalThis.StorageEvent.prototype.constructor = globalThis.StorageEvent;

// Hook de ingreso: el chrome lo llama cuando otra pestaña del mismo origen
// mutó el storage. `area` ∈ 'local'|'session' selecciona qué storage va en
// `storageArea`. Despacha el StorageEvent REAL (no el dispatch genérico de
// Fase 7.39, que clona props sobre un event plano y perdería el instanceof)
// sobre `window.onstorage` + addEventListener('storage'). Devuelve el count.
globalThis.__puriy_dispatch_storage = function(key, oldValue, newValue, area, url) {
    var storageArea = (area === 'session') ? globalThis.sessionStorage : globalThis.localStorage;
    var event = new globalThis.StorageEvent('storage', {
        key: key,
        oldValue: oldValue,
        newValue: newValue,
        url: (url != null) ? url : ((globalThis.location && globalThis.location.href) || ''),
        storageArea: storageArea
    });
    var count = 0;
    var on = globalThis.onstorage;
    if (typeof on === 'function') {
        try { on.call(globalThis, event); count++; }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    var list = globalThis.__puriy_window_listeners['storage'];
    if (list) {
        var snapshot = list.slice();
        for (var i = 0; i < snapshot.length; i++) {
            var entry = snapshot[i];
            try { entry.fn.call(globalThis, event); count++; }
            catch (e2) { globalThis.__puriy_stderr += String(e2) + '\n'; }
            if (entry.once) {
                var idx = list.indexOf(entry);
                if (idx >= 0) list.splice(idx, 1);
            }
        }
    }
    return count;
};
"#;
