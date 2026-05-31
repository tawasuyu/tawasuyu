pub(crate) const WEBCODECS_BOOTSTRAP: &str = r#"
// Fase 7.145 — WebCodecs (`VideoEncoder`/`VideoDecoder`/`AudioEncoder`/`AudioDecoder`
// + `EncodedVideoChunk`/`EncodedAudioChunk`/`VideoFrame`/`AudioData`). Acceso de bajo
// nivel a los codecs de medios (sin contenedor). Los chunks/frames y la máquina de
// estados son JS-puros; el codec real (H.264/VP9/AV1/Opus/AAC) es del chrome (wiring
// PENDIENTE): `configure`/`encode`/`decode`/`flush` publican mutaciones y el host
// reinyecta la salida con `__puriy_videoencoder_output(id, init)` → `output(chunk)` /
// `__puriy_videodecoder_output(id, init)` → `output(frame)` (análogo para audio), o
// un fallo con `__puriy_codec_error(id, msg)` → `error(DOMException)`.
(function() {
    if (globalThis.VideoEncoder != null) return;
    var GS = String.fromCharCode(0x1D);
    var nextId = 1;
    var codecs = {};
    globalThis.__puriy_codec_next_id = 1;

    function toBytes(data) {
        if (data == null) return new Uint8Array(0);
        if (data instanceof Uint8Array) return data;
        if (data.buffer instanceof ArrayBuffer) return new Uint8Array(data.buffer, data.byteOffset || 0, data.byteLength);
        if (data instanceof ArrayBuffer) return new Uint8Array(data);
        return new Uint8Array(0);
    }

    // ---- EncodedVideoChunk / EncodedAudioChunk ----
    function makeChunk(name) {
        function Chunk(init) {
            init = init || {};
            this.type = init.type != null ? init.type : 'key';
            this.timestamp = init.timestamp != null ? init.timestamp : 0;
            this.duration = init.duration != null ? init.duration : null;
            this._bytes = toBytes(init.data);
            this.byteLength = this._bytes.length;
        }
        Chunk.prototype.copyTo = function(dest) {
            var d = toBytes(dest);
            for (var i = 0; i < this._bytes.length && i < d.length; i++) d[i] = this._bytes[i];
        };
        Chunk.prototype.constructor = Chunk;
        return Chunk;
    }
    var EncodedVideoChunk = makeChunk('EncodedVideoChunk');
    var EncodedAudioChunk = makeChunk('EncodedAudioChunk');

    // ---- VideoFrame ----
    function VideoFrame(source, init) {
        init = init || {};
        if (source != null && typeof source === 'object' && source.codedWidth == null && init.codedWidth == null) {
            // construido desde otra cosa (canvas/bitmap): tomamos init como config
            init = source; source = null;
        }
        this.format = init.format != null ? init.format : 'I420';
        this.codedWidth = init.codedWidth != null ? init.codedWidth : (init.displayWidth || 0);
        this.codedHeight = init.codedHeight != null ? init.codedHeight : (init.displayHeight || 0);
        this.displayWidth = init.displayWidth != null ? init.displayWidth : this.codedWidth;
        this.displayHeight = init.displayHeight != null ? init.displayHeight : this.codedHeight;
        this.timestamp = init.timestamp != null ? init.timestamp : 0;
        this.duration = init.duration != null ? init.duration : null;
        this.colorSpace = init.colorSpace || {};
        this._closed = false;
    }
    VideoFrame.prototype.allocationSize = function() { return this.codedWidth * this.codedHeight * 3 / 2 | 0; };
    VideoFrame.prototype.copyTo = function(dest) { return Promise.resolve([]); };
    VideoFrame.prototype.clone = function() {
        return new VideoFrame(null, { format: this.format, codedWidth: this.codedWidth, codedHeight: this.codedHeight,
            displayWidth: this.displayWidth, displayHeight: this.displayHeight, timestamp: this.timestamp, duration: this.duration });
    };
    VideoFrame.prototype.close = function() { this._closed = true; };

    // ---- AudioData ----
    function AudioData(init) {
        init = init || {};
        this.format = init.format != null ? init.format : 'f32';
        this.sampleRate = init.sampleRate != null ? init.sampleRate : 44100;
        this.numberOfFrames = init.numberOfFrames != null ? init.numberOfFrames : 0;
        this.numberOfChannels = init.numberOfChannels != null ? init.numberOfChannels : 1;
        this.timestamp = init.timestamp != null ? init.timestamp : 0;
        this.duration = (this.numberOfFrames / this.sampleRate) * 1e6;
        this._closed = false;
    }
    AudioData.prototype.allocationSize = function() { return this.numberOfFrames * this.numberOfChannels * 4; };
    AudioData.prototype.copyTo = function(dest, opts) {};
    AudioData.prototype.clone = function() { return new AudioData(this); };
    AudioData.prototype.close = function() { this._closed = true; };

    // ---- Plantilla de codec (encoder/decoder) ----
    function makeCodec(name, mutationPrefix, supportedDefault) {
        function Codec(init) {
            init = init || {};
            this._id = nextId++; globalThis.__puriy_codec_next_id = nextId;
            codecs[this._id] = this;
            this._output = init.output || function() {};
            this._error = init.error || function() {};
            this.state = 'unconfigured';
            this.encodeQueueSize = 0;
            this.decodeQueueSize = 0;
        }
        Codec.prototype.configure = function(config) {
            if (this.state === 'closed') throw new globalThis.DOMException('codec cerrado', 'InvalidStateError');
            this.state = 'configured';
            this._config = config || {};
            globalThis.__puriy_dirty.push({ id: '__window__', kind: mutationPrefix + '-configure',
                value: this._id + GS + ((config && config.codec) || '') });
        };
        Codec.prototype._submit = function(verb, when) {
            if (this.state !== 'configured') throw new globalThis.DOMException('codec no configurado', 'InvalidStateError');
            this.encodeQueueSize++; this.decodeQueueSize++;
            globalThis.__puriy_dirty.push({ id: '__window__', kind: mutationPrefix + '-' + verb,
                value: this._id + GS + String(when || 0) });
        };
        Codec.prototype.flush = function() {
            this.encodeQueueSize = 0; this.decodeQueueSize = 0;
            return Promise.resolve();
        };
        Codec.prototype.reset = function() {
            this.state = 'unconfigured'; this.encodeQueueSize = 0; this.decodeQueueSize = 0;
        };
        Codec.prototype.close = function() {
            this.state = 'closed';
            delete codecs[this._id];
        };
        Codec.isConfigSupported = function(config) {
            return Promise.resolve({ supported: supportedDefault, config: config || {} });
        };
        return Codec;
    }

    var VideoEncoder = makeCodec('VideoEncoder', 'videoencoder', true);
    VideoEncoder.prototype.encode = function(frame, opts) { this._submit('encode', frame ? frame.timestamp : 0); };
    var VideoDecoder = makeCodec('VideoDecoder', 'videodecoder', true);
    VideoDecoder.prototype.decode = function(chunk) { this._submit('decode', chunk ? chunk.timestamp : 0); };
    var AudioEncoder = makeCodec('AudioEncoder', 'audioencoder', true);
    AudioEncoder.prototype.encode = function(data) { this._submit('encode', data ? data.timestamp : 0); };
    var AudioDecoder = makeCodec('AudioDecoder', 'audiodecoder', true);
    AudioDecoder.prototype.decode = function(chunk) { this._submit('decode', chunk ? chunk.timestamp : 0); };

    // ---- Hooks del host ----
    function dequeue(c) { if (c.encodeQueueSize > 0) c.encodeQueueSize--; if (c.decodeQueueSize > 0) c.decodeQueueSize--; }
    globalThis.__puriy_videoencoder_output = function(id, init) {
        var c = codecs[id]; if (!c) return false; dequeue(c);
        c._output(new EncodedVideoChunk(init), init && init.metadata ? init.metadata : {});
        return true;
    };
    globalThis.__puriy_videodecoder_output = function(id, init) {
        var c = codecs[id]; if (!c) return false; dequeue(c);
        c._output(new VideoFrame(null, init));
        return true;
    };
    globalThis.__puriy_audioencoder_output = function(id, init) {
        var c = codecs[id]; if (!c) return false; dequeue(c);
        c._output(new EncodedAudioChunk(init), init && init.metadata ? init.metadata : {});
        return true;
    };
    globalThis.__puriy_audiodecoder_output = function(id, init) {
        var c = codecs[id]; if (!c) return false; dequeue(c);
        c._output(new AudioData(init));
        return true;
    };
    globalThis.__puriy_codec_error = function(id, msg) {
        var c = codecs[id]; if (!c) return false;
        c._error(new globalThis.DOMException(msg != null ? String(msg) : 'codec error', 'EncodingError'));
        return true;
    };

    globalThis.VideoEncoder = VideoEncoder;
    globalThis.VideoDecoder = VideoDecoder;
    globalThis.AudioEncoder = AudioEncoder;
    globalThis.AudioDecoder = AudioDecoder;
    globalThis.EncodedVideoChunk = EncodedVideoChunk;
    globalThis.EncodedAudioChunk = EncodedAudioChunk;
    globalThis.VideoFrame = VideoFrame;
    globalThis.AudioData = AudioData;
    void 0;
})();
"#;
