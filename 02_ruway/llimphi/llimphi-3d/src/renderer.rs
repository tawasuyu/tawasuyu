//! `Renderer3d` — pipeline wgpu mínimo que dibuja geometría 3D indexada con
//! test de profundidad sobre la textura intermedia del frame de Llimphi.
//!
//! La firma de [`Renderer3d::render`] es la que pide la closure de
//! `View::gpu_paint_with` (`device, queue, encoder, target_view, (w, h)`), más
//! la cámara — así un nodo 3D entra en el árbol `View<Msg>` sin tocar el
//! runtime. Mantiene su **propio depth buffer** (recreado al cambiar de
//! tamaño); el color se compone con `LoadOp::Load` para preservar la UI vello
//! que ya está debajo.

use crate::camera::Camera3d;
use crate::mesh::{cube, Vertex3d};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Depth attachment cacheado, recreado cuando cambia el tamaño del viewport.
struct DepthTex {
    view: wgpu::TextureView,
    w: u32,
    h: u32,
}

/// Renderer 3D de M0: dibuja un mesh indexado (por defecto un cubo) visto desde
/// una [`Camera3d`]. Cachea pipeline, buffers de geometría, uniform y depth.
pub struct Renderer3d {
    pipeline: wgpu::RenderPipeline,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
    ubuf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    depth: Option<DepthTex>,
}

impl Renderer3d {
    /// Crea el renderer para un `color_format` dado (el de la textura
    /// intermedia del frame — `Rgba8Unorm` en headless, el de la surface en
    /// vivo). Arranca con el cubo de prueba cargado.
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let (verts, indices) = cube();
        Self::with_mesh(device, color_format, &verts, &indices)
    }

    /// Igual que [`Self::new`] pero con una malla arbitraria.
    pub fn with_mesh(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        verts: &[Vertex3d],
        indices: &[u16],
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-3d-shader"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-bgl"),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-3d-pl"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-3d-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: Vertex3d::SIZE as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // M0 sin cull: el depth test ya resuelve la oclusión y nos
                // ahorra bugs de winding al sumar mallas. El cull entra en M1+.
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    // Opaco: el cubo reemplaza el fondo vello donde lo cubre.
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        // Geometría → buffers (idiom `to_ne_bytes`, sin bytemuck).
        let mut vbytes = Vec::with_capacity(verts.len() * Vertex3d::SIZE);
        for v in verts {
            v.write_to(&mut vbytes);
        }
        let vbuf = create_buffer_init(device, "llimphi-3d-vbuf", wgpu::BufferUsages::VERTEX, &vbytes);

        let mut ibytes = Vec::with_capacity(indices.len() * 2);
        for &i in indices {
            ibytes.extend_from_slice(&i.to_ne_bytes());
        }
        let ibuf = create_buffer_init(device, "llimphi-3d-ibuf", wgpu::BufferUsages::INDEX, &ibytes);

        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-ubuf"),
            size: 64, // una mat4x4<f32>
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-3d-bg"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ubuf.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            vbuf,
            ibuf,
            index_count: indices.len() as u32,
            ubuf,
            bind_group,
            depth: None,
        }
    }

    /// Dibuja la escena 3D sobre `target` (la textura intermedia del frame).
    /// Firma compatible con la closure de `View::gpu_paint_with`. El color se
    /// preserva (`LoadOp::Load`); el depth es propio y se limpia cada frame.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        (w, h): (u32, u32),
        camera: &Camera3d,
    ) {
        if w == 0 || h == 0 {
            return;
        }
        self.ensure_depth(device, w, h);
        let depth_view = &self.depth.as_ref().unwrap().view;

        let mvp = camera.view_proj(w as f32 / h as f32);
        // glam es column-major; el shader WGSL espera column-major → upload tal cual.
        let cols = mvp.to_cols_array();
        let mut ubytes = Vec::with_capacity(64);
        for v in cols {
            ubytes.extend_from_slice(&v.to_ne_bytes());
        }
        queue.write_buffer(&self.ubuf, 0, &ubytes);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("llimphi-3d-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }

    fn ensure_depth(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if matches!(&self.depth, Some(d) if d.w == w && d.h == h) {
            return;
        }
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-depth"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.depth = Some(DepthTex { view, w, h });
    }
}

/// Crea un buffer ya inicializado con `data` (sin `wgpu::util::DeviceExt`, para
/// no arrastrar la feature `util`): `mapped_at_creation` + copia + `unmap`.
fn create_buffer_init(
    device: &wgpu::Device,
    label: &str,
    usage: wgpu::BufferUsages,
    data: &[u8],
) -> wgpu::Buffer {
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: data.len() as u64,
        usage,
        mapped_at_creation: true,
    });
    buf.slice(..).get_mapped_range_mut().copy_from_slice(data);
    buf.unmap();
    buf
}

const WGSL: &str = r#"
struct Uniforms { mvp: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec3<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs(in: VIn) -> VOut {
    var out: VOut;
    out.clip = u.mvp * vec4<f32>(in.pos, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
"#;
