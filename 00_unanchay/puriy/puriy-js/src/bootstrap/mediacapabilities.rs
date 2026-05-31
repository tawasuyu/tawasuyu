pub(crate) const MEDIACAPABILITIES_BOOTSTRAP: &str = r#"
// Fase 7.149 — Media Capabilities API (`navigator.mediaCapabilities` +
// `MediaCapabilities`). Los reproductores la consultan antes de elegir un stream
// adaptativo: ¿este codec/resolución/bitrate decodifica suave y eficiente en energía?
//   · `decodingInfo(config)` / `encodingInfo(config)` → Promise<{supported, smooth,
//     powerEfficient, configuration}>.
//   · `supported` lo decide el motor por el `contentType` (codec conocido → true);
//     `smooth`/`powerEfficient` son host-decided (default true) y el chrome los fija
//     con `__puriy_set_media_capabilities({smooth, powerEfficient})` cuando conoce el
//     hardware real (mismo molde host-driven que battery/pressure).
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.mediaCapabilities != null) return;

    // Estado host-decided (defaults optimistas; el chrome los baja según el hardware).
    var hints = globalThis.__puriy_media_caps_hints = globalThis.__puriy_media_caps_hints || {
        smooth: true, powerEfficient: true
    };
    globalThis.__puriy_set_media_capabilities = function(h) {
        if (h == null) return;
        if (h.smooth != null) hints.smooth = !!h.smooth;
        if (h.powerEfficient != null) hints.powerEfficient = !!h.powerEfficient;
    };

    function codecSoportado(contentType) {
        if (contentType == null) return false;
        var s = String(contentType);
        // Familias que el resto de puriy ya modela (WebCodecs/MSE).
        return /(avc1|hev1|hvc1|h26[45]|vp0?[89]|vp8|av01|mp4a|opus|vorbis|flac|theora|aac|mp3|webm|mp4|ogg)/i.test(s);
    }

    function evaluar(configuration, esEncode) {
        configuration = configuration || {};
        var sup = false;
        if (configuration.video && codecSoportado(configuration.video.contentType)) sup = true;
        if (configuration.audio && codecSoportado(configuration.audio.contentType)) sup = true;
        // Sin video ni audio explícito → no soportado (config inválida).
        var info = {
            supported: sup,
            smooth: sup ? hints.smooth : false,
            powerEfficient: sup ? hints.powerEfficient : false,
            configuration: configuration
        };
        return Promise.resolve(info);
    }

    function MediaCapabilities() {}
    MediaCapabilities.prototype.decodingInfo = function(configuration) {
        if (configuration == null || (configuration.video == null && configuration.audio == null)) {
            return Promise.reject(new globalThis.TypeError('configuración de decoding inválida'));
        }
        return evaluar(configuration, false);
    };
    MediaCapabilities.prototype.encodingInfo = function(configuration) {
        if (configuration == null || (configuration.video == null && configuration.audio == null)) {
            return Promise.reject(new globalThis.TypeError('configuración de encoding inválida'));
        }
        return evaluar(configuration, true);
    };

    globalThis.MediaCapabilities = MediaCapabilities;
    nav.mediaCapabilities = new MediaCapabilities();
    void 0;
})();
"#;
