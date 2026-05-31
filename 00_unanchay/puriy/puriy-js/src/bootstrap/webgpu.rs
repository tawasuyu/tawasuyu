pub(crate) const WEBGPU_BOOTSTRAP: &str = r#"
// Fase 7.157 — WebGPU (`navigator.gpu` + `GPUAdapter`/`GPUDevice`/`GPUQueue` + los
// objetos de recurso `GPUBuffer`/`GPUTexture`/`GPUTextureView`/`GPUSampler`/
// `GPUShaderModule`/`GPUBindGroupLayout`/`GPUPipelineLayout`/`GPUBindGroup`/
// `GPURenderPipeline`/`GPUComputePipeline`/`GPUCommandEncoder`/`GPURenderPassEncoder`/
// `GPUComputePassEncoder`/`GPUCommandBuffer`/`GPURenderBundleEncoder`/`GPUQuerySet`/
// `GPUCanvasContext` + flags `GPUBufferUsage`/`GPUTextureUsage`/`GPUShaderStage`/
// `GPUMapMode`/`GPUColorWrite`). Vecino moderno de WebGL (Fase 7.151): el device
// registra los comandos en `device._cmds` y publica `kind: 'webgpu-submit'`; el
// rasterizado real sobre la GPU (compile de WGSL, pipelines, draws) es del chrome
// (wiring PENDIENTE — `__puriy_webgpu_request_adapter` lo reemplaza por uno real).
//   · Punto de entrada: `navigator.gpu.requestAdapter()` y, para el canvas,
//     `new OffscreenCanvas(w,h).getContext('webgpu')`.
//   · Compile/link se simulan exitosos (`getCompilationInfo` → sin mensajes) para
//     que la máquina de estados de una app WebGPU avance sin GPU.
(function() {
    if (globalThis.navigator != null && globalThis.navigator.gpu != null) return;
    var GS = String.fromCharCode(0x1D);
    var nextId = 1;

    // ---------- flags estáticos ----------
    globalThis.GPUBufferUsage = {
        MAP_READ: 0x0001, MAP_WRITE: 0x0002, COPY_SRC: 0x0004, COPY_DST: 0x0008,
        INDEX: 0x0010, VERTEX: 0x0020, UNIFORM: 0x0040, STORAGE: 0x0080,
        INDIRECT: 0x0100, QUERY_RESOLVE: 0x0200
    };
    globalThis.GPUTextureUsage = {
        COPY_SRC: 0x01, COPY_DST: 0x02, TEXTURE_BINDING: 0x04,
        STORAGE_BINDING: 0x08, RENDER_ATTACHMENT: 0x10
    };
    globalThis.GPUShaderStage = { VERTEX: 0x1, FRAGMENT: 0x2, COMPUTE: 0x4 };
    globalThis.GPUColorWrite = { RED: 0x1, GREEN: 0x2, BLUE: 0x4, ALPHA: 0x8, ALL: 0xF };
    globalThis.GPUMapMode = { READ: 0x0001, WRITE: 0x0002 };

    // ---------- cáscaras de recurso ----------
    function tagResource(obj, label) { obj.label = label != null ? String(label) : ''; obj._id = nextId++; return obj; }

    function GPUTextureView() {}
    function GPUSampler() {}
    function GPUBindGroupLayout() {}
    function GPUPipelineLayout() {}
    function GPUBindGroup() {}
    function GPUQuerySet(desc) { desc = desc || {}; this.type = desc.type || 'occlusion'; this.count = desc.count || 0; }
    GPUQuerySet.prototype.destroy = function() {};

    function GPUShaderModule(desc) { this._code = (desc && desc.code) || ''; }
    GPUShaderModule.prototype.getCompilationInfo = function() {
        return Promise.resolve({ messages: [] });
    };

    function GPUBuffer(desc) {
        desc = desc || {};
        this.size = desc.size || 0;
        this.usage = desc.usage || 0;
        this.mapState = 'unmapped';
        this._mapped = desc.mappedAtCreation ? new ArrayBuffer(this.size) : null;
        if (desc.mappedAtCreation) this.mapState = 'mapped';
    }
    GPUBuffer.prototype.mapAsync = function(mode, offset, size) {
        this.mapState = 'pending';
        var self = this;
        return new Promise(function(resolve) {
            self._mapped = new ArrayBuffer(self.size);
            self.mapState = 'mapped';
            resolve(undefined);
        });
    };
    GPUBuffer.prototype.getMappedRange = function(offset, size) {
        if (!this._mapped) this._mapped = new ArrayBuffer(this.size);
        offset = offset || 0;
        size = size != null ? size : (this.size - offset);
        return this._mapped.slice(offset, offset + size);
    };
    GPUBuffer.prototype.unmap = function() { this.mapState = 'unmapped'; this._mapped = null; };
    GPUBuffer.prototype.destroy = function() { this.mapState = 'unmapped'; this._mapped = null; };

    function GPUTexture(desc) {
        desc = desc || {};
        var sz = desc.size || [1, 1, 1];
        if (typeof sz === 'object' && !Array.isArray(sz)) {
            this.width = sz.width || 1; this.height = sz.height || 1; this.depthOrArrayLayers = sz.depthOrArrayLayers || 1;
        } else {
            this.width = sz[0] || 1; this.height = sz[1] || 1; this.depthOrArrayLayers = sz[2] || 1;
        }
        this.format = desc.format || 'rgba8unorm';
        this.usage = desc.usage || 0;
        this.dimension = desc.dimension || '2d';
        this.mipLevelCount = desc.mipLevelCount || 1;
        this.sampleCount = desc.sampleCount || 1;
    }
    GPUTexture.prototype.createView = function(d) { return tagResource(new GPUTextureView(), d && d.label); };
    GPUTexture.prototype.destroy = function() {};

    // ---------- pipelines ----------
    function makeBindGroupLayoutGetter(obj) {
        obj.getBindGroupLayout = function(index) { return tagResource(new GPUBindGroupLayout()); };
    }
    function GPURenderPipeline() { makeBindGroupLayoutGetter(this); }
    function GPUComputePipeline() { makeBindGroupLayoutGetter(this); }

    // ---------- pass encoders ----------
    function recordPass(cmds, prefix) {
        var pass = {};
        var verbs = ['setPipeline', 'setBindGroup', 'setVertexBuffer', 'setIndexBuffer',
                     'setViewport', 'setScissorRect', 'setBlendConstant', 'setStencilReference',
                     'draw', 'drawIndexed', 'drawIndirect', 'drawIndexedIndirect',
                     'dispatchWorkgroups', 'dispatchWorkgroupsIndirect',
                     'pushDebugGroup', 'popDebugGroup', 'insertDebugMarker',
                     'beginOcclusionQuery', 'endOcclusionQuery', 'executeBundles'];
        verbs.forEach(function(v) {
            pass[v] = function() { cmds.push([prefix + '.' + v].concat(Array.prototype.slice.call(arguments))); };
        });
        pass.end = function() { cmds.push([prefix + '.end']); };
        return pass;
    }

    function GPUCommandEncoder(device) { this._device = device; this._cmds = device._cmds; }
    GPUCommandEncoder.prototype.beginRenderPass = function(desc) {
        this._cmds.push(['beginRenderPass']);
        return recordPass(this._cmds, 'renderPass');
    };
    GPUCommandEncoder.prototype.beginComputePass = function(desc) {
        this._cmds.push(['beginComputePass']);
        return recordPass(this._cmds, 'computePass');
    };
    GPUCommandEncoder.prototype.copyBufferToBuffer = function() { this._cmds.push(['copyBufferToBuffer']); };
    GPUCommandEncoder.prototype.copyBufferToTexture = function() { this._cmds.push(['copyBufferToTexture']); };
    GPUCommandEncoder.prototype.copyTextureToBuffer = function() { this._cmds.push(['copyTextureToBuffer']); };
    GPUCommandEncoder.prototype.copyTextureToTexture = function() { this._cmds.push(['copyTextureToTexture']); };
    GPUCommandEncoder.prototype.clearBuffer = function() { this._cmds.push(['clearBuffer']); };
    GPUCommandEncoder.prototype.resolveQuerySet = function() { this._cmds.push(['resolveQuerySet']); };
    GPUCommandEncoder.prototype.pushDebugGroup = function() {};
    GPUCommandEncoder.prototype.popDebugGroup = function() {};
    GPUCommandEncoder.prototype.insertDebugMarker = function() {};
    GPUCommandEncoder.prototype.finish = function(desc) {
        var buf = {}; tagResource(buf, desc && desc.label); buf._kind = 'GPUCommandBuffer'; return buf;
    };

    function GPURenderBundleEncoder(device) { this._cmds = device._cmds; var p = recordPass(this._cmds, 'bundle'); for (var k in p) this[k] = p[k]; }
    GPURenderBundleEncoder.prototype.finish = function(desc) { var b = {}; tagResource(b, desc && desc.label); b._kind = 'GPURenderBundle'; return b; };

    // ---------- queue ----------
    function GPUQueue(device) { this._device = device; this.label = ''; }
    GPUQueue.prototype.submit = function(buffers) {
        this._device._cmds.push(['submit', (buffers || []).length]);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'webgpu-submit', value: this._device._id + GS + (buffers || []).length });
        var snap = this._device._cmds.slice();
        this._device._cmds.length = 0;
        this._device._submitted.push(snap);
    };
    GPUQueue.prototype.writeBuffer = function() { this._device._cmds.push(['writeBuffer']); };
    GPUQueue.prototype.writeTexture = function() { this._device._cmds.push(['writeTexture']); };
    GPUQueue.prototype.copyExternalImageToTexture = function() { this._device._cmds.push(['copyExternalImageToTexture']); };
    GPUQueue.prototype.onSubmittedWorkDone = function() { return Promise.resolve(undefined); };

    // ---------- device ----------
    var DEFAULT_LIMITS = {
        maxTextureDimension1D: 8192, maxTextureDimension2D: 8192, maxTextureDimension3D: 2048,
        maxTextureArrayLayers: 256, maxBindGroups: 4, maxBindingsPerBindGroup: 1000,
        maxBufferSize: 268435456, maxVertexBuffers: 8, maxVertexAttributes: 16,
        maxComputeWorkgroupSizeX: 256, maxComputeWorkgroupSizeY: 256, maxComputeWorkgroupSizeZ: 64,
        maxComputeInvocationsPerWorkgroup: 256, maxComputeWorkgroupsPerDimension: 65535,
        maxColorAttachments: 8, minUniformBufferOffsetAlignment: 256, minStorageBufferOffsetAlignment: 256
    };

    function GPUDevice(desc) {
        var et = (typeof globalThis.EventTarget === 'function') ? new globalThis.EventTarget() : { addEventListener: function() {}, removeEventListener: function() {}, dispatchEvent: function() { return true; } };
        for (var m in et) { if (typeof et[m] === 'function') this[m] = et[m].bind(et); }
        this._id = nextId++;
        this._cmds = [];
        this._submitted = [];
        this.label = (desc && desc.label) || '';
        this.features = makeFeatureSet((desc && desc.requiredFeatures) || []);
        this.limits = Object.assign({}, DEFAULT_LIMITS, (desc && desc.requiredLimits) || {});
        this.queue = new GPUQueue(this);
        this.onuncapturederror = null;
        var self = this;
        this.lost = new Promise(function() {});  // nunca se pierde por defecto
        this._errorScopes = [];
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'webgpu-device', value: String(this._id) });
    }
    GPUDevice.prototype.createBuffer = function(d) { return tagResource(new GPUBuffer(d), d && d.label); };
    GPUDevice.prototype.createTexture = function(d) { return tagResource(new GPUTexture(d), d && d.label); };
    GPUDevice.prototype.createSampler = function(d) { return tagResource(new GPUSampler(), d && d.label); };
    GPUDevice.prototype.createBindGroupLayout = function(d) { return tagResource(new GPUBindGroupLayout(), d && d.label); };
    GPUDevice.prototype.createPipelineLayout = function(d) { return tagResource(new GPUPipelineLayout(), d && d.label); };
    GPUDevice.prototype.createBindGroup = function(d) { return tagResource(new GPUBindGroup(), d && d.label); };
    GPUDevice.prototype.createShaderModule = function(d) { return tagResource(new GPUShaderModule(d), d && d.label); };
    GPUDevice.prototype.createRenderPipeline = function(d) { return tagResource(new GPURenderPipeline(), d && d.label); };
    GPUDevice.prototype.createComputePipeline = function(d) { return tagResource(new GPUComputePipeline(), d && d.label); };
    GPUDevice.prototype.createRenderPipelineAsync = function(d) { return Promise.resolve(this.createRenderPipeline(d)); };
    GPUDevice.prototype.createComputePipelineAsync = function(d) { return Promise.resolve(this.createComputePipeline(d)); };
    GPUDevice.prototype.createCommandEncoder = function(d) { return tagResource(new GPUCommandEncoder(this), d && d.label); };
    GPUDevice.prototype.createRenderBundleEncoder = function(d) { return tagResource(new GPURenderBundleEncoder(this), d && d.label); };
    GPUDevice.prototype.createQuerySet = function(d) { return tagResource(new GPUQuerySet(d), d && d.label); };
    GPUDevice.prototype.destroy = function() {};
    GPUDevice.prototype.pushErrorScope = function(filter) { this._errorScopes.push(filter); };
    GPUDevice.prototype.popErrorScope = function() { this._errorScopes.pop(); return Promise.resolve(null); };

    // ---------- adapter ----------
    function makeFeatureSet(list) {
        var s = new Set(list || []);
        return s;
    }

    function GPUAdapter(info) {
        this.features = makeFeatureSet(['texture-compression-bc', 'timestamp-query']);
        this.limits = Object.assign({}, DEFAULT_LIMITS);
        this.isFallbackAdapter = false;
        this._info = info || { vendor: '', architecture: '', device: '', description: 'puriy-webgpu' };
    }
    GPUAdapter.prototype.requestDevice = function(desc) {
        return Promise.resolve(new GPUDevice(desc));
    };
    GPUAdapter.prototype.requestAdapterInfo = function() { return Promise.resolve(this._info); };
    Object.defineProperty(GPUAdapter.prototype, 'info', { get: function() { return this._info; }, configurable: true });

    // ---------- GPU (navigator.gpu) ----------
    function GPU() {}
    GPU.prototype.requestAdapter = function(options) {
        if (typeof globalThis.__puriy_webgpu_request_adapter === 'function') {
            return globalThis.__puriy_webgpu_request_adapter(options);
        }
        return Promise.resolve(new GPUAdapter());
    };
    GPU.prototype.getPreferredCanvasFormat = function() { return 'bgra8unorm'; };
    Object.defineProperty(GPU.prototype, 'wgslLanguageFeatures', {
        get: function() { return new Set(['readonly_and_readwrite_storage_textures']); }, configurable: true });

    // ---------- GPUCanvasContext (canvas.getContext('webgpu')) ----------
    function GPUCanvasContext(canvas) { this.canvas = canvas; this._config = null; this._device = null; }
    GPUCanvasContext.prototype.configure = function(config) { this._config = config || {}; this._device = this._config.device || null; };
    GPUCanvasContext.prototype.unconfigure = function() { this._config = null; };
    GPUCanvasContext.prototype.getCurrentTexture = function() {
        var w = this.canvas ? (this.canvas.width || 1) : 1;
        var h = this.canvas ? (this.canvas.height || 1) : 1;
        var fmt = (this._config && this._config.format) || 'bgra8unorm';
        return tagResource(new GPUTexture({ size: [w, h, 1], format: fmt, usage: globalThis.GPUTextureUsage.RENDER_ATTACHMENT }));
    };

    // Engancha 'webgpu' al getContext de OffscreenCanvas (mismo molde que webgl 7.151).
    if (typeof globalThis.OffscreenCanvas === 'function') {
        var proto = globalThis.OffscreenCanvas.prototype;
        var origGetContext = proto.getContext;
        proto.getContext = function(type, attrs) {
            if (type === 'webgpu') {
                if (this._ctx && this._ctx instanceof GPUCanvasContext) return this._ctx;
                this._ctx = new GPUCanvasContext(this);
                return this._ctx;
            }
            return origGetContext ? origGetContext.call(this, type, attrs) : null;
        };
    }

    // Exporta los constructores como globals (instanceof en apps reales).
    globalThis.GPU = GPU;
    globalThis.GPUAdapter = GPUAdapter;
    globalThis.GPUDevice = GPUDevice;
    globalThis.GPUQueue = GPUQueue;
    globalThis.GPUBuffer = GPUBuffer;
    globalThis.GPUTexture = GPUTexture;
    globalThis.GPUTextureView = GPUTextureView;
    globalThis.GPUSampler = GPUSampler;
    globalThis.GPUShaderModule = GPUShaderModule;
    globalThis.GPUBindGroupLayout = GPUBindGroupLayout;
    globalThis.GPUPipelineLayout = GPUPipelineLayout;
    globalThis.GPUBindGroup = GPUBindGroup;
    globalThis.GPURenderPipeline = GPURenderPipeline;
    globalThis.GPUComputePipeline = GPUComputePipeline;
    globalThis.GPUCommandEncoder = GPUCommandEncoder;
    globalThis.GPURenderBundleEncoder = GPURenderBundleEncoder;
    globalThis.GPUQuerySet = GPUQuerySet;
    globalThis.GPUCanvasContext = GPUCanvasContext;

    var nav = globalThis.navigator = globalThis.navigator || {};
    nav.gpu = new GPU();
    void 0;
})();
"#;
