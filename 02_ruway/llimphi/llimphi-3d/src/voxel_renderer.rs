//! `VoxelRenderer` — render por **ray-marching DDA** de un [`VoxelGrid`] denso.
//!
//! No mesha (ruta elegida en `MOTOR-VOXEL.md` §11.1): sube el grid a una textura
//! 3D y dibuja un *fullscreen triangle*; el fragment shader reconstruye un rayo
//! por píxel desde la cámara (con la inversa de `view_proj`) y lo marcha por la
//! grilla con el algoritmo de Amanatides-Woo hasta el primer voxel sólido. Los
//! píxeles que no pegan hacen `discard` → se preserva la UI vello debajo
//! (`LoadOp::Load`).
//!
//! La firma de [`VoxelRenderer::render`] calza con la closure de
//! `View::gpu_paint_with`, igual que `Renderer3d`.

use crate::camera::Camera3d;
use crate::voxel::VoxelGrid;

/// Renderer de voxels por ray-march. Cachea textura 3D, uniform y pipeline.
pub struct VoxelRenderer {
    tex: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    dim: [u32; 3],
}

impl VoxelRenderer {
    /// Crea el renderer y sube `grid` a una textura 3D `Rgba8Unorm`.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
        grid: &VoxelGrid,
    ) -> Self {
        let dim = grid.dim();
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-voxel-tex"),
            size: wgpu::Extent3d {
                width: dim[0],
                height: dim[1],
                depth_or_array_layers: dim[2],
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let tex_view = tex.create_view(&wgpu::TextureViewDescriptor::default());

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-3d-voxel-shader"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-voxel-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-voxel-ubuf"),
            size: 96, // mat4 (64) + cam_eye vec4 (16) + grid_dim vec4 (16)
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-3d-voxel-bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&tex_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-3d-voxel-pl"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-3d-voxel-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
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

        let r = Self {
            tex,
            bind_group,
            pipeline,
            ubuf,
            dim,
        };
        r.upload(queue, grid);
        r
    }

    /// Re-sube el grid a la textura 3D (cimiento de M3: edición/sim en vivo).
    /// El `dim` debe coincidir con el de creación.
    pub fn upload(&self, queue: &wgpu::Queue, grid: &VoxelGrid) {
        let dim = grid.dim();
        debug_assert_eq!(dim, self.dim, "upload: dim != dim de creación");
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            grid.bytes(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(dim[0] * 4),
                rows_per_image: Some(dim[1]),
            },
            wgpu::Extent3d {
                width: dim[0],
                height: dim[1],
                depth_or_array_layers: dim[2],
            },
        );
    }

    /// Ray-marchea la grilla vista desde `camera` sobre `target` (intermedia del
    /// frame). Color `LoadOp::Load` (preserva la UI); los misses hacen `discard`.
    /// La grilla se centra en el origen del mundo (cada voxel mide 1).
    pub fn render(
        &mut self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        (w, h): (u32, u32),
        camera: &Camera3d,
    ) {
        if w == 0 || h == 0 {
            return;
        }
        let inv_vp = camera.view_proj(w as f32 / h as f32).inverse();
        let mut u = Vec::with_capacity(96);
        for v in inv_vp.to_cols_array() {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [camera.eye.x, camera.eye.y, camera.eye.z, 0.0] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [self.dim[0] as f32, self.dim[1] as f32, self.dim[2] as f32, 0.0] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        queue.write_buffer(&self.ubuf, 0, &u);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("llimphi-3d-voxel-pass"),
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

const WGSL: &str = r#"
struct U {
    inv_vp: mat4x4<f32>,
    cam_eye: vec4<f32>,
    grid_dim: vec4<f32>,
};
@group(0) @binding(0) var vox: texture_3d<f32>;
@group(0) @binding(1) var<uniform> u: U;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

// Fullscreen triangle: cubre [-1,1]² con 3 vértices.
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var out: VOut;
    out.clip = vec4<f32>(p[vi], 0.0, 1.0);
    out.ndc = p[vi];
    return out;
}

// Intersección rayo↔AABB. Devuelve (t_near, t_far); miss si near > far.
fn ray_box(ro: vec3<f32>, inv_rd: vec3<f32>, bmin: vec3<f32>, bmax: vec3<f32>) -> vec2<f32> {
    let t0 = (bmin - ro) * inv_rd;
    let t1 = (bmax - ro) * inv_rd;
    let tmin = min(t0, t1);
    let tmax = max(t0, t1);
    let tnear = max(max(tmin.x, tmin.y), tmin.z);
    let tfar = min(min(tmax.x, tmax.y), tmax.z);
    return vec2<f32>(tnear, tfar);
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // Reconstruir el rayo de cámara desde la inversa de view_proj.
    let p_near = u.inv_vp * vec4<f32>(in.ndc, 0.0, 1.0);
    let p_far  = u.inv_vp * vec4<f32>(in.ndc, 1.0, 1.0);
    let ro_world = u.cam_eye.xyz;
    let rd = normalize(p_far.xyz / p_far.w - p_near.xyz / p_near.w);

    let dim = u.grid_dim.xyz;
    // Grilla centrada en el origen: world box [-dim/2, +dim/2] → local [0, dim].
    let ro = ro_world + dim * 0.5;

    // Evitar divisiones por cero en ejes paralelos.
    let safe_rd = vec3<f32>(
        select(rd.x, 1e-6, abs(rd.x) < 1e-6),
        select(rd.y, 1e-6, abs(rd.y) < 1e-6),
        select(rd.z, 1e-6, abs(rd.z) < 1e-6),
    );
    let inv_rd = 1.0 / safe_rd;

    let tb = ray_box(ro, inv_rd, vec3<f32>(0.0), dim);
    if (tb.x > tb.y || tb.y < 0.0) {
        discard;
    }
    let t_enter = max(tb.x, 0.0);

    // Setup DDA (Amanatides-Woo).
    let entry = ro + safe_rd * t_enter;
    var voxel = clamp(floor(entry), vec3<f32>(0.0), dim - vec3<f32>(1.0));
    let step = sign(safe_rd);
    let t_delta = abs(inv_rd);
    // Frontera del próximo cruce por eje.
    let vbound = voxel + max(step, vec3<f32>(0.0));
    var t_max = (vbound - ro) * inv_rd;

    // Normal de entrada (cara del AABB por la que entró el rayo).
    var normal = vec3<f32>(0.0);
    if (t_enter == tb.x) {
        let t0 = (vec3<f32>(0.0) - ro) * inv_rd;
        let t1 = (dim - ro) * inv_rd;
        let tmin = min(t0, t1);
        if (tb.x == tmin.x) { normal = vec3<f32>(-step.x, 0.0, 0.0); }
        else if (tb.x == tmin.y) { normal = vec3<f32>(0.0, -step.y, 0.0); }
        else { normal = vec3<f32>(0.0, 0.0, -step.z); }
    }

    var hit = false;
    var color = vec3<f32>(0.0);
    let max_steps = i32(dim.x + dim.y + dim.z) + 3;
    for (var i = 0; i < max_steps; i = i + 1) {
        let c = textureLoad(vox, vec3<i32>(voxel), 0);
        if (c.a > 0.5) {
            hit = true;
            color = c.rgb;
            break;
        }
        // Avanzar al voxel vecino por el eje de menor t_max.
        if (t_max.x < t_max.y && t_max.x < t_max.z) {
            voxel.x = voxel.x + step.x;
            t_max.x = t_max.x + t_delta.x;
            normal = vec3<f32>(-step.x, 0.0, 0.0);
        } else if (t_max.y < t_max.z) {
            voxel.y = voxel.y + step.y;
            t_max.y = t_max.y + t_delta.y;
            normal = vec3<f32>(0.0, -step.y, 0.0);
        } else {
            voxel.z = voxel.z + step.z;
            t_max.z = t_max.z + t_delta.z;
            normal = vec3<f32>(0.0, 0.0, -step.z);
        }
        if (any(voxel < vec3<f32>(0.0)) || any(voxel >= dim)) {
            break;
        }
    }

    if (!hit) {
        discard;
    }

    // Sombreado: difuso direccional + ambiente.
    let ldir = normalize(vec3<f32>(0.45, 0.85, 0.3));
    let diff = max(dot(normal, ldir), 0.0);
    let shade = 0.35 + 0.65 * diff;
    return vec4<f32>(color * shade, 1.0);
}
"#;
