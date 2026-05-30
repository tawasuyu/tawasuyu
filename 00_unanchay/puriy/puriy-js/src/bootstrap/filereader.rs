pub(crate) const FILEREADER_BOOTSTRAP: &str = r#"
// Fase 7.69 — FileReader. API event-based (anterior a Blob.text()/arrayBuffer)
// muy usada en flujos de upload: leer un File de <input type=file> a texto /
// dataURL / ArrayBuffer / binary string. Los bytes ya viven en memoria (Blob.
// _bytes), así que computamos el result de una y disparamos los eventos en un
// microtask (`Promise.resolve().then`) para respetar la asincronía del spec.
//
// EventTarget mínimo igual que XHR: addEventListener + on<tipo>. Dispara
// loadstart → load → loadend (o error/abort + loadend).
globalThis.FileReader = function() {
    this.readyState = 0;   // EMPTY
    this.result = null;
    this.error = null;
    this.onloadstart = null;
    this.onprogress = null;
    this.onload = null;
    this.onabort = null;
    this.onerror = null;
    this.onloadend = null;
    this._listeners = {};
    this._aborted = false;
};
globalThis.FileReader.EMPTY = 0;
globalThis.FileReader.LOADING = 1;
globalThis.FileReader.DONE = 2;
globalThis.FileReader.prototype.addEventListener = function(type, fn) {
    if (typeof fn !== 'function') return;
    type = String(type);
    if (!this._listeners[type]) this._listeners[type] = [];
    this._listeners[type].push(fn);
};
globalThis.FileReader.prototype.removeEventListener = function(type, fn) {
    type = String(type);
    var arr = this._listeners[type];
    if (!arr) return;
    for (var i = arr.length - 1; i >= 0; i--) {
        if (arr[i] === fn) arr.splice(i, 1);
    }
};
globalThis.FileReader.prototype.__puriy_fire = function(type, evt) {
    evt = evt || {};
    evt.type = type;
    evt.target = this;
    var on = this['on' + type];
    if (typeof on === 'function') {
        try { on.call(this, evt); }
        catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
    }
    var arr = this._listeners[type];
    if (arr) {
        var snapshot = arr.slice();
        for (var i = 0; i < snapshot.length; i++) {
            try { snapshot[i].call(this, evt); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
    }
};
// Núcleo compartido por los read*: valida, dispara loadstart, y agenda el
// cómputo del result + load/loadend en un microtask. `compute(bytes, type)`
// devuelve el result según el método.
globalThis.FileReader.prototype.__puriy_read = function(blob, compute) {
    if (!(globalThis.Blob && blob instanceof globalThis.Blob)) {
        throw new TypeError("FileReader.read*: el argumento no es un Blob");
    }
    if (this.readyState === 1) {
        throw new Error('InvalidStateError: lectura en progreso');
    }
    this.readyState = 1;
    this.result = null;
    this.error = null;
    this._aborted = false;
    var self = this;
    var bytes = blob._bytes.slice();
    var type = blob.type || '';
    var total = bytes.length;
    self.__puriy_fire('loadstart', { lengthComputable: true, loaded: 0, total: total });
    Promise.resolve().then(function() {
        if (self._aborted) return;
        try {
            self.result = compute(bytes, type);
        } catch (e) {
            self.error = e;
            self.readyState = 2;
            self.__puriy_fire('error', { lengthComputable: false, loaded: 0, total: total });
            self.__puriy_fire('loadend', { lengthComputable: false, loaded: 0, total: total });
            return;
        }
        self.readyState = 2;
        self.__puriy_fire('progress', { lengthComputable: true, loaded: total, total: total });
        self.__puriy_fire('load', { lengthComputable: true, loaded: total, total: total });
        self.__puriy_fire('loadend', { lengthComputable: true, loaded: total, total: total });
    });
};
globalThis.FileReader.prototype.readAsText = function(blob, encoding) {
    // Default UTF-8; el label se pasa a TextDecoder (que hoy ignora no-UTF-8).
    this.__puriy_read(blob, function(bytes) {
        var view = new Uint8Array(bytes.length);
        for (var i = 0; i < bytes.length; i++) view[i] = bytes[i];
        return new globalThis.TextDecoder(encoding || 'utf-8').decode(view);
    });
};
globalThis.FileReader.prototype.readAsArrayBuffer = function(blob) {
    this.__puriy_read(blob, function(bytes) {
        var buf = new ArrayBuffer(bytes.length);
        var view = new Uint8Array(buf);
        for (var i = 0; i < bytes.length; i++) view[i] = bytes[i];
        return buf;
    });
};
globalThis.FileReader.prototype.readAsBinaryString = function(blob) {
    this.__puriy_read(blob, function(bytes) {
        var s = '';
        for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
        return s;
    });
};
globalThis.FileReader.prototype.readAsDataURL = function(blob) {
    this.__puriy_read(blob, function(bytes, type) {
        var s = '';
        for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
        return 'data:' + (type || 'application/octet-stream') + ';base64,' + globalThis.btoa(s);
    });
};
globalThis.FileReader.prototype.abort = function() {
    if (this.readyState !== 1) return;
    this._aborted = true;
    this.readyState = 2;
    this.result = null;
    this.__puriy_fire('abort', { lengthComputable: false, loaded: 0, total: 0 });
    this.__puriy_fire('loadend', { lengthComputable: false, loaded: 0, total: 0 });
};
"#;
