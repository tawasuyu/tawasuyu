pub(crate) const COMPRESSION_BOOTSTRAP: &str = r#"
// Fase 7.161 — Compression Streams API (`CompressionStream`/`DecompressionStream`).
// Comprime/descomprime un flujo de bytes con gzip/deflate/deflate-raw. Ambos son
// GenericTransformStream: exponen `readable` (ReadableStream de Fase streams) +
// `writable` (writable con getWriter()). El códec real es del chrome (wiring
// PENDIENTE — no embebemos zlib en JS): el writer publica los chunks de entrada
// vía kind: 'compress'/'decompress' (value `<id> GS format GS write`) y el cierre
// vía `...flush`; el chrome devuelve los bytes procesados con
// __puriy_compress_output(id, chunk) (los encola en el readable) y cierra con
// __puriy_compress_end(id). Modelo de plomería fiel para feature-detection y para
// el data-flow que el JS observa (write → readable.read()).
(function() {
    if (globalThis.CompressionStream != null) return;
    var GS = String.fromCharCode(0x1D);
    var FORMATS = ['gzip', 'deflate', 'deflate-raw'];
    globalThis.__puriy_compress_next_id = globalThis.__puriy_compress_next_id || 1;
    globalThis.__puriy_compress_registry = globalThis.__puriy_compress_registry || {};

    function build(format, channel) {
        if (FORMATS.indexOf(format) < 0) {
            throw new TypeError("formato inválido '" + format + "' (gzip|deflate|deflate-raw)");
        }
        var id = globalThis.__puriy_compress_next_id++;
        var ctrl = null;
        var closed = false;
        var pendingClose = false;
        var readable = (typeof globalThis.ReadableStream === 'function')
            ? new globalThis.ReadableStream({ start: function(c) { ctrl = c; } })
            : { _chunks: [] };
        var entry = {
            enqueue: function(chunk) { if (ctrl) ctrl.enqueue(chunk); else readable._chunks.push(chunk); },
            close: function() {
                if (closed) return;
                closed = true;
                if (ctrl) ctrl.close(); else pendingClose = true;
            }
        };
        globalThis.__puriy_compress_registry[id] = entry;

        var writable = {
            getWriter: function() {
                return {
                    write: function(chunk) {
                        globalThis.__puriy_dirty.push({ id: '__window__', kind: channel, value: id + GS + format + GS + 'write' });
                        return Promise.resolve();
                    },
                    close: function() {
                        globalThis.__puriy_dirty.push({ id: '__window__', kind: channel, value: id + GS + format + GS + 'flush' });
                        return Promise.resolve();
                    },
                    abort: function(reason) {
                        globalThis.__puriy_dirty.push({ id: '__window__', kind: channel, value: id + GS + format + GS + 'abort' });
                        return Promise.resolve();
                    },
                    releaseLock: function() {},
                    get closed() { return Promise.resolve(undefined); },
                    get ready() { return Promise.resolve(undefined); },
                    get desiredSize() { return 1; }
                };
            },
            get locked() { return false; }
        };
        return { readable: readable, writable: writable, _id: id, _format: format };
    }

    function CompressionStream(format) {
        if (!(this instanceof CompressionStream)) { throw new TypeError("CompressionStream requiere 'new'"); }
        var b = build(String(format), 'compress');
        this.readable = b.readable; this.writable = b.writable; this._id = b._id; this._format = b._format;
    }
    globalThis.CompressionStream = CompressionStream;

    function DecompressionStream(format) {
        if (!(this instanceof DecompressionStream)) { throw new TypeError("DecompressionStream requiere 'new'"); }
        var b = build(String(format), 'decompress');
        this.readable = b.readable; this.writable = b.writable; this._id = b._id; this._format = b._format;
    }
    globalThis.DecompressionStream = DecompressionStream;

    // El chrome inyecta los bytes ya procesados al lado readable.
    globalThis.__puriy_compress_output = function(id, chunk) {
        var e = globalThis.__puriy_compress_registry[id];
        if (!e) return false;
        e.enqueue(chunk);
        return true;
    };
    globalThis.__puriy_compress_end = function(id) {
        var e = globalThis.__puriy_compress_registry[id];
        if (!e) return false;
        e.close();
        delete globalThis.__puriy_compress_registry[id];
        return true;
    };
    void 0;
})();
"#;
