//! `VoxelRenderer` — render por **ray-marching** de un [`VoxelGrid`], con
//! almacenamiento **sparse de verdad** (brick pool).
//!
//! No mesha (ruta elegida en `MOTOR-VOXEL.md` §11.1): el rayo se marcha por la
//! estructura y el color sale del voxel golpeado.
//!
//! Evolución de la memoria:
//! - **M1** denso (toda la grilla, incluido el aire, en una textura 3D).
//! - **M2** traversal sparse (DDA de dos niveles sobre un mapa grueso).
//! - **Ahora (brick pool, prereq de M5/M6)**: la memoria también es sparse.
//!   Sólo los *bricks* (`BRICK³` voxels) que contienen algo se guardan, en un
//!   **atlas 3D** (el *pool*); una textura de **indirección** del tamaño grueso
//!   mapea cada celda → slot del pool (`0` = brick vacío, no ocupa memoria). El
//!   shader resuelve cada voxel: celda gruesa → slot → texel del atlas.
//!
//! El DDA de dos niveles usa la indirección como mapa de ocupación (skip de
//! aire) Y como tabla de slots (lookup fino). Mutar voxels (M3) sigue siendo
//! incremental: un slot map + free list permiten allocar/liberar bricks y subir
//! sólo los bricks tocados.
//!
//! Entidades (M4) y la firma de [`VoxelRenderer::render`] (compatible con
//! `View::gpu_paint_with`) intactas.

use crate::camera::Camera3d;
use crate::voxel::VoxelGrid;

/// Tamaño de brick (voxels por lado).
const BRICK: u32 = 8;

/// Máximo de entidades vivas por frame (cabe holgado en un uniform).
const MAX_ENTITIES: usize = 64;

/// Una entidad (agente) — una caja analítica ray-marcheada en el mismo pase que
/// los voxels (M4). Posición en coordenadas de voxel `[0, dim]` (sub-voxel, así
/// se mueve suave), `half` = medio-tamaño por eje, color RGB.
#[derive(Clone, Copy)]
pub struct Entity3d {
    pub pos: [f32; 3],
    pub half: [f32; 3],
    pub color: [u8; 3],
}

/// Renderer de voxels por ray-march de dos niveles sobre un brick pool sparse.
pub struct VoxelRenderer {
    pool: wgpu::Texture,
    indir: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    ubuf_ent: wgpu::Buffer,
    dim: [u32; 3],
    cdim: [u32; 3],
    /// Slots del atlas por eje (cuántos bricks entran en cada dimensión).
    atlas: [u32; 3],
    /// Por celda gruesa: `slot + 1`, o `0` si el brick está vacío. Espeja la
    /// textura de indirección en CPU (para el camino incremental).
    slots: Vec<u32>,
    /// Slots libres del pool (free list para allocar bricks nuevos).
    free: Vec<u32>,
    /// Dirección hacia el sol (normalizada). Editable antes de `render`.
    pub sun_dir: [f32; 3],
    /// Entidades vivas — se empacan y suben en cada `render`.
    pub entities: Vec<Entity3d>,
}

