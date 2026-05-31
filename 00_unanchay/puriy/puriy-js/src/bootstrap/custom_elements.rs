//! `customElements` registry. Fase 7.71.
//!
//! El registro de Web Components. Frameworks y librerías (Lit, FAST,
//! Shoelace, componentes a mano) hacen `customElements.define('mi-tag',
//! MiClase)` al cargar; sin el registro, eso tira `is not defined` y el
//! módulo entero se cae antes de pintar nada.
//!
//! Implementamos el **registro** (define/get/getName/whenDefined/upgrade)
//! sin el **upgrade real** de elementos: el runtime no tiene un DOM vivo
//! conectado a las instancias de los elementos, así que registrar una
//! definición no instancia ni corre `connectedCallback` sobre los `<mi-tag>`
//! del documento. Suficiente para que el código de definición arranque y
//! para `whenDefined(...).then(...)` (patrón de espera de hidratación).
//!
//! **Limitaciones explícitas**:
//! 1. **No upgradea elementos** — los custom elements del HTML inicial no
//!    se instancian con su clase ni reciben `connectedCallback`/`attribute
//!    ChangedCallback`. Cuando el chrome conecte el árbol vivo al runtime,
//!    cablear el upgrade ahí.
//! 2. **`define` valida el nombre** (debe tener un `-`, como exige el spec)
//!    y tira si se redefine o si el constructor ya está registrado.
//! 3. **`upgrade(root)` es no-op**; **`get`/`getName`** consultan el
//!    registro; **`whenDefined`** resuelve al definir (o ya resuelto si
//!    estaba definido).

pub(crate) const CUSTOM_ELEMENTS_BOOTSTRAP: &str = r#"
(function(){
  if (globalThis.customElements) return;

  var defs = {};        // name -> constructor
  var byCtor = [];      // [{ctor, name}] para getName
  var waiters = {};     // name -> { promise, resolve }

  function isValidName(name) {
    // Debe contener un guion y empezar con letra ASCII minúscula (aprox spec).
    return typeof name === 'string' && name.indexOf('-') > 0 && /^[a-z]/.test(name);
  }

  globalThis.customElements = {
    define: function(name, ctor, options) {
      name = String(name);
      if (!isValidName(name)) {
        throw new Error("SyntaxError: nombre de custom element inválido: '" + name + "'");
      }
      if (Object.prototype.hasOwnProperty.call(defs, name)) {
        throw new Error("NotSupportedError: '" + name + "' ya está definido");
      }
      for (var i = 0; i < byCtor.length; i++) {
        if (byCtor[i].ctor === ctor) {
          throw new Error("NotSupportedError: el constructor ya está registrado");
        }
      }
      defs[name] = ctor;
      byCtor.push({ ctor: ctor, name: name });
      // Resolvé el whenDefined pendiente, si lo hay.
      if (waiters[name]) { waiters[name].resolve(ctor); }
    },
    get: function(name) {
      name = String(name);
      return Object.prototype.hasOwnProperty.call(defs, name) ? defs[name] : undefined;
    },
    getName: function(ctor) {
      for (var i = 0; i < byCtor.length; i++) {
        if (byCtor[i].ctor === ctor) return byCtor[i].name;
      }
      return null;
    },
    whenDefined: function(name) {
      name = String(name);
      if (Object.prototype.hasOwnProperty.call(defs, name)) {
        return Promise.resolve(defs[name]);
      }
      if (!waiters[name]) {
        var resolveFn;
        var p = new Promise(function(res){ resolveFn = res; });
        waiters[name] = { promise: p, resolve: resolveFn };
      }
      return waiters[name].promise;
    },
    upgrade: function(_root) { /* no-op: sin DOM vivo conectado */ }
  };
})();
"#;
