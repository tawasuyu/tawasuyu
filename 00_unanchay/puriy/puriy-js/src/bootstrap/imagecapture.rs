pub(crate) const IMAGECAPTURE_BOOTSTRAP: &str = r#"
// Fase 7.158bis → 7.160 — ImageCapture API (`new ImageCapture(videoTrack)` +
// takePhoto/grabFrame/getPhotoCapabilities/getPhotoSettings). Toma fotos fijas y
// frames de un `MediaStreamTrack` de vídeo (cuelga de mediaDevices 7.101 + Blob/
// ImageBitmap de canvas2d 7.150). Puriy no tiene captura real: `takePhoto`
// resuelve un Blob `image/png` sintético y `grabFrame` un `ImageBitmap` sintético
// (mismo criterio que `OffscreenCanvas.convertToBlob` 7.150) y publica
// kind: 'imagecapture-takephoto'/'imagecapture-grabframe' para que el chrome
// inyecte el frame real (wiring PENDIENTE). Capacidades/ajustes son rangos
// sintéticos plausibles para feature-detection.
(function() {
    if (globalThis.ImageCapture != null) return;
    globalThis.__puriy_imagecapture_next_id = globalThis.__puriy_imagecapture_next_id || 1;

    function ImageCapture(videoTrack) {
        if (!(this instanceof ImageCapture)) { throw new TypeError("ImageCapture requiere 'new'"); }
        if (!videoTrack || videoTrack.kind !== 'video') {
            throw new globalThis.DOMException('ImageCapture requiere un MediaStreamTrack de vídeo', 'NotSupportedError');
        }
        this.track = videoTrack;
        this._id = globalThis.__puriy_imagecapture_next_id++;
    }
    ImageCapture.prototype.takePhoto = function(photoSettings) {
        if (this.track.readyState === 'ended') {
            return Promise.reject(new globalThis.DOMException('El track terminó', 'InvalidStateError'));
        }
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'imagecapture-takephoto', value: String(this._id) });
        return Promise.resolve(new globalThis.Blob([], { type: 'image/png' }));
    };
    ImageCapture.prototype.grabFrame = function() {
        if (this.track.readyState === 'ended') {
            return Promise.reject(new globalThis.DOMException('El track terminó', 'InvalidStateError'));
        }
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'imagecapture-grabframe', value: String(this._id) });
        var w = 1280, h = 720;
        return Promise.resolve(new globalThis.ImageBitmap(w, h));
    };
    ImageCapture.prototype.getPhotoCapabilities = function() {
        return Promise.resolve({
            redEyeReduction: 'never',
            imageHeight: { min: 1, max: 1080, step: 1 },
            imageWidth: { min: 1, max: 1920, step: 1 },
            fillLightMode: ['auto', 'off', 'flash']
        });
    };
    ImageCapture.prototype.getPhotoSettings = function() {
        return Promise.resolve({
            fillLightMode: 'auto',
            imageHeight: 720,
            imageWidth: 1280,
            redEyeReduction: false
        });
    };
    globalThis.ImageCapture = ImageCapture;
    void 0;
})();
"#;
