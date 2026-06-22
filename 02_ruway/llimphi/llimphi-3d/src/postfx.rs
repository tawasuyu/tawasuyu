//! `PostFx` — **antialiasing por supersampling (SSAA) + bloom** genéricos como
//! pase de post-proceso reutilizable sobre cualquier render 3D wgpu.
//!
//! Cosechado del renderer 2.5D de Doom (`supay-render-llimphi::wgpu3d`), donde
//! estos efectos nacieron pegados a tipos del juego. Acá viven en su forma
//! agnóstica: nada sabe de paredes, sprites ni WAD. Cualquier escena que se
//! pueda dibujar en un pase wgpu (las mallas de [`Renderer3d`](crate::Renderer3d),
//! los voxels de [`VoxelRenderer`](crate::VoxelRenderer), una carta 3D de cosmos,
//! una galería de nahual) obtiene AA + glow «gratis» envolviendo su dibujo en
//! [`PostFx::render_with`].
//!
//! ## Flujo (igual que el de supay, sin sus tipos)
//!
//! 1. La escena se rinde a un color intermedio a `supersample`× la resolución
//!    de salida (4× de fragmentos con factor 2) con su propio depth.
//! 2. **Bright-pass + blur 5×5** de ese color → textura de bloom a media res
//!    (extrae lo que supera el umbral de luminancia y lo suaviza).
//! 3. **Blit final**: baja la escena al target real con filtro lineal (eso es
//!    el antialiasing) y le suma el bloom (eso es el glow). Compone con
//!    `LoadOp::Load`, así no pisa la UI vello que ya esté debajo.
//!
//! ## Uso
//!
//! ```ignore
//! let mut fx = PostFx::new(&device, color_format);
//! // ... en gpu_paint_with(dev, q, enc, view, rect, _vp):
//! fx.render_with(dev, q, enc, view, (w, h), wgpu::Color::BLACK, |pass| {
//!     // `pass` ya está a supersample× con color+depth; sólo dibujá.
//!     renderer3d.upload(q, w as f32 / h as f32, &camera); // antes del pase
//!     renderer3d.draw(pass);
//! });
//! ```

use crate::scene::DEPTH_FORMAT;

/// Parámetros del post-proceso. Los defaults replican el «look» de supay
/// (supersample 2×, glow suave). Ajustables por escena: una carta astral quiere
/// poco bloom, un FPS infernal mucho.
#[derive(Clone, Copy, Debug)]
pub struct PostFxConfig {
    /// Factor de supersampling por eje. 2 = rinde a 2× y promedia al bajar
    /// (antialiasing 4×). 1 = sin SSAA (sólo bloom). Útil bajarlo en GPUs flojas.
    pub supersample: u32,
    /// Cuánto se suma el bloom al blit final (0 = sin glow).
    pub bloom_strength: f32,
    /// Umbral de luminancia del bright-pass: por encima de esto un píxel sangra.
    pub bloom_threshold: f32,
    /// Rampa del bright-pass (knee): ancho de la transición sobre el umbral.
    pub bloom_knee: f32,
    /// Radio del blur del bright-pass, en téxeles de la textura de bloom.
    pub bloom_radius: f32,
}

impl Default for PostFxConfig {
    fn default() -> Self {
        Self {
            supersample: 2,
            bloom_strength: 0.85,
            bloom_threshold: 0.62,
            bloom_knee: 0.38,
            bloom_radius: 2.5,
        }
    }
}

/// Target intermedio de la escena (color supersampleado + depth) más el bind
/// group que expone su color como textura para el bright-pass y el blit.
struct SceneTarget {
    w: u32,
    h: u32,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    tex_bg: wgpu::BindGroup,
}

/// Target de bloom (media resolución): color + bind group para sumarlo en el blit.
struct BloomTarget {
    w: u32,
    h: u32,
    color_view: wgpu::TextureView,
    tex_bg: wgpu::BindGroup,
}

