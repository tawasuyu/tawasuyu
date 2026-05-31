//! Eventos de teclado / edición / rueda — `KeyboardEvent`, `InputEvent`,
//! `CompositionEvent`, `WheelEvent`. Fase 7.106.
//!
//! Continúa la jerarquía abierta en Fase 7.105 (`UIEvent`/`MouseEvent`):
//! - `KeyboardEvent` extiende `UIEvent` — `key`/`code`/`location`/modifiers,
//!   `getModifierState`, constantes `DOM_KEY_LOCATION_*`. Los editores y
//!   atajos hacen `new KeyboardEvent('keydown', { key: 'a', ctrlKey: true })`.
//! - `InputEvent` extiende `UIEvent` — `data`/`inputType`/`isComposing`,
//!   `getTargetRanges()`. Lo emite el navegador real al teclear; las libs
//!   de edición rica lo construyen sintético.
//! - `CompositionEvent` extiende `UIEvent` — `data` (IME / dead keys).
//! - `WheelEvent` extiende `MouseEvent` (7.105) — `deltaX/Y/Z`/`deltaMode`,
//!   constantes `DOM_DELTA_*`. Mapas, sliders y editores de código lo usan.
//!
//! Depende de que `UI_EVENTS_BOOTSTRAP` (7.105) corra antes — `WheelEvent`
//! encadena su prototipo a `MouseEvent.prototype`.
//!
//! **Limitaciones explícitas**:
//! 1. **Sólo constructores** — el dispatch real del teclado del usuario sigue
//!    por `__puriy_dispatch` con su event object (Fase 7.9), no por estas
//!    clases.
//! 2. **`keyCode`/`charCode`/`which`** son legacy. Fase 7.161 — si el init no
//!    los pasa pero sí trae `key`, derivamos `keyCode`/`which` vía la tabla
//!    US-layout `__puriy_key_to_keycode` (7.159), igual que el chrome (7.160).
//!    `charCode` queda `0` salvo que venga explícito (sólo aplica a keypress).
//! 3. **`getTargetRanges()`** → `[]` (no exponemos StaticRange del modelo).

pub(crate) const KEYBOARD_EVENTS_BOOTSTRAP: &str = r#"
(function(){
  if (typeof globalThis.KeyboardEvent === 'function') return;
  if (typeof globalThis.UIEvent !== 'function') return;

  var UIEvent = globalThis.UIEvent;
  function num(init, key, def) {
    return (init && typeof init[key] === 'number') ? init[key] : def;
  }
  function str(init, key, def) {
    return (init && init[key] !== undefined && init[key] !== null) ? String(init[key]) : def;
  }
  function modifierState(self, key) {
    switch (key) {
      case 'Control': return !!self.ctrlKey;
      case 'Shift': return !!self.shiftKey;
      case 'Alt': return !!self.altKey;
      case 'Meta': return !!self.metaKey;
      case 'AltGraph': return !!self.altGraphKey;
      default: return false;
    }
  }

  // ---- KeyboardEvent extends UIEvent ----
  function KeyboardEvent(type, init) {
    init = init || {};
    UIEvent.call(this, type, init);
    this.key = str(init, 'key', '');
    this.code = str(init, 'code', '');
    this.location = num(init, 'location', 0);
    this.ctrlKey = !!init.ctrlKey;
    this.shiftKey = !!init.shiftKey;
    this.altKey = !!init.altKey;
    this.metaKey = !!init.metaKey;
    this.altGraphKey = !!init.altGraphKey;
    this.repeat = !!init.repeat;
    this.isComposing = !!init.isComposing;
    this.keyCode = num(init, 'keyCode', 0);
    this.charCode = num(init, 'charCode', 0);
    // Fase 7.161 — si el init no trajo keyCode pero sí `key`, derivá el legacy
    // (layout US) vía el helper de dom_events (7.159). `new KeyboardEvent(
    // 'keydown', {key:'Enter'})` deja `keyCode === 13` sin que el autor lo
    // enumere, igual que el evento del chrome (7.160).
    if (!this.keyCode && this.key &&
        typeof globalThis.__puriy_key_to_keycode === 'function') {
      this.keyCode = globalThis.__puriy_key_to_keycode(this.key, this.code);
    }
    this.which = num(init, 'which', this.keyCode);
  }
  KeyboardEvent.prototype = Object.create(UIEvent.prototype);
  KeyboardEvent.prototype.constructor = KeyboardEvent;
  KeyboardEvent.prototype.getModifierState = function(key) { return modifierState(this, key); };
  KeyboardEvent.DOM_KEY_LOCATION_STANDARD = 0;
  KeyboardEvent.DOM_KEY_LOCATION_LEFT = 1;
  KeyboardEvent.DOM_KEY_LOCATION_RIGHT = 2;
  KeyboardEvent.DOM_KEY_LOCATION_NUMPAD = 3;
  KeyboardEvent.prototype.DOM_KEY_LOCATION_STANDARD = 0;
  KeyboardEvent.prototype.DOM_KEY_LOCATION_LEFT = 1;
  KeyboardEvent.prototype.DOM_KEY_LOCATION_RIGHT = 2;
  KeyboardEvent.prototype.DOM_KEY_LOCATION_NUMPAD = 3;

  // ---- InputEvent extends UIEvent ----
  function InputEvent(type, init) {
    init = init || {};
    UIEvent.call(this, type, init);
    this.data = (init.data !== undefined && init.data !== null) ? String(init.data) : null;
    this.inputType = str(init, 'inputType', '');
    this.isComposing = !!init.isComposing;
    this.dataTransfer = (init.dataTransfer !== undefined) ? init.dataTransfer : null;
  }
  InputEvent.prototype = Object.create(UIEvent.prototype);
  InputEvent.prototype.constructor = InputEvent;
  InputEvent.prototype.getTargetRanges = function() { return []; };

  // ---- CompositionEvent extends UIEvent ----
  function CompositionEvent(type, init) {
    init = init || {};
    UIEvent.call(this, type, init);
    this.data = str(init, 'data', '');
  }
  CompositionEvent.prototype = Object.create(UIEvent.prototype);
  CompositionEvent.prototype.constructor = CompositionEvent;

  // ---- WheelEvent extends MouseEvent ----
  var WheelBase = (typeof globalThis.MouseEvent === 'function') ? globalThis.MouseEvent : UIEvent;
  function WheelEvent(type, init) {
    init = init || {};
    WheelBase.call(this, type, init);
    this.deltaX = num(init, 'deltaX', 0);
    this.deltaY = num(init, 'deltaY', 0);
    this.deltaZ = num(init, 'deltaZ', 0);
    this.deltaMode = num(init, 'deltaMode', 0);
  }
  WheelEvent.prototype = Object.create(WheelBase.prototype);
  WheelEvent.prototype.constructor = WheelEvent;
  WheelEvent.DOM_DELTA_PIXEL = 0;
  WheelEvent.DOM_DELTA_LINE = 1;
  WheelEvent.DOM_DELTA_PAGE = 2;
  WheelEvent.prototype.DOM_DELTA_PIXEL = 0;
  WheelEvent.prototype.DOM_DELTA_LINE = 1;
  WheelEvent.prototype.DOM_DELTA_PAGE = 2;

  globalThis.KeyboardEvent = KeyboardEvent;
  globalThis.InputEvent = InputEvent;
  globalThis.CompositionEvent = CompositionEvent;
  globalThis.WheelEvent = WheelEvent;
})();
"#;
