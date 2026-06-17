//! `VoxelRenderer` — render por **ray-marching** de un [`VoxelGrid`].
//!
//! No mesha (ruta elegida en `MOTOR-VOXEL.md` §11.1): sube el grid a una textura
//! 3D y dibuja un *fullscreen triangle*; el fragment shader reconstruye un rayo
//! por píxel y lo marcha por la grilla.
//!
//! **M2** sobre M1:
//! - **Empty-space skipping (sparse)**: un mapa de ocupación grueso por *bricks*
//!   (`BRICK³` voxels) en una segunda textura 3D. El rayo hace un **DDA de dos
//!   niveles**: marcha la grilla gruesa y salta los bricks vacíos enteros en un
//!   paso; sólo baja al DDA fino dentro de los bricks que contienen algo.
//! - **AO** suave por esquinas (estilo voxel, interpolado en la cara del hit).
//! - **Sol direccional con sombra dura** (un segundo rayo hacia la luz, misma
//!   traversal de dos niveles).
//!
//! Los píxeles que no pegan hacen `discard` → preserva la UI vello
//! (`LoadOp::Load`). La firma de [`VoxelRenderer::render`] calza con la closure
//! de `View::gpu_paint_with`.

use crate::camera::Camera3d;
use crate::voxel::VoxelGrid;

/// Tamaño de brick (voxels por lado) para el mapa de ocupación grueso.
const BRICK: u32 = 8;

/// Renderer de voxels por ray-march de dos niveles. Cachea ambas texturas 3D
/// (fina + gruesa), uniform y pipeline.
pub struct VoxelRenderer {
    tex: wgpu::Texture,
    coarse: wgpu::Texture,
    coarse_dim: [u32; 3],
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    dim: [u32; 3],
    /// Dirección hacia el sol (normalizada). Editable antes de `render`.
    pub sun_dir: [f32; 3],
}

