//! `Renderer3d` — pipeline wgpu mínimo que dibuja geometría 3D indexada con
//! test de profundidad sobre la textura intermedia del frame de Llimphi.
//!
//! La firma de [`Renderer3d::render`] es la que pide la closure de
//! `View::gpu_paint_with` (`device, queue, encoder, target_view, (w, h)`), más
//! la cámara — así un nodo 3D entra en el árbol `View<Msg>` sin tocar el
//! runtime. Mantiene su **propio depth buffer** (recreado al cambiar de
//! tamaño); el color se compone con `LoadOp::Load` para preservar la UI vello
//! que ya está debajo.

use glam::Mat4;

use crate::camera::Camera3d;
use crate::mesh::{cube, Vertex3d};
use crate::scene::{ensure_depth, DepthBuffer, DEPTH_FORMAT};
use crate::voxel_renderer::PointLight;

/// Tope de luces puntuales por frame en el forward-mesh (caben en el uniform).
/// El caller puede agregar más a [`Renderer3d::lights`]; las que excedan se
/// ignoran (las primeras `MESH_MAX_LIGHTS`).
pub const MESH_MAX_LIGHTS: usize = 8;

/// Tamaño del uniform en bytes: `view_proj`(64) + `model`(64) + `eye_nlights`(16)
/// + `ambient`(16) + `lpos`(MAX·16) + `lcol`(MAX·16).
const UNIFORM_SIZE: usize = 64 + 64 + 16 + 16 + MESH_MAX_LIGHTS * 16 * 2;

/// Renderer de **mallas** indexadas (por defecto un cubo) visto desde una
/// [`Camera3d`]. Cachea pipeline, buffers de geometría, uniform y (para el
/// camino standalone) un depth propio. En [`Scene3d`](crate::Scene3d) comparte
/// el depth con el pase de voxels para ocluirse mutuamente.
///
/// `model` ubica la malla en el mundo (default identidad): `mvp = view_proj ·
/// model`, así una misma malla se instancia/posiciona sin reconstruir buffers.
pub struct Renderer3d {
    pipeline: wgpu::RenderPipeline,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
    ubuf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    model: Mat4,
    depth: Option<DepthBuffer>,
    /// Luces puntuales coloreadas que iluminan la malla (≤ [`MESH_MAX_LIGHTS`];
    /// las que sobren se ignoran). `pos`/`range` en **coordenadas de mundo** (no
    /// de voxel: ojo con el doc de [`PointLight`], que está redactado para el
    /// ray-marcher); `range` = radio de caída, `radius` se ignora (no hay
    /// sombras en el forward-mesh). Editable antes de `upload`/`render`. Vacío =
    /// sin luces → sólo el término ambiente.
    pub lights: Vec<PointLight>,
    /// Luz ambiente RGB que multiplica el color base donde no llega ninguna luz.
    /// Default `[1, 1, 1]`: sin luces, el render es **idéntico** al plano de
    /// antes (color base tal cual). Para que las luces se noten, bajalo
    /// (p.ej. `[0.12, 0.12, 0.14]`).
    pub ambient: [f32; 3],
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
                // El fragment también lee el uniform (luces + ambiente + eye).
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
            size: UNIFORM_SIZE as u64,
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
            model: Mat4::IDENTITY,
            depth: None,
            lights: Vec::new(),
            ambient: [1.0, 1.0, 1.0],
        }
    }

    /// Ubica la malla en el mundo (`mvp = view_proj · model`). Default identidad.
    pub fn set_model(&mut self, model: Mat4) {
        self.model = model;
    }

    /// Reemplaza la geometría (recrea los buffers de vértices/índices). Pensado
    /// para mallas que cambian cada frame — p.ej. un **muñeco articulado** cuya
    /// pose se rehornea en CPU (limbos rotados) y se vuelve a subir. Las mallas
    /// son chicas (decenas-cientos de vértices), así que recrear los buffers por
    /// frame es despreciable; el pipeline/uniform/bind-group se conservan.
    pub fn set_geometry(&mut self, device: &wgpu::Device, verts: &[Vertex3d], indices: &[u16]) {
        let mut vbytes = Vec::with_capacity(verts.len() * Vertex3d::SIZE);
        for v in verts {
            v.write_to(&mut vbytes);
        }
        self.vbuf =
            create_buffer_init(device, "llimphi-3d-vbuf", wgpu::BufferUsages::VERTEX, &vbytes);

        let mut ibytes = Vec::with_capacity(indices.len() * 2);
        for &i in indices {
            ibytes.extend_from_slice(&i.to_ne_bytes());
        }
        self.ibuf =
            create_buffer_init(device, "llimphi-3d-ibuf", wgpu::BufferUsages::INDEX, &ibytes);
        self.index_count = indices.len() as u32;
    }

    /// Sube el uniform del frame: `view_proj` y `model` por separado (el shader
    /// necesita la posición de mundo `model · pos` para iluminar), `eye`, número
    /// de luces, ambiente y los arrays de luces. Lo llama [`Self::render`] y
    /// [`Scene3d`](crate::Scene3d). `aspect` = w/h. glam es column-major, igual
    /// que WGSL → se sube tal cual.
    pub fn upload(&self, queue: &wgpu::Queue, aspect: f32, camera: &Camera3d) {
        let view_proj = camera.view_proj(aspect);
        let mut b = Vec::with_capacity(UNIFORM_SIZE);
        for v in view_proj.to_cols_array() {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        for v in self.model.to_cols_array() {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        let n = self.lights.len().min(MESH_MAX_LIGHTS);
        for v in [camera.eye.x, camera.eye.y, camera.eye.z, n as f32] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [self.ambient[0], self.ambient[1], self.ambient[2], 0.0] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        // Posiciones (xyz) + range (w).
        for i in 0..MESH_MAX_LIGHTS {
            let l = self.lights.get(i).copied();
            let (p, r) = l.map_or(([0.0; 3], 1.0), |l| (l.pos, l.range));
            for v in [p[0], p[1], p[2], r] {
                b.extend_from_slice(&v.to_ne_bytes());
            }
        }
        // Colores (rgb) + relleno.
        for i in 0..MESH_MAX_LIGHTS {
            let c = self.lights.get(i).copied().map_or([0.0; 3], |l| l.color);
            for v in [c[0], c[1], c[2], 0.0] {
                b.extend_from_slice(&v.to_ne_bytes());
            }
        }
        debug_assert_eq!(b.len(), UNIFORM_SIZE);
        queue.write_buffer(&self.ubuf, 0, &b);
    }

    /// Dibuja la malla indexada en un pase **ya abierto** (color + depth). Lo usa
    /// [`Scene3d`](crate::Scene3d) para compartir el pase con los voxels.
    /// Requiere `upload` previo en el mismo frame.
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }

    /// Dibuja la malla sola sobre `target` (camino standalone, depth propio).
    /// Firma compatible con `View::gpu_paint_with`; color preservado
    /// (`LoadOp::Load`), depth propio limpiado cada frame.
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
        self.upload(queue, w as f32 / h as f32, camera);
        ensure_depth(&mut self.depth, device, w, h);
        let depth_view = &self.depth.as_ref().unwrap().view;

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
        self.draw(&mut pass);
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
const MAXL: u32 = 8u;
struct Uniforms {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    eye_n: vec4<f32>,      // xyz = ojo en mundo, w = nº de luces
    ambient: vec4<f32>,    // rgb = luz ambiente
    lpos: array<vec4<f32>, MAXL>,  // xyz = posición mundo, w = range (radio caída)
    lcol: array<vec4<f32>, MAXL>,  // rgb = color de la luz
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec3<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) world: vec3<f32>,
};

