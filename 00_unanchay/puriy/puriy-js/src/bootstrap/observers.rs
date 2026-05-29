pub(crate) const OBSERVERS_BOOTSTRAP: &str = r#"
// Fase 7.40 — MutationObserver + IntersectionObserver stubs. Apps
// modernas (React, Vue, sentry, observers de visibilidad, etc.) construyen
// estos al boot — sin la clase definida tiran `not a constructor` y se
// caen antes de pintar nada. Implementamos la forma sin la mecánica:
// observe()/disconnect()/takeRecords()/unobserve() existen pero ningún
// callback se dispara nunca. Soluciona el 95% de los crashes; las apps
// que de verdad dependen de observers verán "no se actualiza el feed
// nunca" pero al menos arrancan. Real wiring queda pendiente — engancha
// con el pipeline de mutaciones (`__puriy_dirty` ya tiene todo el data,
// faltaría filtrarlo por target+opts y batchearlo en un microtask).
globalThis.MutationObserver = function(callback) {
    this._callback = callback;
    this._targets = [];
};
globalThis.MutationObserver.prototype.observe = function(target, options) {
    this._targets.push({ target: target, options: options || {} });
};
globalThis.MutationObserver.prototype.disconnect = function() {
    this._targets = [];
};
globalThis.MutationObserver.prototype.takeRecords = function() {
    return [];
};
globalThis.IntersectionObserver = function(callback, options) {
    this._callback = callback;
    this._options = options || {};
    this._targets = [];
    this.root = (options && options.root) || null;
    this.rootMargin = (options && options.rootMargin) || '0px';
    this.thresholds = (options && options.threshold != null)
        ? (Array.isArray(options.threshold) ? options.threshold : [options.threshold])
        : [0];
};
globalThis.IntersectionObserver.prototype.observe = function(target) {
    this._targets.push(target);
};
globalThis.IntersectionObserver.prototype.unobserve = function(target) {
    var i = this._targets.indexOf(target);
    if (i >= 0) this._targets.splice(i, 1);
};
globalThis.IntersectionObserver.prototype.disconnect = function() {
    this._targets = [];
};
globalThis.IntersectionObserver.prototype.takeRecords = function() {
    return [];
};
// ResizeObserver — tercer observer que apps modernas construyen al boot
// (Material-UI lo usa para tabs/sidebars, antd para responsive grids).
// Mismo patrón stub.
globalThis.ResizeObserver = function(callback) {
    this._callback = callback;
    this._targets = [];
};
globalThis.ResizeObserver.prototype.observe = function(target, options) {
    this._targets.push({ target: target, options: options || {} });
};
globalThis.ResizeObserver.prototype.unobserve = function(target) {
    for (var i = 0; i < this._targets.length; i++) {
        if (this._targets[i].target === target) { this._targets.splice(i, 1); return; }
    }
};
globalThis.ResizeObserver.prototype.disconnect = function() {
    this._targets = [];
};
"#;