impl VoxelRenderer {
    /// Crea el renderer y sube `grid` a las texturas 3D (fina + gruesa).
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
        grid: &VoxelGrid,
    ) -> Self {
        let dim = grid.dim();
        let (coarse_dim, _) = grid.coarse_occupancy(BRICK);

        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-voxel-tex"),
            size: extent(dim),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let tex_view = tex.create_view(&wgpu::TextureViewDescriptor::default());

        let coarse = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-voxel-coarse"),
            size: extent(coarse_dim),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let coarse_view = coarse.create_view(&wgpu::TextureViewDescriptor::default());

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-3d-voxel-shader"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });

        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D3,
                multisampled: false,
            },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-3d-voxel-bgl"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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
            size: 112, // mat4(64) + cam_eye(16) + grid_dim+brick(16) + sun(16)
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
                    resource: wgpu::BindingResource::TextureView(&coarse_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
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
            coarse,
            coarse_dim,
            bind_group,
            pipeline,
            ubuf,
            dim,
            sun_dir: normalize3([0.5, 1.0, 0.35]),
        };
        r.upload(queue, grid);
        r
    }

    /// Re-sube el grid ENTERO a ambas texturas (upload inicial / reset).
    /// El `dim` debe coincidir con el de creación.
    pub fn upload(&self, queue: &wgpu::Queue, grid: &VoxelGrid) {
        let dim = grid.dim();
        debug_assert_eq!(dim, self.dim, "upload: dim != dim de creación");
        write_3d(queue, &self.tex, [0; 3], dim, 4, grid.bytes());
        let (cdim, cbytes) = grid.coarse_occupancy(BRICK);
        debug_assert_eq!(cdim, self.coarse_dim);
        write_3d(queue, &self.coarse, [0; 3], cdim, 1, &cbytes);
    }

    /// **Actualización incremental (M3).** Sube SÓLO la región mutada desde el
    /// último sync: la sub-caja fina cambiada + los bricks gruesos que toca. Si
    /// no hubo cambios, no hace nada. Devuelve los bytes subidos (0 si nada) —
    /// el reemplazo barato del re-mesh: mutar un voxel = subir un puñado de
    /// bytes, no regenerar geometría ni el grid entero.
    ///
    /// Nota: la región pendiente es una AABB que UNE todos los cambios desde el
    /// último sync. Para ediciones localizadas (un pincel, un bloque que cae)
    /// la caja es chica; para ediciones dispersas conviene sincronizar seguido
    /// (un sync por lote localizado) para no agrandar la caja.
    pub fn sync(&self, queue: &wgpu::Queue, grid: &mut VoxelGrid) -> u32 {
        let Some(r) = grid.take_dirty() else {
            return 0;
        };
        let origin = [r[0], r[1], r[2]];
        let ext = [r[3] - r[0] + 1, r[4] - r[1] + 1, r[5] - r[2] + 1];
        let fine = grid.extract_fine(origin, ext);
        write_3d(queue, &self.tex, origin, ext, 4, &fine);

        // Bricks gruesos tocados por la caja.
        let cmin = [r[0] / BRICK, r[1] / BRICK, r[2] / BRICK];
        let cmax = [r[3] / BRICK, r[4] / BRICK, r[5] / BRICK];
        let cext = [
            cmax[0] - cmin[0] + 1,
            cmax[1] - cmin[1] + 1,
            cmax[2] - cmin[2] + 1,
        ];
        let cbytes = grid.coarse_region(BRICK, cmin, cext);
        write_3d(queue, &self.coarse, cmin, cext, 1, &cbytes);

        fine.len() as u32 + cbytes.len() as u32
    }

    /// Ray-marchea la grilla vista desde `camera` sobre `target` (intermedia del
    /// frame). Color `LoadOp::Load`; misses por `discard`. Grilla centrada en el
    /// origen del mundo (cada voxel mide 1).
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
        let mut u = Vec::with_capacity(112);
        for v in inv_vp.to_cols_array() {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [camera.eye.x, camera.eye.y, camera.eye.z, 0.0] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [
            self.dim[0] as f32,
            self.dim[1] as f32,
            self.dim[2] as f32,
            BRICK as f32,
        ] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        let s = normalize3(self.sun_dir);
        for v in [s[0], s[1], s[2], 0.0] {
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

fn extent(dim: [u32; 3]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: dim[0],
        height: dim[1],
        depth_or_array_layers: dim[2],
    }
}

fn write_3d(
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
    origin: [u32; 3],
    ext: [u32; 3],
    bpp: u32,
    data: &[u8],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: origin[0],
                y: origin[1],
                z: origin[2],
            },
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(ext[0] * bpp),
            rows_per_image: Some(ext[1]),
        },
        extent(ext),
    );
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / l, v[1] / l, v[2] / l]
}

const WGSL: &str = r#"
struct U {
    inv_vp: mat4x4<f32>,
    cam_eye: vec4<f32>,
    grid_dim: vec4<f32>,   // xyz = dim fino, w = brick size
    sun_dir: vec4<f32>,    // xyz = dirección hacia el sol (normalizada)
};
@group(0) @binding(0) var vox: texture_3d<f32>;
@group(0) @binding(1) var coarse: texture_3d<f32>;
@group(0) @binding(2) var<uniform> u: U;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

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

fn ray_box(ro: vec3<f32>, inv_rd: vec3<f32>, bmin: vec3<f32>, bmax: vec3<f32>) -> vec2<f32> {
    let t0 = (bmin - ro) * inv_rd;
    let t1 = (bmax - ro) * inv_rd;
    let tmin = min(t0, t1);
    let tmax = max(t0, t1);
    return vec2<f32>(max(max(tmin.x, tmin.y), tmin.z), min(min(tmax.x, tmax.y), tmax.z));
}

struct Hit {
    hit: bool,
    vox: vec3<f32>,
    normal: vec3<f32>,
    t: f32,
};

