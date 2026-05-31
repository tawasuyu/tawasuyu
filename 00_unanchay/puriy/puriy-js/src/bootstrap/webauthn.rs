pub(crate) const WEBAUTHN_BOOTSTRAP: &str = r#"
// Fase 7.128 — Web Authentication API (WebAuthn). Login sin contraseña con passkeys /
// llaves de seguridad (FIDO2). Se monta sobre `navigator.credentials` (Fase 7.110):
// `create({publicKey})` registra una credencial nueva (attestation) y `get({publicKey})`
// autentica (assertion). El motor no tiene autenticador real: ambos publican una mutación
// (`kind: 'webauthn-create'`/`'webauthn-get'`, value `<id>`) y devuelven una Promise
// pendiente que el chrome resuelve con `__puriy_webauthn_resolve(id, data)` (construye un
// `PublicKeyCredential` con su `AuthenticatorAttestationResponse`/`AssertionResponse`) o
// rechaza con `__puriy_webauthn_reject(id, name)` (`NotAllowedError` si el usuario cancela) —
// patrón pending-id como credentials 7.110. Cuando `options.publicKey` no está, delega en
// el create/get original (password/federated). Los estáticos
// `isUserVerifyingPlatformAuthenticatorAvailable()` / `isConditionalMediationAvailable()`
// son host-decided (default false; el chrome los habilita con `__puriy_set_uvpaa(bool)`).
(function() {
    if (globalThis.PublicKeyCredential != null) return;

    globalThis.__puriy_webauthn_pending = globalThis.__puriy_webauthn_pending || {};
    globalThis.__puriy_webauthn_next_id = globalThis.__puriy_webauthn_next_id || 1;
    var uvpaa = false;
    var condMediation = false;

    function AuthenticatorResponse(init) {
        init = init || {};
        this.clientDataJSON = init.clientDataJSON != null ? init.clientDataJSON : '';
    }
    function AuthenticatorAttestationResponse(init) {
        init = init || {};
        AuthenticatorResponse.call(this, init);
        this.attestationObject = init.attestationObject != null ? init.attestationObject : '';
    }
    AuthenticatorAttestationResponse.prototype = Object.create(AuthenticatorResponse.prototype);
    AuthenticatorAttestationResponse.prototype.constructor = AuthenticatorAttestationResponse;
    function AuthenticatorAssertionResponse(init) {
        init = init || {};
        AuthenticatorResponse.call(this, init);
        this.authenticatorData = init.authenticatorData != null ? init.authenticatorData : '';
        this.signature = init.signature != null ? init.signature : '';
        this.userHandle = init.userHandle != null ? init.userHandle : null;
    }
    AuthenticatorAssertionResponse.prototype = Object.create(AuthenticatorResponse.prototype);
    AuthenticatorAssertionResponse.prototype.constructor = AuthenticatorAssertionResponse;

    function PublicKeyCredential(init, op) {
        init = init || {};
        this.id = String(init.id != null ? init.id : '');
        this.rawId = init.rawId != null ? init.rawId : this.id;
        this.type = 'public-key';
        this.authenticatorAttachment = init.authenticatorAttachment != null
            ? String(init.authenticatorAttachment) : null;
        if (op === 'get') {
            this.response = new AuthenticatorAssertionResponse(init.response);
        } else {
            this.response = new AuthenticatorAttestationResponse(init.response);
        }
    }
    PublicKeyCredential.prototype.getClientExtensionResults = function() { return {}; };
    PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable = function() {
        return Promise.resolve(uvpaa);
    };
    PublicKeyCredential.isConditionalMediationAvailable = function() {
        return Promise.resolve(condMediation);
    };

    globalThis.AuthenticatorResponse = AuthenticatorResponse;
    globalThis.AuthenticatorAttestationResponse = AuthenticatorAttestationResponse;
    globalThis.AuthenticatorAssertionResponse = AuthenticatorAssertionResponse;
    globalThis.PublicKeyCredential = PublicKeyCredential;

    function publicar(op) {
        return new Promise(function(resolve, reject) {
            var id = globalThis.__puriy_webauthn_next_id++;
            globalThis.__puriy_webauthn_pending[id] = { resolve: resolve, reject: reject, op: op };
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'webauthn-' + op, value: String(id)
            });
        });
    }

    // Envuelve create/get de navigator.credentials (Fase 7.110) para interceptar
    // las peticiones publicKey; el resto delega en el original.
    var cc = globalThis.navigator && globalThis.navigator.credentials;
    if (cc) {
        var origCreate = cc.create.bind(cc);
        var origGet = cc.get.bind(cc);
        cc.create = function(options) {
            if (options && options.publicKey) return publicar('create');
            return origCreate(options);
        };
        cc.get = function(options) {
            if (options && options.publicKey) return publicar('get');
            return origGet(options);
        };
    }

    globalThis.__puriy_webauthn_resolve = function(id, data) {
        var p = globalThis.__puriy_webauthn_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_webauthn_pending[id];
        p.resolve(new PublicKeyCredential(data || {}, p.op));
        return true;
    };
    globalThis.__puriy_webauthn_reject = function(id, name, message) {
        var p = globalThis.__puriy_webauthn_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_webauthn_pending[id];
        p.reject(new globalThis.DOMException(
            (message != null) ? String(message) : 'WebAuthn cancelado',
            (name != null) ? String(name) : 'NotAllowedError'));
        return true;
    };
    globalThis.__puriy_set_uvpaa = function(flag) { uvpaa = !!flag; return true; };
    globalThis.__puriy_set_conditional_mediation = function(flag) { condMediation = !!flag; return true; };
    void 0;
})();
"#;
