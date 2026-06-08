//! Pipeline GPU para la grilla de celdas del modo TUI (Fase 4 del SDD-TERMINAL).
//!
//! Define las **estructuras POD** (instance + uniforms), el **shader WGSL**
//! y el **pipeline wgpu** (`CellPipeline`) que las consume. El wireado al
//! `generic_grid_panel` vía `gpu_paint_with` va en el commit siguiente —
//! acá ya se valida que el shader compila en un device headless.
//!
//! ## Layouts
//!
//! - **Instance** (32 B): `[cell_x: f32, cell_y: f32, uv_x: f32, uv_y: f32,
//!   uv_w: f32, uv_h: f32, fg_rgba: u32, bg_rgba: u32]`.
//!   Una por celda visible; el vertex stage emite los 4 corners (TriangleStrip).
//! - **Uniforms** (32 B): `[viewport_w: f32, viewport_h: f32, cell_w: f32,
//!   cell_h: f32, atlas_w: f32, atlas_h: f32, _pad: [f32; 2]]`.
//!
//! El fragment samplea el atlas grayscale en `uv`; alpha = cobertura;
//! out = mix(bg, fg, alpha). Blending estándar `OVER` por encima.
//!
//! ## Por qué quads instanciados
//!
//! Una grilla de 100×40 = 4000 celdas; en vello eso son ~4000 Views + 4000
//! draws + el shaping de cada char. Con quads instanciados es UNA draw call
//! de 4000 instancias y la GPU pinta todo en paralelo. Igual de simple
//! para 200×80 (16k celdas) — patrón ya validado en `GpuPipelines.rects`.

/// Una celda lista para dibujar. **POD, repr(C)** — `as_bytes` la serializa
/// a una secuencia plana de `f32`/`u32` little-endian para el buffer GPU.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct CellInstance {
    /// Posición (px) de la esquina superior-izquierda de la celda en
    /// viewport coords.
    pub cell_x: f32,
    pub cell_y: f32,
    /// Coords UV (px) del glifo en la textura del atlas. El shader las
    /// divide por `atlas_size` para obtener UVs normalizadas 0..1.
    pub uv_x: f32,
    pub uv_y: f32,
    pub uv_w: f32,
    pub uv_h: f32,
    /// Color foreground del glifo, RGBA8 empacado little-endian
    /// (`r | g<<8 | b<<16 | a<<24`).
    pub fg_rgba: u32,
    /// Color background de la celda, RGBA8 empacado.
    pub bg_rgba: u32,
}

impl CellInstance {
    /// Tamaño en bytes del layout — debe coincidir con `array_stride` del
    /// pipeline en wgpu. Compile-time const para que el caller arme el
    /// vertex layout sin recalcular.
    pub const SIZE: usize = 32;

    /// Serializa a 32 bytes little-endian para `Queue::write_buffer`.
    pub fn as_bytes(&self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..4].copy_from_slice(&self.cell_x.to_le_bytes());
        out[4..8].copy_from_slice(&self.cell_y.to_le_bytes());
        out[8..12].copy_from_slice(&self.uv_x.to_le_bytes());
        out[12..16].copy_from_slice(&self.uv_y.to_le_bytes());
        out[16..20].copy_from_slice(&self.uv_w.to_le_bytes());
        out[20..24].copy_from_slice(&self.uv_h.to_le_bytes());
        out[24..28].copy_from_slice(&self.fg_rgba.to_le_bytes());
        out[28..32].copy_from_slice(&self.bg_rgba.to_le_bytes());
        out
    }
}

/// Empaca un color `(r, g, b, a)` en un `u32` RGBA little-endian que el
/// shader lee como `vec4<u32>` y normaliza a `vec4<f32>(r,g,b,a)/255`.
pub fn pack_rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24)
}

/// Serializa un slice de instancias a un buffer de bytes contiguo. Útil
/// para `Queue::write_buffer`.
pub fn instances_to_bytes(cells: &[CellInstance]) -> Vec<u8> {
    let mut out = Vec::with_capacity(cells.len() * CellInstance::SIZE);
    for c in cells {
        out.extend_from_slice(&c.as_bytes());
    }
    out
}

