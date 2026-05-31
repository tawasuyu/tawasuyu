//! Eventos de formulario, descarga y CSP — `FormDataEvent`,
//! `BeforeUnloadEvent`, `SecurityPolicyViolationEvent`. Fase 7.111.
//!
//! Cierra los constructores de eventos que faltaban de la familia abierta en
//! 7.105-7.109. Los tres extienden `Event`:
//! - `FormDataEvent` — evento `formdata` (`formData`, una instancia
//!   `FormData` de Fase 7.62). Lo construyen libs que interceptan submits
//!   para inyectar campos.
//! - `BeforeUnloadEvent` — `beforeunload` (`returnValue`, string mutable;
//!   setearlo dispara el prompt "¿salir?" en navegadores reales).
//! - `SecurityPolicyViolationEvent` — `securitypolicyviolation` del CSP
//!   (`blockedURI`/`violatedDirective`/`effectiveDirective`/`documentURI`/
//!   `disposition`/`statusCode`/...). Telemetría de seguridad lo escucha.
//!
//! **Limitaciones explícitas**:
//! 1. **Sólo constructores** — el chrome no emite estos eventos: no hay submit
//!    real cableado al `formdata`, no hay prompt de unload, no hay enforcement
//!    de CSP que dispare violations. Sirven para construcción programática +
//!    feature-detect.
//! 2. **`BeforeUnloadEvent.returnValue`** se guarda pero no bloquea nada (no
//!    hay navegación que cancelar headless).

pub(crate) const FORM_EVENTS_BOOTSTRAP: &str = r#"
(function(){
  if (typeof globalThis.FormDataEvent === 'function') return;
  if (typeof globalThis.Event !== 'function') return;

  var Event = globalThis.Event;
  function num(init, key, def) {
    return (init && typeof init[key] === 'number') ? init[key] : def;
  }
  function str(init, key, def) {
    return (init && init[key] !== undefined && init[key] !== null) ? String(init[key]) : def;
  }
  function extend(Ctor) {
    Ctor.prototype = Object.create(Event.prototype);
    Ctor.prototype.constructor = Ctor;
  }

  // ---- FormDataEvent extends Event ----
  function FormDataEvent(type, init) {
    init = init || {};
    Event.call(this, type, init);
    this.formData = (init.formData !== undefined) ? init.formData : null;
  }
  extend(FormDataEvent);

  // ---- BeforeUnloadEvent extends Event ----
  function BeforeUnloadEvent(type, init) {
    init = init || {};
    Event.call(this, type || 'beforeunload', init);
    // `returnValue` aquí es un string propio del evento (mutable); shadowea
    // el accessor legacy boolean que Fase 7.112 pone en Event.prototype.
    Object.defineProperty(this, 'returnValue', {
      value: str(init, 'returnValue', ''), writable: true, enumerable: true, configurable: true
    });
  }
  extend(BeforeUnloadEvent);

  // ---- SecurityPolicyViolationEvent extends Event ----
  function SecurityPolicyViolationEvent(type, init) {
    init = init || {};
    Event.call(this, type, init);
    this.documentURI = str(init, 'documentURI', '');
    this.referrer = str(init, 'referrer', '');
    this.blockedURI = str(init, 'blockedURI', '');
    this.violatedDirective = str(init, 'violatedDirective', '');
    this.effectiveDirective = str(init, 'effectiveDirective', '');
    this.originalPolicy = str(init, 'originalPolicy', '');
    this.sourceFile = str(init, 'sourceFile', '');
    this.sample = str(init, 'sample', '');
    this.disposition = str(init, 'disposition', 'enforce');
    this.statusCode = num(init, 'statusCode', 0);
    this.lineNumber = num(init, 'lineNumber', 0);
    this.columnNumber = num(init, 'columnNumber', 0);
  }
  extend(SecurityPolicyViolationEvent);

  globalThis.FormDataEvent = FormDataEvent;
  globalThis.BeforeUnloadEvent = BeforeUnloadEvent;
  globalThis.SecurityPolicyViolationEvent = SecurityPolicyViolationEvent;
})();
"#;