/// Pase de post-proceso reutilizable: SSAA + bloom sobre cualquier render wgpu.
/// Cachea pipelines/sampler/layouts y recrea los targets al cambiar de tamaño.
pub struct PostFx {
    cfg: PostFxConfig,
    output_format: wgpu::TextureFormat,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Bright-pass + blur: escena → bloom. group0 = escena, group1 = uniform.
    bright_pipeline: wgpu::RenderPipeline,
    /// Blit final: escena + bloom → target. group0 = escena, 1 = bloom, 2 = uniform.
    blit_pipeline: wgpu::RenderPipeline,
    bright_uniform: wgpu::Buffer,
    bright_uniform_bg: wgpu::BindGroup,
    blit_uniform: wgpu::Buffer,
    blit_uniform_bg: wgpu::BindGroup,
    scene: Option<SceneTarget>,
    bloom: Option<BloomTarget>,
}

impl PostFx {
    /// Crea el post-proceso para el `output_format` del target final (el de la
    /// textura intermedia del frame: `Rgba8Unorm` headless, el de la surface en
    /// vivo). Config por defecto.
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        Self::with_config(device, output_format, PostFxConfig::default())
    }

    /// Igual que [`Self::new`] con una [`PostFxConfig`] explícita.
    pub fn with_config(
        device: &wgpu::Device,
        output_format: wgpu::TextureFormat,
        cfg: PostFxConfig,
    ) -> Self {
        // Layout textura+sampler (compartido por escena y bloom como entradas).
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-postfx-tex-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-postfx-uniform-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("llimphi-3d-postfx-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Helper para pipelines fullscreen (triángulo cubre-pantalla, sin VBO).
        let make_fullscreen = |layouts: &[&wgpu::BindGroupLayout],
                               wgsl: &str,
                               label: &str| {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                push_constant_ranges: &[],
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: output_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            })
        };

        let bright_pipeline = make_fullscreen(
            &[&tex_layout, &uniform_layout],
            BRIGHT_WGSL,
            "llimphi-3d-postfx-bright",
        );
        let blit_pipeline = make_fullscreen(
            &[&tex_layout, &tex_layout, &uniform_layout],
            BLIT_WGSL,
            "llimphi-3d-postfx-blit",
        );

        let mk_uniform = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 16, // un vec4<f32>
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let bright_uniform = mk_uniform("llimphi-3d-postfx-bright-u");
        let blit_uniform = mk_uniform("llimphi-3d-postfx-blit-u");
        let mk_uniform_bg = |buf: &wgpu::Buffer, label: &str| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buf.as_entire_binding(),
                }],
            })
        };
        let bright_uniform_bg = mk_uniform_bg(&bright_uniform, "llimphi-3d-postfx-bright-u-bg");
        let blit_uniform_bg = mk_uniform_bg(&blit_uniform, "llimphi-3d-postfx-blit-u-bg");

        Self {
            cfg,
            output_format,
            tex_layout,
            sampler,
            bright_pipeline,
            blit_pipeline,
            bright_uniform,
            bright_uniform_bg,
            blit_uniform,
            blit_uniform_bg,
            scene: None,
            bloom: None,
        }
    }

    /// Config actual (lectura).
    pub fn config(&self) -> PostFxConfig {
        self.cfg
    }

    /// Cambia la config (toma efecto el próximo frame). Si cambia `supersample`
    /// los targets se recrean solos al verificar el tamaño.
    pub fn set_config(&mut self, cfg: PostFxConfig) {
        self.cfg = cfg;
    }

    /// Formato del target final con el que se creó.
    pub fn output_format(&self) -> wgpu::TextureFormat {
        self.output_format
    }

    /// Tamaño supersampleado al que el llamador debe rendir su escena, dado el
    /// `output_size` final. (= `output × supersample`.) Útil para el aspect/UI.
    pub fn scene_size(&self, output_size: (u32, u32)) -> (u32, u32) {
        let ss = self.cfg.supersample.max(1);
        (output_size.0 * ss, output_size.1 * ss)
    }

    /// Asegura los targets internos para una salida `output_size` y sube los
    /// uniforms del frame. **Llamar antes** de [`Self::scene_pass`] y
    /// [`Self::resolve`] con el mismo `output_size`.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_size: (u32, u32),
    ) {
        let (ow, oh) = output_size;
        if ow == 0 || oh == 0 {
            return;
        }
        let (rw, rh) = self.scene_size(output_size);
        self.ensure_scene(device, rw, rh);
        self.ensure_bloom(device, (ow / 2).max(1), (oh / 2).max(1));
        self.upload_uniforms(queue);
    }

    /// Abre el pase de escena a resolución supersampleada (color limpiado a
    /// `clear` + depth limpiado a lejano) y devuelve el [`wgpu::RenderPass`] para
    /// que el llamador setee pipeline(s) y dibuje. Requiere [`Self::prepare`]
    /// previo en el mismo frame. Al soltar el pase, llamar [`Self::resolve`].
    ///
    /// El depth es `Depth32Float` (igual que [`Renderer3d`](crate::Renderer3d) y
    /// [`Scene3d`](crate::Scene3d)), así sus pipelines encajan sin cambios.
    pub fn scene_pass<'p>(
        &'p self,
        encoder: &'p mut wgpu::CommandEncoder,
        clear: wgpu::Color,
    ) -> wgpu::RenderPass<'p> {
        let scene = self
            .scene
            .as_ref()
            .expect("PostFx::scene_pass sin prepare() previo");
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("llimphi-3d-postfx-scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &scene.color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &scene.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        })
    }

    /// Resuelve la escena ya dibujada: bright-pass + blur → bloom, luego blit
    /// (bajada SSAA + suma de bloom) sobre `target`, preservando lo que haya
    /// debajo (`LoadOp::Load`). Requiere [`Self::prepare`] + el pase de
    /// [`Self::scene_pass`] ya soltado.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let scene_bg = match self.scene.as_ref() {
            Some(s) => &s.tex_bg,
            None => return,
        };
        let bloom = self.bloom.as_ref().unwrap();

        // --- Bright-pass + blur: escena → textura de bloom (media res). ---
        {
            let mut bp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("llimphi-3d-postfx-bright"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &bloom.color_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            bp.set_pipeline(&self.bright_pipeline);
            bp.set_bind_group(0, scene_bg, &[]);
            bp.set_bind_group(1, &self.bright_uniform_bg, &[]);
            bp.draw(0..3, 0..1);
        }

        // --- Blit final: escena (SSAA) + bloom → target real (preserva debajo). ---
        {
            let mut blit = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("llimphi-3d-postfx-blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            blit.set_pipeline(&self.blit_pipeline);
            blit.set_bind_group(0, scene_bg, &[]);
            blit.set_bind_group(1, &bloom.tex_bg, &[]);
            blit.set_bind_group(2, &self.blit_uniform_bg, &[]);
            blit.draw(0..3, 0..1);
        }
    }

    fn upload_uniforms(&self, queue: &wgpu::Queue) {
        // bright: (threshold, knee, radius, _)
        let mut b = Vec::with_capacity(16);
        for v in [
            self.cfg.bloom_threshold,
            self.cfg.bloom_knee.max(1e-4),
            self.cfg.bloom_radius,
            0.0,
        ] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        queue.write_buffer(&self.bright_uniform, 0, &b);
        // blit: (bloom_strength, _, _, _)
        let mut s = Vec::with_capacity(16);
        for v in [self.cfg.bloom_strength, 0.0, 0.0, 0.0] {
            s.extend_from_slice(&v.to_ne_bytes());
        }
        queue.write_buffer(&self.blit_uniform, 0, &s);
    }

    fn ensure_scene(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if matches!(&self.scene, Some(s) if s.w == w && s.h == h) {
            return;
        }
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-postfx-scene-color"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let color_view = color.create_view(&Default::default());
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-postfx-scene-depth"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        let tex_bg = self.make_tex_bg(device, &color_view, "llimphi-3d-postfx-scene-bg");
        self.scene = Some(SceneTarget { w, h, color_view, depth_view, tex_bg });
    }

    fn ensure_bloom(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if matches!(&self.bloom, Some(b) if b.w == w && b.h == h) {
            return;
        }
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-postfx-bloom-color"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let color_view = color.create_view(&Default::default());
        let tex_bg = self.make_tex_bg(device, &color_view, "llimphi-3d-postfx-bloom-bg");
        self.bloom = Some(BloomTarget { w, h, color_view, tex_bg });
    }

    fn make_tex_bg(
        &self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        label: &str,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }
}