// DDA de dos niveles: marcha la grilla gruesa, baja a la fina sólo en bricks
// ocupados. `dim` = dim fino, `B` = brick size.
fn trace(ro: vec3<f32>, rd_in: vec3<f32>, dim: vec3<f32>, B: f32) -> Hit {
    var h: Hit;
    h.hit = false;

    let safe_rd = vec3<f32>(
        select(rd_in.x, 1e-6, abs(rd_in.x) < 1e-6),
        select(rd_in.y, 1e-6, abs(rd_in.y) < 1e-6),
        select(rd_in.z, 1e-6, abs(rd_in.z) < 1e-6),
    );
    let inv_rd = 1.0 / safe_rd;
    let step = sign(safe_rd);

    let tb = ray_box(ro, inv_rd, vec3<f32>(0.0), dim);
    if (tb.x > tb.y || tb.y < 0.0) { return h; }
    let t_enter = max(tb.x, 0.0);

    // Normal de entrada al AABB (cara por la que entró).
    let te0 = (vec3<f32>(0.0) - ro) * inv_rd;
    let te1 = (dim - ro) * inv_rd;
    let temin = min(te0, te1);
    var box_n = vec3<f32>(0.0, 0.0, -step.z);
    if (tb.x == temin.x) { box_n = vec3<f32>(-step.x, 0.0, 0.0); }
    else if (tb.x == temin.y) { box_n = vec3<f32>(0.0, -step.y, 0.0); }

    let cdim = ceil(dim / B);
    let p_enter = ro + safe_rd * t_enter;
    var cc = clamp(floor(p_enter / B), vec3<f32>(0.0), cdim - 1.0);
    let t_delta_c = abs(B * inv_rd);
    var t_max_c = ((cc + max(step, vec3<f32>(0.0))) * B - ro) * inv_rd;
    var t_cell = t_enter;
    var cnorm = box_n;

    let max_coarse = i32(cdim.x + cdim.y + cdim.z) + 3;
    for (var ci = 0; ci < max_coarse; ci = ci + 1) {
        let occ = textureLoad(coarse, vec3<i32>(cc), 0).r;
        if (occ > 0.5) {
            // DDA fino dentro del brick `cc`.
            var voxel = clamp(floor(ro + safe_rd * (t_cell + 1e-4)), vec3<f32>(0.0), dim - 1.0);
            var t_max_f = ((voxel + max(step, vec3<f32>(0.0))) - ro) * inv_rd;
            let t_delta_f = abs(inv_rd);
            var fnorm = cnorm;
            var t_vox = t_cell;
            let max_fine = i32(B) * 3 + 3;
            for (var fi = 0; fi < max_fine; fi = fi + 1) {
                if (any(voxel < vec3<f32>(0.0)) || any(voxel >= dim)) { return h; }
                if (any(floor(voxel / B) != cc)) { break; }
                let c = textureLoad(vox, vec3<i32>(voxel), 0);
                if (c.a > 0.5) {
                    h.hit = true;
                    h.vox = voxel;
                    h.normal = fnorm;
                    h.t = t_vox;
                    return h;
                }
                if (t_max_f.x < t_max_f.y && t_max_f.x < t_max_f.z) {
                    voxel.x = voxel.x + step.x;
                    t_vox = t_max_f.x;
                    t_max_f.x = t_max_f.x + t_delta_f.x;
                    fnorm = vec3<f32>(-step.x, 0.0, 0.0);
                } else if (t_max_f.y < t_max_f.z) {
                    voxel.y = voxel.y + step.y;
                    t_vox = t_max_f.y;
                    t_max_f.y = t_max_f.y + t_delta_f.y;
                    fnorm = vec3<f32>(0.0, -step.y, 0.0);
                } else {
                    voxel.z = voxel.z + step.z;
                    t_vox = t_max_f.z;
                    t_max_f.z = t_max_f.z + t_delta_f.z;
                    fnorm = vec3<f32>(0.0, 0.0, -step.z);
                }
            }
        }
        // Avanzar la grilla gruesa (salta el brick vacío entero).
        if (t_max_c.x < t_max_c.y && t_max_c.x < t_max_c.z) {
            cc.x = cc.x + step.x;
            t_cell = t_max_c.x;
            t_max_c.x = t_max_c.x + t_delta_c.x;
            cnorm = vec3<f32>(-step.x, 0.0, 0.0);
        } else if (t_max_c.y < t_max_c.z) {
            cc.y = cc.y + step.y;
            t_cell = t_max_c.y;
            t_max_c.y = t_max_c.y + t_delta_c.y;
            cnorm = vec3<f32>(0.0, -step.y, 0.0);
        } else {
            cc.z = cc.z + step.z;
            t_cell = t_max_c.z;
            t_max_c.z = t_max_c.z + t_delta_c.z;
            cnorm = vec3<f32>(0.0, 0.0, -step.z);
        }
        if (any(cc < vec3<f32>(0.0)) || any(cc >= cdim)) { return h; }
    }
    return h;
}

