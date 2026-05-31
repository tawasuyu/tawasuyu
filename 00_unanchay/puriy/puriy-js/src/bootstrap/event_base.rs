//! Completa la clase base `Event` (definida mínima en Fase 7.25) con los
//! miembros del spec WHATWG DOM que faltaban. Fase 7.112.
//!
//! `event_class.rs` (7.25) dejó `Event` con lo esencial (`type`/`bubbles`/
//! `cancelable`/`defaultPrevented`/`eventPhase`/`target`/`currentTarget` +
//! `preventDefault`/`stopPropagation`). Faltaban:
//! - **Constantes de fase** `NONE`/`CAPTURING_PHASE`/`AT_TARGET`/
//!   `BUBBLING_PHASE` (estáticas y en el prototipo).
//! - **`composed`** (desde init), **`composedPath()`**, **`isTrusted`**
//!   (siempre `false` para eventos sintéticos), **`timeStamp`**.
//! - **Legacy**: `initEvent(type, bubbles, cancelable)`, `cancelBubble`,
//!   `returnValue`, `srcElement`.
//!
//! **Técnica clave — envolver sin romper `instanceof`**: este módulo corre al
//! final de `bootstrap::ALL`, después de que todos los eventos tipados
//! (7.96/7.105-7.111) hicieron `X.prototype = Object.create(globalThis.Event.
//! prototype)`. Reemplazamos `globalThis.Event` por un wrapper que **reusa el
//! MISMO objeto prototipo** (`Wrapper.prototype = Old.prototype`), así todas
//! las cadenas ya capturadas siguen apuntando al mismo prototipo y
//! `instanceof Event` se mantiene. Como `CustomEvent`/`PointerEvent`/etc.
//! invocan `globalThis.Event.call(this, ...)` en tiempo de ejecución, también
//! reciben los campos nuevos sin tocar sus módulos.
//!
//! **Limitaciones explícitas**:
//! 1. **`composedPath()`** devuelve `[target]` (o `[]`) — sin árbol de
//!    ancestros real ni shadow DOM, no reconstruye la ruta completa.
//! 2. **`isTrusted`** siempre `false` (todo evento aquí es sintético).
//! 3. **`initEvent`** (deprecated) sólo setea `type`/`bubbles`/`cancelable`.

pub(crate) const EVENT_BASE_BOOTSTRAP: &str = r#"
(function(){
  if (typeof globalThis.Event !== 'function') return;
  if (globalThis.Event.__puriy_completed) return;

  var OldEvent = globalThis.Event;

  function Event(type, init) {
    OldEvent.call(this, type, init);
    this.composed = !!(init && init.composed);
    this.isTrusted = false;
    this.timeStamp = (globalThis.performance && typeof globalThis.performance.now === 'function')
      ? globalThis.performance.now() : 0;
  }
  // MISMO prototipo que el viejo → instanceof intacto para cadenas ya creadas.
  Event.prototype = OldEvent.prototype;
  Event.prototype.constructor = Event;
  Event.__puriy_completed = true;

  // Constantes de fase (estáticas + prototipo, como el spec).
  Event.NONE = 0;
  Event.CAPTURING_PHASE = 1;
  Event.AT_TARGET = 2;
  Event.BUBBLING_PHASE = 3;
  Event.prototype.NONE = 0;
  Event.prototype.CAPTURING_PHASE = 1;
  Event.prototype.AT_TARGET = 2;
  Event.prototype.BUBBLING_PHASE = 3;

  // composedPath() — sin árbol real, devolvemos [target] o [].
  // Fase 7.135 — el spec devuelve [] cuando el evento NO se está despachando
  // (path vacío si no hay currentTarget). Antes devolvía [target] siempre,
  // incluso para un `new Event(...)` nunca despachado o ya terminado. Gatear
  // por currentTarget: durante un dispatch sin árbol (EventTarget standalone)
  // sigue siendo [target] (single node); fuera del dispatch, [].
  if (typeof Event.prototype.composedPath !== 'function') {
    Event.prototype.composedPath = function() {
      return this.currentTarget ? [this.target] : [];
    };
  }

  // initEvent(type, bubbles, cancelable) — legacy.
  if (typeof Event.prototype.initEvent !== 'function') {
    Event.prototype.initEvent = function(type, bubbles, cancelable) {
      // Fase 7.158 — no-op si el evento está en vuelo (spec: "if this's
      // dispatch flag is set, then return").
      if (this._dispatch) return;
      this.type = String(type);
      this.bubbles = !!bubbles;
      this.cancelable = !!cancelable;
    };
  }

  // stopImmediatePropagation — por si event_target.rs (7.69) no corrió antes.
  // Fase 7.113 — el flag canónico que leen los dispatch (__puriy_dispatch /
  // __puriy_dispatch_event) es `_immediate`; mantenerlo consistente acá.
  if (typeof Event.prototype.stopImmediatePropagation !== 'function') {
    Event.prototype.stopImmediatePropagation = function() {
      this._stopped = true;
      this._immediate = true;
    };
  }

  // Accessors legacy: cancelBubble ↔ _stopped, returnValue ↔ !defaultPrevented,
  // srcElement ↔ target.
  if (!Object.getOwnPropertyDescriptor(Event.prototype, 'cancelBubble')) {
    Object.defineProperty(Event.prototype, 'cancelBubble', {
      get: function() { return !!this._stopped; },
      set: function(v) { if (v) this._stopped = true; },
      enumerable: false, configurable: true
    });
    Object.defineProperty(Event.prototype, 'returnValue', {
      get: function() { return !this.defaultPrevented; },
      set: function(v) { if (v === false && this.cancelable) this.defaultPrevented = true; },
      enumerable: false, configurable: true
    });
    Object.defineProperty(Event.prototype, 'srcElement', {
      get: function() { return this.target; },
      enumerable: false, configurable: true
    });
  }

  // Re-parenta los eventos "stragglers" que copiaban campos vía
  // `Event.call(this,...)` pero nunca encadenaron su prototipo (CustomEvent
  // 7.25, PointerEvent/TouchEvent 7.96). `setPrototypeOf` preserva sus
  // métodos propios (getCoalescedEvents, etc.) y a la vez los hace
  // `instanceof Event`.
  var stragglers = ['CustomEvent', 'PointerEvent', 'TouchEvent'];
  for (var si = 0; si < stragglers.length; si++) {
    var Ctor = globalThis[stragglers[si]];
    if (typeof Ctor === 'function' && Ctor.prototype &&
        Object.getPrototypeOf(Ctor.prototype) === Object.prototype) {
      Object.setPrototypeOf(Ctor.prototype, Event.prototype);
    }
  }

  globalThis.Event = Event;
})();
"#;
