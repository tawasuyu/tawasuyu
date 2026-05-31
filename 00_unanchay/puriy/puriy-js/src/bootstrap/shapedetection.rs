pub(crate) const SHAPEDETECTION_BOOTSTRAP: &str = r#"
// Fase 7.168 — Shape Detection API (`BarcodeDetector` / `FaceDetector` / `TextDetector`).
// Detecta códigos de barras/QR, caras y texto en imágenes — apps de escaneo (pagar con
// QR, leer un código de producto), accesibilidad (OCR), filtros de cámara. Las tres
// clases comparten molde: constructor con opciones, `.detect(imageBitmapSource)` →
// `Promise<Detected*[]>`. `BarcodeDetector.getSupportedFormats()` (estático) lista los
// formatos. La detección real (visión por computadora sobre el framebuffer) es del chrome
// (PENDIENTE): por defecto `.detect()` resuelve `[]` (nada detectado, válido para feature
// detection); el chrome puede instalar `__puriy_shape_detect_hook(type, source, options)`
// que devuelva los resultados sincrónicamente. Cada resultado lleva `boundingBox`
// (DOMRectReadOnly-ish) + `cornerPoints`; barcode añade `rawValue`/`format`, text añade
// `rawValue`, face añade `landmarks`.
(function() {
    if (globalThis.BarcodeDetector != null) return;

    var FORMATS = [
        'aztec', 'code_128', 'code_39', 'code_93', 'codabar', 'data_matrix',
        'ean_13', 'ean_8', 'itf', 'pdf417', 'qr_code', 'upc_a', 'upc_e'
    ];

    function runHook(type, source, options) {
        var hook = globalThis.__puriy_shape_detect_hook;
        if (typeof hook === 'function') {
            try {
                var r = hook(type, source, options || null);
                if (Array.isArray(r)) return r;
            } catch (e) {
                globalThis.__puriy_stderr += String(e) + '\n';
            }
        }
        return [];
    }

    function BarcodeDetector(options) {
        var want = (options && Array.isArray(options.formats)) ? options.formats.slice() : null;
        // El spec rechaza al construir si pide un formato no soportado.
        if (want) {
            for (var i = 0; i < want.length; i++) {
                if (FORMATS.indexOf(want[i]) < 0) {
                    throw new globalThis.TypeError('Unsupported barcode format: ' + want[i]);
                }
            }
        }
        this._formats = want;
    }
    BarcodeDetector.getSupportedFormats = function() {
        return Promise.resolve(FORMATS.slice());
    };
    BarcodeDetector.prototype.detect = function(source) {
        return Promise.resolve(runHook('barcode', source, { formats: this._formats }));
    };
    globalThis.BarcodeDetector = BarcodeDetector;

    function FaceDetector(options) {
        this._maxDetectedFaces = (options && options.maxDetectedFaces != null)
            ? (options.maxDetectedFaces | 0) : null;
        this._fastMode = !!(options && options.fastMode);
    }
    FaceDetector.prototype.detect = function(source) {
        var r = runHook('face', source, {
            maxDetectedFaces: this._maxDetectedFaces, fastMode: this._fastMode });
        if (this._maxDetectedFaces != null && r.length > this._maxDetectedFaces) {
            r = r.slice(0, this._maxDetectedFaces);
        }
        return Promise.resolve(r);
    };
    globalThis.FaceDetector = FaceDetector;

    function TextDetector() {}
    TextDetector.prototype.detect = function(source) {
        return Promise.resolve(runHook('text', source, null));
    };
    globalThis.TextDetector = TextDetector;
    void 0;
})();
"#;
