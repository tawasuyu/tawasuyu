pub(crate) const MEDIARECORDER_BOOTSTRAP: &str = r#"
// Fase 7.146 — MediaRecorder API (`MediaRecorder` + `BlobEvent`). Graba un
// `MediaStream` (Fase mediadevices) a chunks `Blob` (Fase 7.62). La máquina de
// estados (`inactive`/`recording`/`paused`) y los eventos son JS-puros; el encode
// real (WebM/MP4) es del chrome (wiring PENDIENTE):
//   · start/stop/pause/resume publican kind: 'mediarecorder-*' (value `<id> GS <arg>`).
//   · El host empuja datos codificados con `__puriy_mediarecorder_data(id, bytes, type)`
//     → `dataavailable` con un `BlobEvent` cuyo `.data` es un Blob.
//   · `stop()` emite un `dataavailable` final (con lo acumulado, o vacío) y luego `stop`.
(function() {
    if (globalThis.MediaRecorder != null) return;
    var GS = String.fromCharCode(0x1D);
    var nextId = 1;
    var recorders = {};

    function BlobEvent(type, init) {
        init = init || {};
        this.type = type;
        this.data = init.data != null ? init.data : null;
        this.timecode = init.timecode != null ? init.timecode : 0;
    }
    globalThis.BlobEvent = BlobEvent;

    function MediaRecorder(stream, opts) {
        globalThis.EventTarget.call(this);
        this._id = nextId++;
        recorders[this._id] = this;
        this.stream = stream || null;
        opts = opts || {};
        this.mimeType = opts.mimeType != null ? opts.mimeType : 'video/webm';
        this.videoBitsPerSecond = opts.videoBitsPerSecond != null ? opts.videoBitsPerSecond : 0;
        this.audioBitsPerSecond = opts.audioBitsPerSecond != null ? opts.audioBitsPerSecond : 0;
        this.state = 'inactive';
        this.onstart = null; this.onstop = null; this.ondataavailable = null;
        this.onpause = null; this.onresume = null; this.onerror = null;
        this._buffered = [];   // bytes acumulados entre dataavailable
    }
    MediaRecorder.prototype = Object.create(globalThis.EventTarget.prototype);
    MediaRecorder.prototype.constructor = MediaRecorder;

    function emit(self, type, ev) {
        var h = self['on' + type];
        if (typeof h === 'function') { try { h.call(self, ev); } catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; } }
        self.dispatchEvent(ev);
    }
    function fireData(self) {
        var blob = new globalThis.Blob(self._buffered.slice(), { type: self.mimeType });
        self._buffered = [];
        emit(self, 'dataavailable', new BlobEvent('dataavailable', { data: blob }));
    }

    MediaRecorder.prototype.start = function(timeslice) {
        if (this.state !== 'inactive') {
            throw new globalThis.DOMException('grabación ya activa', 'InvalidStateError');
        }
        this.state = 'recording';
        this._buffered = [];
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'mediarecorder-start',
            value: this._id + GS + String(timeslice != null ? timeslice : '') });
        emit(this, 'start', { type: 'start' });
    };
    MediaRecorder.prototype.stop = function() {
        if (this.state === 'inactive') return;
        this.state = 'inactive';
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'mediarecorder-stop', value: String(this._id) });
        fireData(this);
        emit(this, 'stop', { type: 'stop' });
    };
    MediaRecorder.prototype.pause = function() {
        if (this.state !== 'recording') return;
        this.state = 'paused';
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'mediarecorder-pause', value: String(this._id) });
        emit(this, 'pause', { type: 'pause' });
    };
    MediaRecorder.prototype.resume = function() {
        if (this.state !== 'paused') return;
        this.state = 'recording';
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'mediarecorder-resume', value: String(this._id) });
        emit(this, 'resume', { type: 'resume' });
    };
    MediaRecorder.prototype.requestData = function() {
        if (this.state === 'inactive') return;
        fireData(this);
    };
    MediaRecorder.isTypeSupported = function(type) {
        if (type == null || type === '') return true;
        return /(webm|mp4|ogg|wav|x-matroska|mpeg)/i.test(String(type));
    };

    // ---- Hook del host: empuja datos codificados ----
    globalThis.__puriy_mediarecorder_data = function(id, bytes, type) {
        var r = recorders[id]; if (!r) return false;
        if (r.state === 'inactive') return false;
        var blob = new globalThis.Blob(bytes != null ? [bytes] : [], { type: type || r.mimeType });
        emit(r, 'dataavailable', new BlobEvent('dataavailable', { data: blob }));
        return true;
    };

    globalThis.MediaRecorder = MediaRecorder;
    void 0;
})();
"#;
