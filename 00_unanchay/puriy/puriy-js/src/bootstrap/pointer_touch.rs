//! Constructores de eventos de input — `PointerEvent`, `TouchEvent`,
//! `Touch`, `TouchList`. Fase 7.96.
//!
//! Librerías de UI (drag&drop, gestos, sliders, mapas) hacen feature-detect
//! con `'PointerEvent' in window` / `'ontouchstart' in window` y construyen
//! eventos sintéticos: `new PointerEvent('pointerdown', { clientX, pointerType })`,
//! `new TouchEvent('touchstart', { touches: [new Touch({...})] })`. Sin las
//! clases definidas, `new PointerEvent(...)` tira `is not a constructor` y la
//! rama de input táctil/puntero se cae al cargar.
//!
//! `PointerEvent` y `TouchEvent` **extienden `Event`** (Fase 7.25): reusan
//! `globalThis.Event.call(this, type, init)` para `type`/`bubbles`/
//! `cancelable`/`preventDefault`/`stopPropagation`, y agregan los campos
//! propios. `Touch`/`TouchList` son objetos de valor.
//!
//! **Limitaciones explícitas**:
//! 1. **Sólo constructores** — el chrome NO despacha pointer/touch events
//!    reales del usuario (no hay digitizer ni multitouch headless); estas
//!    clases sirven para `new PointerEvent(...)` programático + feature-detect.
//! 2. **`getCoalescedEvents`/`getPredictedEvents`** → `[]` (sin historial de
//!    coalescing del compositor).
//! 3. **`TouchList` es array-like simple** — soporta `length`/`item(i)`/`[i]`,
//!    no es un `TouchList` vivo del DOM.

pub(crate) const POINTER_TOUCH_BOOTSTRAP: &str = r#"
(function(){
  if (typeof globalThis.PointerEvent === 'function') return;

  function baseEvent(self, type, init) {
    if (typeof globalThis.Event === 'function') {
      globalThis.Event.call(self, type, init);
    } else {
      self.type = String(type);
      self.bubbles = !!(init && init.bubbles);
      self.cancelable = !!(init && init.cancelable);
      self.defaultPrevented = false;
      self.preventDefault = function() { if (self.cancelable) self.defaultPrevented = true; };
      self.stopPropagation = function() {};
    }
  }

  function num(init, key, def) {
    return (init && typeof init[key] === 'number') ? init[key] : def;
  }

  // ---- mouse/pointer shared fields ----
  function applyMouseFields(self, init) {
    init = init || {};
    self.clientX = num(init, 'clientX', 0);
    self.clientY = num(init, 'clientY', 0);
    self.screenX = num(init, 'screenX', 0);
    self.screenY = num(init, 'screenY', 0);
    self.pageX = num(init, 'pageX', self.clientX);
    self.pageY = num(init, 'pageY', self.clientY);
    self.offsetX = num(init, 'offsetX', 0);
    self.offsetY = num(init, 'offsetY', 0);
    self.movementX = num(init, 'movementX', 0);
    self.movementY = num(init, 'movementY', 0);
    self.button = num(init, 'button', 0);
    self.buttons = num(init, 'buttons', 0);
    self.ctrlKey = !!init.ctrlKey;
    self.shiftKey = !!init.shiftKey;
    self.altKey = !!init.altKey;
    self.metaKey = !!init.metaKey;
    self.relatedTarget = (init.relatedTarget !== undefined) ? init.relatedTarget : null;
    self.detail = num(init, 'detail', 0);
    self.view = (init.view !== undefined) ? init.view : globalThis;
  }

  // ---- PointerEvent ----
  function PointerEvent(type, init) {
    init = init || {};
    baseEvent(this, type, init);
    applyMouseFields(this, init);
    this.pointerId = num(init, 'pointerId', 0);
    this.width = num(init, 'width', 1);
    this.height = num(init, 'height', 1);
    this.pressure = num(init, 'pressure', 0);
    this.tangentialPressure = num(init, 'tangentialPressure', 0);
    this.tiltX = num(init, 'tiltX', 0);
    this.tiltY = num(init, 'tiltY', 0);
    this.twist = num(init, 'twist', 0);
    this.pointerType = (typeof init.pointerType === 'string') ? init.pointerType : '';
    this.isPrimary = !!init.isPrimary;
  }
  PointerEvent.prototype.getCoalescedEvents = function() { return []; };
  PointerEvent.prototype.getPredictedEvents = function() { return []; };

  // ---- Touch ----
  function Touch(init) {
    init = init || {};
    this.identifier = num(init, 'identifier', 0);
    this.target = (init.target !== undefined) ? init.target : null;
    this.clientX = num(init, 'clientX', 0);
    this.clientY = num(init, 'clientY', 0);
    this.screenX = num(init, 'screenX', 0);
    this.screenY = num(init, 'screenY', 0);
    this.pageX = num(init, 'pageX', this.clientX);
    this.pageY = num(init, 'pageY', this.clientY);
    this.radiusX = num(init, 'radiusX', 1);
    this.radiusY = num(init, 'radiusY', 1);
    this.rotationAngle = num(init, 'rotationAngle', 0);
    this.force = num(init, 'force', 1);
  }

  // ---- TouchList (array-like) ----
  function makeTouchList(arr) {
    arr = Array.isArray(arr) ? arr : [];
    var list = { length: arr.length, item: function(i) { return (i >= 0 && i < arr.length) ? arr[i] : null; } };
    for (var i = 0; i < arr.length; i++) list[i] = arr[i];
    return list;
  }

  // ---- TouchEvent ----
  function TouchEvent(type, init) {
    init = init || {};
    baseEvent(this, type, init);
    this.touches = makeTouchList(init.touches);
    this.targetTouches = makeTouchList(init.targetTouches);
    this.changedTouches = makeTouchList(init.changedTouches);
    this.ctrlKey = !!init.ctrlKey;
    this.shiftKey = !!init.shiftKey;
    this.altKey = !!init.altKey;
    this.metaKey = !!init.metaKey;
  }

  globalThis.PointerEvent = PointerEvent;
  globalThis.Touch = Touch;
  globalThis.TouchEvent = TouchEvent;
  globalThis.TouchList = makeTouchList;
})();
"#;
