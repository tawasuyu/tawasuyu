pub(crate) const MSE_BOOTSTRAP: &str = r#"
// Fase 7.147 — Media Source Extensions (`MediaSource`/`SourceBuffer`/
// `SourceBufferList`/`TimeRanges` + alias `ManagedMediaSource`/`ManagedSourceBuffer`).
// Permite alimentar un `<video>`/`<audio>` con segmentos de medios desde JS
// (streaming adaptativo: HLS/DASH). La máquina de estados (`closed`/`open`/`ended`),
// la lista de source buffers y el ciclo `updating`→`update`→`updateend` son JS-puros
// y funcionales; el demux/decode real es del chrome (wiring PENDIENTE):
//   · `URL.createObjectURL(mediaSource)` ya registra el objeto (Fase 7.50); cuando el
//     chrome lo adjunta a un elemento media debe abrir la fuente con `__puriy_mse_open(id)`
//     → readyState='open' + evento `sourceopen`.
//   · `addSourceBuffer`/`appendBuffer`/`remove`/`endOfStream` publican kind: 'mse-*'.
//   · `appendBuffer(data)` bufferiza los bytes y completa el ciclo vía microtask
//     (update/updateend) sin necesidad del chrome — `buffered` refleja [0, duration].
(function() {
    if (globalThis.MediaSource != null) return;
    var GS = String.fromCharCode(0x1D);
    var nextId = 1;
    var sources = {};

    function micro(fn) { Promise.resolve().then(fn); }
    function emit(self, type, ev) {
        ev = ev || { type: type };
        var h = self['on' + type];
        if (typeof h === 'function') { try { h.call(self, ev); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; } }
        self.dispatchEvent(ev);
    }

    // ---- TimeRanges ----
    function TimeRanges(ranges) {
        this._r = ranges || [];
        this.length = this._r.length;
    }
    TimeRanges.prototype.start = function(i) {
        if (i < 0 || i >= this._r.length) throw new globalThis.DOMException('índice fuera de rango', 'IndexSizeError');
        return this._r[i][0];
    };
    TimeRanges.prototype.end = function(i) {
        if (i < 0 || i >= this._r.length) throw new globalThis.DOMException('índice fuera de rango', 'IndexSizeError');
        return this._r[i][1];
    };
    globalThis.TimeRanges = TimeRanges;

    // ---- SourceBufferList ----
    function SourceBufferList() {
        globalThis.EventTarget.call(this);
        this._items = [];
        this.length = 0;
        this.onaddsourcebuffer = null;
        this.onremovesourcebuffer = null;
    }
    SourceBufferList.prototype = Object.create(globalThis.EventTarget.prototype);
    SourceBufferList.prototype.constructor = SourceBufferList;
    SourceBufferList.prototype._add = function(sb) {
        this._items.push(sb);
        this[this._items.length - 1] = sb;
        this.length = this._items.length;
        emit(this, 'addsourcebuffer');
    };
    SourceBufferList.prototype._remove = function(sb) {
        var idx = this._items.indexOf(sb);
        if (idx < 0) return;
        this._items.splice(idx, 1);
        // recompacta los índices numéricos
        for (var i = 0; i < this._items.length; i++) this[i] = this._items[i];
        delete this[this._items.length];
        this.length = this._items.length;
        emit(this, 'removesourcebuffer');
    };

    // ---- SourceBuffer ----
    function SourceBuffer(parent, type) {
        globalThis.EventTarget.call(this);
        this._parent = parent;
        this._type = type;
        this.mode = 'segments';
        this.updating = false;
        this.timestampOffset = 0;
        this.appendWindowStart = 0;
        this.appendWindowEnd = Infinity;
        this._bytes = 0;
        this.onupdatestart = null; this.onupdate = null; this.onupdateend = null;
        this.onerror = null; this.onabort = null;
    }
    SourceBuffer.prototype = Object.create(globalThis.EventTarget.prototype);
    SourceBuffer.prototype.constructor = SourceBuffer;
    Object.defineProperty(SourceBuffer.prototype, 'buffered', {
        get: function() {
            var ms = this._parent;
            var end = (ms && typeof ms.duration === 'number' && ms.duration === ms.duration) ? ms.duration : 0;
            return (this._bytes > 0) ? new TimeRanges([[0, end]]) : new TimeRanges([]);
        }
    });
    SourceBuffer.prototype.appendBuffer = function(data) {
        if (this.updating) throw new globalThis.DOMException('append en curso', 'InvalidStateError');
        var n = 0;
        if (data != null) {
            if (data.byteLength != null) n = data.byteLength;
            else if (data.length != null) n = data.length;
        }
        this._bytes += n;
        this.updating = true;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'mse-append',
            value: (this._parent ? this._parent._id : 0) + GS + String(n) });
        emit(this, 'updatestart');
        var self = this;
        micro(function() {
            self.updating = false;
            emit(self, 'update');
            emit(self, 'updateend');
        });
    };
    SourceBuffer.prototype.abort = function() {
        this.updating = false;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'mse-abort', value: '' });
        emit(this, 'abort');
        emit(this, 'updateend');
    };
    SourceBuffer.prototype.remove = function(start, end) {
        if (this.updating) throw new globalThis.DOMException('append en curso', 'InvalidStateError');
        this.updating = true;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'mse-remove',
            value: String(start) + GS + String(end) });
        emit(this, 'updatestart');
        var self = this;
        micro(function() { self.updating = false; emit(self, 'update'); emit(self, 'updateend'); });
    };
    SourceBuffer.prototype.changeType = function(type) {
        if (this.updating) throw new globalThis.DOMException('append en curso', 'InvalidStateError');
        this._type = type;
    };

    // ---- MediaSource ----
    function MediaSource() {
        globalThis.EventTarget.call(this);
        this._id = nextId++;
        sources[this._id] = this;
        this.sourceBuffers = new SourceBufferList();
        this.activeSourceBuffers = new SourceBufferList();
        this.readyState = 'closed';
        this.duration = NaN;
        this.onsourceopen = null; this.onsourceended = null; this.onsourceclose = null;
    }
    MediaSource.prototype = Object.create(globalThis.EventTarget.prototype);
    MediaSource.prototype.constructor = MediaSource;
    MediaSource.prototype.addSourceBuffer = function(type) {
        if (this.readyState !== 'open') throw new globalThis.DOMException('MediaSource no abierta', 'InvalidStateError');
        if (type == null || type === '') throw new globalThis.DOMException('tipo vacío', 'TypeError');
        var sb = new SourceBuffer(this, type);
        this.sourceBuffers._add(sb);
        this.activeSourceBuffers._add(sb);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'mse-add-source-buffer',
            value: this._id + GS + String(type) });
        return sb;
    };
    MediaSource.prototype.removeSourceBuffer = function(sb) {
        this.sourceBuffers._remove(sb);
        this.activeSourceBuffers._remove(sb);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'mse-remove-source-buffer', value: String(this._id) });
    };
    MediaSource.prototype.endOfStream = function(reason) {
        if (this.readyState !== 'open') throw new globalThis.DOMException('MediaSource no abierta', 'InvalidStateError');
        this.readyState = 'ended';
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'mse-end-of-stream',
            value: this._id + GS + (reason != null ? String(reason) : '') });
        emit(this, 'sourceended');
    };
    MediaSource.prototype.setLiveSeekableRange = function(start, end) {
        this._liveSeekable = [start, end];
    };
    MediaSource.prototype.clearLiveSeekableRange = function() { this._liveSeekable = null; };
    MediaSource.isTypeSupported = function(type) {
        if (type == null || type === '') return false;
        return /(webm|mp4|ogg|mpeg|x-matroska|aac|mp3|wav)/i.test(String(type));
    };

    // ---- Hook del host: abre/cierra la fuente al adjuntarla a un elemento media ----
    globalThis.__puriy_mse_open = function(id) {
        var ms = sources[id]; if (!ms) return false;
        if (ms.readyState !== 'closed') return false;
        ms.readyState = 'open';
        emit(ms, 'sourceopen');
        return true;
    };
    globalThis.__puriy_mse_close = function(id) {
        var ms = sources[id]; if (!ms) return false;
        ms.readyState = 'closed';
        emit(ms, 'sourceclose');
        return true;
    };

    globalThis.MediaSource = MediaSource;
    globalThis.SourceBuffer = SourceBuffer;
    globalThis.SourceBufferList = SourceBufferList;
    // Variante moderna gestionada por memoria (mismo contrato observable).
    globalThis.ManagedMediaSource = MediaSource;
    globalThis.ManagedSourceBuffer = SourceBuffer;
    void 0;
})();
"#;