impl VoxelRenderer {
    /// Crea el renderer y construye el brick pool a partir de `grid`.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
        grid: &VoxelGrid,
    ) -> Self {
        let dim = grid.dim();
        let cdim = [
            dim[0].div_ceil(BRICK),
            dim[1].div_ceil(BRICK),
            dim[2].div_ceil(BRICK),
        ];
        let n_cells = (cdim[0] * cdim[1] * cdim[2]) as usize;

        // Bricks ocupados → capacidad con holgura para crecer (M3).
        let occupied: u32 = (0..cdim[2])
            .flat_map(|cz| (0..cdim[1]).flat_map(move |cy| (0..cdim[0]).map(move |cx| (cx, cy, cz))))
            .filter(|&(cx, cy, cz)| grid.brick_occupied(BRICK, cx, cy, cz) != 0)
            .count() as u32;
        let want = occupied + occupied / 2 + 64;
        // Atlas cúbico-ish: ax·ay·az ≥ want.
        let ax = ((want as f64).cbrt().ceil() as u32).max(1);
        let ay = ax;
        let az = want.div_ceil(ax * ay).max(1);
        let capacity = ax * ay * az;
        let atlas = [ax, ay, az];

        let pool = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-voxel-pool"),
            size: extent([ax * BRICK, ay * BRICK, az * BRICK]),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let pool_view = pool.create_view(&wgpu::TextureViewDescriptor::default());

        let indir = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-voxel-indir"),
            size: extent(cdim),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let indir_view = indir.create_view(&wgpu::TextureViewDescriptor::default());

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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                uniform_entry(2),
                uniform_entry(3),
            ],
        });

        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-voxel-ubuf"),
            // mat4(64)+cam_eye(16)+grid_dim/brick(16)+sun(16)+cdim(16)+atlas(16)
            size: 144,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ubuf_ent = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-3d-voxel-ubuf-ent"),
            size: (16 + MAX_ENTITIES * 48) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-3d-voxel-bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&pool_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&indir_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: ubuf_ent.as_entire_binding(),
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

        let mut r = Self {
            pool,
            indir,
            bind_group,
            pipeline,
            ubuf,
            ubuf_ent,
            dim,
            cdim,
            atlas,
            slots: vec![0u32; n_cells],
            free: Vec::new(),
            sun_dir: normalize3([0.5, 1.0, 0.35]),
            entities: Vec::new(),
        };

        // Poblar el pool: cada brick ocupado toma un slot incremental.
        let mut next: u32 = 0;
        for cz in 0..cdim[2] {
            for cy in 0..cdim[1] {
                for cx in 0..cdim[0] {
                    if grid.brick_occupied(BRICK, cx, cy, cz) != 0 {
                        let slot = next;
                        next += 1;
                        let idx = r.cell_idx(cx, cy, cz);
                        r.slots[idx] = slot + 1;
                        r.upload_brick(queue, slot, grid, cx, cy, cz);
                    }
                }
            }
        }
        r.free = (next..capacity).rev().collect();
        r.upload_indirection_full(queue);
        r
    }

    /// Bricks ocupados (slots usados) y total de celdas gruesas — para reportar
    /// el ahorro de memoria del pool frente al denso.
    pub fn brick_usage(&self) -> (u32, u32) {
        let used = self.slots.iter().filter(|&&s| s != 0).count() as u32;
        (used, (self.cdim[0] * self.cdim[1] * self.cdim[2]))
    }

    /// Bytes del pool (atlas) vs. lo que costaría el grid denso completo.
    pub fn memory_bytes(&self) -> (u64, u64) {
        let (used, _) = self.brick_usage();
        let per_brick = (BRICK * BRICK * BRICK * 4) as u64;
        let pool = used as u64 * per_brick;
        let dense = (self.dim[0] * self.dim[1] * self.dim[2] * 4) as u64;
        (pool, dense)
    }

    #[inline]
    fn cell_idx(&self, cx: u32, cy: u32, cz: u32) -> usize {
        (cx + cy * self.cdim[0] + cz * self.cdim[0] * self.cdim[1]) as usize
    }

    /// Origen del slot en el atlas (en celdas de brick).
    fn slot_origin(&self, slot: u32) -> [u32; 3] {
        let ax = self.atlas[0];
        let ay = self.atlas[1];
        [slot % ax, (slot / ax) % ay, slot / (ax * ay)]
    }

    fn upload_brick(&self, queue: &wgpu::Queue, slot: u32, grid: &VoxelGrid, cx: u32, cy: u32, cz: u32) {
        let data = grid.extract_brick(BRICK, cx, cy, cz);
        let o = self.slot_origin(slot);
        write_3d(
            queue,
            &self.pool,
            [o[0] * BRICK, o[1] * BRICK, o[2] * BRICK],
            [BRICK, BRICK, BRICK],
            4,
            &data,
        );
    }

    fn upload_indirection_full(&self, queue: &wgpu::Queue) {
        let mut bytes = Vec::with_capacity(self.slots.len() * 4);
        for &s in &self.slots {
            bytes.extend_from_slice(&s.to_ne_bytes());
        }
        write_3d(queue, &self.indir, [0; 3], self.cdim, 4, &bytes);
    }

    /// **Actualización incremental (M3).** Sube sólo los bricks tocados por la
    /// región mutada: re-sube cada brick afectado a su slot (allocando slots
    /// nuevos para bricks que pasan de vacío→ocupado, liberándolos al revés) y
    /// re-sube la sub-región de indirección. Devuelve los bytes subidos.
    pub fn sync(&mut self, queue: &wgpu::Queue, grid: &mut VoxelGrid) -> u32 {
        let Some(r) = grid.take_dirty() else {
            return 0;
        };
        let cmin = [r[0] / BRICK, r[1] / BRICK, r[2] / BRICK];
        let cmax = [r[3] / BRICK, r[4] / BRICK, r[5] / BRICK];
        let mut uploaded = 0u32;
        let per_brick = BRICK * BRICK * BRICK * 4;

        for cz in cmin[2]..=cmax[2] {
            for cy in cmin[1]..=cmax[1] {
                for cx in cmin[0]..=cmax[0] {
                    let idx = self.cell_idx(cx, cy, cz);
                    let occ = grid.brick_occupied(BRICK, cx, cy, cz) != 0;
                    let cur = self.slots[idx];
                    if occ {
                        let slot = if cur != 0 {
                            cur - 1
                        } else {
                            match self.free.pop() {
                                Some(s) => {
                                    self.slots[idx] = s + 1;
                                    s
                                }
                                None => {
                                    // Pool lleno: el brick no entra (raro con la
                                    // holgura inicial). Lo saltamos sin romper.
                                    continue;
                                }
                            }
                        };
                        self.upload_brick(queue, slot, grid, cx, cy, cz);
                        uploaded += per_brick;
                    } else if cur != 0 {
                        // Brick vaciado: liberar el slot (el atlas queda
                        // huérfano pero la indirección en 0 lo hace invisible).
                        self.free.push(cur - 1);
                        self.slots[idx] = 0;
                    }
                }
            }
        }

        // Re-subir la sub-región de indirección tocada.
        let cext = [
            cmax[0] - cmin[0] + 1,
            cmax[1] - cmin[1] + 1,
            cmax[2] - cmin[2] + 1,
        ];
        let mut ind = Vec::with_capacity((cext[0] * cext[1] * cext[2] * 4) as usize);
        for cz in cmin[2]..=cmax[2] {
            for cy in cmin[1]..=cmax[1] {
                for cx in cmin[0]..=cmax[0] {
                    ind.extend_from_slice(&self.slots[self.cell_idx(cx, cy, cz)].to_ne_bytes());
                }
            }
        }
        write_3d(queue, &self.indir, cmin, cext, 4, &ind);
        uploaded + ind.len() as u32
    }

    /// Ray-marchea la grilla vista desde `camera` sobre `target`. Color
    /// `LoadOp::Load`; misses por `discard`. Grilla centrada en el origen.
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
        let mut u = Vec::with_capacity(144);
        for v in inv_vp.to_cols_array() {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [camera.eye.x, camera.eye.y, camera.eye.z, 0.0] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [self.dim[0] as f32, self.dim[1] as f32, self.dim[2] as f32, BRICK as f32] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        let s = normalize3(self.sun_dir);
        for v in [s[0], s[1], s[2], 0.0] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [self.cdim[0] as f32, self.cdim[1] as f32, self.cdim[2] as f32, 0.0] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [self.atlas[0] as f32, self.atlas[1] as f32, self.atlas[2] as f32, 0.0] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        queue.write_buffer(&self.ubuf, 0, &u);

        // Entidades: count (vec4) + array de [pos, half, color] (3×vec4 c/u).
        let n = self.entities.len().min(MAX_ENTITIES);
        let mut e = Vec::with_capacity(16 + MAX_ENTITIES * 48);
        for v in [n as f32, 0.0, 0.0, 0.0] {
            e.extend_from_slice(&v.to_ne_bytes());
        }
        for i in 0..MAX_ENTITIES {
            let ent = self.entities.get(i).copied().unwrap_or(Entity3d {
                pos: [0.0; 3],
                half: [0.0; 3],
                color: [0, 0, 0],
            });
            for v in [ent.pos[0], ent.pos[1], ent.pos[2], 0.0] {
                e.extend_from_slice(&v.to_ne_bytes());
            }
            for v in [ent.half[0], ent.half[1], ent.half[2], 0.0] {
                e.extend_from_slice(&v.to_ne_bytes());
            }
            for v in [
                ent.color[0] as f32 / 255.0,
                ent.color[1] as f32 / 255.0,
                ent.color[2] as f32 / 255.0,
                0.0,
            ] {
                e.extend_from_slice(&v.to_ne_bytes());
            }
        }
        queue.write_buffer(&self.ubuf_ent, 0, &e);

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

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
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
    cdim: vec4<f32>,       // xyz = dim grueso (celdas de brick)
    atlas: vec4<f32>,      // xyz = slots por eje en el atlas del pool
};
struct Entity {
    pos: vec4<f32>,
    half: vec4<f32>,
    color: vec4<f32>,
};
struct EntU {
    count: vec4<f32>,
    ents: array<Entity, 64>,
};
@group(0) @binding(0) var pool: texture_3d<f32>;
@group(0) @binding(1) var indir: texture_3d<u32>;
@group(0) @binding(2) var<uniform> u: U;
@group(0) @binding(3) var<uniform> ent: EntU;

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

