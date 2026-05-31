pub(crate) const TRUSTEDTYPES_BOOTSTRAP: &str = r#"
// Fase 7.135 — Trusted Types (`trustedTypes` + `TrustedHTML`/`TrustedScript`/
// `TrustedScriptURL`). Frameworks que respetan CSP `require-trusted-types-for` envuelven
// todo sink peligroso (innerHTML, script.src, eval) en un valor "confiable" producido por
// una policy. Implementación 100% JS-puro y funcional (no necesita chrome): cada policy
// expone create{HTML,Script,ScriptURL} que corre la regla del usuario y envuelve el string
// en el wrapper correspondiente; los wrappers no son construibles directo (Illegal
// constructor) y su toString() devuelve el string crudo. `getAttributeType`/`getPropertyType`
// quedan mínimos (null) — no modelamos el mapa sink→tipo del spec todavía.
(function() {
    if (globalThis.trustedTypes != null) return;

    // Los wrappers no se construyen directo (spec: Illegal constructor). El acuñado interno
    // usa Object.create para no pasar por el constructor.
    function illegal() { throw new TypeError('Illegal constructor'); }
    function TrustedHTML() { illegal(); }
    function TrustedScript() { illegal(); }
    function TrustedScriptURL() { illegal(); }
    function defToString(Ctor) {
        Ctor.prototype.toString = function() { return this._v; };
        Ctor.prototype.toJSON = function() { return this._v; };
        Ctor.prototype.valueOf = function() { return this._v; };
    }
    defToString(TrustedHTML);
    defToString(TrustedScript);
    defToString(TrustedScriptURL);
    function mint(Ctor, s) {
        var o = Object.create(Ctor.prototype);
        o._v = (s == null) ? '' : String(s);
        return o;
    }

    var policies = [];
    var defaultPolicy = null;

    function makeMethod(name, rules, Ctor) {
        return function(input) {
            if (typeof rules[name] !== 'function') {
                throw new TypeError("La policy no especificó un miembro '" + name + "'.");
            }
            return mint(Ctor, rules[name].apply(null, arguments));
        };
    }

    var factory = {};
    factory.createPolicy = function(name, rules) {
        rules = rules || {};
        var policy = { name: name };
        policy.createHTML = makeMethod('createHTML', rules, TrustedHTML);
        policy.createScript = makeMethod('createScript', rules, TrustedScript);
        policy.createScriptURL = makeMethod('createScriptURL', rules, TrustedScriptURL);
        policies.push(policy);
        if (name === 'default') defaultPolicy = policy;
        return policy;
    };
    factory.isHTML = function(v) { return v instanceof TrustedHTML; };
    factory.isScript = function(v) { return v instanceof TrustedScript; };
    factory.isScriptURL = function(v) { return v instanceof TrustedScriptURL; };
    factory.getAttributeType = function() { return null; };
    factory.getPropertyType = function() { return null; };
    Object.defineProperty(factory, 'emptyHTML', { get: function() { return mint(TrustedHTML, ''); } });
    Object.defineProperty(factory, 'emptyScript', { get: function() { return mint(TrustedScript, ''); } });
    Object.defineProperty(factory, 'defaultPolicy', { get: function() { return defaultPolicy; } });

    globalThis.trustedTypes = factory;
    globalThis.TrustedHTML = TrustedHTML;
    globalThis.TrustedScript = TrustedScript;
    globalThis.TrustedScriptURL = TrustedScriptURL;
    globalThis.TrustedTypePolicyFactory = function() { illegal(); };
    void 0;
})();
"#;
