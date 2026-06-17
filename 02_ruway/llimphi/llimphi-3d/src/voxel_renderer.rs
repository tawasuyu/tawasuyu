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

/// Lado de brick (voxels) expuesto: el streaming toroidal exige orígenes de
/// ventana alineados a este múltiplo (ver [`VoxelRenderer::scroll_to`]).
pub const VOXEL_BRICK: u32 = BRICK;

/// Tope de luces puntuales por frame, expuesto para que el caller no exceda.
pub const VOXEL_MAX_LIGHTS: usize = MAX_LIGHTS;

/// Máximo de entidades vivas por frame (cabe holgado en un uniform).
const MAX_ENTITIES: usize = 64;

/// Máximo de luces puntuales por frame (cabe en el uniform principal).
const MAX_LIGHTS: usize = 4;

/// Luz puntual coloreada (antorcha/lámpara): ilumina los voxels/entidades
/// cercanos con caída suave por distancia. Posición en coordenadas de voxel
/// `[0, dim]` (igual que las entidades), color RGB lineal (puede pasar de 1.0
/// para un brillo intenso), `range` = radio de alcance en voxels.
#[derive(Clone, Copy)]
pub struct PointLight {
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub range: f32,
}

/// Una entidad (agente) — una caja analítica ray-marcheada en el mismo pase que
/// los voxels (M4). Posición en coordenadas de voxel `[0, dim]` (sub-voxel, así
/// se mueve suave), `half` = medio-tamaño por eje, color RGB.
#[derive(Clone, Copy)]
pub struct Entity3d {
    pub pos: [f32; 3],
    pub half: [f32; 3],
    pub color: [u8; 3],
}

/// Atmósfera del mundo (primera rebanada de M6): cielo gradiente + niebla por
/// distancia ("aerial perspective"). Editable antes de `render`.
///
/// `fog_density` controla todo el efecto: con `0.0` (default) el renderer se
/// comporta como antes — los rayos que no pegan nada hacen `discard` (deja ver
/// el fondo vello) y no hay niebla. Con `> 0.0` el motor pinta su **propio
/// cielo** en los misses y desvanece lo lejano hacia el color del horizonte, que
/// es lo que hace legible un mundo grande (sin esto, el borde lejano del terreno
/// se ve como un muro recortado).
#[derive(Clone, Copy)]
pub struct Atmosphere {
    /// Color del cielo en el cenit (mirando hacia arriba).
    pub sky_zenith: [u8; 3],
    /// Color del cielo en el horizonte — también el color hacia el que
    /// desvanece la niebla.
    pub sky_horizon: [u8; 3],
    /// Densidad de niebla por unidad de voxel. `0.0` = desactivada (miss →
    /// `discard`, sin niebla); valores típicos `0.002..0.02`.
    pub fog_density: f32,
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self {
            sky_zenith: [70, 120, 200],
            sky_horizon: [188, 208, 230],
            fog_density: 0.0,
        }
    }
}