// Slot del brick que contiene la celda gruesa `cc` (0 = vacío).
fn slot_at(cc: vec3<i32>) -> u32 {
    if (any(cc < vec3<i32>(0)) || any(vec3<f32>(cc) >= u.cdim.xyz)) { return 0u; }
    return textureLoad(indir, cc, 0).r;
}

// Voxel fino vía indirección → pool. `.a > 0.5` = sólido.
fn voxel_at(voxel: vec3<f32>) -> vec4<f32> {
    let vi = vec3<i32>(voxel);
    if (any(vi < vec3<i32>(0)) || any(vec3<f32>(vi) >= u.grid_dim.xyz)) { return vec4<f32>(0.0); }
    let bu = i32(u.grid_dim.w);
    let cc = vi / bu;
    let s = slot_at(cc);
    if (s == 0u) { return vec4<f32>(0.0); }
    let slot = i32(s - 1u);
    let ax = i32(u.atlas.x);
    let ay = i32(u.atlas.y);
    let acell = vec3<i32>(slot % ax, (slot / ax) % ay, slot / (ax * ay));
    let local = vi - cc * bu;
    return textureLoad(pool, acell * bu + local, 0);
}

struct Hit {
    hit: bool,
    vox: vec3<f32>,
    normal: vec3<f32>,
    t: f32,
};

