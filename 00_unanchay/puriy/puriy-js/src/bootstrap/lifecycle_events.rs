//! Eventos de navegación, ciclo de vida del documento y CSS —
//! `HashChangeEvent`, `PopStateEvent`, `PageTransitionEvent`, `StorageEvent`,
//! `SubmitEvent`, `ToggleEvent`, `AnimationEvent`, `TransitionEvent`.
//! Fase 7.108. Cierra la familia de constructores de eventos tipados
//! abierta en 7.105.
//!
//! Todos extienden `Event` directamente:
//! - `HashChangeEvent` — `hashchange` (`oldURL`/`newURL`).
//! - `PopStateEvent` — `popstate` del History API (`state`).
//! - `PageTransitionEvent` — `pageshow`/`pagehide` (`persisted`).
//! - `StorageEvent` — `storage` de `localStorage` cross-tab
//!   (`key`/`oldValue`/`newValue`/`url`/`storageArea`).
//! - `SubmitEvent` — `submit` de `<form>` (`submitter`).
//! - `ToggleEvent` — `<details>`/popover (`oldState`/`newState`).
//! - `AnimationEvent` — `animationstart`/`animationend` CSS
//!   (`animationName`/`elapsedTime`/`pseudoElement`).
//! - `TransitionEvent` — `transitionend` CSS
//!   (`propertyName`/`elapsedTime`/`pseudoElement`).
//!
//! **Limitaciones explícitas**:
//! 1. **Sólo constructores** — el chrome no despacha estos eventos de forma
//!    reactiva (el History API de 7.x ya maneja `popstate` por su canal; CSS
//!    animations 7.103 coordina por `setTimeout`, no emite estos events al
//!    BoxTree). Sirven para construcción programática + feature-detect.
//! 2. **`storageArea`/`submitter`/`state`** se guardan verbatim del init.

pub(crate) const LIFECYCLE_EVENTS_BOOTSTRAP: &str = r#"
(function(){
  if (typeof globalThis.HashChangeEvent === 'function') return;
  if (typeof globalThis.Event !== 'function') return;

  var Event = globalThis.Event;
  function num(init, key, def) {
    return (init && typeof init[key] === 'number') ? init[key] : def;
  }
  function str(init, key, def) {
    return (init && init[key] !== undefined && init[key] !== null) ? String(init[key]) : def;
  }
  function strOrNull(init, key) {
    return (init && init[key] !== undefined && init[key] !== null) ? String(init[key]) : null;
  }
  function extend(Ctor) {
    Ctor.prototype = Object.create(Event.prototype);
    Ctor.prototype.constructor = Ctor;
  }

  function HashChangeEvent(type, init) {
    init = init || {};
    Event.call(this, type, init);
    this.oldURL = str(init, 'oldURL', '');
    this.newURL = str(init, 'newURL', '');
  }
  extend(HashChangeEvent);

  function PopStateEvent(type, init) {
    init = init || {};
    Event.call(this, type, init);
    this.state = (init.state !== undefined) ? init.state : null;
  }
  extend(PopStateEvent);

  function PageTransitionEvent(type, init) {
    init = init || {};
    Event.call(this, type, init);
    this.persisted = !!init.persisted;
  }
  extend(PageTransitionEvent);

  // StorageEvent lo define net en storage_event.rs (con su plumbing
  // __puriy_dispatch_storage); no lo redefinimos acá para no pisarlo.

  function SubmitEvent(type, init) {
    init = init || {};
    Event.call(this, type, init);
    this.submitter = (init.submitter !== undefined) ? init.submitter : null;
  }
  extend(SubmitEvent);

  function ToggleEvent(type, init) {
    init = init || {};
    Event.call(this, type, init);
    this.oldState = str(init, 'oldState', '');
    this.newState = str(init, 'newState', '');
  }
  extend(ToggleEvent);

  function AnimationEvent(type, init) {
    init = init || {};
    Event.call(this, type, init);
    this.animationName = str(init, 'animationName', '');
    this.elapsedTime = num(init, 'elapsedTime', 0);
    this.pseudoElement = str(init, 'pseudoElement', '');
  }
  extend(AnimationEvent);

  function TransitionEvent(type, init) {
    init = init || {};
    Event.call(this, type, init);
    this.propertyName = str(init, 'propertyName', '');
    this.elapsedTime = num(init, 'elapsedTime', 0);
    this.pseudoElement = str(init, 'pseudoElement', '');
  }
  extend(TransitionEvent);

  globalThis.HashChangeEvent = HashChangeEvent;
  globalThis.PopStateEvent = PopStateEvent;
  globalThis.PageTransitionEvent = PageTransitionEvent;
  globalThis.SubmitEvent = SubmitEvent;
  globalThis.ToggleEvent = ToggleEvent;
  globalThis.AnimationEvent = AnimationEvent;
  globalThis.TransitionEvent = TransitionEvent;
})();
"#;