@vertex
fn vs(in: VIn) -> VOut {
    var out: VOut;
    let world = (u.model * vec4<f32>(in.pos, 1.0)).xyz;
    out.clip = u.view_proj * vec4<f32>(world, 1.0);
    out.color = in.color;
    out.world = world;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // Normal geométrica plana (faceteada) por derivadas screen-space: no exige
    // normales por vértice y sirve para cualquier malla (cubos, muñecos…).
    let dx = dpdx(in.world);
    let dy = dpdy(in.world);
    let ncross = cross(dx, dy);
    let nlen = length(ncross);
    if (nlen < 1e-8) {
        // Triángulo degenerado / visto de canto: sólo ambiente.
        return vec4<f32>(in.color * u.ambient.rgb, 1.0);
    }
    var n = ncross / nlen;
    // Orientar la normal hacia la cámara (independiente del winding/signo de las
    // derivadas) para que la cara visible siempre se ilumine bien.
    let view_dir = u.eye_n.xyz - in.world;
    if (dot(n, view_dir) < 0.0) {
        n = -n;
    }

    var lit = u.ambient.rgb;
    let count = u32(u.eye_n.w);
    for (var i: u32 = 0u; i < count; i = i + 1u) {
        let lp = u.lpos[i];
        let d = lp.xyz - in.world;
        let dist = length(d);
        let range = max(lp.w, 1e-3);
        // Caída suave: (1 - d/range)^2, recortada a 0 fuera del radio.
        let a = clamp(1.0 - dist / range, 0.0, 1.0);
        let atten = a * a;
        let ndotl = max(dot(n, d / max(dist, 1e-4)), 0.0);
        lit = lit + u.lcol[i].rgb * (ndotl * atten);
    }
    return vec4<f32>(in.color * lit, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// El shader del forward-mesh (con luces + derivadas) parsea y valida sin
    /// GPU. Ataja errores que sólo saldrían al crear el pipeline en runtime.
    #[test]
    fn shader_wgsl_valida() {
        let module = naga::front::wgsl::parse_str(WGSL).expect("WGSL no parsea");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("WGSL no valida");
    }

    /// El uniform empaquetado mide exactamente lo que declara el struct WGSL.
    #[test]
    fn uniform_size_coincide() {
        assert_eq!(UNIFORM_SIZE, 64 + 64 + 16 + 16 + 8 * 16 * 2);
        assert_eq!(UNIFORM_SIZE % 16, 0);
    }
}