/// Renderer de voxels por ray-march de dos niveles sobre un brick pool sparse.
pub struct VoxelRenderer {
    pool: wgpu::Texture,
    indir: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    /// Layout del bind group, guardado para re-armar el bind group cuando el pool
    /// crece (la textura del atlas cambia → su view también).
    bgl: wgpu::BindGroupLayout,
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
    /// Origen de brick de la ventana (streaming toroidal): `slots`/indirección se
    /// indexan por celda **física** = `(celda_lógica + brick_origin) mod cdim`.
    /// `[0,0,0]` = sin scroll (lógica = física, camino clásico).
    brick_origin: [i32; 3],
    /// Dirección hacia el sol (normalizada). Editable antes de `render`.
    pub sun_dir: [f32; 3],
    /// Atmósfera (cielo + niebla). `fog_density = 0` → comportamiento clásico.
    pub atmosphere: Atmosphere,
    /// Depth buffer propio para el camino *standalone* ([`Self::render`]); en
    /// `Scene3d` se usa el depth compartido y este queda sin tocar.
    depth: Option<crate::scene::DepthBuffer>,
    /// Entidades vivas — se empacan y suben en cada `render`.
    pub entities: Vec<Entity3d>,
    /// Luces puntuales coloreadas (≤ [`MAX_LIGHTS`]) — antorchas/lámparas que
    /// iluminan voxels y entidades cercanos. Se empacan y suben en cada `render`.
    pub lights: Vec<PointLight>,
    /// Si las luces puntuales proyectan sombra (un shadow ray por luz hacia su
    /// posición, acotado a la distancia a la luz). `true` por defecto. Apagarlo
    /// recupera el MVP plano (más barato) — útil para comparar off/on.
    pub point_shadows: bool,
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
            // COPY_SRC para poder copiar el atlas a uno más grande al crecer.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
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
            // inv_vp(64)+cam_eye(16)+grid_dim/brick(16)+sun(16)+cdim(16)+atlas(16)
            // +sky_zenith/fog(16)+sky_horizon(16)+vp(64)+scroll(16)
            // +n_lights(16)+lights(4×32=128)
            size: 400,
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
            // Escribe profundidad (del voxel golpeado) → convive con mallas en
            // un depth buffer compartido (`Scene3d`): el render volumétrico y el
            // de triángulos se ocluyen correctamente entre sí.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::scene::DEPTH_FORMAT,
                depth_write_enabled: true,
                // LessEqual (no Less): el cielo en los misses escribe profundidad
                // lejana (1.0) y debe pasar contra el clear 1.0; un `Less` lo
                // rechazaría y dejaría ver el fondo negro. Sólo hay un fragmento
                // de voxel por píxel (el del rayo), así que no hay z-fighting.
                depth_compare: wgpu::CompareFunction::LessEqual,
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
            bgl,
            pipeline,
            ubuf,
            ubuf_ent,
            dim,
            cdim,
            atlas,
            slots: vec![0u32; n_cells],
            free: Vec::new(),
            brick_origin: [0, 0, 0],
            sun_dir: normalize3([0.5, 1.0, 0.35]),
            atmosphere: Atmosphere::default(),
            depth: None,
            entities: Vec::new(),
            lights: Vec::new(),
            point_shadows: true,
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

    /// Índice **físico** (en `slots`/indirección) de la celda gruesa **lógica**
    /// `c` (relativa a la ventana): `(c + brick_origin) mod cdim`. Espeja el
    /// `slot_at` del shader. Con `brick_origin = 0` es `cell_idx(c)` directo.
    #[inline]
    fn phys_cell(&self, c: [i32; 3]) -> usize {
        let p = [
            floormod(c[0] + self.brick_origin[0], self.cdim[0] as i32) as u32,
            floormod(c[1] + self.brick_origin[1], self.cdim[1] as i32) as u32,
            floormod(c[2] + self.brick_origin[2], self.cdim[2] as i32) as u32,
        ];
        self.cell_idx(p[0], p[1], p[2])
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
                    // Celda FÍSICA (espeja el `slot_at` toroidal del shader); con
                    // `brick_origin = 0` es la celda lógica directa.
                    let idx = self.phys_cell([cx as i32, cy as i32, cz as i32]);
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

        // Re-subir la indirección tocada. Sin scroll (caso común de edición), la
        // física = la lógica → sub-región contigua (barato). Con scroll, las
        // celdas físicas están envueltas (no contiguas) → re-subimos la
        // indirección entera (es chica: cdim³ u32).
        if self.brick_origin == [0, 0, 0] {
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
        } else {
            self.upload_indirection_full(queue);
            uploaded + (self.slots.len() * 4) as u32
        }
    }

