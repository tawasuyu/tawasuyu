pub(crate) const CONTACTS_BOOTSTRAP: &str = r#"
// Fase 7.118 — Contact Picker API (`navigator.contacts`). Formularios web la usan para
// que el usuario elija contactos de su agenda sin dar acceso completo. El motor no tiene
// agenda: `select(props, opts)` publica una mutación `kind: 'contacts-select'` al chrome
// (mismo canal que share 7.97 / eyedropper 7.116) y devuelve una Promise que resuelve con
// un array de contactos cuando el chrome elige vía `__puriy_contacts_resolve(id, list)`, o
// rechaza con `AbortError` si el usuario cancela vía `__puriy_contacts_reject(id, name)`
// — mismo patrón pending-id. `getProperties()` lista las propiedades soportadas.
(function() {
    if (globalThis.navigator == null) globalThis.navigator = {};
    var nav = globalThis.navigator;
    if (nav.contacts != null) return;

    var SUPPORTED = ['name', 'email', 'tel', 'address', 'icon'];

    globalThis.__puriy_contacts_pending = globalThis.__puriy_contacts_pending || {};
    globalThis.__puriy_contacts_next_id = globalThis.__puriy_contacts_next_id || 1;

    var contacts = {};

    contacts.getProperties = function() {
        return Promise.resolve(SUPPORTED.slice());
    };

    contacts.select = function(properties, options) {
        if (!Array.isArray(properties) || properties.length === 0) {
            return Promise.reject(new TypeError('contacts.select requiere propiedades'));
        }
        var id = globalThis.__puriy_contacts_next_id++;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'contacts-select', value: String(id)
        });
        return new Promise(function(resolve, reject) {
            globalThis.__puriy_contacts_pending[id] = { resolve: resolve, reject: reject };
        });
    };

    globalThis.__puriy_contacts_resolve = function(id, list) {
        var p = globalThis.__puriy_contacts_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_contacts_pending[id];
        p.resolve(Array.isArray(list) ? list : []);
        return true;
    };
    globalThis.__puriy_contacts_reject = function(id, name, message) {
        var p = globalThis.__puriy_contacts_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_contacts_pending[id];
        p.reject(new globalThis.DOMException(
            (message != null) ? String(message) : 'Contact picker canceled',
            (name != null) ? String(name) : 'AbortError'));
        return true;
    };

    nav.contacts = contacts;
    void 0;
})();
"#;