/// Uniforms del pipeline (un único buffer por draw). **POD, repr(C)**.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct CellUniforms {
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub cell_w: f32,
    pub cell_h: f32,
    pub atlas_w: f32,
    pub atlas_h: f32,
    pub _pad0: f32,
    pub _pad1: f32,
}

impl CellUniforms {
    /// 32 B — el bind group binding debe tener `min_binding_size = Some(32)`.
    pub const SIZE: usize = 32;

    pub fn as_bytes(&self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        let fields = [
            self.viewport_w,
            self.viewport_h,
            self.cell_w,
            self.cell_h,
            self.atlas_w,
            self.atlas_h,
            self._pad0,
            self._pad1,
        ];
        for (i, v) in fields.iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// El shader WGSL del pipeline. Vertex stage usa `vertex_index` (0..4) para
/// emitir los corners del quad como TriangleStrip. Fragment samplea el atlas
/// grayscale y combina fg/bg por cobertura.
pub const CELL_WGSL: &str = r#"
struct Uniforms {
    viewport_size: vec2<f32>,
    cell_size: vec2<f32>,
    atlas_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var atlas_tex: texture_2d<f32>;
@group(0) @binding(2) var atlas_samp: sampler;

struct VsIn {
    @builtin(vertex_index) vi: u32,
    @location(0) cell_xy: vec2<f32>,
    @location(1) uv_rect: vec4<f32>,
    @location(2) fg_rgba: u32,
    @location(3) bg_rgba: u32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fg: vec4<f32>,
    @location(2) bg: vec4<f32>,
};

fn unpack_rgba(c: u32) -> vec4<f32> {
    let r = f32(c & 0xFFu) / 255.0;
    let g = f32((c >> 8u) & 0xFFu) / 255.0;
    let b = f32((c >> 16u) & 0xFFu) / 255.0;
    let a = f32((c >> 24u) & 0xFFu) / 255.0;
    return vec4<f32>(r, g, b, a);
}

@vertex
fn vs_cell(in: VsIn) -> VsOut {
    // 4 corners del quad, TriangleStrip: (0,0) (1,0) (0,1) (1,1).
    let corner = vec2<f32>(f32(in.vi & 1u), f32((in.vi >> 1u) & 1u));
    let pixel_pos = in.cell_xy + corner * u.cell_size;
    // px → NDC: x in [-1,1], y in [1,-1] (y invertido para alinear con la
    // convención px-origin-top-left de viewport).
    let ndc = vec2<f32>(
        (pixel_pos.x / u.viewport_size.x) * 2.0 - 1.0,
        1.0 - (pixel_pos.y / u.viewport_size.y) * 2.0,
    );
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    // UV en pixels → UV normalizadas del atlas.
    let uv_px = in.uv_rect.xy + corner * in.uv_rect.zw;
    out.uv = uv_px / u.atlas_size;
    out.fg = unpack_rgba(in.fg_rgba);
    out.bg = unpack_rgba(in.bg_rgba);
    return out;
}

@fragment
fn fs_cell(in: VsOut) -> @location(0) vec4<f32> {
    // Atlas grayscale: la cobertura del glifo está en el canal R (la
    // textura R8Unorm devuelve (R, 0, 0, 1)).
    let cov = textureSample(atlas_tex, atlas_samp, in.uv).r;
    // Mezcla bg → fg por cobertura. Pre-multiplica alpha del fg para
    // que cubrir 100% rinda fg.a (no 1.0).
    let rgb = mix(in.bg.rgb, in.fg.rgb, cov * in.fg.a);
    let a = max(in.bg.a, cov * in.fg.a);
    return vec4<f32>(rgb, a);
}
"#;

use llimphi_hal::wgpu;

/// Pipeline wgpu del cell renderer — compila el shader y arma el bind
/// group layout. Una sola instancia por proceso (o por `color_format`); el
/// `draw` la consume con un atlas + instancias frescas por frame.
///
/// El atlas se sube aparte (su propio `wgpu::Texture` con format
/// `R8Unorm`), y entra al bind group por la `binding=1`. Reusar el mismo
/// atlas entre frames es OK — sólo se actualiza con `Queue::write_texture`
/// cuando aparecen glifos nuevos (el `GlyphAtlas::take_dirty` lo señala).
pub struct CellPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_layout: wgpu::BindGroupLayout,
    pub sampler: wgpu::Sampler,
}

impl CellPipeline {
    /// Compila el shader WGSL y construye el pipeline para escribir al
    /// `color_format` dado (típicamente `Rgba8Unorm`, el de la intermedia
    /// del `WinitSurface`).
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-widget-terminal-cell-shader"),
            source: wgpu::ShaderSource::Wgsl(CELL_WGSL.into()),
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-widget-terminal-cell-bgl"),
            entries: &[
                // 0: uniforms (32 B).
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 1: atlas texture (R8Unorm).
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // 2: sampler.
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-widget-terminal-cell-pl"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });

        let color_targets = [Some(wgpu::ColorTargetState {
            format: color_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        // Instance buffer: 32 B / instancia, 4 attributes (vec2 + vec4 + u32 + u32).
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-widget-terminal-cell-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_cell"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: CellInstance::SIZE as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        // cell_xy
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        // uv_rect (vec4: uv_x, uv_y, uv_w, uv_h)
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 8,
                            shader_location: 1,
                        },
                        // fg_rgba
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Uint32,
                            offset: 24,
                            shader_location: 2,
                        },
                        // bg_rgba
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Uint32,
                            offset: 28,
                            shader_location: 3,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_cell"),
                compilation_options: Default::default(),
                targets: &color_targets,
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("llimphi-widget-terminal-cell-sampler"),
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
            bind_layout,
            sampler,
        }
    }

    /// Helper: crea una textura `R8Unorm` del tamaño del atlas, sube los
    /// bytes y devuelve `(textura, view)`. El caller la mantiene viva
    /// entre frames y la pasa a `draw`. Sólo re-crear si las dimensiones
    /// del atlas cambian (p. ej. tras `GlyphAtlas::grow`).
    pub fn create_atlas_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas_pixels: &[u8],
        atlas_size: (u32, u32),
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let (w, h) = atlas_size;
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-widget-terminal-atlas"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
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
            atlas_pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        (tex, view)
    }

    /// Dibuja las celdas en `target_view`. **No** limpia el target (load:
    /// Load) — el caller decide la pasada previa (vello + selección). El
    /// blending alpha mezcla los glifos sobre lo que ya hay (la
    /// "pre-pasada vello" del SDD).
    pub fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        atlas_view: &wgpu::TextureView,
        cells: &[CellInstance],
        uniforms: CellUniforms,
    ) {
        if cells.is_empty() {
            return;
        }
        // Uniforms.
        let u_bytes = uniforms.as_bytes();
        let u_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-widget-terminal-cell-u"),
            size: CellUniforms::SIZE as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&u_buf, 0, &u_bytes);