    /// **Streaming toroidal (M6).** Desliza la ventana a `origin_voxel` (esquina
    /// local `(0,0,0)` en coordenadas de mundo, alineada a brick) re-subiendo
    /// **sólo los bricks que entran** — la franja nueva — sin reconstruir el
    /// renderer ni re-subir la ventana entera. `grid` es la ventana ya generada
    /// en ese origen (local `[0,dim)`), de la que se extraen los bricks de la
    /// franja. Los bricks que salen se reemplazan en su misma celda física (la
    /// textura es un ring buffer: `world_brick mod cdim`). Devuelve los bytes
    /// subidos (≈ tamaño de la franja, no de la ventana). Llamar con
    /// `origin_voxel` múltiplo de [`VOXEL_BRICK`].
    pub fn scroll_to(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        origin_voxel: [i32; 3],
        grid: &VoxelGrid,
    ) -> u32 {
        let b = BRICK as i32;
        debug_assert!(
            origin_voxel.iter().all(|v| v % b == 0),
            "scroll_to: origin_voxel debe estar alineado a VOXEL_BRICK"
        );
        let new = [origin_voxel[0] / b, origin_voxel[1] / b, origin_voxel[2] / b];
        let old = self.brick_origin;
        if new == old {
            return 0;
        }
        // El uniform de scroll se actualiza ANTES de poblar para que `phys_cell`
        // calcule las celdas físicas del nuevo origen.
        self.brick_origin = new;

        let cd = [self.cdim[0] as i32, self.cdim[1] as i32, self.cdim[2] as i32];
        let mut uploaded = 0u32;
        let per_brick = BRICK * BRICK * BRICK * 4;

        let entered = |ccx: i32, ccy: i32, ccz: i32| -> bool {
            let wb = [ccx + new[0], ccy + new[1], ccz + new[2]];
            !((0..cd[0]).contains(&(wb[0] - old[0]))
                && (0..cd[1]).contains(&(wb[1] - old[1]))
                && (0..cd[2]).contains(&(wb[2] - old[2])))
        };

        // **Pre-crecer el pool si hace falta, ANTES de subir nada** (la copia del
        // atlas viejo es estable, sin carrera con write_texture en vuelo). Cota
        // segura del pico de slots: los usados ahora + los bricks que entran
        // ocupados (los que salen aún no se liberaron). Crecer una vez al inicio.
        let mut entered_occ = 0u32;
        for ccz in 0..cd[2] {
            for ccy in 0..cd[1] {
                for ccx in 0..cd[0] {
                    if entered(ccx, ccy, ccz)
                        && grid.brick_occupied(BRICK, ccx as u32, ccy as u32, ccz as u32) != 0
                    {
                        entered_occ += 1;
                    }
                }
            }
        }
        let used_now = self.slots.iter().filter(|&&s| s != 0).count() as u32;
        let need = used_now + entered_occ;
        while self.pool_capacity() < need {
            self.grow_layers(device, queue);
        }

        // Recorre las celdas LÓGICAS de la ventana nueva; procesa sólo las que
        // ENTRARON (su brick de mundo no estaba en la ventana vieja). El bulk
        // (presente en ambas) conserva su contenido físico intacto.
        for ccz in 0..cd[2] {
            for ccy in 0..cd[1] {
                for ccx in 0..cd[0] {
                    if !entered(ccx, ccy, ccz) {
                        continue;
                    }
                    // Celda física (= celda vieja que sale, por el ring buffer).
                    let idx = self.phys_cell([ccx, ccy, ccz]);
                    let (lx, ly, lz) = (ccx as u32, ccy as u32, ccz as u32);
                    let occ = grid.brick_occupied(BRICK, lx, ly, lz) != 0;
                    let cur = self.slots[idx];
                    if occ {
                        let slot = if cur != 0 {
                            cur - 1
                        } else {
                            let s = self.free.pop().expect("capacidad pre-crecida");
                            self.slots[idx] = s + 1;
                            s
                        };
                        self.upload_brick(queue, slot, grid, lx, ly, lz);
                        uploaded += per_brick;
                    } else if cur != 0 {
                        self.free.push(cur - 1);
                        self.slots[idx] = 0;
                    }
                }
            }
        }

        self.upload_indirection_full(queue);
        uploaded + (self.slots.len() * 4) as u32
    }

    /// Capacidad del pool en slots de brick (`atlas.x·y·z`). Crece con
    /// [`grow_layers`](Self::grow_layers).
    pub fn pool_capacity(&self) -> u32 {
        self.atlas[0] * self.atlas[1] * self.atlas[2]
    }

