//! Dimensiones / mundos paralelos (M5) — `MOTOR-VOXEL.md` §3.8.
//!
//! Una **dimensión = un mundo voxel independiente** con su propio grid, su sol,
//! su cielo (color de fondo) y sus entidades. "Viajar" = cambiar qué dimensión
//! renderiza la cámara (un portal = un `switch`). No agrega complejidad de motor
//! (cada dimensión reusa el `VoxelRenderer` sparse tal cual): es contenido.
//!
//! El [`Multiverse`] mantiene N dimensiones y la activa; cada una materializa su
//! `VoxelRenderer` (su brick pool) perezosamente la primera vez que se la pinta,
//! y queda "tibia" en memoria para que el switch sea instantáneo.

use crate::camera::Camera3d;
use crate::voxel::VoxelGrid;
use crate::voxel_renderer::{Atmosphere, Entity3d, VoxelRenderer};

/// Formato de la textura intermedia de Llimphi (target de `gpu_paint_with`).
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Un mundo voxel independiente.
pub struct Dimension {
    pub name: String,
    pub grid: VoxelGrid,
    /// Color de fondo (cielo) sugerido para la pasada vello base.
    pub sky: [u8; 3],
    /// Dirección hacia el sol de esta dimensión.
    pub sun_dir: [f32; 3],
    /// Atmósfera (cielo + niebla) de esta dimensión. Default = niebla off, así
    /// una dimensión sin configurar se comporta como en M5 (miss → discard).
    pub atmosphere: Atmosphere,
    /// Entidades (agentes) de esta dimensión; se copian al renderer por frame.
    pub entities: Vec<Entity3d>,
    renderer: Option<VoxelRenderer>,
}

impl Dimension {
    /// Dimensión nueva con cielo/sol por defecto y sin entidades.
    pub fn new(name: impl Into<String>, grid: VoxelGrid) -> Self {
        Self {
            name: name.into(),
            grid,
            sky: [18, 22, 32],
            sun_dir: [0.5, 1.0, 0.35],
            atmosphere: Atmosphere::default(),
            entities: Vec::new(),
            renderer: None,
        }
    }

    pub fn with_sky(mut self, sky: [u8; 3]) -> Self {
        self.sky = sky;
        self
    }
    pub fn with_sun(mut self, sun_dir: [f32; 3]) -> Self {
        self.sun_dir = sun_dir;
        self
    }
    /// Activa cielo + niebla propios para esta dimensión (el `render` los aplica
    /// al renderer). Con `fog_density > 0`, el motor pinta su propio cielo en los
    /// misses (ya no se ve el fondo vello).
    pub fn with_atmosphere(mut self, atmosphere: Atmosphere) -> Self {
        self.atmosphere = atmosphere;
        self
    }
    pub fn with_entities(mut self, entities: Vec<Entity3d>) -> Self {
        self.entities = entities;
        self
    }
}

/// Conjunto de dimensiones con una activa. La cámara siempre ve la activa.
pub struct Multiverse {
    dims: Vec<Dimension>,
    active: usize,
    format: wgpu::TextureFormat,
}

impl Multiverse {
    pub fn new(dims: Vec<Dimension>) -> Self {
        Self {
            dims,
            active: 0,
            format: FMT,
        }
    }

    /// Cambia el formato de color del target (default `Rgba8Unorm`, la
    /// intermedia de Llimphi). Sólo afecta a renderers aún no materializados.
    pub fn with_format(mut self, format: wgpu::TextureFormat) -> Self {
        self.format = format;
        self
    }

    pub fn count(&self) -> usize {
        self.dims.len()
    }
    pub fn active(&self) -> usize {
        self.active
    }
    pub fn active_name(&self) -> &str {
        &self.dims[self.active].name
    }
    pub fn names(&self) -> Vec<String> {
        self.dims.iter().map(|d| d.name.clone()).collect()
    }
    pub fn skies(&self) -> Vec<[u8; 3]> {
        self.dims.iter().map(|d| d.sky).collect()
    }

    /// Viaja a la dimensión `i` (no-op si fuera de rango).
    pub fn switch(&mut self, i: usize) {
        if i < self.dims.len() {
            self.active = i;
        }
    }
    pub fn next(&mut self) {
        self.active = (self.active + 1) % self.dims.len();
    }
    pub fn prev(&mut self) {
        self.active = (self.active + self.dims.len() - 1) % self.dims.len();
    }

    pub fn active_dim(&self) -> &Dimension {
        &self.dims[self.active]
    }
    pub fn active_dim_mut(&mut self) -> &mut Dimension {
        &mut self.dims[self.active]
    }

    /// Ray-marchea la dimensión activa sobre `target`. Materializa su brick pool
    /// la primera vez. Firma compatible con la closure de `gpu_paint_with`.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: (u32, u32),
        camera: &Camera3d,
    ) {
        let fmt = self.format;
        let d = &mut self.dims[self.active];
        let r = d
            .renderer
            .get_or_insert_with(|| VoxelRenderer::new(device, queue, fmt, &d.grid));
        r.sun_dir = d.sun_dir;
        r.atmosphere = d.atmosphere;
        r.entities = d.entities.clone();
        r.render(device, queue, encoder, target, viewport, camera);
    }

    /// Acceso al renderer ya materializado de la dimensión activa (para `sync`
    /// incremental de mutaciones, stats, etc.). `None` si aún no se pintó.
    pub fn active_renderer_mut(&mut self) -> Option<&mut VoxelRenderer> {
        self.dims[self.active].renderer.as_mut()
    }
}
