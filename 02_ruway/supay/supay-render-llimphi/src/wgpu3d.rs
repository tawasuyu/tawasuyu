//! Renderer **wgpu 2.5D** de Doom (Fase 2 — el cimiento nuevo).
//!
//! El renderer viejo (`frame.rs`/`walls.rs`) usa vello, un rasterizador
//! vectorial 2D: finge la perspectiva partiendo las paredes en tiras afines
//! (→ texturas deformadas) y ordena con painter's sort sin depth buffer (→
//! paredes/techos que faltan). Este módulo lo reemplaza por un pase **wgpu**
//! real: la geometría de Doom se sube como **malla 3D** y la GPU hace la
//! proyección perspectiva-correcta y la oclusión con **depth buffer** —
//! ambas cosas gratis y exactas.
//!
//! Geometría:
//! - **Paredes** → quads (sólidas: piso→techo; two-sided: upper/lower/middle)
//!   con UV a lo largo del linedef y vertical por altura. La perspectiva
//!   correcta la da la GPU: **sin warping**.
//! - **Pisos/techos** → el polígono **convexo** de cada subsector,
//!   reconstruido clippeando el bounding-box del mapa con los semiplanos del
//!   camino raíz→hoja del BSP (`snap.nodes`). Esto arregla los huecos que
//!   dejaba la cadena-de-segs incompleta del renderer viejo.
//!
//! Integra vía `View::gpu_paint_with` (corre DESPUÉS de la pasada vello base,
//! `LoadOp::Load`), con su propio depth buffer recreado al cambiar de tamaño.
//!
//! Milestone 1: paridad geométrica + texturas perspectiva-correctas. Sin
//! agua/reflejos/luces dinámicas todavía — eso va sobre esta base.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat4, Vec3};
use supay_scene::{
    NodeSnap, SceneSnapshot, SpriteSnap, WeaponSpriteSnap, NF_SUBSECTOR, NO_SECTOR, NO_SKY_PIC,
};

use crate::WadAtlas;

/// Vértice: posición mundo + UV + multiplicador de luz + `kind` (0 = normal,
/// 1 = superficie líquida → el shader la ondula y la hace brillar). 7×f32 =
/// 28 bytes, empaquetado native-endian.
#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 3],
    uv: [f32; 2],
    light: f32,
    kind: f32,
}

impl Vertex {
    const SIZE: u64 = 7 * 4;
    fn write(&self, out: &mut Vec<u8>) {
        for v in self.pos {
            out.extend_from_slice(&v.to_ne_bytes());
        }
        for v in self.uv {
            out.extend_from_slice(&v.to_ne_bytes());
        }
        out.extend_from_slice(&self.light.to_ne_bytes());
        out.extend_from_slice(&self.kind.to_ne_bytes());
    }
}

/// Clave de textura: pared (nombre de lump) o flat (pic_idx).
#[derive(Clone, PartialEq, Eq, Hash)]
enum TexKey {
    Wall(String),
    Flat(u16),
}

/// Parámetros de cámara, en convención Doom (x este, y norte, z arriba;
/// `yaw` en radianes 0 = +x, CCW; `pitch` positivo = mirar arriba).
#[derive(Clone, Copy)]
pub struct CameraParams {
    pub x: f32,
    pub y: f32,
    pub eye_z: f32,
    pub yaw: f32,
    pub pitch: f32,
    /// FOV horizontal en radianes (Doom clásico ≈ π/2 = 90°).
    pub fov_x: f32,
    /// Tiempo en segundos para animar el agua. Monótono; el host lo pasa
    /// desde un `Instant` (o un contador de ticks/35 en headless).
    pub time: f32,
}

impl CameraParams {
    /// Matriz `world→clip` para wgpu (z en `[0,1]`) dado el aspect del viewport.
    fn mvp(&self, aspect: f32) -> Mat4 {
        let eye = Vec3::new(self.x, self.y, self.eye_z);
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        let (cy, sy) = (self.yaw.cos(), self.yaw.sin());
        let fwd = Vec3::new(cp * cy, cp * sy, sp);
        let view = Mat4::look_at_rh(eye, eye + fwd, Vec3::Z);
        // fovy a partir del fovx horizontal y el aspect.
        let fovy = 2.0 * ((self.fov_x * 0.5).tan() / aspect.max(1e-3)).atan();
        let proj = Mat4::perspective_rh(fovy, aspect.max(1e-3), 1.0, 20_000.0);
        proj * view
    }
}

/// Matriz que refleja el mundo a través del plano horizontal `z = h`
/// (x,y,z) → (x,y, 2h-z). Para la reflexión planar del agua.
fn reflect_across_z(h: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(0.0, 0.0, h))
        * Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0))
        * Mat4::from_translation(Vec3::new(0.0, 0.0, -h))
}

/// Textura subida a GPU con su bind group listo para dibujar.
struct GpuTexture {
    bind_group: wgpu::BindGroup,
}

/// Un lote de triángulos que comparten textura: su vertex buffer + conteo.
struct Batch {
    key: TexKey,
    buffer: wgpu::Buffer,
    count: u32,
}

/// Lote de piso líquido: como un Batch pero con la **secuencia de frames** de
/// la animación de flat de Doom (NUKAGE1→2→3, FWATER1→4, …). `draw` elige el
/// frame por tiempo. Si el flat no anima, `frames` tiene un solo elemento.
struct WaterBatch {
    buffer: wgpu::Buffer,
    count: u32,
    frames: Vec<Arc<GpuTexture>>,
    /// Índice del plano de agua (en `water_planes`) al que pertenece esta
    /// superficie → de qué mapa de reflexión se muestrea.
    plane: usize,
}

/// Máximo de planos de agua distintos con reflexión por escena (cada uno es
/// un pase + textura de reflexión extra). Las superficies a otras alturas se
/// asocian al plano más cercano.
const MAX_WATER_PLANES: usize = 4;

/// El renderer. Cachea las texturas (caras) entre frames; la geometría se
/// reconstruye por frame desde el snapshot (las puertas/plataformas se
/// mueven), que para E1M1 son unos pocos miles de tris — barato.
pub struct DoomGpuRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    uniform_bg: wgpu::BindGroup,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Cache de texturas por clave (nombre de pared / pic de flat).
    textures: HashMap<TexKey, Arc<GpuTexture>>,
    /// Textura blanca 1×1 para superficies sin lump resuelto.
    white: Arc<GpuTexture>,
    batches: Vec<Batch>,
    /// Backdrop de cielo: pipeline fullscreen + su uniform + la textura SKY.
    /// Se dibuja primero, sin escribir depth, así el mundo lo tapa donde hay
    /// geometría y queda visible donde el techo es cielo (skip de techo).
    sky_pipeline: wgpu::RenderPipeline,
    sky_uniform_buf: wgpu::Buffer,
    sky_uniform_bg: wgpu::BindGroup,
    sky_tex: Option<Arc<GpuTexture>>,
    /// Datos para billboards de sprites (resueltos por frame en `draw`, ya que
    /// la rotación 1..8 depende de la cámara). Guardados en `set_scene`.
    atlas: Option<Arc<WadAtlas>>,
    sprites: Vec<SpriteSnap>,
    sector_lights: Vec<u8>,
    /// Cache de texturas de sprite por (spritenum, frame_letter, angle 1..8).
    sprite_tex: HashMap<(u16, u8, u8), Arc<GpuTexture>>,
    /// Pipeline para overlays 2D en clip-space (el arma en mano). Sin depth
    /// write, se dibuja al final encima de todo.
    overlay_pipeline: wgpu::RenderPipeline,
    /// psprites del arma del frame actual (weapon + weapon_flash).
    weapon: WeaponSpriteSnap,
    weapon_flash: WeaponSpriteSnap,
    /// Reflexión planar: batches de pisos líquidos (separados para muestrear
    /// el mapa de reflexión), uniforme con la MVP reflejada, y el render
    /// target de reflexión (color+depth+bind group), recreado al cambiar size.
    water_batches: Vec<WaterBatch>,
    /// Cache de texturas de frame de flat por nombre (NUKAGE1, NUKAGE2, …).
    flat_frame_tex: HashMap<String, Arc<GpuTexture>>,
    /// Alturas `z` de los planos de agua distintos (≤ MAX_WATER_PLANES). Cada
    /// uno tiene su propio pase + textura de reflexión.
    water_planes: Vec<f32>,
    /// Uniformes (MVP reflejada) por plano — MAX_WATER_PLANES creados al
    /// inicio. Y los render targets de reflexión, recreados al cambiar size.
    refl_uniform_bufs: Vec<wgpu::Buffer>,
    refl_uniform_bgs: Vec<wgpu::BindGroup>,
    refl_targets: Vec<ReflTarget>,
    /// SSAA: la escena se rinde a `SUPERSAMPLE`× la resolución en este target
    /// propio y luego se baja al target real con filtro lineal (antialiasing).
    scene: Option<SceneTarget>,
    blit_pipeline: wgpu::RenderPipeline,
}