    /// **Crece el brick pool** agregando capas `z` al atlas (×1.5, mín +8 slots).
    /// Sólo crece `atlas.z` para no remapear los slots existentes (`slot_origin`
    /// depende de `atlas.x/y`, que quedan fijos): copia el atlas viejo al nuevo,
    /// re-arma el bind group (la view del pool cambió) y agrega los slots nuevos a
    /// la free list. Lo dispara `scroll_to` cuando la franja que entra no tiene
    /// slots libres (ventana más densa que la inicial).
    fn grow_layers(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let [ax, ay, az] = self.atlas;
        let new_az = az + (az / 2).max(8);
        let new_pool = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-3d-voxel-pool"),
            size: extent([ax * BRICK, ay * BRICK, new_az * BRICK]),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        // Copia el atlas viejo (mismas dimensiones x/y, az capas) al nuevo.
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("llimphi-3d-voxel-pool-grow"),
        });
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.pool,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &new_pool,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            extent([ax * BRICK, ay * BRICK, az * BRICK]),
        );
        queue.submit(std::iter::once(enc.finish()));

        // Re-armar el bind group con la view del pool nuevo (indir/ubufs intactos).
        let pool_view = new_pool.create_view(&wgpu::TextureViewDescriptor::default());
        let indir_view = self.indir.create_view(&wgpu::TextureViewDescriptor::default());
        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-3d-voxel-bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&pool_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&indir_view) },
                wgpu::BindGroupEntry { binding: 2, resource: self.ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.ubuf_ent.as_entire_binding() },
            ],
        });

        let old_cap = ax * ay * az;
        let new_cap = ax * ay * new_az;
        self.pool = new_pool;
        self.atlas[2] = new_az;
        self.free.extend((old_cap..new_cap).rev());
    }

    /// Sube los uniforms (cámara/atmósfera/entidades) del frame. Lo llama tanto
    /// [`Self::render`] (standalone) como [`Scene3d`](crate::Scene3d) antes de
    /// abrir el pase compartido. `aspect` = w/h del viewport.
    pub fn upload(&self, queue: &wgpu::Queue, aspect: f32, camera: &Camera3d) {
        let vp = camera.view_proj(aspect);
        let inv_vp = vp.inverse();
        let mut u = Vec::with_capacity(400);
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
        let a = &self.atmosphere;
        for v in [
            a.sky_zenith[0] as f32 / 255.0,
            a.sky_zenith[1] as f32 / 255.0,
            a.sky_zenith[2] as f32 / 255.0,
            a.fog_density.max(0.0),
        ] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        for v in [
            a.sky_horizon[0] as f32 / 255.0,
            a.sky_horizon[1] as f32 / 255.0,
            a.sky_horizon[2] as f32 / 255.0,
            0.0,
        ] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        // Matriz forward (world→clip) para escribir frag_depth del voxel golpeado.
        for v in vp.to_cols_array() {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        // Origen de brick (streaming toroidal): el shader envuelve la celda lógica.
        // Se sube YA REDUCIDO a `[0, cdim)` (floormod) para que el `%` del shader
        // nunca opere sobre un negativo — el `%` de WGSL sobre enteros con signo es
        // ambiguo entre plataformas y rompía el wrap con orígenes negativos.
        for v in [
            floormod(self.brick_origin[0], self.cdim[0] as i32) as f32,
            floormod(self.brick_origin[1], self.cdim[1] as i32) as f32,
            floormod(self.brick_origin[2], self.cdim[2] as i32) as f32,
            0.0,
        ] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        // Luces puntuales: count (vec4) + MAX_LIGHTS × [pos+range, color].
        let nl = self.lights.len().min(MAX_LIGHTS);
        let shadow_flag = if self.point_shadows { 1.0 } else { 0.0 };
        for v in [nl as f32, shadow_flag, 0.0, 0.0] {
            u.extend_from_slice(&v.to_ne_bytes());
        }
        for i in 0..MAX_LIGHTS {
            let l = self.lights.get(i).copied().unwrap_or(PointLight {
                pos: [0.0; 3],
                color: [0.0; 3],
                range: 1.0,
            });
            for v in [l.pos[0], l.pos[1], l.pos[2], l.range] {
                u.extend_from_slice(&v.to_ne_bytes());
            }
            for v in [l.color[0], l.color[1], l.color[2], 0.0] {
                u.extend_from_slice(&v.to_ne_bytes());
            }
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
    }

    /// Dibuja el fullscreen-triangle del ray-march en un pase **ya abierto** (con
    /// color + depth). Lo usa [`Scene3d`](crate::Scene3d) para compartir el pase
    /// con las mallas. Requiere `upload` previo en el mismo frame.
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Ray-marchea la grilla vista desde `camera` sobre `target` (camino
    /// *standalone*, con depth propio). Color `LoadOp::Load`; misses por
    /// `discard` (o cielo, con niebla). Grilla centrada en el origen.
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
        crate::scene::ensure_depth(&mut self.depth, device, w, h);
        let depth_view = &self.depth.as_ref().unwrap().view;

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

/// Módulo con resultado siempre en `[0, m)` (maneja `a` negativo). Espeja el
/// `((x % m) + m) % m` del shader para el direccionamiento toroidal.
#[inline]
fn floormod(a: i32, m: i32) -> i32 {
    ((a % m) + m) % m
}

const WGSL: &str = r#"
struct U {
    inv_vp: mat4x4<f32>,
    cam_eye: vec4<f32>,
    grid_dim: vec4<f32>,   // xyz = dim fino, w = brick size
    sun_dir: vec4<f32>,    // xyz = dirección hacia el sol (normalizada)
    cdim: vec4<f32>,       // xyz = dim grueso (celdas de brick)
    atlas: vec4<f32>,      // xyz = slots por eje en el atlas del pool
    sky_zenith: vec4<f32>, // xyz = color cenit, w = densidad de niebla (0 = off)
    sky_horizon: vec4<f32>,// xyz = color horizonte / hacia el que niebla desvanece
    vp: mat4x4<f32>,       // world→clip (forward) para escribir frag_depth
    scroll: vec4<f32>,     // xyz = origen de brick (streaming toroidal); 0 = sin scroll
    n_lights: vec4<f32>,   // x = cantidad de luces puntuales, y = sombras on/off
    lights: array<vec4<f32>, 8>, // por luz: [pos.xyz, range], [color.rgb, _]
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

// Slot del brick que contiene la celda gruesa LÓGICA `cc` (0 = vacío).
// Streaming toroidal: la textura de indirección es un ring buffer indexado por
// `world_brick mod cdim`. La celda lógica `cc` (relativa a la ventana, [0,cdim))
// se traduce a su celda FÍSICA sumando el origen de brick y envolviendo. Sin
// scroll (`scroll = 0`) la física = la lógica (camino clásico, sin cambios).
fn slot_at(cc: vec3<i32>) -> u32 {
    if (any(cc < vec3<i32>(0)) || any(vec3<f32>(cc) >= u.cdim.xyz)) { return 0u; }
    let cd = vec3<i32>(u.cdim.xyz);
    let bo = vec3<i32>(u.scroll.xyz);
    let phys = ((cc + bo) % cd + cd) % cd; // floormod (maneja origen negativo)
    return textureLoad(indir, phys, 0).r;
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

// Cielo procedural: gradiente horizonte→cenit por la altura del rayo, con un
// disco solar y su halo. Es también el color al que desvanece la niebla.
fn sky_color(rd: vec3<f32>) -> vec3<f32> {
    let t = clamp(rd.y * 0.5 + 0.5, 0.0, 1.0);
    var c = mix(u.sky_horizon.xyz, u.sky_zenith.xyz, pow(t, 0.55));
    let s = max(dot(rd, u.sun_dir.xyz), 0.0);
    c = c + vec3<f32>(1.0, 0.96, 0.84) * pow(s, 260.0) * 1.6;   // disco
    c = c + vec3<f32>(1.0, 0.90, 0.72) * pow(s, 9.0) * 0.16;    // halo
    return c;
}

// Profundidad NDC (0..1, wgpu) del punto golpeado `p` (en espacio de grilla):
// se lleva a mundo (la grilla está centrada en el origen → world = p - dim/2) y
// se proyecta con la matriz forward. Permite que las mallas se ocluyan con los
// voxels en el depth buffer compartido de `Scene3d`.
fn frag_depth(p: vec3<f32>, dim: vec3<f32>) -> f32 {
    let clip = u.vp * vec4<f32>(p - dim * 0.5, 1.0);
    return clip.z / clip.w;
}

struct FOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs(in: VOut) -> FOut {
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

    let fog_density = u.sky_zenith.w;

    var albedo: vec3<f32>;
    var normal: vec3<f32>;
    var p: vec3<f32>;
    var ao: f32;
    var t_hit: f32;
    if (eh.hit) {
        albedo = eh.color;
        normal = eh.normal;
        p = ro + rd * eh.t;
        ao = 1.0;
        t_hit = eh.t;
    } else if (h.hit) {
        albedo = voxel_at(h.vox).rgb;
        normal = h.normal;
        p = ro + rd * h.t;
        ao = compute_ao(h.vox, h.normal, p, dim);
        t_hit = h.t;
    } else {
        // Sin impacto: con niebla activa pintamos cielo propio (a profundidad
        // lejana, así una malla por delante igual se dibuja); sin niebla,
        // descartamos para dejar ver el fondo vello (comportamiento clásico).
        if (fog_density > 0.0) {
            var sky: FOut;
            sky.color = vec4<f32>(sky_color(rd), 1.0);
            sky.depth = 1.0;
            return sky;
        }
        discard;
    }

    let ldir = u.sun_dir.xyz;
    let diff = max(dot(normal, ldir), 0.0);

    let so = p + normal * 0.5 + ldir * 0.01;
    let sh_v = trace(so, ldir, dim, B);
    let sh_e = trace_entities(so, ldir, 1e30);
    let shadow = select(1.0, 0.25, sh_v.hit || sh_e.hit);

    // Luz CON COLOR (look cinematográfico) sin uniforms nuevos: el color del sol
    // sale de su elevación (cálido al ras del horizonte → blanco en lo alto) y el
    // ambiente del color del cielo (rebote frío del cenit). El mood se controla
    // moviendo `sun_dir` y la paleta de cielo, que ya viajan en el uniform.
    let ao_term = 0.35 + 0.65 * ao;
    let sun_h = clamp(u.sun_dir.y, 0.0, 1.0);
    let sun_col = mix(vec3<f32>(1.0, 0.52, 0.24), vec3<f32>(1.0, 0.97, 0.9), sun_h);
    // Ambiente tintado por el cielo pero con ~la misma luminancia que el flat 0.32
    // de antes (no oscurece las caras que no ven el sol).
    let amb_col = mix(vec3<f32>(0.45), u.sky_zenith.xyz, 0.45) * 0.70;
    var light = amb_col + sun_col * (0.78 * diff * shadow);

    // Luces puntuales coloreadas (antorchas/lámparas): caída suave por distancia
    // + sombra dura opcional (un shadow ray hacia la luz, acotado a la distancia
    // a la luz: si un voxel/entidad intercepta *antes* de llegar, la luz no llega).
    // `p` está en espacio de voxel, igual que `light.pos`.
    let nlights = i32(u.n_lights.x);
    let pt_shadows = u.n_lights.y > 0.5;
    for (var li = 0; li < nlights; li = li + 1) {
        let lp = u.lights[2 * li];
        let lc = u.lights[2 * li + 1];
        let to = lp.xyz - p;
        let d = length(to);
        let range = max(lp.w, 1e-3);
        var att = clamp(1.0 - d / range, 0.0, 1.0);
        att = att * att; // caída cuadrática suave
        let ldir2 = to / max(d, 1e-3);
        let ndl = max(dot(normal, ldir2), 0.0);
        var vis = 1.0;
        if (pt_shadows && att > 0.0 && ndl > 0.0) {
            // Sale de la superficie un pelo hacia la luz para no auto-sombrearse.
            let lso = p + normal * 0.5 + ldir2 * 0.01;
            let bias = 0.75; // tolerancia: no contar el propio voxel ni la luz.
            let hv = trace(lso, ldir2, dim, B);
            let blocked_v = hv.hit && hv.t < d - bias;
            let he = trace_entities(lso, ldir2, d - bias);
            vis = select(1.0, 0.0, blocked_v || he.hit);
        }
        light = light + lc.rgb * (att * ndl * vis);
    }

    var color = albedo * light * ao_term;

    // Niebla / perspectiva aérea: lo lejano desvanece hacia el cielo en esa
    // dirección, lo que hace legible el borde de un mundo grande.
    if (fog_density > 0.0) {
        let f = 1.0 - exp(-t_hit * fog_density);
        color = mix(color, sky_color(rd), f);
    }

    var out: FOut;
    out.color = vec4<f32>(color, 1.0);
    out.depth = frag_depth(p, dim);
    return out;
}
"#;
