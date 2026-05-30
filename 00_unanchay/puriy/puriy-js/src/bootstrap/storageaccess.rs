pub(crate) const STORAGE_ACCESS_BOOTSTRAP: &str = r#"
// Fase 7.115 — Storage Access API (`document.requestStorageAccess` /
// `document.hasStorageAccess`). En contextos de terceros (un <iframe> embebido)
// los navegadores bloquean cookies/localStorage por privacidad; esta API deja
// que el embed pida acceso explícito tras un gesto del usuario. El motor no
// tiene partición de almacenamiento real: el estado vive en
// `__puriy_storage_access_state` y el chrome decide vía
// `__puriy_set_storage_access_permission(bool)` (default false — terceros sin
// acceso; mismo patrón synchronous-permission que wakelock 7.103 / mediaDevices
// 7.101). `requestStorageAccess()` publica una mutación `kind: 'storage-access'`
// y resuelve si está permitido (marca `granted` true) o rechaza `NotAllowedError`
// si el chrome lo niega. `hasStorageAccess()` → `Promise<bool>` con el flag
// `granted` actual.
(function() {
    var doc = globalThis.document = globalThis.document || {};
    if (doc.requestStorageAccess != null) return;

    var state = globalThis.__puriy_storage_access_state = globalThis.__puriy_storage_access_state || {
        permitido: false, granted: false
    };

    doc.hasStorageAccess = function() {
        return Promise.resolve(globalThis.__puriy_storage_access_state.granted);
    };
    doc.requestStorageAccess = function() {
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'storage-access', value: '' });
        if (!globalThis.__puriy_storage_access_state.permitido) {
            return Promise.reject(new globalThis.DOMException(
                'requestStorageAccess: acceso denegado', 'NotAllowedError'));
        }
        globalThis.__puriy_storage_access_state.granted = true;
        return Promise.resolve(undefined);
    };

    // Hook de ingreso: el chrome concede o niega el acceso al almacenamiento
    // particionado. Conceder permite que el próximo request resuelva; negar
    // resetea ambos flags.
    globalThis.__puriy_set_storage_access_permission = function(allowed) {
        state.permitido = !!allowed;
        if (!allowed) state.granted = false;
        return true;
    };
})();
"#;
