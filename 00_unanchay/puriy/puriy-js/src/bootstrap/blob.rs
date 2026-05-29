pub(crate) const BLOB_BOOTSTRAP: &str = r#"
// Fase 7.47 — Blob mínimo. Usado por `XMLHttpRequest.responseType = 'blob'`
// (y reutilizable por `Response.blob()` cuando se implemente). Guarda los
// bytes crudos en `_bytes` (Array de 0..255). Soporta partes string,
// ArrayBuffer, TypedArray y otros Blob al construir.
//
// Limitaciones: sin endings normalization de los string parts (\r\n).
// Suficiente para "recibí un blob del server y lo leo con
// .text()/.arrayBuffer()/.slice()/.stream()". `URL.createObjectURL` vive
// en el módulo `objecturl` (Fase 7.50).
globalThis.Blob = function(parts, options) {
    var bytes = [];
    if (parts) {
        for (var i = 0; i < parts.length; i++) {
            var p = parts[i];
            if (typeof p === 'string') {
                for (var j = 0; j < p.length; j++) bytes.push(p.charCodeAt(j) & 0xff);
            } else if (p instanceof ArrayBuffer) {
                var v = new Uint8Array(p);
                for (var j = 0; j < v.length; j++) bytes.push(v[j]);
            } else if (p instanceof globalThis.Blob) {
                for (var j = 0; j < p._bytes.length; j++) bytes.push(p._bytes[j]);
            } else if (p && p.buffer instanceof ArrayBuffer && typeof p.length === 'number') {
                // TypedArray (Uint8Array, etc.).
                for (var j = 0; j < p.length; j++) bytes.push(p[j] & 0xff);
            } else {
                var s = String(p);
                for (var j = 0; j < s.length; j++) bytes.push(s.charCodeAt(j) & 0xff);
            }
        }
    }
    this._bytes = bytes;
    this.size = bytes.length;
    this.type = (options && options.type) ? String(options.type) : '';
};
globalThis.Blob.prototype.text = function() {
    var s = '';
    for (var i = 0; i < this._bytes.length; i++) s += String.fromCharCode(this._bytes[i]);
    return Promise.resolve(s);
};
globalThis.Blob.prototype.arrayBuffer = function() {
    var len = this._bytes.length;
    var buf = new ArrayBuffer(len);
    var view = new Uint8Array(buf);
    for (var i = 0; i < len; i++) view[i] = this._bytes[i];
    return Promise.resolve(buf);
};
// Fase 7.49 — `stream()` devuelve un ReadableStream de un solo chunk
// (Uint8Array de los bytes) seguido de done. Reusa la máquina de
// `bootstrap/streams`. Snapshot de `_bytes` al construir el source para
// que mutar el Blob después no afecte el stream ya emitido.
globalThis.Blob.prototype.stream = function() {
    var bytes = this._bytes;
    var emitted = false;
    return new globalThis.ReadableStream({
        pull: function(controller) {
            if (emitted) { controller.close(); return; }
            emitted = true;
            var len = bytes.length;
            var view = new Uint8Array(len);
            for (var i = 0; i < len; i++) view[i] = bytes[i];
            controller.enqueue(view);
        }
    });
};
globalThis.Blob.prototype.slice = function(start, end, contentType) {
    var n = this._bytes.length;
    if (start == null) start = 0;
    if (end == null) end = n;
    if (start < 0) start = Math.max(n + start, 0);
    if (end < 0) end = Math.max(n + end, 0);
    var b = new globalThis.Blob([], { type: contentType ? String(contentType) : '' });
    b._bytes = this._bytes.slice(start, end);
    b.size = b._bytes.length;
    return b;
};
"#;