/// Factor de supersampling (antialiasing). 2 = rinde a 2× en cada eje (4× de
/// fragmentos) y promedia al bajar al target real.
const SUPERSAMPLE: u32 = 2;

/// Target intermedio de la escena (color + depth) a resolución supersampleada.
struct SceneTarget {
    w: u32,
    h: u32,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    /// Bind group que expone el color como textura para el blit de bajada.
    blit_bg: wgpu::BindGroup,
}

/// Render target de la reflexión planar.
struct ReflTarget {
    w: u32,
    h: u32,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    /// Bind group (group 2 del pipeline principal) que expone el color como
    /// textura para que la superficie de agua lo muestree.
    bind_group: wgpu::BindGroup,
}

impl DoomGpuRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
    ) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("doom3d-uniform-layout"),
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
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("doom3d-tex-layout"),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("doom3d-pipeline-layout"),
            // group 0 = uniform, 1 = textura de superficie, 2 = mapa de reflexión.
            bind_group_layouts: &[&uniform_layout, &tex_layout, &tex_layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("doom3d-shader"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("doom3d-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: Vertex::SIZE,
                    step_mode: wgpu::VertexStepMode::Vertex,
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
                            format: wgpu::VertexFormat::Float32,
                            offset: 20,
                            shader_location: 2,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: 24,
                            shader_location: 3,
                        },
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                // Sin back-face cull: cada pared se ve de los dos lados sin
                // pelearse con el winding. El depth buffer resuelve la
                // oclusión correctamente igual.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("doom3d-uniform"),
            size: 112, // mat4(64) + eye(16) + params(16) + clip(16)
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("doom3d-uniform-bg"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        // Uniformes gemelos para los pases de reflexión (MVP reflejada), uno
        // por plano de agua posible.
        let mut refl_uniform_bufs = Vec::with_capacity(MAX_WATER_PLANES);
        let mut refl_uniform_bgs = Vec::with_capacity(MAX_WATER_PLANES);
        for i in 0..MAX_WATER_PLANES {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("doom3d-uniform-reflect"),
                size: 112,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("doom3d-uniform-reflect-bg"),
                layout: &uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buf.as_entire_binding(),
                }],
            });
            let _ = i;
            refl_uniform_bufs.push(buf);
            refl_uniform_bgs.push(bg);
        }
        // Filtrado lineal (mag+min) → texturas suaves "alta definición" en vez
        // del pixelado nearest clásico. Es el look GZDoom-con-filtrado.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("doom3d-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let white = Arc::new(upload_texture(
            device,
            queue,
            &tex_layout,
            &sampler,
            &[255, 255, 255, 255],
            1,
            1,
        ));

        // --- Backdrop de cielo ---
        let sky_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("doom3d-sky-uniform-layout"),
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
        let sky_uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("doom3d-sky-uniform"),
            size: 16, // yaw, pitch, fov_x, aspect
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sky_uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("doom3d-sky-uniform-bg"),
            layout: &sky_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: sky_uniform_buf.as_entire_binding(),
            }],
        });
        let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("doom3d-sky-shader"),
            source: wgpu::ShaderSource::Wgsl(SKY_WGSL.into()),
        });
        let sky_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("doom3d-sky-pipeline-layout"),
                bind_group_layouts: &[&sky_uniform_layout, &tex_layout],
                push_constant_ranges: &[],
            });
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("doom3d-sky-pipeline"),
            layout: Some(&sky_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &sky_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // No escribe depth y siempre pasa: es un fondo, el mundo lo tapa.
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sky_shader,
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

        // --- Overlay 2D (arma en mano) ---
        let overlay_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("doom3d-overlay-shader"),
            source: wgpu::ShaderSource::Wgsl(OVERLAY_WGSL.into()),
        });
        let overlay_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("doom3d-overlay-pipeline-layout"),
            bind_group_layouts: &[&tex_layout],
            push_constant_ranges: &[],
        });
        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("doom3d-overlay-pipeline"),
            layout: Some(&overlay_layout),
            vertex: wgpu::VertexState {
                module: &overlay_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 16, // pos[2] + uv[2]
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &overlay_shader,
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

        // --- Blit de bajada SSAA: muestrea la escena supersampleada y la
        //     escribe (downscale lineal) al target real. ---
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("doom3d-blit-shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("doom3d-blit-layout"),
            bind_group_layouts: &[&tex_layout],
            push_constant_ranges: &[],
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("doom3d-blit-pipeline"),
            layout: Some(&blit_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
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

        let zero_weap = WeaponSpriteSnap {
            active: false,
            sprite: 0,
            frame: 0,
            sx: 0.0,
            sy: 0.0,
        };

        Self {
            pipeline,
            uniform_buf,
            uniform_bg,
            tex_layout,
            sampler,
            textures: HashMap::new(),
            white,
            batches: Vec::new(),
            sky_pipeline,
            sky_uniform_buf,
            sky_uniform_bg,
            sky_tex: None,
            atlas: None,
            sprites: Vec::new(),
            sector_lights: Vec::new(),
            sprite_tex: HashMap::new(),
            overlay_pipeline,
            weapon: zero_weap,
            weapon_flash: zero_weap,
            water_batches: Vec::new(),
            flat_frame_tex: HashMap::new(),
            water_planes: Vec::new(),
            refl_uniform_bufs,
            refl_uniform_bgs,
            refl_targets: Vec::new(),
            scene: None,
            blit_pipeline,
        }
    }

    /// Reconstruye la geometría desde `snap` y sube/cachea las texturas.
    /// Llamar antes de [`Self::draw`] cuando cambia el snapshot.
    pub fn set_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &Arc<WadAtlas>,
        snap: &SceneSnapshot,
    ) {
        // Guardamos lo que `draw` necesita para los sprites (su rotación 1..8
        // depende de la cámara, que sólo está en `draw`).
        self.atlas = Some(atlas.clone());
        self.sprites = snap.sprites.to_vec();
        self.sector_lights = snap.sectors.iter().map(|s| s.light_level).collect();
        self.weapon = snap.weapon.clone();
        self.weapon_flash = snap.weapon_flash.clone();

        // 1. Acumular vértices por clave de textura (CPU). Las paredes
        //    necesitan el tamaño real de cada textura (para UV correctas +
        //    pegging de Doom), así que el builder consulta el atlas. Los pisos
        //    líquidos van a un mapa aparte (`water_by_tex`) para la reflexión.
        let mut by_tex: HashMap<TexKey, Vec<Vertex>> = HashMap::new();
        let mut water_map: WaterMap = HashMap::new();
        build_walls(snap, atlas, &mut by_tex);
        build_flats(snap, atlas, &mut by_tex, &mut water_map);
        // Planos de agua: hasta MAX_WATER_PLANES alturas distintas, elegidas
        // por área (cantidad de vértices). Cada superficie se asocia luego al
        // plano más cercano.
        self.water_planes = pick_water_planes(&water_map);
        if std::env::var_os("SUPAY_DIAG").is_some() {
            eprintln!(
                "wgpu3d: {} grupos de agua → {} planos de reflexión {:?}",
                water_map.len(),
                self.water_planes.len(),
                self.water_planes,
            );
        }

        // 2. Asegurar que cada textura referida esté subida a GPU.
        for key in by_tex.keys() {
            if self.textures.contains_key(key) {
                continue;
            }
            let tex = match key {
                TexKey::Wall(name) => atlas
                    .wall_texture(name)
                    .map(|t| (t.rgba.clone(), t.width as u32, t.height as u32)),
                TexKey::Flat(pic) => atlas
                    .flat_rgba(*pic)
                    .map(|rgba| ((*rgba).clone(), 64, 64)),
            };
            if let Some((rgba, w, h)) = tex {
                if w > 0 && h > 0 && rgba.len() as u32 >= w * h * 4 {
                    let gpu = upload_texture(
                        device,
                        queue,
                        &self.tex_layout,
                        &self.sampler,
                        &rgba,
                        w,
                        h,
                    );
                    self.textures.insert(key.clone(), Arc::new(gpu));
                }
            }
        }

        // 2b. Textura de cielo (una vez). El snapshot no expone el nombre del
        //     skytexture, así que probamos los lumps canónicos por episodio.
        if self.sky_tex.is_none() {
            for name in ["SKY1", "SKY2", "SKY3", "SKY4"] {
                if let Some(t) = atlas.wall_texture(name) {
                    if t.width > 0 && t.height > 0 {
                        self.sky_tex = Some(Arc::new(upload_texture(
                            device,
                            queue,
                            &self.tex_layout,
                            &self.sampler,
                            &t.rgba,
                            t.width as u32,
                            t.height as u32,
                        )));
                        break;
                    }
                }
            }
        }

        // 3. Subir un vertex buffer por lote. El mundo seco normal; el agua
        //    con su secuencia de frames de animación de flat.
        self.batches = build_batches(device, by_tex);
        self.water_batches.clear();
        for ((pic, _hk), (height, verts)) in water_map {
            if verts.is_empty() {
                continue;
            }
            // Plano más cercano a esta altura de agua.
            let plane = nearest_plane(&self.water_planes, height);
            // Nombre base del flat → nombres de los frames de la animación.
            let frame_names = atlas
                .flat_name(pic)
                .as_deref()
                .map(liquid_anim_frames)
                .unwrap_or_default();
            // Resolver/cachear la textura GPU de cada frame.
            let mut frames: Vec<Arc<GpuTexture>> = Vec::new();
            for name in &frame_names {
                if let Some(t) = self.flat_frame_tex.get(name) {
                    frames.push(t.clone());
                    continue;
                }
                if let Some(rgba) = atlas.decode_flat(name) {
                    if rgba.len() >= 64 * 64 * 4 {
                        let t = Arc::new(upload_texture(
                            device,
                            queue,
                            &self.tex_layout,
                            &self.sampler,
                            &rgba,
                            64,
                            64,
                        ));
                        self.flat_frame_tex.insert(name.clone(), t.clone());
                        frames.push(t);
                    }
                }
            }
            // Fallback: si no se resolvió ningún frame (flat raro o forzado),
            // usamos la textura por pic del cache normal o `white`.
            if frames.is_empty() {
                if let Some(rgba) = atlas.flat_rgba(pic) {
                    if rgba.len() >= 64 * 64 * 4 {
                        frames.push(Arc::new(upload_texture(
                            device,
                            queue,
                            &self.tex_layout,
                            &self.sampler,
                            &rgba,
                            64,
                            64,
                        )));
                    }
                }
            }
            if frames.is_empty() {
                frames.push(self.white.clone());
            }

            let mut bytes = Vec::with_capacity(verts.len() * Vertex::SIZE as usize);
            for v in &verts {
                v.write(&mut bytes);
            }
            let buffer =
                create_buffer_init(device, "doom3d-water", wgpu::BufferUsages::VERTEX, &bytes);
            self.water_batches.push(WaterBatch {
                buffer,
                count: verts.len() as u32,
                frames,
                plane,
            });
        }
    }

    /// Pinta la escena en `target` (firma compatible con `gpu_paint_with`).
    /// Color preservado (`LoadOp::Load`); depth propio limpiado cada frame.
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        (w, h): (u32, u32),
        cam: &CameraParams,
    ) {
        if w == 0 || h == 0 {
            return;
        }
        // SSAA: rendimos a (rw,rh) = SUPERSAMPLE× y bajamos al target real.
        let (rw, rh) = (w * SUPERSAMPLE, h * SUPERSAMPLE);
        let aspect = w as f32 / h as f32;
        let mvp = cam.mvp(aspect);

        // ¿Hay agua con planos? → reflexión planar multi-plano.
        let n_planes = self.water_planes.len().min(MAX_WATER_PLANES);
        let do_reflect = !self.water_batches.is_empty() && n_planes > 0;

        // Uniform: MVP + eye/fog + params(time,w,h,refl_strength) +
        // clip(water_z, clip_enable, 0, 0). `clip_enable=1` en el pase de
        // reflexión → el fragment descarta lo que está bajo ese plano de agua.
        let write_uniform = |buf: &wgpu::Buffer, mat: &Mat4, refl: f32, water_z: f32, clip: f32| {
            let mut bytes = Vec::with_capacity(112);
            for v in mat.to_cols_array() {
                bytes.extend_from_slice(&v.to_ne_bytes());
            }
            for v in [cam.x, cam.y, cam.eye_z, 1.0 / 2500.0] {
                bytes.extend_from_slice(&v.to_ne_bytes());
            }
            for v in [cam.time, rw as f32, rh as f32, refl] {
                bytes.extend_from_slice(&v.to_ne_bytes());
            }
            for v in [water_z, clip, 0.0, 0.0] {
                bytes.extend_from_slice(&v.to_ne_bytes());
            }
            queue.write_buffer(buf, 0, &bytes);
        };
        // SUPAY_NO_CLIP desactiva el clip bajo el agua (para A/B del efecto).
        let clip_enable = if std::env::var_os("SUPAY_NO_CLIP").is_some() { 0.0 } else { 1.0 };
        write_uniform(&self.uniform_buf, &mvp, if do_reflect { 1.0 } else { 0.0 }, 0.0, 0.0);
        if do_reflect {
            for i in 0..n_planes {
                let z = self.water_planes[i];
                let mvp_r = mvp * reflect_across_z(z);
                write_uniform(&self.refl_uniform_bufs[i], &mvp_r, 0.0, z, clip_enable);
            }
        }

        // Uniform del cielo: yaw, pitch, fov_x, aspect.
        let mut sky_bytes = Vec::with_capacity(16);
        for v in [cam.yaw, cam.pitch, cam.fov_x, aspect] {
            sky_bytes.extend_from_slice(&v.to_ne_bytes());
        }
        queue.write_buffer(&self.sky_uniform_buf, 0, &sky_bytes);

        let sprite_draws = self.build_sprite_draws(device, queue, cam);
        let weapon_draws = self.build_weapon_draws(device, queue, rw, rh);

        self.ensure_scene(device, rw, rh);
        if do_reflect {
            self.ensure_refls(device, rw, rh, n_planes);
        }

        // --- Pases de reflexión: uno por plano de agua. El mundo seco +
        //     sprites con la MVP reflejada de ese plano (clippeando bajo él). ---
        for i in 0..(if do_reflect { n_planes } else { 0 }) {
            let refl = &self.refl_targets[i];
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("doom3d-reflect-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &refl.color_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.05, g: 0.06, b: 0.10, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &refl.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &self.refl_uniform_bgs[i], &[]);
            rp.set_bind_group(2, &self.white.bind_group, &[]); // sin reflexión anidada
            for b in &self.batches {
                let tex = self.textures.get(&b.key).unwrap_or(&self.white);
                rp.set_bind_group(1, &tex.bind_group, &[]);
                rp.set_vertex_buffer(0, b.buffer.slice(..));
                rp.draw(0..b.count, 0..1);
            }
            for (tex, buf, count) in &sprite_draws {
                rp.set_bind_group(1, &tex.bind_group, &[]);
                rp.set_vertex_buffer(0, buf.slice(..));
                rp.draw(0..*count, 0..1);
            }
        }

        // --- Pase principal (a la escena supersampleada) ---
        let scene = self.scene.as_ref().unwrap();
        let scene_color = &scene.color_view;
        let depth_view = &scene.depth_view;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("doom3d-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: scene_color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    // La escena se rinde fresca cada frame; el cielo cubre el
                    // fondo, este clear sólo se ve en gaps (azul nocturno).
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.05, g: 0.06, b: 0.10, a: 1.0 }),
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
        // Cielo primero (fondo, sin escribir depth).
        if let Some(sky) = &self.sky_tex {
            pass.set_pipeline(&self.sky_pipeline);
            pass.set_bind_group(0, &self.sky_uniform_bg, &[]);
            pass.set_bind_group(1, &sky.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bg, &[]);
        // Mundo seco: group 2 = white (no muestrea reflexión).
        pass.set_bind_group(2, &self.white.bind_group, &[]);
        for b in &self.batches {
            let tex = self.textures.get(&b.key).unwrap_or(&self.white);
            pass.set_bind_group(1, &tex.bind_group, &[]);
            pass.set_vertex_buffer(0, b.buffer.slice(..));
            pass.draw(0..b.count, 0..1);
        }
        // Agua: frame de animación por tiempo (8 tics/frame a 35 Hz) + el mapa
        // de reflexión de SU plano (group 2).
        let frame_idx = (cam.time * (35.0 / 8.0)).max(0.0) as usize;
        for b in &self.water_batches {
            let refl_bg = if do_reflect {
                &self.refl_targets[b.plane.min(n_planes - 1)].bind_group
            } else {
                &self.white.bind_group
            };
            pass.set_bind_group(2, refl_bg, &[]);
            let n = b.frames.len().max(1);
            let tex = &b.frames[frame_idx % n];
            pass.set_bind_group(1, &tex.bind_group, &[]);
            pass.set_vertex_buffer(0, b.buffer.slice(..));
            pass.draw(0..b.count, 0..1);
        }
        // Sprites (billboards).
        for (tex, buf, count) in &sprite_draws {
            pass.set_bind_group(1, &tex.bind_group, &[]);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..*count, 0..1);
        }
        // Arma en mano (overlay 2D), encima de todo.
        if !weapon_draws.is_empty() {
            pass.set_pipeline(&self.overlay_pipeline);
            for (tex, buf) in &weapon_draws {
                pass.set_bind_group(0, &tex.bind_group, &[]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..6, 0..1);
            }
        }
        drop(pass);

        // --- Blit de bajada SSAA: la escena supersampleada → target real,
        //     promediada con filtro lineal (antialiasing). ---
        let blit_bg = &self.scene.as_ref().unwrap().blit_bg;
        let mut blit = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("doom3d-blit"),
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
        blit.set_pipeline(&self.blit_pipeline);
        blit.set_bind_group(0, blit_bg, &[]);
        blit.draw(0..3, 0..1);
    }

    /// Asegura el target de escena supersampleado (color + depth + bind group
    /// para el blit) a `(w,h)`. Recrea al cambiar de tamaño.
    fn ensure_scene(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if matches!(&self.scene, Some(s) if s.w == w && s.h == h) {
            return;
        }
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("doom3d-scene-color"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let color_view = color.create_view(&Default::default());
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("doom3d-scene-depth"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        let blit_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("doom3d-scene-blit-bg"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.scene = Some(SceneTarget { w, h, color_view, depth_view, blit_bg });
    }

    /// Asegura `n` render targets de reflexión (color+depth+bind group) al
    /// tamaño `(w,h)`. Recrea todos si cambió el tamaño o el conteo.
    fn ensure_refls(&mut self, device: &wgpu::Device, w: u32, h: u32, n: usize) {
        let ok = self.refl_targets.len() == n
            && self.refl_targets.iter().all(|r| r.w == w && r.h == h);
        if ok {
            return;
        }
        self.refl_targets.clear();
        for _ in 0..n {
            let color = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("doom3d-refl-color"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let color_view = color.create_view(&Default::default());
            let depth = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("doom3d-refl-depth"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let depth_view = depth.create_view(&Default::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("doom3d-refl-bg"),
                layout: &self.tex_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&color_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.refl_targets.push(ReflTarget { w, h, color_view, depth_view, bind_group });
        }
    }

    /// Construye los quads clip-space del arma en mano (weapon + flash) según
    /// `sx/sy` del psprite, igual que el HUD del renderer viejo.
    fn build_weapon_draws(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
    ) -> Vec<(Arc<GpuTexture>, wgpu::Buffer)> {
        const DOOM_VIEW_W: f32 = 320.0;
        const DOOM_VIEW_H: f32 = 200.0;
        const WEAPON_TOP: f32 = 32.0;
        let mut out = Vec::new();
        let Some(atlas) = self.atlas.clone() else {
            return out;
        };
        let scale = (w as f32 / DOOM_VIEW_W).min(h as f32 / DOOM_VIEW_H);
        // weapon primero, weapon_flash encima (muzzle).
        for weap in [self.weapon.clone(), self.weapon_flash.clone()] {
            if !weap.active {
                continue;
            }
            let Some((patch, mirror)) = atlas.sprite_patch(weap.sprite, weap.frame, 1) else {
                continue;
            };
            if patch.width == 0 || patch.height == 0 {
                continue;
            }
            // Cache reusa el mismo HashMap que los sprites del mundo.
            let key = (weap.sprite, weap.frame & 0x1F, 1u8);
            let tex = if let Some(t) = self.sprite_tex.get(&key) {
                t.clone()
            } else {
                let t = Arc::new(upload_texture(
                    device,
                    queue,
                    &self.tex_layout,
                    &self.sampler,
                    &patch.rgba,
                    patch.width as u32,
                    patch.height as u32,
                ));
                self.sprite_tex.insert(key, t.clone());
                t
            };

            let pw = patch.width as f32 * scale;
            let ph = patch.height as f32 * scale;
            let cx = w as f32 * 0.5 + weap.sx * scale;
            let left = cx - pw * 0.5;
            let top = h as f32 - ph + (weap.sy - WEAPON_TOP) * scale;
            let right = left + pw;
            let bottom = top + ph;
            // Pixel → clip space.
            let to_clip = |px: f32, py: f32| (px / w as f32 * 2.0 - 1.0, 1.0 - py / h as f32 * 2.0);
            let (lx, ty) = to_clip(left, top);
            let (rx, by) = to_clip(right, bottom);
            let (ul, ur) = if mirror { (1.0, 0.0) } else { (0.0, 1.0) };
            // pos[2] + uv[2] por vértice; dos triángulos.
            let v = |x: f32, y: f32, u: f32, vv: f32| [x, y, u, vv];
            let quad = [
                v(lx, ty, ul, 0.0),
                v(lx, by, ul, 1.0),
                v(rx, by, ur, 1.0),
                v(lx, ty, ul, 0.0),
                v(rx, by, ur, 1.0),
                v(rx, ty, ur, 0.0),
            ];
            let mut bytes = Vec::with_capacity(6 * 16);
            for vert in &quad {
                for f in vert {
                    bytes.extend_from_slice(&f.to_ne_bytes());
                }
            }
            let buf =
                create_buffer_init(device, "doom3d-weapon", wgpu::BufferUsages::VERTEX, &bytes);
            out.push((tex, buf));
        }
        out
    }

    /// Resuelve los sprites del snapshot a billboards orientados a la cámara,
    /// subiendo/cacheando su textura. Devuelve (textura, vbuf, n_vértices).
    fn build_sprite_draws(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cam: &CameraParams,
    ) -> Vec<(Arc<GpuTexture>, wgpu::Buffer, u32)> {
        use std::f32::consts::{FRAC_PI_4, FRAC_PI_8, TAU};
        let mut draws = Vec::new();
        let Some(atlas) = self.atlas.clone() else {
            return draws;
        };
        // Eje horizontal del billboard: perpendicular al forward de la cámara.
        let right = (-cam.yaw.sin(), cam.yaw.cos());
        let sprites = self.sprites.clone();
        for spr in &sprites {
            // Rotación 1..8: ángulo desde el que vemos el sprite vs su facing.
            let to_view = (cam.y - spr.y).atan2(cam.x - spr.x);
            let rel = (to_view - spr.angle + FRAC_PI_8).rem_euclid(TAU);
            let angle_arg = (rel / FRAC_PI_4).floor() as u8 % 8 + 1;
            let letter = spr.frame & 0x1F;
            let key = (spr.sprite, letter, angle_arg);

            let Some((patch, mirror)) = atlas.sprite_patch(spr.sprite, spr.frame, angle_arg)
            else {
                continue;
            };
            if patch.width == 0 || patch.height == 0 {
                continue;
            }
            // Subir/cachear la textura del patch.
            let tex = if let Some(t) = self.sprite_tex.get(&key) {
                t.clone()
            } else {
                let t = Arc::new(upload_texture(
                    device,
                    queue,
                    &self.tex_layout,
                    &self.sampler,
                    &patch.rgba,
                    patch.width as u32,
                    patch.height as u32,
                ));
                self.sprite_tex.insert(key, t.clone());
                t
            };

            let w = patch.width as f32;
            let h = patch.height as f32;
            let lo = patch.leftoffset as f32;
            let to = patch.topoffset as f32;
            let z_top = spr.z + to;
            let z_bot = z_top - h;
            // Columnas 0..w mapeadas a lo largo de `right`, anclando la columna
            // `lo` (el origen del patch) en (spr.x, spr.y).
            let lx = spr.x - right.0 * lo;
            let ly = spr.y - right.1 * lo;
            let rx = spr.x + right.0 * (w - lo);
            let ry = spr.y + right.1 * (w - lo);
            let (ul, ur) = if mirror { (1.0, 0.0) } else { (0.0, 1.0) };
            let fullbright = spr.frame & 0x80 != 0;
            let light = if fullbright {
                1.0
            } else {
                light_of(self.sector_lights.get(spr.sector as usize).copied().unwrap_or(160))
            };
            let tl = Vertex { pos: [lx, ly, z_top], uv: [ul, 0.0], light, kind: 0.0 };
            let tr = Vertex { pos: [rx, ry, z_top], uv: [ur, 0.0], light, kind: 0.0 };
            let bl = Vertex { pos: [lx, ly, z_bot], uv: [ul, 1.0], light, kind: 0.0 };
            let br = Vertex { pos: [rx, ry, z_bot], uv: [ur, 1.0], light, kind: 0.0 };
            let verts = [tl, bl, br, tl, br, tr];
            let mut bytes = Vec::with_capacity(6 * Vertex::SIZE as usize);
            for v in &verts {
                v.write(&mut bytes);
            }
            let buf = create_buffer_init(device, "doom3d-sprite", wgpu::BufferUsages::VERTEX, &bytes);
            draws.push((tex, buf, 6));
        }
        draws
    }
}

// =====================================================================
// Construcción de geometría
// =====================================================================

/// Lee 8 bytes null-padded de un slot de textura como nombre, o `None` si
/// está vacío (todo cero = "sin textura" de Doom).
fn tex_name(slot: &[u8; 8]) -> Option<String> {
    if slot.iter().all(|&b| b == 0) {
        return None;
    }
    let end = slot.iter().position(|&b| b == 0).unwrap_or(8);
    std::str::from_utf8(&slot[..end])
        .ok()
        .map(|s| s.to_string())
}

fn light_of(level: u8) -> f32 {
    // Doom va de oscuro (0) a claro (255). Mapeo lineal con un piso para que
    // los cuartos oscuros no queden negros del todo.
    0.12 + 0.88 * (level as f32 / 255.0)
}

/// Sube un mapa de (textura → vértices) a un Vec de batches GPU.
fn build_batches(device: &wgpu::Device, by_tex: HashMap<TexKey, Vec<Vertex>>) -> Vec<Batch> {
    let mut batches = Vec::new();
    for (key, verts) in by_tex {
        if verts.is_empty() {
            continue;
        }
        let mut bytes = Vec::with_capacity(verts.len() * Vertex::SIZE as usize);
        for v in &verts {
            v.write(&mut bytes);
        }
        let buffer =
            create_buffer_init(device, "doom3d-verts", wgpu::BufferUsages::VERTEX, &bytes);
        batches.push(Batch {
            key,
            buffer,
            count: verts.len() as u32,
        });
    }
    batches
}

/// Elige hasta [`MAX_WATER_PLANES`] alturas de agua distintas, priorizando las
/// de mayor área (más vértices). El resto de superficies se asociará al plano
/// más cercano vía [`nearest_plane`].
fn pick_water_planes(water: &WaterMap) -> Vec<f32> {
    // Área (n.º de vértices) por altura redondeada.
    let mut by_h: HashMap<i32, (u64, f32)> = HashMap::new();
    for ((_pic, hk), (h, verts)) in water {
        let e = by_h.entry(*hk).or_insert((0, *h));
        e.0 += verts.len() as u64;
    }
    let mut v: Vec<(u64, f32)> = by_h.into_values().collect();
    v.sort_by(|a, b| b.0.cmp(&a.0)); // mayor área primero
    v.truncate(MAX_WATER_PLANES);
    v.into_iter().map(|(_, h)| h).collect()
}

/// Índice del plano de `planes` más cercano a la altura `h` (0 si vacío).
fn nearest_plane(planes: &[f32], h: f32) -> usize {
    planes
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            (**a - h).abs().partial_cmp(&(**b - h).abs()).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Tamaño (w, h) en téxeles de una textura de pared, o `None` si no resuelve.
fn wall_tex_dims(atlas: &WadAtlas, name: &str) -> Option<(f32, f32)> {
    atlas
        .wall_texture(name)
        .map(|t| (t.width as f32, t.height as f32))
}

/// Emite los quads de una pared. Cada linedef se procesa por lado: el lado
/// frente (front_sector como "cerca") y, si es two-sided, el lado de atrás.
fn build_walls(snap: &SceneSnapshot, atlas: &WadAtlas, out: &mut HashMap<TexKey, Vec<Vertex>>) {
    for wall in snap.walls.iter() {
        // Lado frente.
        emit_wall_side(
            snap,
            atlas,
            out,
            wall.x1,
            wall.y1,
            wall.x2,
            wall.y2,
            wall.front_sector,
            wall.back_sector,
            wall.flags,
            [&wall.textures[0], &wall.textures[1], &wall.textures[2]],
            wall.tex_x_offsets[0],
            wall.tex_y_offsets[0],
        );
        // Lado de atrás (two-sided): se mira en sentido opuesto, así que
        // invertimos v1/v2 y usamos las texturas/offsets del back side.
        if wall.back_sector != NO_SECTOR {
            emit_wall_side(
                snap,
                atlas,
                out,
                wall.x2,
                wall.y2,
                wall.x1,
                wall.y1,
                wall.back_sector,
                wall.front_sector,
                wall.flags,
                [&wall.textures[3], &wall.textures[4], &wall.textures[5]],
                wall.tex_x_offsets[1],
                wall.tex_y_offsets[1],
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_wall_side(
    snap: &SceneSnapshot,
    atlas: &WadAtlas,
    out: &mut HashMap<TexKey, Vec<Vertex>>,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    near_idx: u32,
    far_idx: u32,
    flags: u32,
    texs: [&[u8; 8]; 3], // [mid, upper, lower]
    xoff: f32,
    yoff: f32,
) {
    let Some(near) = snap.sectors.get(near_idx as usize) else {
        return;
    };
    let nf = near.floor_height;
    let nc = near.ceiling_height;
    let light = light_of(near.light_level);
    let len = ((x2 - x1).powi(2) + (y2 - y1).powi(2)).sqrt();

    let far = if far_idx != NO_SECTOR {
        snap.sectors.get(far_idx as usize)
    } else {
        None
    };
    let (far_floor, far_ceil) = (far.map(|f| f.floor_height), far.map(|f| f.ceiling_height));

    // Helper: resuelve dims + pegging (vía wall_v_top) y empuja el quad.
    let mut emit = |out: &mut HashMap<TexKey, Vec<Vertex>>,
                    slot: &[u8; 8],
                    kind: usize,
                    z_bot: f32,
                    z_top: f32| {
        let Some(name) = tex_name(slot) else {
            return;
        };
        let (tw, th) = wall_tex_dims(atlas, &name).unwrap_or((64.0, 64.0));
        // V (en téxeles) en el borde superior del slab, según pegging Doom.
        let v_top = crate::walls::wall_v_top(
            kind, flags, nf, nc, far_floor, far_ceil, z_top, th, yoff,
        );
        push_wall_quad(
            out, name, tw, th, x1, y1, x2, y2, z_bot, z_top, len, xoff, v_top, light,
        );
    };

    match far {
        None => {
            // Pared sólida: un quad piso→techo con la textura "mid" (kind 0).
            emit(out, texs[0], 0, nf, nc);
        }
        Some(far) => {
            // Lower (kind 2): el piso del far sube por encima del near.
            if far.floor_height > nf {
                emit(out, texs[2], 2, nf, far.floor_height);
            }
            // Upper (kind 1): el techo del far baja por debajo del near.
            if far.ceiling_height < nc {
                emit(out, texs[1], 1, far.ceiling_height, nc);
            }
            // Middle (kind 0): textura colgante (rejas/transparencias). El
            // shader descarta los téxeles con alpha < 0.5.
            let zb = nf.max(far.floor_height);
            let zt = nc.min(far.ceiling_height);
            if zt > zb {
                emit(out, texs[0], 0, zb, zt);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_wall_quad(
    out: &mut HashMap<TexKey, Vec<Vertex>>,
    tex_name: String,
    tex_w: f32,
    tex_h: f32,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    z_bot: f32,
    z_top: f32,
    len: f32,
    xoff: f32,
    v_top_texels: f32,
    light: f32,
) {
    // UV normalizadas al tamaño REAL de la textura (el sampler Repeat envuelve
    // las fracciones > 1). U a lo largo del linedef; V con el pegging de Doom
    // ya resuelto por `wall_v_top` (téxeles desde el borde superior).
    let tw = tex_w.max(1.0);
    let th = tex_h.max(1.0);
    let u0 = xoff / tw;
    let u1 = (xoff + len) / tw;
    let v_t = v_top_texels / th;
    let v_b = (v_top_texels + (z_top - z_bot)) / th;
    let verts = out.entry(TexKey::Wall(tex_name)).or_default();
    let tl = Vertex { pos: [x1, y1, z_top], uv: [u0, v_t], light, kind: 0.0 };
    let tr = Vertex { pos: [x2, y2, z_top], uv: [u1, v_t], light, kind: 0.0 };
    let bl = Vertex { pos: [x1, y1, z_bot], uv: [u0, v_b], light, kind: 0.0 };
    let br = Vertex { pos: [x2, y2, z_bot], uv: [u1, v_b], light, kind: 0.0 };
    // Dos triángulos (sin cull, el winding no importa).
    verts.extend_from_slice(&[tl, bl, br, tl, br, tr]);
}

/// Reconstruye el polígono convexo de cada subsector vía clipping de los
/// semiplanos del BSP y emite sus triángulos de piso y techo.
/// Mapa de superficies líquidas: clave (pic_idx del flat, altura redondeada)
/// → (altura real, vértices). Separar por altura permite un plano de
/// reflexión por nivel de agua.
type WaterMap = HashMap<(u16, i32), (f32, Vec<Vertex>)>;

fn build_flats(
    snap: &SceneSnapshot,
    atlas: &WadAtlas,
    out: &mut HashMap<TexKey, Vec<Vertex>>,
    water_out: &mut WaterMap,
) {
    if snap.nodes.is_empty() {
        return;
    }
    // Bounding box del mapa a partir de los vértices de las paredes (+margen).
    let (mut minx, mut miny, mut maxx, mut maxy) =
        (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for w in snap.walls.iter() {
        for (x, y) in [(w.x1, w.y1), (w.x2, w.y2)] {
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
    }
    if !minx.is_finite() {
        return;
    }
    let margin = 64.0;
    let bounds = [
        (minx - margin, miny - margin),
        (maxx + margin, miny - margin),
        (maxx + margin, maxy + margin),
        (minx - margin, maxy + margin),
    ];

    // Raíz del BSP: último nodo (convención Doom).
    let root = (snap.nodes.len() - 1) as u16;
    let mut planes: Vec<(f32, f32, f32, f32, bool)> = Vec::new();
    collect_subsectors(
        &snap.nodes,
        root,
        &bounds,
        &mut planes,
        snap,
        atlas,
        out,
        water_out,
    );
}

/// DFS del BSP acumulando semiplanos; en cada hoja, clippea el bounding box
/// por los semiplanos del camino y emite piso+techo del subsector.
#[allow(clippy::too_many_arguments)]
fn collect_subsectors(
    nodes: &[NodeSnap],
    child: u16,
    bounds: &[(f32, f32); 4],
    // (px, py, dx, dy, front_side): el subsector está del lado `front_side`.
    planes: &mut Vec<(f32, f32, f32, f32, bool)>,
    snap: &SceneSnapshot,
    atlas: &WadAtlas,
    out: &mut HashMap<TexKey, Vec<Vertex>>,
    water_out: &mut WaterMap,
) {
    if child & NF_SUBSECTOR != 0 {
        let ssidx = (child & !NF_SUBSECTOR) as usize;
        emit_subsector_flats(ssidx, bounds, planes, snap, atlas, out, water_out);
        return;
    }
    let Some(node) = nodes.get(child as usize) else {
        return;
    };
    for (idx, front) in [(node.children[0], true), (node.children[1], false)] {
        planes.push((
            node.partition_x,
            node.partition_y,
            node.partition_dx,
            node.partition_dy,
            front,
        ));
        collect_subsectors(nodes, idx, bounds, planes, snap, atlas, out, water_out);
        planes.pop();
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_subsector_flats(
    ssidx: usize,
    bounds: &[(f32, f32); 4],
    planes: &[(f32, f32, f32, f32, bool)],
    snap: &SceneSnapshot,
    atlas: &WadAtlas,
    out: &mut HashMap<TexKey, Vec<Vertex>>,
    water_out: &mut WaterMap,
) {
    let Some(sub) = snap.subsectors.get(ssidx) else {
        return;
    };
    let Some(sec) = snap.sectors.get(sub.sector as usize) else {
        return;
    };
    // Clippear el bounding box por cada semiplano del camino raíz→hoja.
    let mut poly: Vec<(f32, f32)> = bounds.to_vec();
    for &(px, py, dx, dy, front) in planes {
        poly = clip_halfplane(&poly, px, py, dx, dy, front);
        if poly.len() < 3 {
            return;
        }
    }
    if poly.len() < 3 {
        return;
    }
    let light = light_of(sec.light_level);

    // ¿El piso es líquido? → kind=1 (lo ondula el shader) + va al mapa de
    // agua (se dibuja con reflexión planar).
    let is_liquid = std::env::var_os("SUPAY_WATER_ALL").is_some()
        || atlas
            .flat_name(sec.floor_pic)
            .map(|n| is_liquid_flat(&n))
            .unwrap_or(false);

    // Piso: triángulo-fan del polígono convexo, winding tal cual.
    if is_liquid {
        // Clave (pic, altura redondeada): un grupo por flat y nivel de agua.
        let key = (sec.floor_pic, sec.floor_height.round() as i32);
        let entry = water_out
            .entry(key)
            .or_insert_with(|| (sec.floor_height, Vec::new()));
        fan_flat(&mut entry.1, &poly, sec.floor_height, light, 1.0);
    } else {
        let floor = out.entry(TexKey::Flat(sec.floor_pic)).or_default();
        fan_flat(floor, &poly, sec.floor_height, light, 0.0);
    }

    // Techo: igual, a la altura del techo, salvo que sea cielo (kind 0 — los
    // techos no ondulan).
    let is_sky = snap.sky_pic != NO_SKY_PIC && sec.ceiling_pic == snap.sky_pic;
    if !is_sky {
        let ceil = out.entry(TexKey::Flat(sec.ceiling_pic)).or_default();
        fan_flat(ceil, &poly, sec.ceiling_height, light, 0.0);
    }
}

/// Triángula un polígono convexo como fan a altura `z`. Los flats de Doom son
/// 64×64 alineados a la grilla mundial → UV = (x,y)/64. `kind` = 1.0 marca un
/// flat líquido (lo ondula el shader).
fn fan_flat(out: &mut Vec<Vertex>, poly: &[(f32, f32)], z: f32, light: f32, kind: f32) {
    let s = 1.0 / 64.0;
    let v = |(x, y): (f32, f32)| Vertex {
        pos: [x, y, z],
        uv: [x * s, y * s],
        light,
        kind,
    };
    for i in 1..poly.len() - 1 {
        out.push(v(poly[0]));
        out.push(v(poly[i]));
        out.push(v(poly[i + 1]));
    }
}

/// Secuencia de nombres de lump de la animación de flat de Doom para `base`
/// (NUKAGE1→[NUKAGE1,NUKAGE2,NUKAGE3], FWATER1→FWATER4, SLIME0X por grupos de
/// 4, …). Si el flat no anima, devuelve `[base]`. Los frames inexistentes en
/// el WAD se descartan luego al decodificar, así que la tabla puede ser
/// generosa.
fn liquid_anim_frames(base: &str) -> Vec<String> {
    let up = base.to_ascii_uppercase();
    let Some(ds) = up.find(|c: char| c.is_ascii_digit()) else {
        return vec![up];
    };
    let prefix = &up[..ds];
    let width = up.len() - ds;
    let num: u32 = up[ds..].parse().unwrap_or(0);

    // SLIME tiene 3 animaciones de 4 frames (01-04, 05-08, 09-12).
    if prefix == "SLIME" && (1..=12).contains(&num) {
        let gs = ((num - 1) / 4) * 4 + 1;
        return (gs..=gs + 3).map(|n| format!("SLIME{n:02}")).collect();
    }

    // (prefijo, primer_frame, último_frame).
    const ANIMS: &[(&str, u32, u32)] = &[
        ("NUKAGE", 1, 3),
        ("FWATER", 1, 4),
        ("SWATER", 1, 4),
        ("LAVA", 1, 4),
        ("BLOOD", 1, 3),
    ];
    for &(p, s, e) in ANIMS {
        if prefix == p && (s..=e).contains(&num) {
            return (s..=e)
                .map(|n| format!("{prefix}{n:0width$}", width = width))
                .collect();
        }
    }
    vec![up]
}

/// `true` si el nombre de flat corresponde a un líquido animable de Doom
/// (agua/nukage/lava/sangre/slime). Detección por prefijo, case-insensitive.
fn is_liquid_flat(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    const LIQUIDS: &[&str] = &[
        "NUKAGE", "FWATER", "SWATER", "BLOOD", "LAVA", "SLIME", "WATER",
    ];
    LIQUIDS.iter().any(|p| n.starts_with(p))
}

/// Clip Sutherland-Hodgman de un polígono convexo contra un semiplano. La
/// recta pasa por `(px,py)` con dirección `(dx,dy)`. `front=true` conserva el
/// lado `side < 0` (la convención front de `walk_bsp`); `false`, `side > 0`.
fn clip_halfplane(
    poly: &[(f32, f32)],
    px: f32,
    py: f32,
    dx: f32,
    dy: f32,
    front: bool,
) -> Vec<(f32, f32)> {
    // side(p) = dx*(y-py) - dy*(x-px). Conservamos side<=0 (front) o >=0.
    let side = |x: f32, y: f32| dx * (y - py) - dy * (x - px);
    let inside = |s: f32| if front { s <= 1e-4 } else { s >= -1e-4 };
    let mut out = Vec::with_capacity(poly.len() + 1);
    let n = poly.len();
    for i in 0..n {
        let (ax, ay) = poly[i];
        let (bx, by) = poly[(i + 1) % n];
        let sa = side(ax, ay);
        let sb = side(bx, by);
        let ia = inside(sa);
        let ib = inside(sb);
        if ia {
            out.push((ax, ay));
        }
        if ia != ib {
            // Intersección del segmento a→b con la recta (side = 0).
            let t = sa / (sa - sb);
            out.push((ax + (bx - ax) * t, ay + (by - ay) * t));
        }
    }
    out
}

// =====================================================================
// Helpers wgpu
// =====================================================================

fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    rgba: &[u8],
    w: u32,
    h: u32,
) -> GpuTexture {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("doom3d-tex"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // write_texture acepta cualquier bytes_per_row (no exige el alineado a 256
    // de las copias buffer→textura), así que subimos las filas tal cual.
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &rgba[..(w * h * 4) as usize],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * w),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&Default::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("doom3d-tex-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    GpuTexture { bind_group }
}

fn create_buffer_init(
    device: &wgpu::Device,
    label: &str,
    usage: wgpu::BufferUsages,
    data: &[u8],
) -> wgpu::Buffer {
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: data.len().max(4) as u64,
        usage,
        mapped_at_creation: true,
    });
    buf.slice(..).get_mapped_range_mut()[..data.len()].copy_from_slice(data);
    buf.unmap();
    buf
}

const WGSL: &str = r#"
struct U {
    mvp: mat4x4<f32>,
    eye: vec4<f32>,     // .xyz = cámara, .w = densidad de diminishing
    params: vec4<f32>,  // .x=time .y=w .z=h .w=fuerza de reflexión (0/1)
    clip: vec4<f32>,    // .x=water_z .y=clip_enable (1 en pase de reflexión)
};
@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(2) @binding(0) var reflmap: texture_2d<f32>;
@group(2) @binding(1) var reflsamp: sampler;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) light: f32,
    @location(2) dist: f32,
    @location(3) kind: f32,
    @location(4) world: vec3<f32>,  // xyz mundo (ondas del agua + clip de reflexión)
};

@vertex
fn vs(
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) light: f32,
    @location(3) kind: f32,
) -> VOut {
    var o: VOut;
    o.clip = u.mvp * vec4<f32>(pos, 1.0);
    o.uv = uv;
    o.light = light;
    o.dist = distance(pos, u.eye.xyz);
    o.kind = kind;
    o.world = pos;
    return o;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // Clip bajo el agua (sólo en el pase de reflexión): no reflejar la
    // geometría sumergida — produciría fugas por encima de la línea de agua.
    if (u.clip.y > 0.5 && in.world.z < u.clip.x - 0.5) {
        discard;
    }
    var uv = in.uv;
    let t = u.params.x;
    let is_water = in.kind > 0.5;

    if (is_water) {
        // Ondulación de la superficie: desplaza la UV con dos senos cruzados
        // en fase con la posición mundial → la textura del líquido ondea.
        let p = in.world * 0.06;
        uv += vec2<f32>(sin(p.y + t * 1.6), cos(p.x + t * 1.4)) * 0.06;
    }

    let c = textureSample(tex, samp, uv);
    if (c.a < 0.5) { discard; }

    // Light diminishing estilo Doom: lo lejano se oscurece (piso 0.22).
    let atten = clamp(1.0 - in.dist * u.eye.w, 0.22, 1.0);
    var col = c.rgb * in.light * atten;

    if (is_water) {
        // Reflexión planar real: muestrea el mapa de reflexión en la posición
        // de pantalla de este píxel, perturbada por la onda → reflejo ondeado
        // del mundo (paredes, sprites, cielo). `in.clip.xy` es el píxel del
        // framebuffer; lo normalizamos con params.yz (w,h).
        if (u.params.w > 0.5) {
            let p = in.world * 0.08;
            let ripple = vec2<f32>(sin(p.y + t * 1.6), cos(p.x + t * 1.4)) * 0.012;
            let suv = in.clip.xy / vec2<f32>(u.params.y, u.params.z) + ripple;
            let refl = textureSample(reflmap, reflsamp, clamp(suv, vec2<f32>(0.0), vec2<f32>(1.0))).rgb;
            // Fresnel aproximado: más reflejo cuanto más rasante la mirada.
            let fres = clamp(0.35 + 0.45 * (1.0 - clamp(in.dist / 600.0, 0.0, 1.0)), 0.25, 0.75);
            col = mix(col, refl, fres);
        }
        // Brillo especular móvil — destellos sobre el líquido.
        let q = in.world * 0.10;
        let s1 = pow(0.5 + 0.5 * sin(q.x + q.y * 0.7 + t * 2.2), 8.0);
        let s2 = pow(0.5 + 0.5 * sin(q.x * 1.7 - q.y + t * 1.5), 10.0);
        col += vec3<f32>(0.14, 0.20, 0.26) * (s1 + s2 * 0.7);
    }

    return vec4<f32>(col, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::{is_liquid_flat, liquid_anim_frames};

    #[test]
    fn anim_frames_nukage_fwater_slime() {
        assert_eq!(liquid_anim_frames("NUKAGE1"), ["NUKAGE1", "NUKAGE2", "NUKAGE3"]);
        assert_eq!(
            liquid_anim_frames("FWATER1"),
            ["FWATER1", "FWATER2", "FWATER3", "FWATER4"]
        );
        // SLIME por grupos de 4, preservando el ancho de 2 dígitos.
        assert_eq!(
            liquid_anim_frames("SLIME06"),
            ["SLIME05", "SLIME06", "SLIME07", "SLIME08"]
        );
        // Un flat seco / desconocido se queda solo (sin animación).
        assert_eq!(liquid_anim_frames("FLOOR0_5"), ["FLOOR0_5"]);
        assert_eq!(liquid_anim_frames("AQF001"), ["AQF001"]);
    }

    #[test]
    fn detecta_liquidos_doom_y_freedoom() {
        for n in ["NUKAGE1", "FWATER4", "SWATER1", "BLOOD3", "LAVA1", "SLIME05", "WATER"] {
            assert!(is_liquid_flat(n), "{n} debería ser líquido");
        }
    }

    #[test]
    fn rechaza_flats_secos() {
        for n in ["FLOOR0_5", "CEIL5_1", "RROCK17", "GRASS1", "MFLR8_3", "CRATOP1", "AQF001"] {
            assert!(!is_liquid_flat(n), "{n} NO debería ser líquido");
        }
    }
}

/// Blit de bajada SSAA: triángulo fullscreen que muestrea la escena
/// supersampleada (filtro lineal del sampler) y la escribe al target real.
const BLIT_WGSL: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var o: VOut;
    let c = p[vi];
    o.clip = vec4<f32>(c, 0.0, 1.0);
    o.uv = vec2<f32>(c.x * 0.5 + 0.5, 1.0 - (c.y * 0.5 + 0.5));
    return o;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
"#;

/// Overlay 2D en clip-space (el arma en mano): posiciones ya en clip, sólo
/// muestrea la textura con alpha-discard.
const OVERLAY_WGSL: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

struct OOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>) -> OOut {
    var o: OOut;
    o.clip = vec4<f32>(pos, 0.0, 1.0);
    o.uv = uv;
    return o;
}

@fragment
fn fs(in: OOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv);
    if (c.a < 0.5) { discard; }
    return c;
}
"#;

/// Backdrop de cielo: triángulo fullscreen + muestreo cilíndrico por yaw.
const SKY_WGSL: &str = r#"
struct SkyU { yaw: f32, pitch: f32, fov_x: f32, aspect: f32 };
@group(0) @binding(0) var<uniform> s: SkyU;
@group(1) @binding(0) var sky: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

const PI: f32 = 3.14159265;

struct SOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) scr: vec2<f32>,   // uv de pantalla, (0,0) = arriba-izquierda
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> SOut {
    // Triángulo grande que cubre la pantalla.
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var o: SOut;
    let c = p[vi];
    o.clip = vec4<f32>(c, 0.0, 1.0);
    o.scr = vec2<f32>(c.x * 0.5 + 0.5, 1.0 - (c.y * 0.5 + 0.5));
    return o;
}

@fragment
fn fs(in: SOut) -> @location(0) vec4<f32> {
    // Ángulo de la columna: centro = yaw, bordes ± fov/2. El cielo de Doom
    // tilea 4× por 360°, así que a 90° de FOV cubre ~1 tile de ancho.
    let colang = s.yaw - (in.scr.x - 0.5) * s.fov_x;
    let su = fract(colang / (2.0 * PI) * 4.0);
    // Vertical: la textura del cielo ocupa la franja SUPERIOR (~55% de la
    // pantalla), así el horizonte/montañas cae cerca del medio y se ve por
    // encima de las paredes (antes se comprimía abajo y quedaba tapado). El
    // pitch lo desplaza al mirar arriba/abajo.
    let sv = clamp(in.scr.y * 1.8 - s.pitch * 0.6, 0.0, 1.0);
    return textureSample(sky, samp, vec2<f32>(su, sv));
}
"#;
