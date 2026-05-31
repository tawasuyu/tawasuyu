pub(crate) const PRESSURE_BOOTSTRAP: &str = r#"
// Fase 7.137 — Compute Pressure API (`PressureObserver` + `PressureRecord`). Apps adaptan
// su carga (bajar fps, pausar trabajo) según la presión térmica/CPU del dispositivo. El motor
// no mide la CPU: `observe(source)` es host-driven (publica kind: 'pressure-observe' y resuelve;
// fuente desconocida → NotSupportedError), y el host empuja muestras con
// `__puriy_pressure_sample(source, state)` (state ∈ nominal/fair/serious/critical), que dispara
// el callback con un PressureRecord. Hermano observer de ReportingObserver (Fase 7.136) y de la
// familia de sensores (Fase 7.132).
(function() {
    if (globalThis.PressureObserver != null) return;

    function nowMs() {
        return (globalThis.performance && typeof performance.now === 'function') ? performance.now() : 0;
    }

    function PressureRecord(source, state, time) {
        this.source = source;
        this.state = state;
        this.time = time;
    }
    PressureRecord.prototype.toJSON = function() {
        return { source: this.source, state: this.state, time: this.time };
    };

    var registered = [];  // entradas { o: observer, source }

    function PressureObserver(callback) {
        if (typeof callback !== 'function') throw new TypeError('callback debe ser función');
        this._cb = callback;
        this._queue = [];
        this._sources = [];
    }
    PressureObserver.knownSources = ['cpu'];
    PressureObserver.prototype.observe = function(source, options) {
        var self = this;
        return new Promise(function(resolve, reject) {
            if (PressureObserver.knownSources.indexOf(source) === -1) {
                reject(new globalThis.DOMException('fuente de presión desconocida: ' + source, 'NotSupportedError'));
                return;
            }
            if (self._sources.indexOf(source) === -1) self._sources.push(source);
            registered.push({ o: self, source: source });
            globalThis.__puriy_dirty.push({ id: '__window__', kind: 'pressure-observe', value: source });
            resolve();
        });
    };
    PressureObserver.prototype.unobserve = function(source) {
        var self = this;
        var i = this._sources.indexOf(source);
        if (i >= 0) this._sources.splice(i, 1);
        registered = registered.filter(function(e) { return !(e.o === self && e.source === source); });
    };
    PressureObserver.prototype.disconnect = function() {
        var self = this;
        this._sources.length = 0;
        this._queue.length = 0;
        registered = registered.filter(function(e) { return e.o !== self; });
    };
    PressureObserver.prototype.takeRecords = function() {
        var r = this._queue.slice();
        this._queue.length = 0;
        return r;
    };

    // El host empuja una muestra de presión a los observers que observan esa fuente.
    globalThis.__puriy_pressure_sample = function(source, state) {
        var rec = new PressureRecord(source, state, nowMs());
        var any = false;
        var seen = [];
        var c = registered.slice();
        for (var i = 0; i < c.length; i++) {
            if (c[i].source !== source) continue;
            var o = c[i].o;
            if (seen.indexOf(o) !== -1) continue;  // un solo callback por observer
            seen.push(o);
            o._queue.push(rec);
            var recs = o._queue.slice();
            o._queue.length = 0;
            try { o._cb(recs, o); } catch (e) {}
            any = true;
        }
        return any;
    };

    globalThis.PressureObserver = PressureObserver;
    globalThis.PressureRecord = PressureRecord;
    void 0;
})();
"#;
