pub(crate) const TYPED_EVENTS_BOOTSTRAP: &str = r#"
// Fase 7.77 — eventos tipados del frente net. MessageEvent / CloseEvent los
// despachan WebSocket y EventSource; ProgressEvent lo usan XHR/FileReader (que
// hoy disparan objetos planos — acá queda la clase disponible para user code y
// para futuros retrofits). Las tres heredan de Event vía cadena de prototipos
// REAL (`Object.create(Event.prototype)`), así que `instanceof Event` se
// cumple — a diferencia de CustomEvent (Fase 7.x) que no la armó.
globalThis.MessageEvent = function(type, init) {
    globalThis.Event.call(this, type, init);
    init = init || {};
    this.data = (init.data !== undefined) ? init.data : null;
    this.origin = (init.origin !== undefined) ? String(init.origin) : '';
    this.lastEventId = (init.lastEventId !== undefined) ? String(init.lastEventId) : '';
    this.source = (init.source !== undefined) ? init.source : null;
    this.ports = (init.ports !== undefined) ? init.ports : [];
};
globalThis.MessageEvent.prototype = Object.create(globalThis.Event.prototype);
globalThis.MessageEvent.prototype.constructor = globalThis.MessageEvent;

globalThis.CloseEvent = function(type, init) {
    globalThis.Event.call(this, type, init);
    init = init || {};
    this.code = (init.code !== undefined) ? (init.code | 0) : 0;
    this.reason = (init.reason !== undefined) ? String(init.reason) : '';
    this.wasClean = !!init.wasClean;
};
globalThis.CloseEvent.prototype = Object.create(globalThis.Event.prototype);
globalThis.CloseEvent.prototype.constructor = globalThis.CloseEvent;

globalThis.ProgressEvent = function(type, init) {
    globalThis.Event.call(this, type, init);
    init = init || {};
    this.lengthComputable = !!init.lengthComputable;
    this.loaded = (init.loaded !== undefined) ? Number(init.loaded) : 0;
    this.total = (init.total !== undefined) ? Number(init.total) : 0;
};
globalThis.ProgressEvent.prototype = Object.create(globalThis.Event.prototype);
globalThis.ProgressEvent.prototype.constructor = globalThis.ProgressEvent;
"#;