/// Bright-pass + blur 5×5 de la escena → textura de bloom (media res). En una
/// pasada: downsample bilinear + extracción de lo brillante + suavizado.
/// `u.params = (threshold, knee, radius, _)`.
pub const BRIGHT_WGSL: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
struct U { params: vec4<f32> };
@group(1) @binding(0) var<uniform> u: U;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var o: VOut;
    let c = p[vi];
    o.clip = vec4<f32>(c, 0.0, 1.0);
    o.uv = vec2<f32>(c.x * 0.5 + 0.5, 1.0 - (c.y * 0.5 + 0.5));
    return o;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let dim = vec2<f32>(textureDimensions(tex, 0));
    let texel = 1.0 / dim;
    let thresh = u.params.x;
    let knee = u.params.y;
    let radius = u.params.z;
    var acc = vec3<f32>(0.0);
    for (var j = -2; j <= 2; j = j + 1) {
        for (var i = -2; i <= 2; i = i + 1) {
            let o = vec2<f32>(f32(i), f32(j)) * texel * radius;
            let c = textureSample(tex, samp, in.uv + o).rgb;
            let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
            // Bright-pass suave: lo que supera el umbral sangra según el knee.
            let k = clamp((l - thresh) / knee, 0.0, 1.0);
            acc += c * k;
        }
    }
    return vec4<f32>(acc / 25.0, 1.0);
}
"#;