        // Instance buffer.
        let inst_bytes = instances_to_bytes(cells);
        let inst_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-widget-terminal-cell-inst"),
            size: inst_bytes.len() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&inst_buf, 0, &inst_bytes);

        // Bind group: uniforms + atlas + sampler.
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-widget-terminal-cell-bg"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: u_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("llimphi-widget-terminal-cell-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, inst_buf.slice(..));
        pass.draw(0..4, 0..cells.len() as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_instance_size_es_32_bytes() {
        // El pipeline asume `array_stride = 32`; un cambio acá rompería el
        // vertex layout silenciosamente. Tener el chequeo en test fija el
        // contrato.
        assert_eq!(CellInstance::SIZE, 32);
        assert_eq!(std::mem::size_of::<CellInstance>(), 32);
    }

    #[test]
    fn cell_uniforms_size_es_32_bytes() {
        assert_eq!(CellUniforms::SIZE, 32);
        assert_eq!(std::mem::size_of::<CellUniforms>(), 32);
    }

    #[test]
    fn as_bytes_de_instance_es_round_trip_de_f32_u32() {
        let c = CellInstance {
            cell_x: 12.5,
            cell_y: 24.0,
            uv_x: 100.0,
            uv_y: 200.0,
            uv_w: 8.0,
            uv_h: 16.0,
            fg_rgba: 0xFF1122EE,
            bg_rgba: 0xAABBCCDD,
        };
        let b = c.as_bytes();
        assert_eq!(b.len(), 32);
        // Re-leemos cada campo del array byte little-endian.
        assert_eq!(f32::from_le_bytes(b[0..4].try_into().unwrap()), 12.5);
        assert_eq!(f32::from_le_bytes(b[4..8].try_into().unwrap()), 24.0);
        assert_eq!(f32::from_le_bytes(b[8..12].try_into().unwrap()), 100.0);
        assert_eq!(f32::from_le_bytes(b[12..16].try_into().unwrap()), 200.0);
        assert_eq!(f32::from_le_bytes(b[16..20].try_into().unwrap()), 8.0);
        assert_eq!(f32::from_le_bytes(b[20..24].try_into().unwrap()), 16.0);
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 0xFF1122EE);
        assert_eq!(u32::from_le_bytes(b[28..32].try_into().unwrap()), 0xAABBCCDD);
    }

    #[test]
    fn pack_rgba_es_little_endian() {
        assert_eq!(pack_rgba(0x11, 0x22, 0x33, 0xFF), 0xFF332211);
        assert_eq!(pack_rgba(0, 0, 0, 0), 0);
        assert_eq!(pack_rgba(255, 255, 255, 255), 0xFFFFFFFF);
    }

    #[test]
    fn instances_to_bytes_concatena_correctamente() {
        let cs = vec![
            CellInstance {
                cell_x: 0.0, cell_y: 0.0, uv_x: 0.0, uv_y: 0.0,
                uv_w: 0.0, uv_h: 0.0, fg_rgba: 0x12345678, bg_rgba: 0,
            },
            CellInstance {
                cell_x: 1.0, cell_y: 2.0, uv_x: 3.0, uv_y: 4.0,
                uv_w: 5.0, uv_h: 6.0, fg_rgba: 0xCAFEBABE, bg_rgba: 0xDEADBEEF,
            },
        ];
        let b = instances_to_bytes(&cs);
        assert_eq!(b.len(), 64);
        // Segunda instancia arranca en byte 32.
        assert_eq!(f32::from_le_bytes(b[32..36].try_into().unwrap()), 1.0);
        assert_eq!(u32::from_le_bytes(b[56..60].try_into().unwrap()), 0xCAFEBABE);
        assert_eq!(u32::from_le_bytes(b[60..64].try_into().unwrap()), 0xDEADBEEF);
    }

    #[test]
    fn uniforms_as_bytes_pone_dims_en_orden() {
        let u = CellUniforms {
            viewport_w: 800.0,
            viewport_h: 600.0,
            cell_w: 8.0,
            cell_h: 16.0,
            atlas_w: 512.0,
            atlas_h: 256.0,
            _pad0: 0.0,
            _pad1: 0.0,
        };
        let b = u.as_bytes();
        assert_eq!(f32::from_le_bytes(b[0..4].try_into().unwrap()), 800.0);
        assert_eq!(f32::from_le_bytes(b[12..16].try_into().unwrap()), 16.0); // cell_h
        assert_eq!(f32::from_le_bytes(b[16..20].try_into().unwrap()), 512.0); // atlas_w
        assert_eq!(f32::from_le_bytes(b[20..24].try_into().unwrap()), 256.0); // atlas_h
    }

    #[test]
    fn wgsl_shader_no_es_vacio_y_define_entry_points() {
        // Smoke check: la string del shader existe y declara las dos
        // entry points que el pipeline va a referenciar. La validación
        // sintáctica WGSL ocurre cuando `device.create_shader_module` la
        // compile en el commit de pipeline.
        assert!(CELL_WGSL.contains("@vertex"));
        assert!(CELL_WGSL.contains("@fragment"));
        assert!(CELL_WGSL.contains("vs_cell"));
        assert!(CELL_WGSL.contains("fs_cell"));
        assert!(CELL_WGSL.len() > 200, "shader sospechosamente corto");
    }
}
