pub(crate) const WEBGL_BOOTSTRAP: &str = r#"
// Fase 7.151 — WebGL (`WebGLRenderingContext` + `WebGL2RenderingContext` + los objetos
// de recurso `WebGLBuffer`/`WebGLShader`/`WebGLProgram`/`WebGLTexture`/`WebGLFramebuffer`/
// `WebGLRenderbuffer`/`WebGLUniformLocation`/`WebGLActiveInfo`/`WebGLVertexArrayObject`/
// `WebGLSampler`/`WebGLQuery`/`WebGLTransformFeedback`/`WebGLSync`). El contexto registra
// los comandos GL en `ctx._cmds` y publica `kind: 'webgl-*'`; el rasterizado real sobre la
// GPU (compile/link de shaders, draws) es del chrome (wiring PENDIENTE). Las consultas
// devuelven defaults razonables (compile/link "exitoso", `getError` → NO_ERROR) para que
// la máquina de estados de una app GL avance sin GPU.
//   · Punto de entrada: `new OffscreenCanvas(w, h).getContext('webgl'|'webgl2')`.
//   · El chrome reemplaza el factory `__puriy_webgl_context` por uno con GPU real.
(function() {
    if (globalThis.WebGLRenderingContext != null) return;
    var GS = String.fromCharCode(0x1D);
    var nextCtxId = 1;

    // Objetos de recurso: cáscaras opacas con un id incremental.
    function resourceClass() {
        var counter = 1;
        function Res() { this._id = counter++; }
        return Res;
    }
    var WebGLBuffer = resourceClass();
    var WebGLShader = resourceClass();
    var WebGLProgram = resourceClass();
    var WebGLTexture = resourceClass();
    var WebGLFramebuffer = resourceClass();
    var WebGLRenderbuffer = resourceClass();
    var WebGLVertexArrayObject = resourceClass();
    var WebGLSampler = resourceClass();
    var WebGLQuery = resourceClass();
    var WebGLTransformFeedback = resourceClass();
    var WebGLSync = resourceClass();
    function WebGLUniformLocation() {}
    function WebGLActiveInfo(name, size, type) { this.name = name; this.size = size; this.type = type; }
    function WebGLShaderPrecisionFormat(rangeMin, rangeMax, precision) {
        this.rangeMin = rangeMin; this.rangeMax = rangeMax; this.precision = precision;
    }

    // Constantes GL (subconjunto amplio compartido por WebGL1/2).
    var K = {
        DEPTH_BUFFER_BIT: 0x0100, STENCIL_BUFFER_BIT: 0x0400, COLOR_BUFFER_BIT: 0x4000,
        POINTS: 0, LINES: 1, LINE_LOOP: 2, LINE_STRIP: 3, TRIANGLES: 4, TRIANGLE_STRIP: 5, TRIANGLE_FAN: 6,
        ZERO: 0, ONE: 1, SRC_COLOR: 0x0300, ONE_MINUS_SRC_COLOR: 0x0301, SRC_ALPHA: 0x0302,
        ONE_MINUS_SRC_ALPHA: 0x0303, DST_ALPHA: 0x0304, ONE_MINUS_DST_ALPHA: 0x0305,
        DST_COLOR: 0x0306, ONE_MINUS_DST_COLOR: 0x0307, SRC_ALPHA_SATURATE: 0x0308,
        FUNC_ADD: 0x8006, BLEND_EQUATION: 0x8009, FUNC_SUBTRACT: 0x800A, FUNC_REVERSE_SUBTRACT: 0x800B,
        BLEND: 0x0BE2, DEPTH_TEST: 0x0B71, STENCIL_TEST: 0x0B90, DITHER: 0x0BD0,
        CULL_FACE: 0x0B44, SCISSOR_TEST: 0x0C11, POLYGON_OFFSET_FILL: 0x8037, SAMPLE_ALPHA_TO_COVERAGE: 0x809E,
        FRONT: 0x0404, BACK: 0x0405, FRONT_AND_BACK: 0x0408, CW: 0x0900, CCW: 0x0901,
        NEVER: 0x0200, LESS: 0x0201, EQUAL: 0x0202, LEQUAL: 0x0203, GREATER: 0x0204,
        NOTEQUAL: 0x0205, GEQUAL: 0x0206, ALWAYS: 0x0207,
        BYTE: 0x1400, UNSIGNED_BYTE: 0x1401, SHORT: 0x1402, UNSIGNED_SHORT: 0x1403,
        INT: 0x1404, UNSIGNED_INT: 0x1405, FLOAT: 0x1406, HALF_FLOAT: 0x140B,
        ARRAY_BUFFER: 0x8892, ELEMENT_ARRAY_BUFFER: 0x8893, UNIFORM_BUFFER: 0x8A11,
        STATIC_DRAW: 0x88E4, DYNAMIC_DRAW: 0x88E8, STREAM_DRAW: 0x88E0,
        FRAGMENT_SHADER: 0x8B30, VERTEX_SHADER: 0x8B31,
        COMPILE_STATUS: 0x8B81, LINK_STATUS: 0x8B82, VALIDATE_STATUS: 0x8B83, DELETE_STATUS: 0x8B80,
        SHADER_TYPE: 0x8B4F,
        ACTIVE_UNIFORMS: 0x8B86, ACTIVE_ATTRIBUTES: 0x8B89, ATTACHED_SHADERS: 0x8B85,
        TEXTURE_2D: 0x0DE1, TEXTURE_CUBE_MAP: 0x8513, TEXTURE0: 0x84C0, TEXTURE1: 0x84C1,
        TEXTURE_MAG_FILTER: 0x2800, TEXTURE_MIN_FILTER: 0x2801, TEXTURE_WRAP_S: 0x2802, TEXTURE_WRAP_T: 0x2803,
        NEAREST: 0x2600, LINEAR: 0x2601, REPEAT: 0x2901, CLAMP_TO_EDGE: 0x812F, MIRRORED_REPEAT: 0x8370,
        RGB: 0x1907, RGBA: 0x1908, ALPHA: 0x1906, LUMINANCE: 0x1909, LUMINANCE_ALPHA: 0x190A,
        RGBA8: 0x8058, RGB8: 0x8051, DEPTH_COMPONENT: 0x1902, DEPTH_COMPONENT16: 0x81A5,
        FRAMEBUFFER: 0x8D40, RENDERBUFFER: 0x8D41, COLOR_ATTACHMENT0: 0x8CE0,
        DEPTH_ATTACHMENT: 0x8D00, STENCIL_ATTACHMENT: 0x8D20, DEPTH_STENCIL_ATTACHMENT: 0x821A,
        FRAMEBUFFER_COMPLETE: 0x8CD5, NO_ERROR: 0, INVALID_ENUM: 0x0500, INVALID_VALUE: 0x0501,
        INVALID_OPERATION: 0x0502, OUT_OF_MEMORY: 0x0505, CONTEXT_LOST_WEBGL: 0x9242,
        VENDOR: 0x1F00, RENDERER: 0x1F01, VERSION: 0x1F02, SHADING_LANGUAGE_VERSION: 0x8B8C,
        MAX_TEXTURE_SIZE: 0x0D33, MAX_VIEWPORT_DIMS: 0x0D3A, MAX_VERTEX_ATTRIBS: 0x8869,
        MAX_TEXTURE_IMAGE_UNITS: 0x8872, MAX_COMBINED_TEXTURE_IMAGE_UNITS: 0x8B4D,
        UNPACK_FLIP_Y_WEBGL: 0x9240, UNPACK_PREMULTIPLY_ALPHA_WEBGL: 0x9241, UNPACK_COLORSPACE_CONVERSION_WEBGL: 0x9243,
        // WebGL2-only de muestra
        READ_FRAMEBUFFER: 0x8CA8, DRAW_FRAMEBUFFER: 0x8CA9, COLOR: 0x1800, DEPTH: 0x1801, STENCIL: 0x1802,
        TRANSFORM_FEEDBACK: 0x8E22, ANY_SAMPLES_PASSED: 0x8C2F
    };

    function applyConstants(target) { for (var k in K) target[k] = K[k]; }

    function makeContext(canvas, type, attrs) {
        var isGL2 = (type === 'webgl2');
        var ctx = {};
        ctx._id = nextCtxId++;
        ctx._glType = type;
        ctx._cmds = [];
        ctx._error = K.NO_ERROR;
        ctx.canvas = canvas || null;
        ctx.drawingBufferWidth = canvas ? (canvas.width | 0) : 0;
        ctx.drawingBufferHeight = canvas ? (canvas.height | 0) : 0;
        ctx.drawingBufferColorSpace = 'srgb';
        ctx._attrs = attrs || {};
        applyConstants(ctx);

        function rec(name) {
            ctx._cmds.push(Array.prototype.slice.call(arguments));
            globalThis.__puriy_dirty.push({ id: '__window__', kind: 'webgl-call', value: ctx._id + GS + name });
        }

        ctx.getContextAttributes = function() {
            return {
                alpha: ctx._attrs.alpha !== false, depth: ctx._attrs.depth !== false,
                stencil: !!ctx._attrs.stencil, antialias: ctx._attrs.antialias !== false,
                premultipliedAlpha: ctx._attrs.premultipliedAlpha !== false,
                preserveDrawingBuffer: !!ctx._attrs.preserveDrawingBuffer,
                powerPreference: ctx._attrs.powerPreference || 'default',
                failIfMajorPerformanceCaveat: !!ctx._attrs.failIfMajorPerformanceCaveat,
                desynchronized: !!ctx._attrs.desynchronized
            };
        };
        ctx.isContextLost = function() { return false; };
        ctx.getError = function() { var e = ctx._error; ctx._error = K.NO_ERROR; return e; };
        ctx.getSupportedExtensions = function() { return []; };
        ctx.getExtension = function() { return null; };

        // Creación de recursos.
        ctx.createBuffer = function() { return new WebGLBuffer(); };
        ctx.createShader = function(t) { var s = new WebGLShader(); s._type = t; s._source = ''; return s; };
        ctx.createProgram = function() { return new WebGLProgram(); };
        ctx.createTexture = function() { return new WebGLTexture(); };
        ctx.createFramebuffer = function() { return new WebGLFramebuffer(); };
        ctx.createRenderbuffer = function() { return new WebGLRenderbuffer(); };
        ctx.createVertexArray = function() { return new WebGLVertexArrayObject(); };
        ctx.createSampler = function() { return new WebGLSampler(); };
        ctx.createQuery = function() { return new WebGLQuery(); };
        ctx.createTransformFeedback = function() { return new WebGLTransformFeedback(); };
        ctx.fenceSync = function() { return new WebGLSync(); };

        ['deleteBuffer', 'deleteShader', 'deleteProgram', 'deleteTexture', 'deleteFramebuffer',
         'deleteRenderbuffer', 'deleteVertexArray', 'deleteSampler', 'deleteQuery',
         'deleteTransformFeedback', 'deleteSync'].forEach(function(op) {
            ctx[op] = function(res) { rec(op); };
        });

        // Shaders / programas: simulamos compile/link exitosos.
        ctx.shaderSource = function(shader, src) { if (shader) shader._source = String(src); rec('shaderSource'); };
        ctx.getShaderSource = function(shader) { return shader ? shader._source : ''; };
        ctx.compileShader = function(shader) { if (shader) shader._compiled = true; rec('compileShader'); };
        ctx.attachShader = function() { rec('attachShader'); };
        ctx.detachShader = function() { rec('detachShader'); };
        ctx.linkProgram = function(program) { if (program) program._linked = true; rec('linkProgram'); };
        ctx.validateProgram = function() { rec('validateProgram'); };
        ctx.useProgram = function() { rec('useProgram'); };
        ctx.bindAttribLocation = function() { rec('bindAttribLocation'); };
        ctx.getShaderParameter = function(shader, pname) {
            if (pname === K.COMPILE_STATUS) return true;
            if (pname === K.DELETE_STATUS) return false;
            if (pname === K.SHADER_TYPE) return shader ? shader._type : 0;
            return null;
        };
        ctx.getProgramParameter = function(program, pname) {
            if (pname === K.LINK_STATUS || pname === K.VALIDATE_STATUS) return true;
            if (pname === K.DELETE_STATUS) return false;
            if (pname === K.ACTIVE_UNIFORMS || pname === K.ACTIVE_ATTRIBUTES) return 0;
            if (pname === K.ATTACHED_SHADERS) return 2;
            return null;
        };
        ctx.getShaderInfoLog = function() { return ''; };
        ctx.getProgramInfoLog = function() { return ''; };
        ctx.getShaderPrecisionFormat = function() { return new WebGLShaderPrecisionFormat(127, 127, 23); };
        ctx.getAttribLocation = function() { return 0; };
        ctx.getUniformLocation = function() { return new WebGLUniformLocation(); };
        ctx.getActiveAttrib = function() { return new WebGLActiveInfo('', 1, K.FLOAT); };
        ctx.getActiveUniform = function() { return new WebGLActiveInfo('', 1, K.FLOAT); };

        // getParameter con defaults sensatos.
        ctx.getParameter = function(pname) {
            switch (pname) {
                case K.VERSION: return isGL2 ? 'WebGL 2.0 (puriy)' : 'WebGL 1.0 (puriy)';
                case K.SHADING_LANGUAGE_VERSION: return isGL2 ? 'WebGL GLSL ES 3.00 (puriy)' : 'WebGL GLSL ES 1.0 (puriy)';
                case K.VENDOR: return 'puriy';
                case K.RENDERER: return 'puriy-webgl';
                case K.MAX_TEXTURE_SIZE: return 4096;
                case K.MAX_VERTEX_ATTRIBS: return 16;
                case K.MAX_TEXTURE_IMAGE_UNITS: return 16;
                case K.MAX_COMBINED_TEXTURE_IMAGE_UNITS: return 32;
                case K.MAX_VIEWPORT_DIMS: return new Int32Array([4096, 4096]);
                default: return null;
            }
        };

        // Comandos de dibujo / estado: registran y no devuelven nada.
        var voidOps = [
            'bindBuffer', 'bufferData', 'bufferSubData', 'bindFramebuffer', 'framebufferTexture2D',
            'framebufferRenderbuffer', 'bindRenderbuffer', 'renderbufferStorage', 'bindTexture',
            'texImage2D', 'texSubImage2D', 'texParameteri', 'texParameterf', 'generateMipmap',
            'activeTexture', 'pixelStorei', 'viewport', 'scissor', 'clear', 'clearColor',
            'clearDepth', 'clearStencil', 'colorMask', 'depthMask', 'depthFunc', 'depthRange',
            'enable', 'disable', 'blendFunc', 'blendFuncSeparate', 'blendEquation', 'blendEquationSeparate',
            'blendColor', 'cullFace', 'frontFace', 'lineWidth', 'polygonOffset', 'stencilFunc',
            'stencilOp', 'stencilMask', 'sampleCoverage', 'hint', 'enableVertexAttribArray',
            'disableVertexAttribArray', 'vertexAttribPointer', 'vertexAttrib1f', 'vertexAttrib2f',
            'vertexAttrib3f', 'vertexAttrib4f', 'vertexAttrib4fv', 'drawArrays', 'drawElements',
            'flush', 'finish', 'readPixels', 'copyTexImage2D', 'copyTexSubImage2D', 'compressedTexImage2D',
            'bindVertexArray', 'bindSampler', 'samplerParameteri', 'beginQuery', 'endQuery',
            'bindTransformFeedback', 'beginTransformFeedback', 'endTransformFeedback',
            'transformFeedbackVaryings', 'bindBufferBase', 'bindBufferRange', 'uniformBlockBinding',
            'drawArraysInstanced', 'drawElementsInstanced', 'vertexAttribDivisor', 'drawBuffers',
            'blitFramebuffer', 'invalidateFramebuffer'
        ];
        voidOps.forEach(function(op) { ctx[op] = function() { rec(op); }; });

        // uniform*: aceptan cualquier firma.
        ['uniform1f', 'uniform2f', 'uniform3f', 'uniform4f', 'uniform1i', 'uniform2i', 'uniform3i',
         'uniform4i', 'uniform1fv', 'uniform2fv', 'uniform3fv', 'uniform4fv', 'uniform1iv',
         'uniform2iv', 'uniform3iv', 'uniform4iv', 'uniformMatrix2fv', 'uniformMatrix3fv',
         'uniformMatrix4fv', 'uniform1ui', 'uniform2ui', 'uniform3ui', 'uniform4ui'].forEach(function(op) {
            ctx[op] = function() { rec(op); };
        });

        ctx.isBuffer = function(o) { return o instanceof WebGLBuffer; };
        ctx.isShader = function(o) { return o instanceof WebGLShader; };
        ctx.isProgram = function(o) { return o instanceof WebGLProgram; };
        ctx.isTexture = function(o) { return o instanceof WebGLTexture; };
        ctx.isFramebuffer = function(o) { return o instanceof WebGLFramebuffer; };
        ctx.isRenderbuffer = function(o) { return o instanceof WebGLRenderbuffer; };
        ctx.checkFramebufferStatus = function() { return K.FRAMEBUFFER_COMPLETE; };

        return ctx;
    }

    // Factory consumido por OffscreenCanvas.getContext (Fase 7.150) y por el chrome.
    globalThis.__puriy_webgl_context = function(canvas, type, attrs) { return makeContext(canvas, type, attrs); };

    // Constructores expuestos (no instanciables por el autor, pero `instanceof` debe andar).
    function WebGLRenderingContext() {}
    function WebGL2RenderingContext() {}
    applyConstants(WebGLRenderingContext);
    applyConstants(WebGL2RenderingContext);
    applyConstants(WebGLRenderingContext.prototype);
    applyConstants(WebGL2RenderingContext.prototype);

    globalThis.WebGLRenderingContext = WebGLRenderingContext;
    globalThis.WebGL2RenderingContext = WebGL2RenderingContext;
    globalThis.WebGLBuffer = WebGLBuffer;
    globalThis.WebGLShader = WebGLShader;
    globalThis.WebGLProgram = WebGLProgram;
    globalThis.WebGLTexture = WebGLTexture;
    globalThis.WebGLFramebuffer = WebGLFramebuffer;
    globalThis.WebGLRenderbuffer = WebGLRenderbuffer;
    globalThis.WebGLUniformLocation = WebGLUniformLocation;
    globalThis.WebGLActiveInfo = WebGLActiveInfo;
    globalThis.WebGLShaderPrecisionFormat = WebGLShaderPrecisionFormat;
    globalThis.WebGLVertexArrayObject = WebGLVertexArrayObject;
    globalThis.WebGLSampler = WebGLSampler;
    globalThis.WebGLQuery = WebGLQuery;
    globalThis.WebGLTransformFeedback = WebGLTransformFeedback;
    globalThis.WebGLSync = WebGLSync;
    void 0;
})();
"#;
