//! `Glows` — billboards **aditivos**: como [`Billboards`](crate::Billboards)
//! (quads de cara a la cámara desde un atlas) pero con **blend aditivo**, sin
//! escribir profundidad y **sin recorte de alpha** — para halos suaves y
//! luminosos: estrellas que brillan, atmósferas, auras de cuerpos, glows que el
//! bloom de [`PostFx`](crate::PostFx) infla. El `Billboards` opaco recorta el
//! borde (disco nítido); éste suma luz (sin borde, se funde).
//!
//! Comparte el tipo de instancia [`Billboard`](crate::Billboard). Se dibuja en
//! un pase con depth attachment, **después** de la geometría opaca (así el mundo
//! sólido ocluye los glows que quedan detrás; los de adelante suman luz):
//!
//! ```ignore
//! let mut g = Glows::new(&device, fmt);
//! g.set_atlas(&device, &queue, w, h, &soft_radial_rgba);
//! g.set_glows(&device, &[Billboard { .. }]);
//! g.upload(&queue, aspect, &camera);
//! g.draw(&mut pass);
//! ```

use crate::billboard::Billboard;
use crate::camera::Camera3d;
use crate::scene::DEPTH_FORMAT;

struct Atlas {
    bind_group: wgpu::BindGroup,
}

/// Renderer de glows aditivos. Misma plomería que `Billboards`, distinto blend.
pub struct Glows {
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    uniform_bg: wgpu::BindGroup,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas: Option<Atlas>,
    instances: Option<wgpu::Buffer>,
    count: u32,
}

impl Glows {
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-glow-uniform-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-glow-tex-layout"),
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-3d-glow-shader"),
            source: wgpu::ShaderSource::Wgsl(GLOW_WGSL.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-3d-glow-pl"),
            bind_group_layouts: &[&uniform_layout, &tex_layout],
            push_constant_ranges: &[],
        });
        // Blend aditivo: la luz se suma al fondo (src*1 + dst*1).
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-3d-glow-pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: Billboard::STRIDE as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 20,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 28,
                            shader_location: 3,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 36,
                            shader_location: 4,
                        },
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(additive),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-glow-uniform"),
            size: 96, // view_proj(64) + right(16) + up(16)
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-3d-glow-uniform-bg"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("llimphi-3d-glow-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        Self {
            pipeline,
            uniform_buf,
            uniform_bg,
            tex_layout,
            sampler,
            atlas: None,
            instances: None,
            count: 0,
        }
    }

    /// Sube/reemplaza el atlas (RGBA8, `w×h`). Para glows conviene un degradé
    /// radial suave (gaussiano), sin núcleo duro.
    pub fn set_atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
        data: &[u8],
    ) {
        assert_eq!(data.len(), (w * h * 4) as usize, "RGBA8 w*h*4 esperado");
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-glow-atlas"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let view = tex.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-3d-glow-atlas-bg"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.atlas = Some(Atlas { bind_group });
    }

    /// Reemplaza la lista de glows (recrea el buffer de instancias). Con blend
    /// aditivo el orden no importa (la suma es conmutativa).
    pub fn set_glows(&mut self, device: &wgpu::Device, items: &[Billboard]) {
        self.count = items.len() as u32;
        if items.is_empty() {
            self.instances = None;
            return;
        }
        let mut bytes = Vec::with_capacity(items.len() * Billboard::STRIDE);
        for b in items {
            b.write_to(&mut bytes);
        }
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-glow-instances"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: true,
        });
        buf.slice(..).get_mapped_range_mut().copy_from_slice(&bytes);
        buf.unmap();
        self.instances = Some(buf);
    }

    /// Sube el uniform del frame: `view_proj` + ejes `right`/`up` de la cámara.
    pub fn upload(&self, queue: &wgpu::Queue, aspect: f32, camera: &Camera3d) {
        let view_proj = camera.view_proj(aspect);
        let forward = (camera.target - camera.eye).normalize_or_zero();
        let right = forward.cross(camera.up).normalize_or_zero();
        let up = right.cross(forward);
        let mut b = Vec::with_capacity(96);
        for v in view_proj.to_cols_array() {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [right.x, right.y, right.z, 0.0] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [up.x, up.y, up.z, 0.0] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        queue.write_buffer(&self.uniform_buf, 0, &b);
    }

    /// Dibuja los glows en un pase ya abierto (con depth). No-op si falta algo.
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let (Some(atlas), Some(inst)) = (self.atlas.as_ref(), self.instances.as_ref()) else {
            return;
        };
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bg, &[]);
        pass.set_bind_group(1, &atlas.bind_group, &[]);
        pass.set_vertex_buffer(0, inst.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}

/// Quad de cara a la cámara; el fragment SUMA luz (sin discard). La salida es
/// premultiplicada (`rgb*a`) para que el blend aditivo dé un halo suave.
const GLOW_WGSL: &str = r#"
struct U {
    view_proj: mat4x4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VIn {
    @location(0) center: vec3<f32>,
    @location(1) size: vec2<f32>,
    @location(2) uv_min: vec2<f32>,
    @location(3) uv_max: vec2<f32>,
    @location(4) tint: vec4<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, in: VIn) -> VOut {
    var cs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let q = cs[vi];
    let off = q - vec2<f32>(0.5, 0.5);
    let world = in.center
        + u.right.xyz * (off.x * in.size.x)
        + u.up.xyz * (off.y * in.size.y);
    var o: VOut;
    o.clip = u.view_proj * vec4<f32>(world, 1.0);
    o.uv = vec2<f32>(
        mix(in.uv_min.x, in.uv_max.x, q.x),
        mix(in.uv_min.y, in.uv_max.y, 1.0 - q.y),
    );
    o.tint = in.tint;
    return o;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv) * in.tint;
    let a = c.a;
    return vec4<f32>(c.rgb * a, a);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glow_wgsl_valida() {
        let module = naga::front::wgsl::parse_str(GLOW_WGSL).expect("GLOW_WGSL no parsea");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("GLOW_WGSL no valida");
    }
}
