pub(crate) const CREDENTIALS_BOOTSTRAP: &str = r#"
// Fase 7.110 — `navigator.credentials` (Credential Management API). Los sitios la
// usan para el login automático: `get()` pide al navegador una credencial
// guardada (password/federada), `store()` ofrece guardar una tras un login
// exitoso, `create()` forja una nueva, `preventSilentAccess()` apaga el auto
// sign-in. El motor no tiene baúl de credenciales: get/store/create publican una
// mutación `kind: 'credentials'` (value `<id>:<op>:<json>`) y devuelven una
// Promise pendiente que el chrome resuelve con `__puriy_credentials_resolve(id, data)`
// (data null = sin credencial, o `{type, ...}` para construir el Credential) o
// rechaza con `__puriy_credentials_reject(id, name, msg)` — patrón pending-id
// como share 7.97 / serviceWorker 7.100. `preventSilentAccess()` resuelve ya.
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.credentials != null) return;

    globalThis.__puriy_credentials_pending = globalThis.__puriy_credentials_pending || {};
    globalThis.__puriy_credentials_next_id = globalThis.__puriy_credentials_next_id || 1;

    function Credential(init) {
        init = init || {};
        this.id = String(init.id != null ? init.id : '');
        this.type = String(init.type != null ? init.type : '');
    }
    function PasswordCredential(init) {
        init = init || {};
        Credential.call(this, { id: init.id, type: 'password' });
        this.name = init.name != null ? String(init.name) : '';
        this.password = init.password != null ? String(init.password) : '';
        this.iconURL = init.iconURL != null ? String(init.iconURL) : '';
    }
    PasswordCredential.prototype = Object.create(Credential.prototype);
    PasswordCredential.prototype.constructor = PasswordCredential;
    function FederatedCredential(init) {
        init = init || {};
        Credential.call(this, { id: init.id, type: 'federated' });
        this.name = init.name != null ? String(init.name) : '';
        this.provider = init.provider != null ? String(init.provider) : '';
    }
    FederatedCredential.prototype = Object.create(Credential.prototype);
    FederatedCredential.prototype.constructor = FederatedCredential;
    globalThis.Credential = Credential;
    globalThis.PasswordCredential = PasswordCredential;
    globalThis.FederatedCredential = FederatedCredential;

    function publicar(op, detail) {
        return new Promise(function(resolve, reject) {
            var id = globalThis.__puriy_credentials_next_id++;
            globalThis.__puriy_credentials_pending[id] = { resolve: resolve, reject: reject };
            var json;
            try { json = JSON.stringify(detail || {}); } catch (e) { json = '{}'; }
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'credentials', value: id + ':' + op + ':' + json
            });
        });
    }

    function CredentialsContainer() {}
    CredentialsContainer.prototype.get = function(options) { return publicar('get', options || {}); };
    CredentialsContainer.prototype.store = function(cred) {
        return publicar('store', cred ? { id: cred.id, type: cred.type } : {});
    };
    CredentialsContainer.prototype.create = function(options) { return publicar('create', options || {}); };
    CredentialsContainer.prototype.preventSilentAccess = function() { return Promise.resolve(undefined); };
    globalThis.CredentialsContainer = CredentialsContainer;
    nav.credentials = new CredentialsContainer();

    globalThis.__puriy_credentials_resolve = function(id, data) {
        var p = globalThis.__puriy_credentials_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_credentials_pending[id];
        var cred = null;
        if (data != null) {
            if (data.type === 'password') cred = new PasswordCredential(data);
            else if (data.type === 'federated') cred = new FederatedCredential(data);
            else cred = new Credential(data);
        }
        p.resolve(cred);
        return true;
    };
    globalThis.__puriy_credentials_reject = function(id, name, msg) {
        var p = globalThis.__puriy_credentials_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_credentials_pending[id];
        p.reject(new globalThis.DOMException(msg || 'Credentials error', name || 'NotAllowedError'));
        return true;
    };
})();
"#;
