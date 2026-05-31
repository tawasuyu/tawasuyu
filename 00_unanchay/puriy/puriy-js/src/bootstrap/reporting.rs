pub(crate) const REPORTING_BOOTSTRAP: &str = r#"
// Fase 7.136 — Reporting API (`ReportingObserver` + `Report`). Apps observan reportes de
// deprecación/intervención/CSP que el navegador genera (`new ReportingObserver(cb).observe()`).
// El motor no genera reportes solo: el host los inyecta con `__puriy_queue_report({type,url,body})`.
// Divergencia documentada: el spec entrega los reportes en una tarea encolada (asíncrona);
// acá la entrega es síncrona al inyectar (determinista). `takeRecords()` sigue disponible y
// `observe({buffered:true})` reentrega el buffer global previo al observer recién montado.
(function() {
    if (globalThis.ReportingObserver != null) return;

    function Report(type, url, body) {
        this.type = type != null ? type : '';
        this.url = url != null ? url : null;
        this.body = body != null ? body : null;
    }
    Report.prototype.toJSON = function() {
        return { type: this.type, url: this.url, body: this.body };
    };

    var registered = [];  // observers con observe() activo
    var buffer = [];      // todos los reportes inyectados (para buffered:true)

    function ReportingObserver(callback, options) {
        if (typeof callback !== 'function') throw new TypeError('callback debe ser función');
        options = options || {};
        this._cb = callback;
        this._types = Array.isArray(options.types) ? options.types : null;  // null = todos
        this._buffered = !!options.buffered;
        this._queue = [];
        this._observing = false;
    }
    ReportingObserver.prototype._matches = function(type) {
        return !this._types || this._types.indexOf(type) !== -1;
    };
    ReportingObserver.prototype._flush = function() {
        if (!this._queue.length) return;
        var recs = this._queue.slice();
        this._queue.length = 0;
        try { this._cb(recs, this); } catch (e) {}
    };
    ReportingObserver.prototype.observe = function() {
        if (this._observing) return;
        this._observing = true;
        if (registered.indexOf(this) === -1) registered.push(this);
        if (this._buffered) {
            for (var i = 0; i < buffer.length; i++) {
                if (this._matches(buffer[i].type)) this._queue.push(buffer[i]);
            }
            this._flush();
        }
    };
    ReportingObserver.prototype.disconnect = function() {
        this._observing = false;
        var i = registered.indexOf(this);
        if (i >= 0) registered.splice(i, 1);
    };
    ReportingObserver.prototype.takeRecords = function() {
        var r = this._queue.slice();
        this._queue.length = 0;
        return r;
    };

    // El host inyecta un reporte; se entrega a todos los observers que matchean su tipo.
    globalThis.__puriy_queue_report = function(report) {
        report = report || {};
        var r = new Report(report.type, report.url, report.body);
        buffer.push(r);
        var any = false;
        var c = registered.slice();
        for (var i = 0; i < c.length; i++) {
            if (c[i]._observing && c[i]._matches(r.type)) {
                c[i]._queue.push(r);
                c[i]._flush();
                any = true;
            }
        }
        return any;
    };

    globalThis.ReportingObserver = ReportingObserver;
    globalThis.Report = Report;
    void 0;
})();
"#;
