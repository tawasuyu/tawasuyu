//! `Lines3d` — líneas 3D como `LineList`, depth-test pero **sin escribir**
//! profundidad, con alpha-blend y un atenuado por profundidad opcional para dar
//! volumen (el frente brilla, el fondo se apaga). Llena el hueco del crate, que
//! sólo tenía triángulos: sirve para anillos (círculos máximos), rejillas,
//! ejes y figuras de constelaciones — cualquier "alambre" 3D.
//!
//! Se dibuja en un pase ya abierto con depth attachment (el de
//! [`PostFx::scene_pass`](crate::PostFx) o [`Scene3d`](crate::Scene3d)), DESPUÉS
//! de la geometría sólida que deba ocluirlo:
//!
//! ```ignore
//! lines.set_lines(device, &verts);     // pares de vértices = segmentos
//! lines.upload(queue, view_proj);
//! lines.draw(&mut pass);
//! ```

use glam::Mat4;

use crate::scene::DEPTH_FORMAT;

/// Vértice de línea: posición en mundo + color RGBA lineal.
#[derive(Debug, Clone, Copy)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

impl LineVertex {
    /// `pos.xyz (12) + color.rgba (16)`.
    pub const STRIDE: usize = 7 * 4;

    fn write_to(&self, out: &mut Vec<u8>) {
        for v in self.pos {
            out.extend_from_slice(&v.to_ne_bytes());
        }
        for v in self.color {
            out.extend_from_slice(&v.to_ne_bytes());
        }
    }
}

/// Renderer de líneas 3D reutilizable. Sin vértices, [`Self::draw`] es no-op.
pub struct Lines3d {
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    uniform_bg: wgpu::BindGroup,
    verts: Option<wgpu::Buffer>,
    count: u32,
}

impl Lines3d {
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-lines-uniform-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-3d-lines-shader"),
            source: wgpu::ShaderSource::Wgsl(LINES_WGSL.into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-3d-lines-pl"),
            bind_group_layouts: &[&uniform_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-3d-lines-pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: LineVertex::STRIDE as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 12,
                            shader_location: 1,
                        },
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-lines-uniform"),
            size: 64, // view_proj mat4
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-3d-lines-uniform-bg"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        Self {
            pipeline,
            uniform_buf,
            uniform_bg,
            verts: None,
            count: 0,
        }
    }

    /// Reemplaza los vértices. Cada **par** consecutivo es un segmento; para una
    /// polilínea, repetir el vértice interior (a,b, b,c, c,d…).
    pub fn set_lines(&mut self, device: &wgpu::Device, verts: &[LineVertex]) {
        if verts.is_empty() {
            self.count = 0;
            self.verts = None;
            return;
        }
        let mut bytes = Vec::with_capacity(verts.len() * LineVertex::STRIDE);
        for v in verts {
            v.write_to(&mut bytes);
        }
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-lines-vbuf"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        buf.slice(..).get_mapped_range_mut().copy_from_slice(&bytes);
        buf.unmap();
        self.verts = Some(buf);
        self.count = verts.len() as u32;
    }

    /// Sube la `view_proj` del frame. Llamar antes de [`Self::draw`].
    pub fn upload(&self, queue: &wgpu::Queue, view_proj: Mat4) {
        let mut b = Vec::with_capacity(64);
        for v in view_proj.to_cols_array() {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        queue.write_buffer(&self.uniform_buf, 0, &b);
    }

    /// Dibuja en un pase ya abierto (con depth attachment). No-op sin vértices.
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let Some(vb) = self.verts.as_ref() else {
            return;
        };
        if self.count < 2 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bg, &[]);
        pass.set_vertex_buffer(0, vb.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}

/// Atenúa el alpha por profundidad NDC (el fondo de la esfera se apaga → da
/// volumen). El color va tal cual; sólo el alpha se modula.
const LINES_WGSL: &str = r#"
struct U { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) ndc_z: f32,
};

@vertex
fn vs(in: VIn) -> VOut {
    var o: VOut;
    o.clip = u.view_proj * vec4<f32>(in.pos, 1.0);
    o.color = in.color;
    o.ndc_z = o.clip.z / max(o.clip.w, 1e-6);
    return o;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // ndc_z ~0 = cerca, ~1 = lejos. El fondo se apaga hasta 0.35.
    let depth_fade = mix(1.0, 0.35, clamp(in.ndc_z, 0.0, 1.0));
    return vec4<f32>(in.color.rgb, in.color.a * depth_fade);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_wgsl_valida() {
        let module = naga::front::wgsl::parse_str(LINES_WGSL).expect("LINES_WGSL no parsea");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("LINES_WGSL no valida");
    }
}
