//! Tests de Canvas2D, WebGL, CSS FontLoading, Geometry, CSSOM, Scheduler, URLPattern, WebGPU, WebXR, BackgroundFetch, ImageCapture, CompressionStreams, WindowManagement, LocalFonts, WebOTP, PiP, DocumentPiP, CloseWatcher, ShapeDetection, EditContext, VirtualKeyboard.
    use super::*;

    // ---- Fase 7.150 — Canvas 2D ----
    #[test]
    fn canvas2d_offscreen_y_contexto() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var cv = new OffscreenCanvas(320, 240); var ctx = cv.getContext('2d');").expect("e");
        assert_eq!(rt.eval("cv.width").expect("e"), JsValue::Number(320.0));
        assert_eq!(rt.eval("cv.height").expect("e"), JsValue::Number(240.0));
        assert_eq!(rt.eval("ctx instanceof CanvasRenderingContext2D").expect("e"), JsValue::Bool(true));
        // getContext('2d') es idempotente: misma instancia.
        assert_eq!(rt.eval("cv.getContext('2d') === ctx").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ctx.canvas === cv").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn canvas2d_fill_rect_registra_comando() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); ctx.fillStyle = '#ff0000'; ctx.fillRect(1, 2, 3, 4);").expect("e");
        assert_eq!(rt.eval("ctx._cmds.length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("ctx._cmds[0][0]").expect("e"), JsValue::String("fillRect".into()));
        assert_eq!(rt.eval("ctx._cmds[0][3]").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("ctx._cmds[0][5]").expect("e"), JsValue::String("#ff0000".into()));
    }

    #[test]
    fn canvas2d_save_restore_estado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); ctx.fillStyle = 'red'; ctx.save(); ctx.fillStyle = 'blue';").expect("e");
        assert_eq!(rt.eval("ctx.fillStyle").expect("e"), JsValue::String("blue".into()));
        rt.eval("ctx.restore();").expect("e");
        assert_eq!(rt.eval("ctx.fillStyle").expect("e"), JsValue::String("red".into()));
    }

    #[test]
    fn canvas2d_transform_acumula() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); ctx.translate(5, 7); ctx.scale(2, 2); var m = ctx.getTransform();").expect("e");
        assert_eq!(rt.eval("m.e").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("m.f").expect("e"), JsValue::Number(7.0));
        assert_eq!(rt.eval("m.a").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("m.isIdentity").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn canvas2d_path2d_y_beginpath() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new Path2D(); p.moveTo(0, 0); p.lineTo(10, 10); p.closePath();").expect("e");
        assert_eq!(rt.eval("p._cmds.length").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("p._cmds[1][0]").expect("e"), JsValue::String("lineTo".into()));
        // ctx delega su path actual.
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); ctx.beginPath(); ctx.moveTo(1, 1); ctx.lineTo(2, 2);").expect("e");
        assert_eq!(rt.eval("ctx._path._cmds.length").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn canvas2d_image_data() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); var id = ctx.createImageData(4, 5);").expect("e");
        assert_eq!(rt.eval("id.width").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("id.height").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("id.data.length").expect("e"), JsValue::Number(80.0));
        assert_eq!(rt.eval("id.data instanceof Uint8ClampedArray").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("new ImageData(2, 2).colorSpace").expect("e"), JsValue::String("srgb".into()));
        // getImageData de un canvas virgen → todo transparente (0).
        assert_eq!(rt.eval("ctx.getImageData(0, 0, 3, 3).data.length").expect("e"), JsValue::Number(36.0));
        assert_eq!(rt.eval("ctx.getImageData(0, 0, 3, 3).data[3]").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn canvas2d_get_put_image_data_roundtrip() {
        // Fase 7.202 — getImageData/putImageData reales sobre un framebuffer JS.
        let mut rt = JsRuntime::new().expect("rt");
        // putImageData de un pixel rojo opaco en (2,3) → getImageData lo lee.
        rt.eval(
            "var ctx = new OffscreenCanvas(10, 10).getContext('2d');\
             var id = ctx.createImageData(1, 1);\
             id.data[0]=255; id.data[1]=0; id.data[2]=0; id.data[3]=255;\
             ctx.putImageData(id, 2, 3);\
             var got = ctx.getImageData(2, 3, 1, 1);",
        )
        .expect("e");
        assert_eq!(rt.eval("got.data[0]").expect("e"), JsValue::Number(255.0));
        assert_eq!(rt.eval("got.data[3]").expect("e"), JsValue::Number(255.0));
        // Un pixel vecino sigue transparente.
        assert_eq!(rt.eval("ctx.getImageData(0,0,1,1).data[3]").expect("e"), JsValue::Number(0.0));
        // fillRect sólido (transform identidad) → getImageData lee el color.
        rt.eval("ctx.fillStyle='#00ff00'; ctx.fillRect(5,5,2,2); var f=ctx.getImageData(5,5,1,1);")
            .expect("e");
        assert_eq!(rt.eval("f.data[1]").expect("e"), JsValue::Number(255.0));
        assert_eq!(rt.eval("f.data[3]").expect("e"), JsValue::Number(255.0));
        // putImageData registró un comando (base64) para que el painter lo dibuje.
        assert_eq!(
            rt.eval("ctx._cmds.some(function(c){return c[0]==='putImageData' && typeof c[5]==='string';})")
                .expect("e"),
            JsValue::Bool(true)
        );
    }

    #[test]
    fn canvas2d_drawimage_refleja_en_getimagedata() {
        // Fase 7.203 — con los píxeles del <img> inyectados, un drawImage
        // rasteriza al framebuffer y getImageData lo lee (pipeline de filtros).
        let mut rt = JsRuntime::new().expect("rt");
        // 1×1 rojo opaco (FF 00 00 FF) keyed por "redpix".
        rt.set_canvas_image_pixels("redpix", 1, 1, "/wAA/w==").expect("inject");
        rt.eval(
            "var ctx=new OffscreenCanvas(10,10).getContext('2d');\
             var g0=ctx.getImageData(3,3,1,1);\
             ctx.drawImage({src:'redpix'}, 3, 3);\
             var g=ctx.getImageData(3,3,1,1);",
        )
        .expect("e");
        // Antes del drawImage: transparente.
        assert_eq!(rt.eval("g0.data[3]").expect("e"), JsValue::Number(0.0));
        // Después: rojo opaco leído del framebuffer.
        assert_eq!(rt.eval("g.data[0]").expect("e"), JsValue::Number(255.0));
        assert_eq!(rt.eval("g.data[1]").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("g.data[3]").expect("e"), JsValue::Number(255.0));
        // Replay: drawImage ANTES del primer getImageData igual se refleja.
        rt.eval(
            "var c2=new OffscreenCanvas(8,8).getContext('2d');\
             c2.drawImage({src:'redpix'}, 0, 0);\
             var gg=c2.getImageData(0,0,1,1);",
        )
        .expect("e");
        assert_eq!(rt.eval("gg.data[0]").expect("e"), JsValue::Number(255.0));
        // Una imagen sin píxeles inyectados → no-op (transparente).
        rt.eval(
            "var c3=new OffscreenCanvas(8,8).getContext('2d');\
             c3.getImageData(0,0,1,1);\
             c3.drawImage({src:'ausente'}, 0, 0);\
             var gh=c3.getImageData(0,0,1,1);",
        )
        .expect("e");
        assert_eq!(rt.eval("gh.data[3]").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn canvas2d_getimagedata_refleja_fillrect_previo() {
        // El framebuffer se crea perezoso en el primer getImageData y reproduce
        // los fillRect/clearRect ya en el log → un fillRect ANTES del primer
        // getImageData igual se lee (Fase 7.202, replay).
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(
            "var c = new OffscreenCanvas(8, 8).getContext('2d');\
             c.fillStyle='rgb(0,0,255)'; c.fillRect(0,0,4,4);\
             var g = c.getImageData(1, 1, 1, 1);",
        )
        .expect("e");
        assert_eq!(rt.eval("g.data[2]").expect("e"), JsValue::Number(255.0));
        assert_eq!(rt.eval("g.data[3]").expect("e"), JsValue::Number(255.0));
        // Fuera del rect lleno → transparente.
        assert_eq!(rt.eval("c.getImageData(6,6,1,1).data[3]").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn canvas2d_measure_text() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); ctx.font = '20px serif'; var tm = ctx.measureText('hola');").expect("e");
        assert_eq!(rt.eval("tm.width").expect("e"), JsValue::Number(40.0));
        assert_eq!(rt.eval("tm.fontBoundingBoxAscent").expect("e"), JsValue::Number(18.0));
    }

    #[test]
    fn canvas2d_gradient_y_image_bitmap() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); var g = ctx.createLinearGradient(0, 0, 100, 0); g.addColorStop(0, 'red'); g.addColorStop(1, 'blue');").expect("e");
        assert_eq!(rt.eval("g instanceof CanvasGradient").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("g._stops.length").expect("e"), JsValue::Number(2.0));
        rt.eval("var bmp = null; createImageBitmap({ width: 64, height: 32 }).then(function(b){ bmp = b; });").expect("e");
        assert_eq!(rt.eval("bmp.width").expect("e"), JsValue::Number(64.0));
        assert_eq!(rt.eval("bmp instanceof ImageBitmap").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.196 — Canvas 2D del DOM cableado al chrome ----
    #[test]
    fn canvas2d_dom_get_context_y_width_height() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "about:test", "").expect("doc");
        let mut snap_c = snap("c", "canvas", "");
        snap_c.attributes = vec![("width".into(), "200".into()), ("height".into(), "120".into())];
        rt.set_elements(&[snap_c]).expect("els");
        // El canvas DOM expone width/height de sus atributos y un getContext('2d').
        assert_eq!(rt.eval("document.getElementById('c').width").expect("e"), JsValue::Number(200.0));
        assert_eq!(rt.eval("document.getElementById('c').height").expect("e"), JsValue::Number(120.0));
        rt.eval("var ctx = document.getElementById('c').getContext('2d');").expect("e");
        assert_eq!(rt.eval("ctx instanceof CanvasRenderingContext2D").expect("e"), JsValue::Bool(true));
        // getContext es idempotente y el contexto apunta al elemento DOM.
        assert_eq!(rt.eval("document.getElementById('c').getContext('2d') === ctx").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ctx.canvas._id").expect("e"), JsValue::String("c".into()));
    }

    #[test]
    fn canvas_json_serializa_comandos_del_dom_canvas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "about:test", "").expect("doc");
        let mut snap_c = snap("c", "canvas", "");
        snap_c.attributes = vec![("width".into(), "100".into()), ("height".into(), "50".into())];
        rt.set_elements(&[snap_c]).expect("els");
        // Sin canvas dibujado todavía: el frame existe pero con cmds vacíos.
        // Tras pedir contexto y pintar, canvas_json refleja el comando.
        rt.eval("var ctx = document.getElementById('c').getContext('2d'); ctx.fillStyle = '#00ff00'; ctx.fillRect(5, 6, 10, 20);").expect("e");
        let json = rt.canvas_json().expect("debería haber un frame de canvas");
        assert!(json.contains("\"id\":\"c\""), "json: {json}");
        assert!(json.contains("\"width\":100"), "json: {json}");
        assert!(json.contains("fillRect"), "json: {json}");
        assert!(json.contains("#00ff00"), "json: {json}");
    }

    #[test]
    fn canvas_json_none_sin_canvas() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.set_document("t", "about:test", "").expect("doc");
        rt.set_elements(&[snap("d", "div", "hola")]).expect("els");
        assert!(rt.canvas_json().is_none(), "sin canvas no hay frame");
    }

    // ---- Fase 7.151 — WebGL ----
    #[test]
    fn webgl_contexto_via_offscreen() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var gl = new OffscreenCanvas(64, 64).getContext('webgl');").expect("e");
        assert_eq!(rt.eval("typeof gl").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("gl.drawingBufferWidth").expect("e"), JsValue::Number(64.0));
        assert_eq!(rt.eval("typeof new OffscreenCanvas(1,1).getContext('webgl2')").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("typeof WebGLRenderingContext").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn webgl_constantes() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var gl = new OffscreenCanvas(1, 1).getContext('webgl');").expect("e");
        assert_eq!(rt.eval("gl.TRIANGLES").expect("e"), JsValue::Number(4.0));
        assert_eq!(rt.eval("gl.COLOR_BUFFER_BIT").expect("e"), JsValue::Number(16384.0));
        assert_eq!(rt.eval("gl.FLOAT").expect("e"), JsValue::Number(5126.0));
        assert_eq!(rt.eval("WebGLRenderingContext.ARRAY_BUFFER").expect("e"), JsValue::Number(34962.0));
    }

    #[test]
    fn webgl_crea_recursos() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var gl = new OffscreenCanvas(1, 1).getContext('webgl');").expect("e");
        assert_eq!(rt.eval("gl.createBuffer() instanceof WebGLBuffer").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("gl.createProgram() instanceof WebGLProgram").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("gl.createTexture() instanceof WebGLTexture").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("gl.isBuffer(gl.createBuffer())").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webgl_compile_link_exitoso() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var gl = new OffscreenCanvas(1, 1).getContext('webgl'); \
                 var sh = gl.createShader(gl.VERTEX_SHADER); gl.shaderSource(sh, 'void main(){}'); gl.compileShader(sh); \
                 var pr = gl.createProgram(); gl.attachShader(pr, sh); gl.linkProgram(pr);").expect("e");
        assert_eq!(rt.eval("gl.getShaderParameter(sh, gl.COMPILE_STATUS)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("gl.getProgramParameter(pr, gl.LINK_STATUS)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("gl.getShaderSource(sh)").expect("e"), JsValue::String("void main(){}".into()));
    }

    #[test]
    fn webgl_get_error_y_parameter() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var gl = new OffscreenCanvas(1, 1).getContext('webgl');").expect("e");
        assert_eq!(rt.eval("gl.getError()").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("gl.getParameter(gl.MAX_TEXTURE_SIZE)").expect("e"), JsValue::Number(4096.0));
        assert_eq!(rt.eval("typeof gl.getParameter(gl.VERSION)").expect("e"), JsValue::String("string".into()));
        assert_eq!(rt.eval("gl.checkFramebufferStatus() === gl.FRAMEBUFFER_COMPLETE").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webgl_draw_publica_comando() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("globalThis.__puriy_dirty = []; var gl = new OffscreenCanvas(1, 1).getContext('webgl'); gl.clear(gl.COLOR_BUFFER_BIT); gl.drawArrays(gl.TRIANGLES, 0, 3);").expect("e");
        assert_eq!(rt.eval("gl._cmds.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("gl._cmds[1][0]").expect("e"), JsValue::String("drawArrays".into()));
        assert_eq!(rt.eval("__puriy_dirty.filter(function(d){return d.kind==='webgl-call';}).length").expect("e"), JsValue::Number(2.0));
    }

    // ---- Fase 7.152 — CSS Font Loading API ----
    #[test]
    fn fontface_existe_y_estado_inicial() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var f = new FontFace('Roboto', 'url(roboto.woff2)', { weight: '700' });").expect("e");
        assert_eq!(rt.eval("f.family").expect("e"), JsValue::String("Roboto".into()));
        assert_eq!(rt.eval("f.weight").expect("e"), JsValue::String("700".into()));
        assert_eq!(rt.eval("f.status").expect("e"), JsValue::String("unloaded".into()));
        assert_eq!(rt.eval("f.loaded instanceof Promise").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn fontface_load_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        // load() resuelve optimista por microtask (drenado por el harness en el eval).
        rt.eval("var f = new FontFace('X', 'url(x.woff2)'); var done = false; f.load().then(function(){ done = true; });").expect("e");
        assert_eq!(rt.eval("f.status").expect("e"), JsValue::String("loaded".into()));
        assert_eq!(rt.eval("done").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn fontface_load_error_host() {
        let mut rt = JsRuntime::new().expect("rt");
        // El host gana la carrera y fuerza el error antes de que corra el microtask optimista.
        rt.eval("var f = new FontFace('Y', 'url(y.woff2)'); var err = null; f.loaded.catch(function(e){ err = e.name; }); f.load(); __puriy_fontface_error(f._id, 'no encontrada');").expect("e");
        assert_eq!(rt.eval("f.status").expect("e"), JsValue::String("error".into()));
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("NetworkError".into()));
    }

    #[test]
    fn document_fonts_set_operaciones() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var f = new FontFace('Z', 'url(z.woff2)'); document.fonts.add(f);").expect("e");
        assert_eq!(rt.eval("document.fonts.has(f)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("document.fonts.size").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("document.fonts.delete(f)").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("document.fonts.size").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn document_fonts_check() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("document.fonts.check('16px serif')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("document.fonts.check('16px \"Fuente Inexistente\"')").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn document_fonts_loading_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var fired = null; document.fonts.addEventListener('loadingdone', function(e){ fired = e.fontfaces[0].family; }); \
                 var f = new FontFace('Evt', 'url(e.woff2)'); document.fonts.add(f); document.fonts.load('16px Evt');").expect("e");
        // load() del set dispara load() de la face; el microtask drena y emite loadingdone.
        assert_eq!(rt.eval("fired").expect("e"), JsValue::String("Evt".into()));
        assert_eq!(rt.eval("document.fonts.status").expect("e"), JsValue::String("loaded".into()));
    }

    #[test]
    fn document_fonts_ready() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ok = false; document.fonts.ready.then(function(s){ ok = (s === document.fonts); });").expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.153 — Geometry Interfaces ----
    #[test]
    fn geometry_dompoint_y_rect() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new DOMPoint(1, 2, 3); p.x = 10;").expect("e");
        assert_eq!(rt.eval("p.x").expect("e"), JsValue::Number(10.0));
        assert_eq!(rt.eval("p.w").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("new DOMPointReadOnly(5).x").expect("e"), JsValue::Number(5.0));
        // DOMRect: width negativo → left/right normalizan.
        rt.eval("var r = new DOMRect(10, 20, -5, 8);").expect("e");
        assert_eq!(rt.eval("r.left").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("r.right").expect("e"), JsValue::Number(10.0));
        assert_eq!(rt.eval("r.bottom").expect("e"), JsValue::Number(28.0));
        assert_eq!(rt.eval("DOMRectReadOnly.fromRect({x:1,y:2,width:3,height:4}).top").expect("e"), JsValue::Number(2.0));
    }

    #[test]
    fn geometry_matrix_identidad_y_translate() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var m = new DOMMatrix();").expect("e");
        assert_eq!(rt.eval("m.isIdentity").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("m.is2D").expect("e"), JsValue::Bool(true));
        rt.eval("var t = m.translate(5, 7);").expect("e");
        assert_eq!(rt.eval("t.e").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("t.f").expect("e"), JsValue::Number(7.0));
        assert_eq!(rt.eval("t.m41").expect("e"), JsValue::Number(5.0));
        // El original no muta (translate es no-Self).
        assert_eq!(rt.eval("m.isIdentity").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn geometry_matrix_multiply_y_transform_point() {
        let mut rt = JsRuntime::new().expect("rt");
        // translate(10,20) luego scale(2): un punto (1,1) → (10+2, 20+2) = (12, 22).
        rt.eval("var m = new DOMMatrix().translateSelf(10, 20).scaleSelf(2); var p = m.transformPoint(new DOMPoint(1, 1));").expect("e");
        assert_eq!(rt.eval("p.x").expect("e"), JsValue::Number(12.0));
        assert_eq!(rt.eval("p.y").expect("e"), JsValue::Number(22.0));
        // a === m11, c === m21, e === m41 (mismo backing).
        rt.eval("var n = new DOMMatrix(); n.a = 3;").expect("e");
        assert_eq!(rt.eval("n.m11").expect("e"), JsValue::Number(3.0));
    }

    #[test]
    fn geometry_matrix_inverse() {
        let mut rt = JsRuntime::new().expect("rt");
        // inverse de translate(5,7) deshace la traslación.
        rt.eval("var m = new DOMMatrix().translateSelf(5, 7); var inv = m.inverse(); var id = m.multiply(inv);").expect("e");
        assert_eq!(rt.eval("Math.round(id.e * 1e6) / 1e6").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("Math.round(id.f * 1e6) / 1e6").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("id.isIdentity").expect("e"), JsValue::Bool(true));
        // Matriz singular (scale 0) → inversa con NaN.
        rt.eval("var s = new DOMMatrix().scaleSelf(0).inverse();").expect("e");
        assert_eq!(rt.eval("Number.isNaN(s.a)").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn geometry_matrix_from_array_y_float32() {
        let mut rt = JsRuntime::new().expect("rt");
        // Array de 6 → matriz 2D afín.
        rt.eval("var m = new DOMMatrix([1, 0, 0, 1, 30, 40]);").expect("e");
        assert_eq!(rt.eval("m.e").expect("e"), JsValue::Number(30.0));
        assert_eq!(rt.eval("m.is2D").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("m.toFloat32Array().length").expect("e"), JsValue::Number(16.0));
        assert_eq!(rt.eval("m.toFloat32Array()[12]").expect("e"), JsValue::Number(30.0));
        // toString 2D.
        assert_eq!(rt.eval("new DOMMatrix().toString()").expect("e"),
                   JsValue::String("matrix(1, 0, 0, 1, 0, 0)".into()));
    }

    #[test]
    fn geometry_quad_y_bounds() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var q = DOMQuad.fromRect({x: 10, y: 20, width: 30, height: 40}); var b = q.getBounds();").expect("e");
        assert_eq!(rt.eval("q.p3.x").expect("e"), JsValue::Number(40.0));
        assert_eq!(rt.eval("b.x").expect("e"), JsValue::Number(10.0));
        assert_eq!(rt.eval("b.width").expect("e"), JsValue::Number(30.0));
        assert_eq!(rt.eval("b instanceof DOMRect").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn geometry_canvas_get_transform_es_dommatrix() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ctx = new OffscreenCanvas(10, 10).getContext('2d'); ctx.translate(5, 7); var m = ctx.getTransform();").expect("e");
        assert_eq!(rt.eval("m instanceof DOMMatrix").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("m.e").expect("e"), JsValue::Number(5.0));
        assert_eq!(rt.eval("m.is2D").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.154 — CSS Object Model ----
    #[test]
    fn cssom_supports() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("CSS.supports('display', 'grid')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("CSS.supports('gap', '1rem')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("CSS.supports('no-such-prop', 'x')").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("CSS.supports('', '')").expect("e"), JsValue::Bool(false));
        // Forma de condición de un argumento.
        assert_eq!(rt.eval("CSS.supports('(display: flex)')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("CSS.supports('color: var(--x)')").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn cssom_escape() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("CSS.escape('foo')").expect("e"), JsValue::String("foo".into()));
        assert_eq!(rt.eval("CSS.escape('.foo#bar')").expect("e"), JsValue::String("\\.foo\\#bar".into()));
        // Empieza con dígito → escape hex.
        assert_eq!(rt.eval("CSS.escape('1a')").expect("e"), JsValue::String("\\31 a".into()));
    }

    #[test]
    fn cssom_register_property() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("CSS.registerProperty({ name: '--my-color', syntax: '<color>', inherits: false, initialValue: 'red' });").expect("e");
        // Re-registrar la misma tira.
        rt.eval("var err = null; try { CSS.registerProperty({ name: '--my-color' }); } catch (e) { err = e; }").expect("e");
        assert_eq!(rt.eval("err !== null").expect("e"), JsValue::Bool(true));
        // Nombre sin -- tira SyntaxError.
        rt.eval("var e2 = null; try { CSS.registerProperty({ name: 'nope' }); } catch (e) { e2 = e.constructor.name; }").expect("e");
        assert_eq!(rt.eval("e2").expect("e"), JsValue::String("SyntaxError".into()));
    }

    #[test]
    fn cssom_typed_om_numerico() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var v = CSS.px(10);").expect("e");
        assert_eq!(rt.eval("v.value").expect("e"), JsValue::Number(10.0));
        assert_eq!(rt.eval("v.unit").expect("e"), JsValue::String("px".into()));
        assert_eq!(rt.eval("CSS.px(10).toString()").expect("e"), JsValue::String("10px".into()));
        assert_eq!(rt.eval("CSS.percent(50).toString()").expect("e"), JsValue::String("50%".into()));
        assert_eq!(rt.eval("CSS.number(3).toString()").expect("e"), JsValue::String("3".into()));
    }

    #[test]
    fn cssom_constructable_stylesheet() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var sheet = new CSSStyleSheet(); sheet.replaceSync('a { color: red; } p { margin: 0; }');").expect("e");
        assert_eq!(rt.eval("sheet.cssRules.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("sheet.cssRules[0].selectorText").expect("e"), JsValue::String("a".into()));
        assert_eq!(rt.eval("sheet.cssRules[0].style.getPropertyValue('color')").expect("e"), JsValue::String("red".into()));
        // insertRule / deleteRule.
        rt.eval("sheet.insertRule('div { width: 100px; }', 0);").expect("e");
        assert_eq!(rt.eval("sheet.cssRules.length").expect("e"), JsValue::Number(3.0));
        assert_eq!(rt.eval("sheet.cssRules[0].selectorText").expect("e"), JsValue::String("div".into()));
        rt.eval("sheet.deleteRule(0);").expect("e");
        assert_eq!(rt.eval("sheet.cssRules.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("sheet instanceof CSSStyleSheet").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("sheet.cssRules[0] instanceof CSSRule").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn cssom_replace_promise_y_adopted() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var done = false; var s = new CSSStyleSheet(); s.replace('b { font-weight: bold; }').then(function(r){ done = (r === s); });").expect("e");
        assert_eq!(rt.eval("done").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("s.cssRules.length").expect("e"), JsValue::Number(1.0));
        // document.adoptedStyleSheets.
        rt.eval("document.adoptedStyleSheets = [s];").expect("e");
        assert_eq!(rt.eval("document.adoptedStyleSheets.length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("document.adoptedStyleSheets[0] === s").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.155 — Scheduler API ----
    #[test]
    fn scheduler_post_task_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var out = null; scheduler.postTask(function(){ return 42; }).then(function(v){ out = v; });").expect("e");
        assert_eq!(rt.eval("out").expect("e"), JsValue::Number(42.0));
        assert_eq!(rt.eval("scheduler.isInputPending()").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn scheduler_orden_por_prioridad() {
        let mut rt = JsRuntime::new().expect("rt");
        // Encolado background-primero, pero user-blocking debe correr antes.
        rt.eval("var seq = [];
            scheduler.postTask(function(){ seq.push('bg'); }, { priority: 'background' });
            scheduler.postTask(function(){ seq.push('uv'); }, { priority: 'user-visible' });
            scheduler.postTask(function(){ seq.push('ub'); }, { priority: 'user-blocking' });").expect("e");
        assert_eq!(rt.eval("seq.join(',')").expect("e"), JsValue::String("ub,uv,bg".into()));
    }

    #[test]
    fn scheduler_signal_abortada_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var err = null; var c = new AbortController(); c.abort();
            scheduler.postTask(function(){ return 1; }, { signal: c.signal }).catch(function(e){ err = String(e); });").expect("e");
        assert_eq!(rt.eval("err !== null").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn scheduler_task_controller_prioridad() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var tc = new TaskController({ priority: 'user-blocking' });").expect("e");
        assert_eq!(rt.eval("tc.signal.priority").expect("e"), JsValue::String("user-blocking".into()));
        // setPriority dispara prioritychange con previousPriority.
        rt.eval("var prev = null; tc.signal.onprioritychange = function(e){ prev = e.previousPriority; };
            tc.setPriority('background');").expect("e");
        assert_eq!(rt.eval("tc.signal.priority").expect("e"), JsValue::String("background".into()));
        assert_eq!(rt.eval("prev").expect("e"), JsValue::String("user-blocking".into()));
    }

    #[test]
    fn scheduler_yield_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var resumed = false; scheduler.yield().then(function(){ resumed = true; });").expect("e");
        assert_eq!(rt.eval("resumed").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn scheduler_task_controller_abort_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        // postTask con delay queda en espera; abortar el TaskController la cancela.
        rt.eval("var err = null; var tc = new TaskController();
            scheduler.postTask(function(){ return 9; }, { signal: tc.signal, delay: 1000 }).catch(function(e){ err = e; });
            tc.abort();").expect("e");
        assert_eq!(rt.eval("err !== null").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.156 — URLPattern API ----
    #[test]
    fn urlpattern_pathname_named_group() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLPattern({ pathname: '/users/:id' });").expect("e");
        assert_eq!(rt.eval("p.test('https://e.com/users/5')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("p.exec('https://e.com/users/5').pathname.groups.id").expect("e"), JsValue::String("5".into()));
    }

    #[test]
    fn urlpattern_no_match() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLPattern({ pathname: '/users/:id' });").expect("e");
        assert_eq!(rt.eval("p.test('https://e.com/posts/5')").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("p.exec('https://e.com/posts/5')").expect("e"), JsValue::Null);
    }

    #[test]
    fn urlpattern_wildcard() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLPattern({ pathname: '/files/*' });").expect("e");
        assert_eq!(rt.eval("p.test('https://e.com/files/a/b/c')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("p.exec('https://e.com/files/a/b/c').pathname.groups['0']").expect("e"), JsValue::String("a/b/c".into()));
    }

    #[test]
    fn urlpattern_from_string() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLPattern('https://example.com/books/:genre');").expect("e");
        assert_eq!(rt.eval("p.test('https://example.com/books/fiction')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("p.exec('https://example.com/books/fiction').pathname.groups.genre").expect("e"), JsValue::String("fiction".into()));
        assert_eq!(rt.eval("p.test('https://otra.com/books/fiction')").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn urlpattern_hostname_named_group() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var p = new URLPattern({ hostname: ':sub.example.com' });").expect("e");
        assert_eq!(rt.eval("p.test('https://api.example.com/x')").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("p.exec('https://api.example.com/x').hostname.groups.sub").expect("e"), JsValue::String("api".into()));
    }


    // ---- Fase 7.157 — WebGPU ----
    #[test]
    fn webgpu_navigator_gpu_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("navigator.gpu != null && typeof navigator.gpu.requestAdapter === 'function'").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("navigator.gpu.getPreferredCanvasFormat()").expect("e"), JsValue::String("bgra8unorm".into()));
    }

    #[test]
    fn webgpu_request_adapter_y_device() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ok = {};
            navigator.gpu.requestAdapter().then(function(a){ ok.adapter = (a instanceof GPUAdapter); return a.requestDevice(); })
                .then(function(d){ ok.device = (d instanceof GPUDevice); ok.queue = (typeof d.queue.submit === 'function'); ok.limit = d.limits.maxBindGroups; });").expect("e");
        assert_eq!(rt.eval("ok.adapter").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ok.device").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ok.queue").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("ok.limit").expect("e"), JsValue::Number(4.0));
    }

    #[test]
    fn webgpu_crea_buffer_y_shader() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ok = {};
            navigator.gpu.requestAdapter().then(function(a){ return a.requestDevice(); }).then(function(d){
                var buf = d.createBuffer({ size: 256, usage: GPUBufferUsage.UNIFORM });
                var sm = d.createShaderModule({ code: '@vertex fn main(){}' });
                ok.size = buf.size; ok.usage = buf.usage; ok.sm = (sm instanceof GPUShaderModule);
            });").expect("e");
        assert_eq!(rt.eval("ok.size").expect("e"), JsValue::Number(256.0));
        assert_eq!(rt.eval("ok.usage").expect("e"), JsValue::Number(64.0));
        assert_eq!(rt.eval("ok.sm").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webgpu_command_encoder_render_pass_submit() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("globalThis.__puriy_dirty = [];
            navigator.gpu.requestAdapter().then(function(a){ return a.requestDevice(); }).then(function(d){
                var enc = d.createCommandEncoder();
                var pass = enc.beginRenderPass({ colorAttachments: [] });
                pass.setPipeline({}); pass.draw(3); pass.end();
                d.queue.submit([enc.finish()]);
            });").expect("e");
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'webgpu-submit'; })").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webgpu_canvas_context() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var c = new OffscreenCanvas(64, 32); var ctx = c.getContext('webgpu');
            ctx.configure({ format: 'rgba8unorm' }); var t = ctx.getCurrentTexture();").expect("e");
        assert_eq!(rt.eval("ctx instanceof GPUCanvasContext").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("t.width").expect("e"), JsValue::Number(64.0));
        assert_eq!(rt.eval("t.format").expect("e"), JsValue::String("rgba8unorm".into()));
    }

    #[test]
    fn webgpu_flags_estaticos() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("GPUBufferUsage.VERTEX").expect("e"), JsValue::Number(32.0));
        assert_eq!(rt.eval("GPUTextureUsage.RENDER_ATTACHMENT").expect("e"), JsValue::Number(16.0));
        assert_eq!(rt.eval("GPUShaderStage.FRAGMENT").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("GPUMapMode.READ").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.158 — WebXR Device API ----
    #[test]
    fn webxr_navigator_xr_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("navigator.xr != null && typeof navigator.xr.requestSession === 'function'").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webxr_is_session_supported() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var r = {};
            navigator.xr.isSessionSupported('inline').then(function(v){ r.inline = v; });
            navigator.xr.isSessionSupported('immersive-vr').then(function(v){ r.vr = v; });").expect("e");
        assert_eq!(rt.eval("r.inline").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("r.vr").expect("e"), JsValue::Bool(false));
    }

    #[test]
    fn webxr_request_session_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var sess = null; var ended = false;
            navigator.xr.requestSession('inline').then(function(s){ sess = s; });
            var ks = Object.keys(__puriy_xr_pending); __puriy_xr_session_resolve(ks[ks.length - 1]);").expect("e");
        assert_eq!(rt.eval("sess !== null && typeof sess.requestAnimationFrame === 'function'").expect("e"), JsValue::Bool(true));
        // end() dispara onend.
        rt.eval("sess.onend = function(){ ended = true; }; sess.end();").expect("e");
        assert_eq!(rt.eval("ended").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webxr_request_session_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var err = null;
            navigator.xr.requestSession('immersive-vr').catch(function(e){ err = String(e); });
            var ks = Object.keys(__puriy_xr_pending); __puriy_xr_session_reject(ks[ks.length - 1], 'NotSupportedError');").expect("e");
        assert_eq!(rt.eval("err !== null").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webxr_session_raf_y_frame() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var sess = null;
            navigator.xr.requestSession('inline').then(function(s){ sess = s; });
            var ks = Object.keys(__puriy_xr_pending); __puriy_xr_session_resolve(ks[ks.length - 1]);").expect("e");
        rt.eval("var views = -1;
            sess.requestAnimationFrame(function(time, frame){ views = frame.getViewerPose({}).views.length; });
            __puriy_xr_frame(sess._id, 16);").expect("e");
        assert_eq!(rt.eval("views").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn webxr_rigid_transform_matrix_es_float32() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var t = new XRRigidTransform({ x: 1, y: 2, z: 3 }, { x: 0, y: 0, z: 0, w: 1 });").expect("e");
        assert_eq!(rt.eval("t.matrix instanceof Float32Array && t.matrix.length === 16").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("t.matrix[12] === 1 && t.matrix[13] === 2 && t.matrix[14] === 3").expect("e"), JsValue::Bool(true));
        // inverse vía DOMMatrix (Fase 7.153): traslación inversa.
        assert_eq!(rt.eval("Math.abs(t.inverse.matrix[12] + 1) < 1e-6").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.159 — Background Fetch API ----
    #[test]
    fn backgroundfetch_manager_existe_en_registration() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        assert_eq!(rt.eval("reg.backgroundFetch instanceof BackgroundFetchManager").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn backgroundfetch_fetch_resuelve_registration() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval("var out = null;
            reg.backgroundFetch.fetch('media', ['/a.mp4', '/b.mp4'], { downloadTotal: 100 }).then(function(r){ out = r; });").expect("e");
        assert_eq!(rt.eval("out instanceof BackgroundFetchRegistration").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("out.id").expect("e"), JsValue::String("media".into()));
        assert_eq!(rt.eval("out.downloadTotal").expect("e"), JsValue::Number(100.0));
    }

    #[test]
    fn backgroundfetch_fetch_id_duplicado_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval("var err = null;
            reg.backgroundFetch.fetch('dup', ['/x']);
            reg.backgroundFetch.fetch('dup', ['/y']).catch(function(e){ err = String(e); });").expect("e");
        assert_eq!(rt.eval("err !== null").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn backgroundfetch_get_y_getids() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval("reg.backgroundFetch.fetch('z', ['/z']);
            var got = null; reg.backgroundFetch.get('z').then(function(r){ got = r ? r.id : null; });
            var ids = null; reg.backgroundFetch.getIds().then(function(l){ ids = l.join(','); });").expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("z".into()));
        assert_eq!(rt.eval("ids").expect("e"), JsValue::String("z".into()));
    }

    #[test]
    fn backgroundfetch_progress_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(PUSH_REG).expect("reg");
        rt.eval("var bgreg = null;
            reg.backgroundFetch.fetch('p', ['/p']).then(function(r){ bgreg = r; });").expect("e");
        rt.eval("var prog = null; bgreg.onprogress = function(e){ prog = bgreg.downloaded; };
            __puriy_backgroundfetch_progress(bgreg._uid, { downloaded: 50, downloadTotal: 100 });").expect("e");
        assert_eq!(rt.eval("prog").expect("e"), JsValue::Number(50.0));
    }


    // ---- Fase 7.160 — ImageCapture API ----
    #[test]
    fn imagecapture_requiere_video_track() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var threw = false; try { new ImageCapture({ kind: 'audio' }); } catch(e){ threw = (e.name === 'NotSupportedError'); }").expect("e");
        assert_eq!(rt.eval("threw").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("(new ImageCapture({ kind: 'video' })) instanceof ImageCapture").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn imagecapture_take_photo_resuelve_blob() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var b = null;
            new ImageCapture({ kind: 'video', readyState: 'live' }).takePhoto().then(function(x){ b = x; });").expect("e");
        assert_eq!(rt.eval("b instanceof Blob && b.type === 'image/png'").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn imagecapture_grab_frame_resuelve_imagebitmap() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var bmp = null;
            new ImageCapture({ kind: 'video' }).grabFrame().then(function(x){ bmp = x; });").expect("e");
        assert_eq!(rt.eval("bmp instanceof ImageBitmap && bmp.width === 1280").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn imagecapture_capabilities_y_settings() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var caps = null; var sett = null; var ic = new ImageCapture({ kind: 'video' });
            ic.getPhotoCapabilities().then(function(c){ caps = c; });
            ic.getPhotoSettings().then(function(s){ sett = s; });").expect("e");
        assert_eq!(rt.eval("caps.imageWidth.max").expect("e"), JsValue::Number(1920.0));
        assert_eq!(rt.eval("sett.imageWidth").expect("e"), JsValue::Number(1280.0));
    }

    // ---- Fase 7.161 — Compression Streams API ----
    #[test]
    fn compression_stream_formato_invalido() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var threw = false; try { new CompressionStream('lzma'); } catch(e){ threw = (e instanceof TypeError); }").expect("e");
        assert_eq!(rt.eval("threw").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn compression_stream_tiene_readable_y_writable() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var cs = new CompressionStream('gzip');").expect("e");
        assert_eq!(rt.eval("cs.readable instanceof ReadableStream && typeof cs.writable.getWriter === 'function'").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn decompression_stream_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ds = new DecompressionStream('deflate');").expect("e");
        assert_eq!(rt.eval("ds._format").expect("e"), JsValue::String("deflate".into()));
    }

    #[test]
    fn compression_write_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("globalThis.__puriy_dirty = [];
            var cs = new CompressionStream('gzip'); var w = cs.writable.getWriter(); w.write(new Uint8Array([1, 2, 3]));").expect("e");
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'compress'; })").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn compression_host_output_llega_a_readable() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var cs = new CompressionStream('gzip');
            var w = cs.writable.getWriter(); w.write(new Uint8Array([1]));
            __puriy_compress_output(cs._id, 42); __puriy_compress_end(cs._id);
            var got = null; cs.readable.getReader().read().then(function(r){ got = r.value; });").expect("e");
        assert_eq!(rt.eval("got").expect("e"), JsValue::Number(42.0));
    }

    // ---- Fase 7.162 — Window Management API ----
    #[test]
    fn windowmanagement_get_screen_details() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var det = null; getScreenDetails().then(function(d){ det = d; });").expect("e");
        assert_eq!(rt.eval("det instanceof ScreenDetails && det.screens.length >= 1 && det.currentScreen != null").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("det.screens[0] instanceof ScreenDetailed").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn windowmanagement_multi_monitor() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_screen_details([{ label: 'A', isPrimary: true }, { label: 'B', left: 1920, isPrimary: false }]);
            var det = null; getScreenDetails().then(function(d){ det = d; });").expect("e");
        assert_eq!(rt.eval("det.screens.length").expect("e"), JsValue::Number(2.0));
        assert_eq!(rt.eval("det.screens[1].label").expect("e"), JsValue::String("B".into()));
        assert_eq!(rt.eval("screen.isExtended").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn windowmanagement_permiso_denegado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_window_management_permission(false);
            var err = null; getScreenDetails().catch(function(e){ err = e.name; });").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("NotAllowedError".into()));
    }


    // ---- Fase 7.163 — Local Font Access API ----
    #[test]
    fn localfonts_query_resuelve_array() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var fonts = null; queryLocalFonts().then(function(f){ fonts = f; });").expect("e");
        assert_eq!(rt.eval("Array.isArray(fonts) && fonts.length >= 1 && typeof fonts[0].family === 'string'").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("fonts[0] instanceof FontData").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn localfonts_blob_resuelve() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var b = null; queryLocalFonts().then(function(f){ f[0].blob().then(function(x){ b = x; }); });").expect("e");
        assert_eq!(rt.eval("b instanceof Blob && b.type === 'font/opentype'").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn localfonts_filtro_postscript() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_local_fonts([{ postscriptName: 'Foo', family: 'Foo' }, { postscriptName: 'Bar', family: 'Bar' }]);
            var fonts = null; queryLocalFonts({ postscriptNames: ['Bar'] }).then(function(f){ fonts = f; });").expect("e");
        assert_eq!(rt.eval("fonts.length").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("fonts[0].postscriptName").expect("e"), JsValue::String("Bar".into()));
    }

    #[test]
    fn localfonts_permiso_denegado() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_set_local_fonts_permission(false);
            var err = null; queryLocalFonts().catch(function(e){ err = e.name; });").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("SecurityError".into()));
    }

    // ---- Fase 7.164 — WebOTP API ----
    #[test]
    fn webotp_otp_credential_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof OTPCredential === 'function'").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webotp_get_resuelve_via_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var cred = null;
            navigator.credentials.get({ otp: { transport: ['sms'] } }).then(function(c){ cred = c; });
            var ks = Object.keys(__puriy_webotp_pending); __puriy_webotp_resolve(ks[ks.length - 1], '123456');").expect("e");
        assert_eq!(rt.eval("cred !== null && cred.type === 'otp' && cred.code === '123456'").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("cred instanceof OTPCredential").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn webotp_get_rechaza() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var err = null;
            navigator.credentials.get({ otp: { transport: ['sms'] } }).catch(function(e){ err = e.name; });
            var ks = Object.keys(__puriy_webotp_pending); __puriy_webotp_reject(ks[ks.length - 1], 'AbortError');").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("AbortError".into()));
    }

    #[test]
    fn webotp_get_otp_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("globalThis.__puriy_dirty = [];
            navigator.credentials.get({ otp: { transport: ['sms'] } });").expect("e");
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'webotp'; })").expect("e"), JsValue::Bool(true));
    }

    // ---- Fase 7.165 — Picture-in-Picture API ----
    #[test]
    fn pip_api_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof document.exitPictureInPicture").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("document.pictureInPictureEnabled").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("document.pictureInPictureElement").expect("e"), JsValue::Null);
        assert_eq!(rt.eval("typeof PictureInPictureWindow").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn pip_request_resuelve_con_window() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval("var win = null; el.requestPictureInPicture().then(function(w){ win = w; });
            __puriy_pip_resolve('el1', 640, 360);").expect("e");
        assert_eq!(rt.eval("win instanceof PictureInPictureWindow && win.width === 640 && win.height === 360").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("document.pictureInPictureElement && document.pictureInPictureElement._id").expect("e"), JsValue::String("el1".into()));
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'pip-request'; })").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn pip_exit_limpia_y_dispara_leave() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval("var left = 0; el.addEventListener('leavepictureinpicture', function(){ left++; });
            el.requestPictureInPicture(); __puriy_pip_resolve('el1', 320, 180);
            var p = document.exitPictureInPicture();").expect("e");
        assert_eq!(rt.eval("document.pictureInPictureElement").expect("e"), JsValue::Null);
        assert_eq!(rt.eval("left").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'pip-exit'; })").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn pip_reject_dispara_error() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval(MAKE_EL).expect("el");
        rt.eval("var err = null; el.requestPictureInPicture().catch(function(e){ err = e.name; });
            __puriy_pip_reject('el1', 'NotAllowedError');").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("NotAllowedError".into()));
        assert_eq!(rt.eval("document.pictureInPictureElement").expect("e"), JsValue::Null);
    }

    // ---- Fase 7.166 — Document Picture-in-Picture API ----
    #[test]
    fn document_pip_api_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof documentPictureInPicture.requestWindow").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("documentPictureInPicture.window").expect("e"), JsValue::Null);
    }

    #[test]
    fn document_pip_request_resuelve_y_dispara_enter() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var win = null, entered = 0;
            documentPictureInPicture.addEventListener('enter', function(e){ entered++; });
            documentPictureInPicture.requestWindow({ width: 400, height: 300 }).then(function(w){ win = w; });
            var ks = Object.keys(__puriy_document_pip_pending); __puriy_document_pip_resolve(ks[ks.length - 1], null);").expect("e");
        assert_eq!(rt.eval("win !== null && documentPictureInPicture.window === win").expect("e"), JsValue::Bool(true));
        assert_eq!(rt.eval("entered").expect("e"), JsValue::Number(1.0));
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'document-pip-request'; })").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn document_pip_reject() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var err = null;
            documentPictureInPicture.requestWindow().catch(function(e){ err = e.name; });
            var ks = Object.keys(__puriy_document_pip_pending); __puriy_document_pip_reject(ks[ks.length - 1], 'NotAllowedError');").expect("e");
        assert_eq!(rt.eval("err").expect("e"), JsValue::String("NotAllowedError".into()));
    }

    // ---- Fase 7.167 — CloseWatcher API ----
    #[test]
    fn closewatcher_api_existe() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof CloseWatcher").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn closewatcher_request_close_dispara_close() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var closed = 0; var cw = new CloseWatcher();
            cw.onclose = function(){ closed++; }; cw.requestClose();").expect("e");
        assert_eq!(rt.eval("closed").expect("e"), JsValue::Number(1.0));
    }

    #[test]
    fn closewatcher_cancel_previene_close() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var closed = 0; var cw = new CloseWatcher();
            cw.oncancel = function(e){ e.preventDefault(); }; cw.onclose = function(){ closed++; };
            cw.requestClose();").expect("e");
        assert_eq!(rt.eval("closed").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn closewatcher_host_request_close_cierra_el_tope() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var a = 0, b = 0;
            var cwa = new CloseWatcher(); cwa.onclose = function(){ a++; };
            var cwb = new CloseWatcher(); cwb.onclose = function(){ b++; };
            __puriy_close_watcher_request_close();").expect("e");
        // El último creado (cwb) está en el tope del stack → cierra primero.
        assert_eq!(rt.eval("a").expect("e"), JsValue::Number(0.0));
        assert_eq!(rt.eval("b").expect("e"), JsValue::Number(1.0));
    }

    // ---- Fase 7.168 — Shape Detection API ----
    #[test]
    fn shape_detection_clases_existen() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof BarcodeDetector").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof FaceDetector").expect("e"), JsValue::String("function".into()));
        assert_eq!(rt.eval("typeof TextDetector").expect("e"), JsValue::String("function".into()));
    }

    #[test]
    fn barcode_detector_supported_formats() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ok = false; BarcodeDetector.getSupportedFormats().then(function(f){ \
            ok = Array.isArray(f) && f.indexOf('qr_code') >= 0; });").expect("e");
        assert_eq!(rt.eval("ok").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn barcode_detector_formato_invalido_lanza() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("var threw = false; try { new BarcodeDetector({formats:['nope']}); } \
            catch (e) { threw = e instanceof TypeError; } threw").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn shape_detect_sin_hook_resuelve_vacio() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var n = -1; new TextDetector().detect({}).then(function(r){ n = r.length; });").expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn shape_detect_usa_hook_del_host() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_shape_detect_hook = function(type, src, opts){ \
            return type === 'barcode' ? [{ rawValue: 'X', format: 'qr_code' }] : []; }; \
            var v = null; new BarcodeDetector().detect({}).then(function(r){ v = r[0].rawValue; });").expect("e");
        assert_eq!(rt.eval("v").expect("e"), JsValue::String("X".into()));
    }

    #[test]
    fn face_detector_respeta_max_detected_faces() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("__puriy_shape_detect_hook = function(){ return [{},{},{}]; }; \
            var n = -1; new FaceDetector({maxDetectedFaces:2}).detect({}).then(function(r){ n = r.length; });").expect("e");
        assert_eq!(rt.eval("n").expect("e"), JsValue::Number(2.0));
    }

    // ---- Fase 7.169 — EditContext API ----
    #[test]
    fn edit_context_existe_y_construye() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof EditContext").expect("e"), JsValue::String("function".into()));
        rt.eval("var ec = new EditContext({ text: 'hola' });").expect("e");
        assert_eq!(rt.eval("ec.text").expect("e"), JsValue::String("hola".into()));
    }

    #[test]
    fn edit_context_update_text_y_selection() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var ec = new EditContext({ text: 'abc' }); \
            ec.updateText(1, 2, 'XYZ'); ec.updateSelection(0, 3);").expect("e");
        assert_eq!(rt.eval("ec.text").expect("e"), JsValue::String("aXYZc".into()));
        assert_eq!(rt.eval("ec.selectionEnd").expect("e"), JsValue::Number(3.0));
    }

    #[test]
    fn edit_context_host_text_update_dispara_evento() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var got = ''; var ec = new EditContext({ text: 'ab' }); \
            ec.ontextupdate = function(e){ got = e.text; }; \
            var ks = Object.keys(__puriy_editcontexts); \
            __puriy_editcontext_text_update(ks[ks.length-1], { updateRangeStart: 2, updateRangeEnd: 2, text: 'c', selectionStart: 3 });").expect("e");
        assert_eq!(rt.eval("ec.text").expect("e"), JsValue::String("abc".into()));
        assert_eq!(rt.eval("got").expect("e"), JsValue::String("c".into()));
    }

    // ---- Fase 7.170 — Virtual Keyboard API ----
    #[test]
    fn virtual_keyboard_existe_en_navigator() {
        let mut rt = JsRuntime::new().expect("rt");
        assert_eq!(rt.eval("typeof navigator.virtualKeyboard").expect("e"), JsValue::String("object".into()));
        assert_eq!(rt.eval("navigator.virtualKeyboard.overlaysContent").expect("e"), JsValue::Bool(false));
        assert_eq!(rt.eval("navigator.virtualKeyboard.boundingRect.height").expect("e"), JsValue::Number(0.0));
    }

    #[test]
    fn virtual_keyboard_show_publica_mutacion() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("navigator.virtualKeyboard.show();").expect("e");
        assert_eq!(rt.eval("__puriy_dirty.some(function(d){ return d.kind === 'virtualkeyboard' && d.value === 'show'; })").expect("e"), JsValue::Bool(true));
    }

    #[test]
    fn virtual_keyboard_geometry_dispara_geometrychange() {
        let mut rt = JsRuntime::new().expect("rt");
        rt.eval("var h = -1; navigator.virtualKeyboard.ongeometrychange = function(){ \
            h = navigator.virtualKeyboard.boundingRect.height; }; \
            __puriy_virtual_keyboard_geometry(0, 500, 360, 260);").expect("e");
        assert_eq!(rt.eval("h").expect("e"), JsValue::Number(260.0));
    }

