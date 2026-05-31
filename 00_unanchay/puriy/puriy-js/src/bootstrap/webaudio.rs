pub(crate) const WEBAUDIO_BOOTSTRAP: &str = r#"
// Fase 7.144 — Web Audio API (`AudioContext`/`OfflineAudioContext` + nodos).
// Síntesis y procesamiento de audio mediante un grafo de nodos. El grafo, los
// parámetros (`AudioParam`) y los buffers son JS-puros y funcionales; la salida
// sonora real (mezcla y render por sample) es del chrome (wiring PENDIENTE):
//   · El ciclo de vida (`resume`/`suspend`/`close`) publica kind: 'audio-state'.
//   · `oscillator.start()`/`stop()` y `bufferSource.start()`/`stop()` publican
//     kind: 'audio-node-start'/'audio-node-stop' (value `<ctxId> GS <nodeKind> GS <when>`).
//   · `decodeAudioData` resuelve con un AudioBuffer sintético local (el decode real
//     —mp3/aac/opus— lo hace el chrome; aquí basta el contrato observable).
(function() {
    if (globalThis.AudioContext != null) return;
    var GS = String.fromCharCode(0x1D);
    var nextCtxId = 1;

    // ---- AudioParam ----
    function AudioParam(value, min, max) {
        this.value = value;
        this.defaultValue = value;
        this.minValue = (min != null) ? min : -3.4028235e38;
        this.maxValue = (max != null) ? max : 3.4028235e38;
        this.automationRate = 'a-rate';
    }
    AudioParam.prototype.setValueAtTime = function(v, t) { this.value = v; return this; };
    AudioParam.prototype.linearRampToValueAtTime = function(v, t) { this.value = v; return this; };
    AudioParam.prototype.exponentialRampToValueAtTime = function(v, t) { this.value = v; return this; };
    AudioParam.prototype.setTargetAtTime = function(v, t, c) { this.value = v; return this; };
    AudioParam.prototype.setValueCurveAtTime = function(curve, t, d) {
        if (curve && curve.length) this.value = curve[curve.length - 1];
        return this;
    };
    AudioParam.prototype.cancelScheduledValues = function(t) { return this; };
    AudioParam.prototype.cancelAndHoldAtTime = function(t) { return this; };

    function mix(proto) {
        proto.addEventListener = function(type, fn) { (this._listeners[type] = this._listeners[type] || []).push(fn); };
        proto.removeEventListener = function(type, fn) {
            var a = this._listeners[type]; if (!a) return;
            var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
        };
        proto.dispatchEvent = function(ev) {
            var a = this._listeners[ev.type];
            if (a) { var c = a.slice(); for (var i = 0; i < c.length; i++) c[i].call(this, ev); }
            if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
            return true;
        };
    }

    // ---- AudioNode base ----
    function AudioNode(ctx, kind) {
        this.context = ctx;
        this._kind = kind;
        this.numberOfInputs = 1;
        this.numberOfOutputs = 1;
        this.channelCount = 2;
        this.channelCountMode = 'max';
        this.channelInterpretation = 'speakers';
        this._outputs = [];
        this._listeners = {};
    }
    mix(AudioNode.prototype);
    AudioNode.prototype.connect = function(dest) {
        this._outputs.push(dest);
        return dest;                 // permite encadenar a.connect(b).connect(c) cuando dest es un AudioNode
    };
    AudioNode.prototype.disconnect = function(dest) {
        if (dest == null) { this._outputs = []; return; }
        var i = this._outputs.indexOf(dest); if (i >= 0) this._outputs.splice(i, 1);
    };

    function extend(Ctor) {
        Ctor.prototype = Object.create(AudioNode.prototype);
        Ctor.prototype.constructor = Ctor;
        return Ctor;
    }

    // ---- AudioBuffer ----
    function AudioBuffer(opts) {
        opts = opts || {};
        this.numberOfChannels = opts.numberOfChannels || 1;
        this.length = opts.length || 0;
        this.sampleRate = opts.sampleRate || 44100;
        this.duration = this.length / this.sampleRate;
        this._channels = [];
        for (var c = 0; c < this.numberOfChannels; c++) this._channels.push(new Float32Array(this.length));
    }
    AudioBuffer.prototype.getChannelData = function(c) {
        if (c < 0 || c >= this.numberOfChannels) throw new globalThis.DOMException('canal fuera de rango', 'IndexSizeError');
        return this._channels[c];
    };
    AudioBuffer.prototype.copyFromChannel = function(dest, c, start) {
        start = start || 0;
        var src = this._channels[c];
        for (var i = 0; i < dest.length && (start + i) < src.length; i++) dest[i] = src[start + i];
    };
    AudioBuffer.prototype.copyToChannel = function(src, c, start) {
        start = start || 0;
        var dst = this._channels[c];
        for (var i = 0; i < src.length && (start + i) < dst.length; i++) dst[start + i] = src[i];
    };

    // ---- Nodos concretos ----
    function OscillatorNode(ctx, opts) {
        AudioNode.call(this, ctx, 'oscillator');
        opts = opts || {};
        this.numberOfInputs = 0;
        this.type = opts.type || 'sine';
        this.frequency = new AudioParam(opts.frequency != null ? opts.frequency : 440, -22050, 22050);
        this.detune = new AudioParam(opts.detune != null ? opts.detune : 0);
        this._started = false; this._stopped = false;
    }
    extend(OscillatorNode);
    OscillatorNode.prototype.start = function(when) {
        if (this._started) throw new globalThis.DOMException('ya iniciado', 'InvalidStateError');
        this._started = true;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'audio-node-start',
            value: this.context._id + GS + 'oscillator' + GS + String(when || 0) });
    };
    OscillatorNode.prototype.stop = function(when) {
        this._stopped = true;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'audio-node-stop',
            value: this.context._id + GS + 'oscillator' + GS + String(when || 0) });
        var self = this;
        Promise.resolve().then(function() { self.dispatchEvent({ type: 'ended' }); });
    };
    OscillatorNode.prototype.setPeriodicWave = function() {};

    function GainNode(ctx, opts) {
        AudioNode.call(this, ctx, 'gain');
        this.gain = new AudioParam((opts && opts.gain != null) ? opts.gain : 1);
    }
    extend(GainNode);

    function AudioBufferSourceNode(ctx, opts) {
        AudioNode.call(this, ctx, 'buffersource');
        opts = opts || {};
        this.numberOfInputs = 0;
        this.buffer = opts.buffer || null;
        this.playbackRate = new AudioParam(1);
        this.detune = new AudioParam(0);
        this.loop = !!opts.loop;
        this.loopStart = 0; this.loopEnd = 0;
        this._started = false;
    }
    extend(AudioBufferSourceNode);
    AudioBufferSourceNode.prototype.start = function(when) {
        if (this._started) throw new globalThis.DOMException('ya iniciado', 'InvalidStateError');
        this._started = true;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'audio-node-start',
            value: this.context._id + GS + 'buffersource' + GS + String(when || 0) });
    };
    AudioBufferSourceNode.prototype.stop = function(when) {
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'audio-node-stop',
            value: this.context._id + GS + 'buffersource' + GS + String(when || 0) });
        var self = this;
        Promise.resolve().then(function() { self.dispatchEvent({ type: 'ended' }); });
    };

    function BiquadFilterNode(ctx, opts) {
        AudioNode.call(this, ctx, 'biquad');
        opts = opts || {};
        this.type = opts.type || 'lowpass';
        this.frequency = new AudioParam(opts.frequency != null ? opts.frequency : 350, 0, 22050);
        this.detune = new AudioParam(0);
        this.Q = new AudioParam(opts.Q != null ? opts.Q : 1);
        this.gain = new AudioParam(opts.gain != null ? opts.gain : 0);
    }
    extend(BiquadFilterNode);
    BiquadFilterNode.prototype.getFrequencyResponse = function() {};

    function AnalyserNode(ctx, opts) {
        AudioNode.call(this, ctx, 'analyser');
        opts = opts || {};
        this.fftSize = opts.fftSize || 2048;
        this.frequencyBinCount = this.fftSize / 2;
        this.minDecibels = -100; this.maxDecibels = -30;
        this.smoothingTimeConstant = 0.8;
    }
    extend(AnalyserNode);
    AnalyserNode.prototype.getByteFrequencyData = function(arr) { for (var i = 0; i < arr.length; i++) arr[i] = 0; };
    AnalyserNode.prototype.getFloatFrequencyData = function(arr) { for (var i = 0; i < arr.length; i++) arr[i] = -Infinity; };
    AnalyserNode.prototype.getByteTimeDomainData = function(arr) { for (var i = 0; i < arr.length; i++) arr[i] = 128; };
    AnalyserNode.prototype.getFloatTimeDomainData = function(arr) { for (var i = 0; i < arr.length; i++) arr[i] = 0; };

    function DelayNode(ctx, opts) {
        AudioNode.call(this, ctx, 'delay');
        this.delayTime = new AudioParam((opts && opts.delayTime != null) ? opts.delayTime : 0, 0, 180);
    }
    extend(DelayNode);

    function StereoPannerNode(ctx, opts) {
        AudioNode.call(this, ctx, 'panner');
        this.pan = new AudioParam((opts && opts.pan != null) ? opts.pan : 0, -1, 1);
    }
    extend(StereoPannerNode);

    function ConstantSourceNode(ctx, opts) {
        AudioNode.call(this, ctx, 'constant');
        this.numberOfInputs = 0;
        this.offset = new AudioParam((opts && opts.offset != null) ? opts.offset : 1);
        this._started = false;
    }
    extend(ConstantSourceNode);
    ConstantSourceNode.prototype.start = function() { this._started = true; };
    ConstantSourceNode.prototype.stop = function() {
        var self = this; Promise.resolve().then(function() { self.dispatchEvent({ type: 'ended' }); });
    };

    function AudioDestinationNode(ctx) {
        AudioNode.call(this, ctx, 'destination');
        this.numberOfOutputs = 0;
        this.maxChannelCount = 2;
    }
    extend(AudioDestinationNode);

    // ---- BaseAudioContext ----
    function setupContext(ctx, sampleRate) {
        ctx._id = nextCtxId++;
        ctx.sampleRate = sampleRate || 44100;
        ctx.currentTime = 0;
        ctx.state = 'suspended';
        ctx.destination = new AudioDestinationNode(ctx);
        ctx.listener = { positionX: new AudioParam(0), positionY: new AudioParam(0), positionZ: new AudioParam(0) };
        ctx._listeners = {};
        ctx.audioWorklet = { addModule: function() { return Promise.resolve(); } };
    }
    function ctxProto(proto) {
        mix(proto);
        proto.createOscillator = function() { return new OscillatorNode(this); };
        proto.createGain = function() { return new GainNode(this); };
        proto.createBufferSource = function() { return new AudioBufferSourceNode(this); };
        proto.createBiquadFilter = function() { return new BiquadFilterNode(this); };
        proto.createAnalyser = function() { return new AnalyserNode(this); };
        proto.createDelay = function(max) { return new DelayNode(this); };
        proto.createStereoPanner = function() { return new StereoPannerNode(this); };
        proto.createConstantSource = function() { return new ConstantSourceNode(this); };
        proto.createBuffer = function(channels, length, sampleRate) {
            return new AudioBuffer({ numberOfChannels: channels, length: length, sampleRate: sampleRate });
        };
        proto.createDynamicsCompressor = function() {
            var n = new AudioNode(this, 'compressor');
            n.threshold = new AudioParam(-24); n.knee = new AudioParam(30);
            n.ratio = new AudioParam(12); n.attack = new AudioParam(0.003);
            n.release = new AudioParam(0.25); n.reduction = 0;
            return n;
        };
        proto.createWaveShaper = function() { var n = new AudioNode(this, 'waveshaper'); n.curve = null; n.oversample = 'none'; return n; };
        proto.createConvolver = function() { var n = new AudioNode(this, 'convolver'); n.buffer = null; n.normalize = true; return n; };
        proto.createChannelSplitter = function(n) { var x = new AudioNode(this, 'splitter'); x.numberOfOutputs = n || 6; return x; };
        proto.createChannelMerger = function(n) { var x = new AudioNode(this, 'merger'); x.numberOfInputs = n || 6; return x; };
        proto.createPanner = function() { return new AudioNode(this, 'panner3d'); };
        proto.createMediaStreamSource = function(stream) { var n = new AudioNode(this, 'mediastreamsource'); n.mediaStream = stream; n.numberOfInputs = 0; return n; };
        proto.createMediaElementSource = function(el) { var n = new AudioNode(this, 'mediaelementsource'); n.mediaElement = el; n.numberOfInputs = 0; return n; };
        proto.createMediaStreamDestination = function() { var n = new AudioNode(this, 'mediastreamdestination'); n.numberOfOutputs = 0; n.stream = (globalThis.MediaStream ? new globalThis.MediaStream([]) : null); return n; };
        proto.decodeAudioData = function(arrayBuffer, successCb, errorCb) {
            var sr = this.sampleRate;
            var bytes = (arrayBuffer && arrayBuffer.byteLength != null) ? arrayBuffer.byteLength : 0;
            var len = Math.max(1, Math.floor(bytes / 4) || sr);  // ~1s si no hay bytes
            var buf = new AudioBuffer({ numberOfChannels: 2, length: len, sampleRate: sr });
            if (typeof successCb === 'function') successCb(buf);
            return Promise.resolve(buf);
        };
    }

    function AudioContext(opts) {
        opts = opts || {};
        setupContext(this, opts.sampleRate);
        this.baseLatency = 0; this.outputLatency = 0;
    }
    ctxProto(AudioContext.prototype);
    AudioContext.prototype.resume = function() {
        this.state = 'running';
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'audio-state', value: this._id + GS + 'running' });
        return Promise.resolve();
    };
    AudioContext.prototype.suspend = function() {
        this.state = 'suspended';
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'audio-state', value: this._id + GS + 'suspended' });
        return Promise.resolve();
    };
    AudioContext.prototype.close = function() {
        this.state = 'closed';
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'audio-state', value: this._id + GS + 'closed' });
        return Promise.resolve();
    };
    AudioContext.prototype.getOutputTimestamp = function() { return { contextTime: this.currentTime, performanceTime: 0 }; };

    function OfflineAudioContext(channelsOrOpts, length, sampleRate) {
        var ch;
        if (channelsOrOpts != null && typeof channelsOrOpts === 'object') {
            ch = channelsOrOpts.numberOfChannels || 1; length = channelsOrOpts.length; sampleRate = channelsOrOpts.sampleRate;
        } else { ch = channelsOrOpts || 1; }
        setupContext(this, sampleRate);
        this.numberOfChannels = ch; this.length = length || 0;
    }
    ctxProto(OfflineAudioContext.prototype);
    OfflineAudioContext.prototype.startRendering = function() {
        this.state = 'running';
        var buf = new AudioBuffer({ numberOfChannels: this.numberOfChannels, length: this.length, sampleRate: this.sampleRate });
        return Promise.resolve(buf);
    };
    OfflineAudioContext.prototype.resume = function() { this.state = 'running'; return Promise.resolve(); };
    OfflineAudioContext.prototype.suspend = function() { return Promise.resolve(); };

    globalThis.AudioContext = AudioContext;
    globalThis.webkitAudioContext = AudioContext;
    globalThis.OfflineAudioContext = OfflineAudioContext;
    globalThis.AudioNode = AudioNode;
    globalThis.AudioParam = AudioParam;
    globalThis.AudioBuffer = AudioBuffer;
    globalThis.OscillatorNode = OscillatorNode;
    globalThis.GainNode = GainNode;
    globalThis.AudioBufferSourceNode = AudioBufferSourceNode;
    globalThis.BiquadFilterNode = BiquadFilterNode;
    globalThis.AnalyserNode = AnalyserNode;
    globalThis.DelayNode = DelayNode;
    globalThis.StereoPannerNode = StereoPannerNode;
    globalThis.ConstantSourceNode = ConstantSourceNode;
    globalThis.AudioDestinationNode = AudioDestinationNode;
    void 0;
})();
"#;
