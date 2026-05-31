pub(crate) const WEBOTP_BOOTSTRAP: &str = r#"
// Fase 7.164 — WebOTP API (`OTPCredential` vía `navigator.credentials.get({otp})`).
// Autocompleta el código de un SMS de verificación (login 2FA) sin que el usuario
// lo tipee. Se monta sobre `navigator.credentials` (Fase 7.110) envolviendo `get`:
// si `options.otp`, rutea a WebOTP (publica `kind: 'webotp'` value `<id>`, Promise
// resuelta por `__puriy_webotp_resolve(id, code)` → `OTPCredential` o rechazada por
// `__puriy_webotp_reject(id, name)` → `AbortError`) — patrón pending-id como
// webauthn 7.128; si no hay `otp`, delega en el `get` previo (webauthn/credentials).
// La recepción real del SMS es del chrome (PENDIENTE — ningún resolve automático).
(function() {
    if (globalThis.OTPCredential != null) return;
    var cc = globalThis.navigator && globalThis.navigator.credentials;
    if (cc == null) return;  // requiere Fase 7.110
    globalThis.__puriy_webotp_pending = globalThis.__puriy_webotp_pending || {};
    globalThis.__puriy_webotp_next_id = globalThis.__puriy_webotp_next_id || 1;

    function OTPCredential(code) {
        if (globalThis.Credential && globalThis.Credential.call) {
            try { globalThis.Credential.call(this); } catch (e) {}
        }
        this.id = '';
        this.type = 'otp';
        this.code = String(code != null ? code : '');
    }
    if (typeof globalThis.Credential === 'function') {
        OTPCredential.prototype = Object.create(globalThis.Credential.prototype);
        OTPCredential.prototype.constructor = OTPCredential;
    }
    globalThis.OTPCredential = OTPCredential;

    var origGet = cc.get.bind(cc);
    cc.get = function(options) {
        if (options && options.otp) {
            var id = globalThis.__puriy_webotp_next_id++;
            globalThis.__puriy_dirty.push({ id: '__window__', kind: 'webotp', value: String(id) });
            return new Promise(function(resolve, reject) {
                globalThis.__puriy_webotp_pending[id] = { resolve: resolve, reject: reject };
            });
        }
        return origGet(options);
    };

    globalThis.__puriy_webotp_resolve = function(id, code) {
        var p = globalThis.__puriy_webotp_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_webotp_pending[id];
        p.resolve(new OTPCredential(code));
        return true;
    };
    globalThis.__puriy_webotp_reject = function(id, name, message) {
        var p = globalThis.__puriy_webotp_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_webotp_pending[id];
        p.reject(new globalThis.DOMException(
            (message != null) ? String(message) : 'WebOTP cancelado',
            (name != null) ? String(name) : 'AbortError'));
        return true;
    };
    void 0;
})();
"#;