// DDA de dos niveles sobre el brick pool: marcha la grilla gruesa (indirección),
// baja a la fina sólo en bricks con slot.
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
        if (slot_at(vec3<i32>(cc)) != 0u) {
            var voxel = clamp(floor(ro + safe_rd * (t_cell + 1e-4)), vec3<f32>(0.0), dim - 1.0);
            var t_max_f = ((voxel + max(step, vec3<f32>(0.0))) - ro) * inv_rd;
            let t_delta_f = abs(inv_rd);
            var fnorm = cnorm;
            var t_vox = t_cell;
            let max_fine = i32(B) * 3 + 3;
            for (var fi = 0; fi < max_fine; fi = fi + 1) {
                if (any(voxel < vec3<f32>(0.0)) || any(voxel >= dim)) { return h; }
                if (any(floor(voxel / B) != cc)) { break; }
                let c = voxel_at(voxel);
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
    return select(0.0, 1.0, voxel_at(vec3<f32>(p)).a > 0.5);
}

fn vertex_ao(s1: f32, s2: f32, c: f32) -> f32 {
    if (s1 > 0.5 && s2 > 0.5) { return 0.0; }
    return (3.0 - (s1 + s2 + c)) / 3.0;
}

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

struct EHit {
    hit: bool,
    t: f32,
    normal: vec3<f32>,
    color: vec3<f32>,
};

fn trace_entities(ro: vec3<f32>, rd: vec3<f32>, max_t: f32) -> EHit {
    var best: EHit;
    best.hit = false;
    best.t = max_t;
    let safe_rd = vec3<f32>(
        select(rd.x, 1e-6, abs(rd.x) < 1e-6),
        select(rd.y, 1e-6, abs(rd.y) < 1e-6),
        select(rd.z, 1e-6, abs(rd.z) < 1e-6),
    );
    let inv_rd = 1.0 / safe_rd;
    let n = i32(ent.count.x);
    for (var i = 0; i < n; i = i + 1) {
        let e = ent.ents[i];
        let bmin = e.pos.xyz - e.half.xyz;
        let bmax = e.pos.xyz + e.half.xyz;
        let tb = ray_box(ro, inv_rd, bmin, bmax);
        if (tb.x <= tb.y && tb.x > 1e-3 && tb.x < best.t) {
            best.hit = true;
            best.t = tb.x;
            best.color = e.color.rgb;
            let p = ro + rd * tb.x;
            let c = (bmin + bmax) * 0.5;
            let d = max((bmax - bmin) * 0.5, vec3<f32>(1e-4));
            let q = (p - c) / d;
            let aq = abs(q);
            if (aq.x >= aq.y && aq.x >= aq.z) { best.normal = vec3<f32>(sign(q.x), 0.0, 0.0); }
            else if (aq.y >= aq.z) { best.normal = vec3<f32>(0.0, sign(q.y), 0.0); }
            else { best.normal = vec3<f32>(0.0, 0.0, sign(q.z)); }
        }
    }
    return best;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let p_near = u.inv_vp * vec4<f32>(in.ndc, 0.0, 1.0);
    let p_far  = u.inv_vp * vec4<f32>(in.ndc, 1.0, 1.0);
    let ro_world = u.cam_eye.xyz;
    let rd = normalize(p_far.xyz / p_far.w - p_near.xyz / p_near.w);

    let dim = u.grid_dim.xyz;
    let B = u.grid_dim.w;
    let ro = ro_world + dim * 0.5;

    let h = trace(ro, rd, dim, B);
    let t_vox = select(1e30, h.t, h.hit);
    let eh = trace_entities(ro, rd, t_vox);

    var albedo: vec3<f32>;
    var normal: vec3<f32>;
    var p: vec3<f32>;
    var ao: f32;
    if (eh.hit) {
        albedo = eh.color;
        normal = eh.normal;
        p = ro + rd * eh.t;
        ao = 1.0;
    } else if (h.hit) {
        albedo = voxel_at(h.vox).rgb;
        normal = h.normal;
        p = ro + rd * h.t;
        ao = compute_ao(h.vox, h.normal, p, dim);
    } else {
        discard;
    }

    let ldir = u.sun_dir.xyz;
    let diff = max(dot(normal, ldir), 0.0);

    let so = p + normal * 0.5 + ldir * 0.01;
    let sh_v = trace(so, ldir, dim, B);
    let sh_e = trace_entities(so, ldir, 1e30);
    let shadow = select(1.0, 0.25, sh_v.hit || sh_e.hit);

    let ambient = 0.32;
    let light = (ambient + 0.72 * diff * shadow) * (0.35 + 0.65 * ao);
    return vec4<f32>(albedo * light, 1.0);
}
"#;
