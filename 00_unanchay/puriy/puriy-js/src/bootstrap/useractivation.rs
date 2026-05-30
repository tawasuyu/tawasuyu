pub(crate) const USER_ACTIVATION_BOOTSTRAP: &str = r#"
// Fase 7.106 — `navigator.userActivation` (User Activation API). Las páginas la
// consultan para saber si tienen "activación transitoria" (un gesto reciente del
// usuario) antes de llamar APIs que la exigen (autoplay con sonido, abrir popup,
// clipboard.write, etc.). Dos flags read-only: `isActive` (transitoria: hay un
// gesto vigente, expira con el tiempo) y `hasBeenActive` (sticky: hubo al menos
// un gesto en la vida del documento). Host-driven (mismo patrón que visibility/
// screen): el estado vive en `__puriy_user_activation_state` y el chrome lo
// flippea con `__puriy_set_user_activation(bool)` cuando llega un click/tecla.
// Activar pone isActive=true y marca hasBeenActive permanentemente; desactivar
// (expiración de la ventana transitoria) sólo baja isActive.
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.userActivation != null) return;

    var state = globalThis.__puriy_user_activation_state = globalThis.__puriy_user_activation_state || {
        isActive: false, hasBeenActive: false
    };

    function UserActivation() {}
    Object.defineProperty(UserActivation.prototype, 'isActive', {
        get: function() { return globalThis.__puriy_user_activation_state.isActive; },
        enumerable: true, configurable: true
    });
    Object.defineProperty(UserActivation.prototype, 'hasBeenActive', {
        get: function() { return globalThis.__puriy_user_activation_state.hasBeenActive; },
        enumerable: true, configurable: true
    });
    globalThis.UserActivation = UserActivation;
    nav.userActivation = new UserActivation();

    globalThis.__puriy_set_user_activation = function(active) {
        var s = globalThis.__puriy_user_activation_state;
        s.isActive = !!active;
        if (active) s.hasBeenActive = true;
        return true;
    };
})();
"#;
