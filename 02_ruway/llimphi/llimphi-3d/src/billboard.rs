//! `Billboards` — quads **siempre de cara a la cámara**, texturizados desde un
//! atlas, con test de profundidad y alpha-discard (recorte, no blend). El
//! ladrillo genérico para sprites, partículas, etiquetas flotantes, íconos en
//! el mundo.
//!
//! Cosechado de los sprites 2.5D de Doom (`supay-render-llimphi::wgpu3d`), donde
//! los quads se armaban en CPU y estaban atados al `WadAtlas`/tabla de tintes.
//! Acá la forma es agnóstica: un **atlas** (una textura) + una lista de
//! [`Billboard`] con su sub-rect UV, tamaño en mundo y tinte. El quad de cara a
//! la cámara lo arma el vertex shader con los ejes `right`/`up` de la cámara, así
//! no hay reconstrucción por CPU cada frame.
//!
//! ```ignore
//! let mut bb = Billboards::new(&device, fmt);
//! bb.set_atlas(&device, &queue, w, h, &rgba);
//! bb.set_billboards(&device, &[Billboard { center: [0.0,1.0,0.0], size: [1.0,2.0],
//!     uv_min: [0.0,0.0], uv_max: [1.0,1.0], tint: [1.0,1.0,1.0,1.0] }]);
//! // en el pase (con depth), después del mundo opaco:
//! bb.upload(&queue, aspect, &camera);
//! bb.draw(&mut pass);
//! ```

use glam::Vec3;

use crate::camera::Camera3d;
use crate::scene::DEPTH_FORMAT;

/// Un billboard: un quad centrado en `center` (mundo), de `size` (ancho, alto en
/// unidades de mundo), texturizado con el sub-rect `[uv_min, uv_max]` del atlas y
/// multiplicado por `tint` (RGBA premultiplica el color; `a` también escala el
/// alpha del atlas para el discard).
#[derive(Clone, Copy, Debug)]
pub struct Billboard {
    pub center: [f32; 3],
    pub size: [f32; 2],
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub tint: [f32; 4],
}

impl Billboard {
    /// Floats por instancia en el buffer (`center`3 + `size`2 + `uv_min`2 +
    /// `uv_max`2 + `tint`4).
    const FLOATS: usize = 3 + 2 + 2 + 2 + 4;
    const STRIDE: usize = Self::FLOATS * 4;

    fn write_to(&self, out: &mut Vec<u8>) {
        for v in self.center {
            out.extend_from_slice(&v.to_ne_bytes());
        }
        for v in self.size {
            out.extend_from_slice(&v.to_ne_bytes());
        }
        for v in self.uv_min {
            out.extend_from_slice(&v.to_ne_bytes());
        }
        for v in self.uv_max {
            out.extend_from_slice(&v.to_ne_bytes());
        }
        for v in self.tint {
            out.extend_from_slice(&v.to_ne_bytes());
        }
    }
}

/// Atlas subido + su bind group.
struct Atlas {
    bind_group: wgpu::BindGroup,
}

/// Renderer de billboards reutilizable. Cachea pipeline/sampler/uniform; el
/// buffer de instancias se recrea en [`Self::set_billboards`].
pub struct Billboards {
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    uniform_bg: wgpu::BindGroup,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas: Option<Atlas>,
    instances: Option<wgpu::Buffer>,
    count: u32,
}

impl Billboards {
    /// Crea el renderer para el `color_format` del target. El pipeline declara
    /// depth [`DEPTH_FORMAT`] (test `Less`, escribe) → se usa en un pase con
    /// depth attachment, después de la geometría opaca.
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-bb-uniform-layout"),
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
            label: Some("llimphi-3d-bb-tex-layout"),
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
            label: Some("llimphi-3d-bb-shader"),
            source: wgpu::ShaderSource::Wgsl(BILLBOARD_WGSL.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-3d-bb-pl"),
            bind_group_layouts: &[&uniform_layout, &tex_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-3d-bb-pipeline"),
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
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-bb-uniform"),
            size: 96, // view_proj(64) + right(16) + up(16)
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-3d-bb-uniform-bg"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("llimphi-3d-bb-sampler"),
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

    /// Sube/reemplaza el atlas (RGBA8, `w×h`, sin padding). `data.len()==w*h*4`.
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
            label: Some("llimphi-3d-bb-atlas"),
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
            label: Some("llimphi-3d-bb-atlas-bg"),
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

    /// Reemplaza la lista de billboards (recrea el buffer de instancias). El
    /// orden importa para el z-fight de alpha-discard: poná los más cercanos
    /// primero si querés (con depth-write el primero gana). Idealmente el caller
    /// los ordena back-to-front.
    pub fn set_billboards(&mut self, device: &wgpu::Device, items: &[Billboard]) {
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
            label: Some("llimphi-3d-bb-instances"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: true,
        });
        buf.slice(..).get_mapped_range_mut().copy_from_slice(&bytes);
        buf.unmap();
        self.instances = Some(buf);
    }

    /// Sube el uniform del frame: `view_proj` + los ejes `right`/`up` de la
    /// cámara (para orientar los quads). `aspect` = w/h.
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

    /// Dibuja los billboards en un pase **ya abierto** (con depth). Requiere
    /// atlas + instancias + [`Self::upload`] previo. No-op si falta algo.
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

    /// Conveniencia para ubicar `up`/`right` desde una cámara sin subir nada (p.
    /// ej. para ordenar los billboards back-to-front por distancia al ojo).
    pub fn eye(camera: &Camera3d) -> Vec3 {
        camera.eye
    }
}

/// Quad de cara a la cámara por instancia: el vertex shader lo arma con los ejes
/// `right`/`up`; el fragment muestrea el atlas y descarta el alpha bajo.
const BILLBOARD_WGSL: &str = r#"
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
    // Dos triángulos: esquinas en {0,1}².
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
    // v invertida: q.y=1 (arriba del quad) ↔ uv_min.y (arriba de la imagen).
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
    if (c.a < 0.5) { discard; }
    return vec4<f32>(c.rgb, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_wgsl_valida() {
        let module =
            naga::front::wgsl::parse_str(BILLBOARD_WGSL).expect("BILLBOARD_WGSL no parsea");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("BILLBOARD_WGSL no valida");
    }

    #[test]
    fn stride_es_13_floats() {
        assert_eq!(Billboard::STRIDE, 13 * 4);
    }
}
