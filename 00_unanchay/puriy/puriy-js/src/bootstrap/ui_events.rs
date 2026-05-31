//! Jerarquía base de eventos de UI — `UIEvent`, `MouseEvent`, `FocusEvent`.
//! Fase 7.105.
//!
//! Hasta aquí sólo existían `Event`/`CustomEvent` (Fase 7.25) y los
//! constructores de puntero/táctil (`PointerEvent`/`TouchEvent`, Fase 7.96,
//! que extienden `Event` directamente). Faltaba la columna vertebral de la
//! jerarquía DOM de eventos: `UIEvent` → `MouseEvent` → (`FocusEvent` reusa
//! `UIEvent`). Frameworks y polyfills hacen `new MouseEvent('click', {...})`
//! para disparar clicks sintéticos, feature-detect `'MouseEvent' in window`,
//! y leen `e.getModifierState('Control')`. Sin las clases, `new MouseEvent`
//! tira `is not a constructor` y la rama se cae.
//!
//! Todos **extienden `Event`** (Fase 7.25) vía `globalThis.Event.call(this,
//! type, init)`, igual molde que `PointerEvent` (7.96). El encadenado de
//! prototipos (`MouseEvent.prototype = Object.create(UIEvent.prototype)`) hace
//! que `instanceof` viaje hacia arriba: un `MouseEvent` es `instanceof UIEvent`
//! y `instanceof Event`.
//!
//! **Limitaciones explícitas**:
//! 1. **Sólo constructores** — el chrome NO despacha estos eventos sintéticos
//!    a handlers reales; el dispatch real del usuario sigue por el canal
//!    `__puriy_dispatch` con su propio event object (Fase 7.6/7.9). Estas
//!    clases sirven para construcción programática + feature-detect.
//! 2. **`getModifierState`** sólo resuelve los modificadores clásicos
//!    (`Control`/`Shift`/`Alt`/`Meta`/`AltGraph`); teclas de bloqueo
//!    (`CapsLock`/`NumScroll`) devuelven `false`.
//! 3. **`PointerEvent` (7.96) no se re-encadena** a `MouseEvent` — quedó
//!    definido antes y extiende `Event` directamente; coexisten sin conflicto.

pub(crate) const UI_EVENTS_BOOTSTRAP: &str = r#"
(function(){
  if (typeof globalThis.UIEvent === 'function') return;
  if (typeof globalThis.Event !== 'function') return;

  function num(init, key, def) {
    return (init && typeof init[key] === 'number') ? init[key] : def;
  }

  // ---- UIEvent extends Event ----
  function UIEvent(type, init) {
    init = init || {};
    globalThis.Event.call(this, type, init);
    this.detail = num(init, 'detail', 0);
    this.view = (init.view !== undefined) ? init.view : globalThis;
    this.which = num(init, 'which', 0);
  }
  UIEvent.prototype = Object.create(globalThis.Event.prototype);
  UIEvent.prototype.constructor = UIEvent;

  // ---- MouseEvent extends UIEvent ----
  function MouseEvent(type, init) {
    init = init || {};
    UIEvent.call(this, type, init);
    this.screenX = num(init, 'screenX', 0);
    this.screenY = num(init, 'screenY', 0);
    this.clientX = num(init, 'clientX', 0);
    this.clientY = num(init, 'clientY', 0);
    this.x = this.clientX;
    this.y = this.clientY;
    this.pageX = num(init, 'pageX', this.clientX);
    this.pageY = num(init, 'pageY', this.clientY);
    this.offsetX = num(init, 'offsetX', 0);
    this.offsetY = num(init, 'offsetY', 0);
    this.movementX = num(init, 'movementX', 0);
    this.movementY = num(init, 'movementY', 0);
    this.button = num(init, 'button', 0);
    this.buttons = num(init, 'buttons', 0);
    this.ctrlKey = !!init.ctrlKey;
    this.shiftKey = !!init.shiftKey;
    this.altKey = !!init.altKey;
    this.metaKey = !!init.metaKey;
    this.altGraphKey = !!init.altGraphKey;
    this.relatedTarget = (init.relatedTarget !== undefined) ? init.relatedTarget : null;
  }
  MouseEvent.prototype = Object.create(UIEvent.prototype);
  MouseEvent.prototype.constructor = MouseEvent;
  MouseEvent.prototype.getModifierState = function(key) {
    switch (key) {
      case 'Control': return !!this.ctrlKey;
      case 'Shift': return !!this.shiftKey;
      case 'Alt': return !!this.altKey;
      case 'Meta': return !!this.metaKey;
      case 'AltGraph': return !!this.altGraphKey;
      default: return false;
    }
  };

  // ---- FocusEvent extends UIEvent ----
  function FocusEvent(type, init) {
    init = init || {};
    UIEvent.call(this, type, init);
    this.relatedTarget = (init.relatedTarget !== undefined) ? init.relatedTarget : null;
  }
  FocusEvent.prototype = Object.create(UIEvent.prototype);
  FocusEvent.prototype.constructor = FocusEvent;

  globalThis.UIEvent = UIEvent;
  globalThis.MouseEvent = MouseEvent;
  globalThis.FocusEvent = FocusEvent;
})();
"#;
