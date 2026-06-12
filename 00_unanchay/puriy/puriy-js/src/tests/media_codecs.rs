//! Tests de WebAudio, WebCodecs, MediaRecorder, MSE, EME, MediaCapabilities.
    use super::*;

    // ---- Fase 7.144 — Web Audio API ----

    #[test]
    fn audio_context_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof AudioContext").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("AudioContext === webkitAudioContext").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("typeof OfflineAudioContext").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof AudioParam").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn audio_context_estado_y_resume() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new AudioContext(); var antes = ctx.state; ctx.resume();").expect("e");
        assert_eq!(rt.eval("antes").expect("e"), JsValue::String("suspended".into()));
        assert_eq!(rt.eval("ctx.state").expect("e"), JsValue::String("running".into()));
        assert_eq!(rt.eval("ctx.sampleRate").expect("e"), JsValue::Number(44100.0));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'audio-state'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn audio_create_oscillator_y_gain() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new AudioContext(); var osc = ctx.createOscillator(); var g = ctx.createGain();")
            .expect("e");
        assert_eq!(rt.eval("osc.type").expect("e"), JsValue::String("sine".into()));
        assert_eq!(rt.eval("osc.frequency.value").expect("e"), JsValue::Number(440.0));
        assert_eq!(rt.eval("g.gain.value").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("osc instanceof OscillatorNode").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("osc instanceof AudioNode").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn audio_param_set_value_at_time() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ctx = new AudioContext(); var g = ctx.createGain(); \
             g.gain.setValueAtTime(0.5, 0); g.gain.linearRampToValueAtTime(0.8, 1);",
        )
        .expect("e");
        assert_eq!(rt.eval("g.gain.value").expect("e"), JsValue::Number(0.8));
    }

    #[test]
    fn audio_oscillator_start_stop_y_onended() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var terminado = false; \
             var ctx = new AudioContext(); var osc = ctx.createOscillator(); \
             osc.onended = function(){ terminado = true; }; \
             osc.connect(ctx.destination); osc.start(); osc.stop(1);",
        )
        .expect("e");
        assert_eq!(rt.eval("terminado").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'audio-node-start'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn audio_connect_encadena() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ctx = new AudioContext(); var osc = ctx.createOscillator(); var g = ctx.createGain(); \
             var ret = osc.connect(g);",
        )
        .expect("e");
        assert_eq!(rt.eval("ret === g").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("osc._outputs.length").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn audio_create_buffer_y_channel_data() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ctx = new AudioContext(); var buf = ctx.createBuffer(2, 100, 48000); \
             var data = buf.getChannelData(0); data[0] = 0.25;",
        )
        .expect("e");
        assert_eq!(rt.eval("buf.numberOfChannels").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("buf.length").expect("e"), JsValue::Number(100.0));
        assert_eq!(rt.eval("data.length").expect("e"), JsValue::Number(100.0));
        assert_eq!(rt.eval("data[0]").expect("e"), JsValue::Number(0.25));
    }

    #[test]
    fn audio_analyser_bin_count() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new AudioContext(); var an = ctx.createAnalyser(); an.fftSize = 1024;").expect("e");
        // frequencyBinCount se fija al construir (fftSize default 2048 → 1024)
        assert_eq!(rt.eval("ctx.createAnalyser().frequencyBinCount").expect("e"), JsValue::Number(1024.0));
        assert_eq!(
            rt.eval("var a = new Uint8Array(8); an.getByteTimeDomainData(a); a[0]").expect("e"),
            JsValue::Number(128.0)
        );
    }

    #[test]
    fn audio_biquad_filter() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new AudioContext(); var f = ctx.createBiquadFilter(); f.type = 'highpass';")
            .expect("e");
        assert_eq!(rt.eval("f.type").expect("e"), JsValue::String("highpass".into()));
        assert_eq!(rt.eval("typeof f.frequency.value").expect("e"), JsValue::String("number".into()));
        assert_eq!(rt.eval("f.Q.value").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn audio_decode_audio_data_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var canales = -1; \
             var ctx = new AudioContext(); var ab = new ArrayBuffer(800); \
             ctx.decodeAudioData(ab).then(function(buf){ canales = buf.numberOfChannels; });",
        )
        .expect("e");
        assert_eq!(rt.eval("canales").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn audio_offline_context_render() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var len = -1; \
             var oc = new OfflineAudioContext(1, 256, 44100); \
             oc.startRendering().then(function(buf){ len = buf.length; });",
        )
        .expect("e");
        assert_eq!(rt.eval("len").expect("e"), JsValue::Number(256.0));
    }

    #[test]
    fn audio_context_close() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new AudioContext(); ctx.close();").expect("e");
        assert_eq!(rt.eval("ctx.state").expect("e"), JsValue::String("closed".into()));
    }

    // ---- Fase 7.145 — WebCodecs ----

    #[test]
    fn webcodecs_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof VideoEncoder").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof VideoDecoder").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof AudioEncoder").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof EncodedVideoChunk").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof VideoFrame").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn webcodecs_video_encoder_configure_cambia_estado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var enc = new VideoEncoder({ output: function(){}, error: function(){} }); \
             var antes = enc.state; \
             enc.configure({ codec: 'avc1.42001f', width: 640, height: 480 });",
        )
        .expect("e");
        assert_eq!(rt.eval("antes").expect("e"), JsValue::String("unconfigured".into()));
        assert_eq!(rt.eval("enc.state").expect("e"), JsValue::String("configured".into()));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'videoencoder-configure'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn webcodecs_video_encoder_encode_publica() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var enc = new VideoEncoder({ output: function(){}, error: function(){} }); \
             enc.configure({ codec: 'avc1.42001f' }); \
             var f = new VideoFrame(null, { codedWidth: 4, codedHeight: 4, timestamp: 1000 }); \
             enc.encode(f);",
        )
        .expect("e");
        assert_eq!(rt.eval("enc.encodeQueueSize").expect("e"), JsValue::Number(1.0));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'videoencoder-encode'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn webcodecs_video_encoder_output_entrega_chunk() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var tipo = null, bytes = -1; \
             var enc = new VideoEncoder({ output: function(chunk){ tipo = chunk.type; bytes = chunk.byteLength; }, error: function(){} }); \
             enc.configure({ codec: 'avc1.42001f' }); \
             enc.encode(new VideoFrame(null, { codedWidth: 2, codedHeight: 2, timestamp: 0 }));",
        )
        .expect("e");
        rt.eval("__puriy_videoencoder_output(1, { type: 'key', timestamp: 0, data: new Uint8Array([1,2,3,4,5]) });")
            .expect("e");
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("key".into()));
        assert_eq!(rt.eval("bytes").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("enc.encodeQueueSize").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn webcodecs_encoded_chunk_copyto() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var c = new EncodedVideoChunk({ type: 'delta', timestamp: 7, data: new Uint8Array([9,8,7]) }); \
             var out = new Uint8Array(3); c.copyTo(out);",
        )
        .expect("e");
        assert_eq!(rt.eval("c.type").expect("e"), JsValue::String("delta".into()));
        assert_eq!(rt.eval("c.timestamp").expect("e"), JsValue::Number(7.0));
        assert_eq!(rt.eval("out[0]").expect("e"), JsValue::Number(9.0));
        assert_eq!(rt.eval("out[2]").expect("e"), JsValue::Number(7.0));
    }

    #[test]
    fn webcodecs_video_frame_propiedades_y_close() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var f = new VideoFrame(null, { codedWidth: 320, codedHeight: 240, timestamp: 5000 }); \
             var alloc = f.allocationSize(); f.close();",
        )
        .expect("e");
        assert_eq!(rt.eval("f.codedWidth").expect("e"), JsValue::Number(320.0));
        assert_eq!(rt.eval("f.displayWidth").expect("e"), JsValue::Number(320.0));
        assert_eq!(rt.eval("alloc").expect("e"), JsValue::Number(115200.0));
        assert_eq!(rt.eval("f._closed").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webcodecs_video_decoder_output_frame() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var w = -1; \
             var dec = new VideoDecoder({ output: function(frame){ w = frame.codedWidth; }, error: function(){} }); \
             dec.configure({ codec: 'avc1.42001f' }); \
             dec.decode(new EncodedVideoChunk({ type: 'key', timestamp: 0, data: new Uint8Array([1]) }));",
        )
        .expect("e");
        rt.eval("__puriy_videodecoder_output(1, { codedWidth: 128, codedHeight: 96, timestamp: 0 });").expect("e");
        assert_eq!(rt.eval("w").expect("e"), JsValue::Number(128.0));
    }

    #[test]
    fn webcodecs_audio_encoder_flujo() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var bytes = -1; \
             var enc = new AudioEncoder({ output: function(chunk){ bytes = chunk.byteLength; }, error: function(){} }); \
             enc.configure({ codec: 'opus', sampleRate: 48000, numberOfChannels: 2 }); \
             enc.encode(new AudioData({ numberOfFrames: 960, numberOfChannels: 2, sampleRate: 48000, timestamp: 0 }));",
        )
        .expect("e");
        rt.eval("__puriy_audioencoder_output(1, { type: 'key', timestamp: 0, data: new Uint8Array([1,2,3,4,5,6]) });")
            .expect("e");
        assert_eq!(rt.eval("bytes").expect("e"), JsValue::Number(6.0));
    }

    #[test]
    fn webcodecs_audio_data_propiedades() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var d = new AudioData({ numberOfFrames: 480, numberOfChannels: 2, sampleRate: 48000, timestamp: 0 });")
            .expect("e");
        assert_eq!(rt.eval("d.numberOfFrames").expect("e"), JsValue::Number(480.0));
        assert_eq!(rt.eval("d.allocationSize()").expect("e"), JsValue::Number(3840.0));
        assert_eq!(rt.eval("Math.round(d.duration)").expect("e"), JsValue::Number(10000.0));
    }

    #[test]
    fn webcodecs_is_config_supported() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var sup = null; \
             VideoEncoder.isConfigSupported({ codec: 'avc1.42001f', width: 640, height: 480 }) \
                 .then(function(r){ sup = r.supported; });",
        )
        .expect("e");
        assert_eq!(rt.eval("sup").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webcodecs_codec_error_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             var enc = new VideoEncoder({ output: function(){}, error: function(e){ err = e.name; } }); \
             enc.configure({ codec: 'avc1.42001f' });",
        )
        .expect("e");
        rt.eval("__puriy_codec_error(1, 'codec no soportado');").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("EncodingError".into()));
    }

    #[test]
    fn webcodecs_encoder_close_cambia_estado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var enc = new VideoEncoder({ output: function(){}, error: function(){} }); \
             enc.configure({ codec: 'avc1.42001f' }); enc.close();",
        )
        .expect("e");
        assert_eq!(rt.eval("enc.state").expect("e"), JsValue::String("closed".into()));
        // tras close, el host ya no entrega salida
        assert_eq!(rt.eval("__puriy_videoencoder_output(1, { type: 'key' })").expect("e"), JsValue::Bool(false));
    }

    // ---- Fase 7.146 — MediaRecorder API ----

    #[test]
    fn media_recorder_existe_y_is_type_supported() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof MediaRecorder").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof BlobEvent").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("MediaRecorder.isTypeSupported('video/webm')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("MediaRecorder.isTypeSupported('application/zip')").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn media_recorder_start_cambia_estado_y_dispara_start() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var arrancado = false; \
             var rec = new MediaRecorder(new MediaStream([]), { mimeType: 'video/webm' }); \
             var antes = rec.state; \
             rec.onstart = function(){ arrancado = true; }; \
             rec.start();",
        )
        .expect("e");
        assert_eq!(rt.eval("antes").expect("e"), JsValue::String("inactive".into()));
        assert_eq!(rt.eval("rec.state").expect("e"), JsValue::String("recording".into()));
        assert_eq!(rt.eval("arrancado").expect("e"), JsValue::Bool(true));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'mediarecorder-start'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn media_recorder_data_host_dispara_dataavailable() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var tam = -1, tipo = null; \
             var rec = new MediaRecorder(new MediaStream([]), { mimeType: 'video/webm' }); \
             rec.ondataavailable = function(ev){ tam = ev.data.size; tipo = ev.data.type; }; \
             rec.start();",
        )
        .expect("e");
        rt.eval("__puriy_mediarecorder_data(1, new Uint8Array([1,2,3,4]), 'video/webm');").expect("e");
        assert_eq!(rt.eval("tam").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("video/webm".into()));
    }

    #[test]
    fn media_recorder_stop_dispara_dataavailable_y_stop() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orden = []; \
             var rec = new MediaRecorder(new MediaStream([])); \
             rec.ondataavailable = function(){ orden.push('data'); }; \
             rec.onstop = function(){ orden.push('stop'); }; \
             rec.start(); rec.stop();",
        )
        .expect("e");
        assert_eq!(rt.eval("rec.state").expect("e"), JsValue::String("inactive".into()));
        assert_eq!(rt.eval("orden.join(',')").expect("e"), JsValue::String("data,stop".into()));
    }

    #[test]
    fn media_recorder_pause_resume() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ev = []; \
             var rec = new MediaRecorder(new MediaStream([])); \
             rec.onpause = function(){ ev.push('p'); }; rec.onresume = function(){ ev.push('r'); }; \
             rec.start(); rec.pause(); rec.resume();",
        )
        .expect("e");
        assert_eq!(rt.eval("ev.join('')").expect("e"), JsValue::String("pr".into()));
        assert_eq!(rt.eval("rec.state").expect("e"), JsValue::String("recording".into()));
    }

    #[test]
    fn media_recorder_request_data() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var n = 0; \
             var rec = new MediaRecorder(new MediaStream([])); \
             rec.ondataavailable = function(){ n++; }; \
             rec.start(); rec.requestData();",
        )
        .expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn media_recorder_mime_type_default() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var rec = new MediaRecorder(new MediaStream([]));").expect("e");
        assert_eq!(rt.eval("rec.mimeType").expect("e"), JsValue::String("video/webm".into()));
        assert_eq!(rt.eval("rec instanceof EventTarget").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn media_recorder_start_doble_lanza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             var rec = new MediaRecorder(new MediaStream([])); \
             rec.start(); \
             try { rec.start(); } catch (e) { err = e.name; }",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("InvalidStateError".into()));
    }

    #[test]
    fn media_recorder_addeventlistener_dataavailable() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var visto = false; \
             var rec = new MediaRecorder(new MediaStream([])); \
             rec.addEventListener('dataavailable', function(ev){ visto = ev instanceof BlobEvent; }); \
             rec.start();",
        )
        .expect("e");
        rt.eval("__puriy_mediarecorder_data(1, new Uint8Array([9]), 'video/webm');").expect("e");
        assert_eq!(rt.eval("visto").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.147 — Media Source Extensions ----
    #[test]
    fn mse_existe_y_is_type_supported() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof MediaSource").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof SourceBuffer").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof SourceBufferList").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof TimeRanges").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("MediaSource.isTypeSupported('video/mp4; codecs=\"avc1.42E01E\"')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("MediaSource.isTypeSupported('application/zip')").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("new MediaSource() instanceof EventTarget").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("typeof ManagedMediaSource").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn mse_estado_inicial_y_open_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var abierto = false; \
             var ms = new MediaSource(); \
             ms.onsourceopen = function(){ abierto = true; }; \
             var url = URL.createObjectURL(ms); \
             var antes = ms.readyState;",
        )
        .expect("e");
        assert_eq!(rt.eval("antes").expect("e"), JsValue::String("closed".into()));
        // El chrome resuelve el blob: URL → la fuente → la abre al adjuntar a un <video>.
        assert_eq!(rt.eval("__puriy_mse_open(__puriy_resolve_blob_url(url)._id)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ms.readyState").expect("e"), JsValue::String("open".into()));
        assert_eq!(rt.eval("abierto").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn mse_add_source_buffer_requiere_open() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; var ms = new MediaSource(); \
             try { ms.addSourceBuffer('video/mp4'); } catch (e) { err = e.name; }",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("InvalidStateError".into()));
        rt.eval("__puriy_mse_open(ms._id); var sb = ms.addSourceBuffer('video/mp4');").expect("e");
        assert_eq!(rt.eval("ms.sourceBuffers.length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("ms.sourceBuffers[0] === sb").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("sb instanceof SourceBuffer").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn mse_append_buffer_ciclo_update() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var orden = []; \
             var ms = new MediaSource(); __puriy_mse_open(ms._id); ms.duration = 10; \
             var sb = ms.addSourceBuffer('video/mp4'); \
             sb.onupdatestart = function(){ orden.push('start'); }; \
             sb.onupdate = function(){ orden.push('update'); }; \
             sb.onupdateend = function(){ orden.push('end'); }; \
             sb.appendBuffer(new Uint8Array([1,2,3,4,5]));",
        )
        .expect("e");
        // El ciclo update/updateend corre vía microtask (drenada por eval).
        assert_eq!(rt.eval("orden.join(',')").expect("e"), JsValue::String("start,update,end".into()));
        assert_eq!(rt.eval("sb.updating").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("sb.buffered.length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("sb.buffered.end(0)").expect("e"), JsValue::Number(10.0));
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'mse-append'; })").expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn mse_end_of_stream_y_remove_source_buffer() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var terminado = false; \
             var ms = new MediaSource(); __puriy_mse_open(ms._id); \
             ms.onsourceended = function(){ terminado = true; }; \
             var sb = ms.addSourceBuffer('audio/mp4'); \
             ms.endOfStream();",
        )
        .expect("e");
        assert_eq!(rt.eval("ms.readyState").expect("e"), JsValue::String("ended".into()));
        assert_eq!(rt.eval("terminado").expect("e"), JsValue::Bool(true));
        rt.eval("ms.removeSourceBuffer(sb);").expect("e");
        assert_eq!(rt.eval("ms.sourceBuffers.length").expect("e"), JsValue::Number(0.0));
    }

    // ---- Fase 7.148 — Encrypted Media Extensions ----
    #[test]
    fn eme_clases_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof MediaKeys").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof MediaKeySession").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof MediaKeySystemAccess").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof MediaKeyStatusMap").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof navigator.requestMediaKeySystemAccess").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn eme_request_access_clearkey_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var ks = null, keys = null; \
             navigator.requestMediaKeySystemAccess('org.w3.clearkey', \
                 [{ initDataTypes: ['cenc'], videoCapabilities: [{ contentType: 'video/mp4; codecs=\"avc1.42E01E\"' }] }]) \
               .then(function(a){ ks = a.keySystem; return a.createMediaKeys(); }) \
               .then(function(mk){ keys = (mk instanceof MediaKeys); });",
        )
        .expect("e");
        assert_eq!(rt.eval("ks").expect("e"), JsValue::String("org.w3.clearkey".into()));
        assert_eq!(rt.eval("keys").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn eme_request_access_no_soportado_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var err = null; \
             navigator.requestMediaKeySystemAccess('com.widevine.alpha', \
                 [{ videoCapabilities: [{ contentType: 'video/mp4' }] }]) \
               .catch(function(e){ err = e.name; });",
        )
        .expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("NotSupportedError".into()));
        // El chrome puede ampliar la lista soportada.
        rt.eval(
            "__puriy_eme_set_supported(['com.widevine.alpha']); var ok = false; \
             navigator.requestMediaKeySystemAccess('com.widevine.alpha', \
                 [{ videoCapabilities: [{ contentType: 'video/mp4' }] }]) \
               .then(function(){ ok = true; });",
        )
        .expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn eme_session_generate_request_y_message_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var mensaje = null, tipo = null; \
             var session = new MediaKeys('org.w3.clearkey').createSession(); \
             session.onmessage = function(ev){ mensaje = ev.message.byteLength; tipo = ev.messageType; }; \
             session.generateRequest('cenc', new Uint8Array([1,2,3]));",
        )
        .expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'eme-message'; })").expect("e"),
            JsValue::Bool(true)
        );
        // El host responde con el mensaje de licencia.
        rt.eval("__puriy_eme_message(session._id, 'license-request', new Uint8Array([9,9,9,9]));").expect("e");
        assert_eq!(rt.eval("mensaje").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("tipo").expect("e"), JsValue::String("license-request".into()));
    }

    #[test]
    fn eme_update_y_keystatus_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var cambio = false; \
             var session = new MediaKeys('org.w3.clearkey').createSession(); \
             session.generateRequest('cenc', new Uint8Array([1])); \
             session.onkeystatuseschange = function(){ cambio = true; }; \
             session.update(new Uint8Array([5,6,7]));",
        )
        .expect("e");
        assert_eq!(
            rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'eme-update'; })").expect("e"),
            JsValue::Bool(true)
        );
        rt.eval("__puriy_eme_keystatus(session._id, 'kid-1', 'usable');").expect("e");
        assert_eq!(rt.eval("cambio").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("session.keyStatuses.size").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("session.keyStatuses.get('kid-1')").expect("e"), JsValue::String("usable".into()));
    }

    #[test]
    fn eme_session_close_resuelve_closed() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var cerrado = false; \
             var session = new MediaKeys('org.w3.clearkey').createSession(); \
             session.closed.then(function(){ cerrado = true; }); \
             session.close();",
        )
        .expect("e");
        assert_eq!(rt.eval("cerrado").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.149 — Media Capabilities API ----
    #[test]
    fn media_capabilities_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof navigator.mediaCapabilities").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("typeof navigator.mediaCapabilities.decodingInfo").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof navigator.mediaCapabilities.encodingInfo").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("navigator.mediaCapabilities instanceof MediaCapabilities").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn media_capabilities_decoding_soportado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var info = null; \
             navigator.mediaCapabilities.decodingInfo({ type: 'media-source', \
                 video: { contentType: 'video/mp4; codecs=\"avc1.42E01E\"', width: 1920, height: 1080, bitrate: 4000000, framerate: 30 } }) \
               .then(function(r){ info = r; });",
        )
        .expect("e");
        assert_eq!(rt.eval("info.supported").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("info.smooth").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("info.powerEfficient").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn media_capabilities_codec_desconocido_no_soportado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var info = null; \
             navigator.mediaCapabilities.decodingInfo({ type: 'file', \
                 video: { contentType: 'video/quicktime; codecs=\"xyz\"', width: 100, height: 100, bitrate: 1000, framerate: 24 } }) \
               .then(function(r){ info = r; });",
        )
        .expect("e");
        assert_eq!(rt.eval("info.supported").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("info.smooth").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn media_capabilities_hints_host_y_config_invalida() {
        let mut rt = JsRuntime::new().expect("rt");
        // El chrome baja smooth según el hardware real.
        rt.eval(
            "__puriy_set_media_capabilities({ smooth: false }); var info = null; \
             navigator.mediaCapabilities.encodingInfo({ type: 'record', \
                 audio: { contentType: 'audio/webm; codecs=\"opus\"' } }).then(function(r){ info = r; });",
        )
        .expect("e");
        assert_eq!(rt.eval("info.supported").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("info.smooth").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("info.powerEfficient").expect("e"), JsValue::Bool(true));
        // Config sin video ni audio → rechaza TypeError.
        rt.eval("var err = null; navigator.mediaCapabilities.decodingInfo({ type: 'file' }).catch(function(e){ err = e.name; });").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("TypeError".into()));
    }

    // ---- Fase 7.150 — Canvas 2D ----
