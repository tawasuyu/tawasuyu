pub(crate) const EVENT_CLASS_BOOTSTRAP: &str = r#"
globalThis.Event = function(type, init) {
    this.type = String(type);
    this.bubbles = !!(init && init.bubbles);
    this.cancelable = !!(init && init.cancelable);
    this.defaultPrevented = false;
    this._stopped = false;
    this.eventPhase = 0;
    this.target = null;
    this.currentTarget = null;
    this.preventDefault = function() {
        if (this.cancelable) this.defaultPrevented = true;
    };
    this.stopPropagation = function() { this._stopped = true; };
    // Fase 7.76 — corta el resto de listeners del MISMO target (lo usa
    // EventTarget.dispatchEvent).
    this.stopImmediatePropagation = function() {
        this._stopped = true;
        this._stopImmediate = true;
    };
};
globalThis.CustomEvent = function(type, init) {
    globalThis.Event.call(this, type, init);
    this.detail = (init && init.detail !== undefined) ? init.detail : null;
};
"#;