fn occ_at(p: vec3<i32>, dim: vec3<f32>) -> f32 {
    if (any(p < vec3<i32>(0)) || any(vec3<f32>(p) >= dim)) { return 0.0; }
    return select(0.0, 1.0, textureLoad(vox, p, 0).a > 0.5);
}

fn vertex_ao(s1: f32, s2: f32, c: f32) -> f32 {
    if (s1 > 0.5 && s2 > 0.5) { return 0.0; }
    return (3.0 - (s1 + s2 + c)) / 3.0;
}

// AO suave estilo voxel: 4 esquinas de la cara golpeada, interpoladas por la
// posición sub-voxel del hit.
fn compute_ao(voxel: vec3<f32>, normal: vec3<f32>, p: vec3<f32>, dim: vec3<f32>) -> f32 {
    var t1 = vec3<i32>(0, 1, 0);
    var t2 = vec3<i32>(0, 0, 1);
    if (abs(normal.y) > 0.5) { t1 = vec3<i32>(1, 0, 0); t2 = vec3<i32>(0, 0, 1); }
    else if (abs(normal.z) > 0.5) { t1 = vec3<i32>(1, 0, 0); t2 = vec3<i32>(0, 1, 0); }

    let base = vec3<i32>(voxel) + vec3<i32>(normal);
    let s1m = occ_at(base - t1, dim);
    let s1p = occ_at(base + t1, dim);
    let s2m = occ_at(base - t2, dim);
    let s2p = occ_at(base + t2, dim);
    let ao_mm = vertex_ao(s1m, s2m, occ_at(base - t1 - t2, dim));
    let ao_pm = vertex_ao(s1p, s2m, occ_at(base + t1 - t2, dim));
    let ao_mp = vertex_ao(s1m, s2p, occ_at(base - t1 + t2, dim));
    let ao_pp = vertex_ao(s1p, s2p, occ_at(base + t1 + t2, dim));

    let fp = fract(p);
    let uu = dot(vec3<f32>(t1), fp);
    let vv = dot(vec3<f32>(t2), fp);
    return mix(mix(ao_mm, ao_pm, uu), mix(ao_mp, ao_pp, uu), vv);
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let p_near = u.inv_vp * vec4<f32>(in.ndc, 0.0, 1.0);
    let p_far  = u.inv_vp * vec4<f32>(in.ndc, 1.0, 1.0);
    let ro_world = u.cam_eye.xyz;
    let rd = normalize(p_far.xyz / p_far.w - p_near.xyz / p_near.w);

    let dim = u.grid_dim.xyz;
    let B = u.grid_dim.w;
    let ro = ro_world + dim * 0.5;   // centrar la grilla en el origen del mundo

    let h = trace(ro, rd, dim, B);
    if (!h.hit) { discard; }

    let albedo = textureLoad(vox, vec3<i32>(h.vox), 0).rgb;
    let p = ro + rd * h.t;
    let ao = compute_ao(h.vox, h.normal, p, dim);

    let ldir = u.sun_dir.xyz;
    let diff = max(dot(h.normal, ldir), 0.0);

    // Sombra dura: segundo rayo desde la cara hacia el sol.
    let sh = trace(p + h.normal * 0.5 + ldir * 0.01, ldir, dim, B);
    let shadow = select(1.0, 0.25, sh.hit);

    let ambient = 0.32;
    let light = (ambient + 0.72 * diff * shadow) * (0.35 + 0.65 * ao);
    return vec4<f32>(albedo * light, 1.0);
}
"#;