/// Blit de bajada SSAA + suma de bloom: triángulo fullscreen que muestrea la
/// escena supersampleada (filtro lineal = antialiasing) y le suma el bloom.
/// `u.params.x = bloom_strength`.
pub const BLIT_WGSL: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(1) @binding(0) var bloom: texture_2d<f32>;
@group(1) @binding(1) var bsamp: sampler;
struct U { params: vec4<f32> };
@group(2) @binding(0) var<uniform> u: U;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var o: VOut;
    let c = p[vi];
    o.clip = vec4<f32>(c, 0.0, 1.0);
    o.uv = vec2<f32>(c.x * 0.5 + 0.5, 1.0 - (c.y * 0.5 + 0.5));
    return o;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let s = textureSample(tex, samp, in.uv).rgb;
    let b = textureSample(bloom, bsamp, in.uv).rgb;
    return vec4<f32>(s + b * u.params.x, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// Los shaders WGSL del post-proceso parsean y validan sin GPU (naga).
    /// Ataja errores que de otro modo sólo saldrían al crear el pipeline.
    #[test]
    fn shaders_wgsl_validan() {
        for (nombre, src) in [("BRIGHT_WGSL", BRIGHT_WGSL), ("BLIT_WGSL", BLIT_WGSL)] {
            let module = naga::front::wgsl::parse_str(src)
                .unwrap_or_else(|e| panic!("{nombre} no parsea: {e:?}"));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{nombre} no valida: {e:?}"));
        }
    }

    #[test]
    fn config_default_replica_supay() {
        let c = PostFxConfig::default();
        assert_eq!(c.supersample, 2);
        assert!((c.bloom_strength - 0.85).abs() < 1e-6);
        assert!((c.bloom_threshold - 0.62).abs() < 1e-6);
    }
}
