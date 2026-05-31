pub(crate) const NFC_BOOTSTRAP: &str = r#"
// Fase 7.133 — Web NFC (`NDEFReader`/`NDEFMessage`/`NDEFRecord`). Apps de puntos de
// venta, tarjetas y etiquetas la usan para leer/escribir tags NFC. El motor no tiene
// radio NFC: `scan()` es host-driven (publica kind: 'nfc-scan' y resuelve; el permiso
// real lo decide el chrome), `write()`/`makeReadOnly()` publican mutaciones. El host
// inyecta tags leídos con `__puriy_nfc_reading(serialNumber, records)` (dispara
// `reading` con un NDEFMessage) o un fallo con `__puriy_nfc_error(message)` (dispara
// `readingerror`). Sólo los readers que están escaneando reciben lecturas.
(function() {
    if (globalThis.NDEFReader != null) return;

    function NDEFRecord(init) {
        init = init || {};
        this.recordType = init.recordType != null ? init.recordType : 'text';
        this.mediaType = init.mediaType != null ? init.mediaType : null;
        this.id = init.id != null ? init.id : null;
        this.encoding = init.encoding != null ? init.encoding : null;
        this.lang = init.lang != null ? init.lang : null;
        this.data = init.data != null ? init.data : null;
    }
    NDEFRecord.prototype.toRecords = function() {
        return (this.data && this.data.records) ? this.data.records : [];
    };

    function NDEFMessage(init) {
        init = init || {};
        this.records = [];
        var recs = init.records || [];
        for (var i = 0; i < recs.length; i++) {
            this.records.push(recs[i] instanceof NDEFRecord ? recs[i] : new NDEFRecord(recs[i]));
        }
    }

    var scanners = [];  // readers con scan() en curso

    function NDEFReader() { this._listeners = {}; this._scanning = false; }
    NDEFReader.prototype.addEventListener = function(type, fn) {
        (this._listeners[type] = this._listeners[type] || []).push(fn);
    };
    NDEFReader.prototype.removeEventListener = function(type, fn) {
        var a = this._listeners[type]; if (!a) return;
        var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    };
    NDEFReader.prototype.dispatchEvent = function(ev) {
        var a = this._listeners[ev.type];
        if (a) { var c = a.slice(); for (var i = 0; i < c.length; i++) c[i].call(this, ev); }
        if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
        return true;
    };
    NDEFReader.prototype.scan = function(options) {
        var self = this;
        return new Promise(function(resolve, reject) {
            if (options && options.signal && options.signal.aborted) {
                reject(new globalThis.DOMException('scan abortado', 'AbortError'));
                return;
            }
            self._scanning = true;
            if (scanners.indexOf(self) === -1) scanners.push(self);
            if (options && options.signal && typeof options.signal.addEventListener === 'function') {
                options.signal.addEventListener('abort', function() {
                    self._scanning = false;
                    var i = scanners.indexOf(self); if (i >= 0) scanners.splice(i, 1);
                });
            }
            globalThis.__puriy_dirty.push({ id: '__window__', kind: 'nfc-scan', value: '' });
            resolve();
        });
    };
    NDEFReader.prototype.write = function(message, options) {
        var value;
        if (typeof message === 'string') value = message;
        else {
            var recs = (message && message.records) || [];
            value = JSON.stringify(recs);
        }
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'nfc-write', value: value });
        return Promise.resolve();
    };
    NDEFReader.prototype.makeReadOnly = function(options) {
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'nfc-makereadonly', value: '' });
        return Promise.resolve();
    };

    // El host inyecta un tag leído a todos los readers escaneando.
    globalThis.__puriy_nfc_reading = function(serialNumber, records) {
        var msg = new NDEFMessage({ records: records || [] });
        var any = false;
        var c = scanners.slice();
        for (var i = 0; i < c.length; i++) {
            if (!c[i]._scanning) continue;
            c[i].dispatchEvent({ type: 'reading', serialNumber: serialNumber || '', message: msg });
            any = true;
        }
        return any;
    };
    // El host señala un fallo de lectura.
    globalThis.__puriy_nfc_error = function(message) {
        var any = false;
        var c = scanners.slice();
        for (var i = 0; i < c.length; i++) {
            if (!c[i]._scanning) continue;
            c[i].dispatchEvent({ type: 'readingerror', message: message || '' });
            any = true;
        }
        return any;
    };

    globalThis.NDEFReader = NDEFReader;
    globalThis.NDEFMessage = NDEFMessage;
    globalThis.NDEFRecord = NDEFRecord;
    void 0;
})();
"#;
