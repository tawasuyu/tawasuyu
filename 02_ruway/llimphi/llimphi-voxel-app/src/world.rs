//! Contenido de la escena: terreno voxel + atmósfera + un "monumento" malla
//! flotante. Aísla el *qué se muestra* del *cómo se maneja* (`main.rs`), así la
//! app puede ganar personalidad (más estructuras, entidades, reglas) tocando
//! sólo acá, sin reescribir el bucle.

use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{Atmosphere, Camera3d, Renderer3d, Scene3d, VoxelRenderer};
use llimphi_ui::llimphi_hal::wgpu;

/// Formato de la textura intermedia de Llimphi (target de `gpu_paint_with`).
pub const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Mundo render-able: el motor voxel (terreno) + una malla de triángulos (el
/// monumento), compuestos por [`Scene3d`] en un depth compartido.
pub struct World {
    scene: Scene3d,
    voxel: VoxelRenderer,
    monument: Renderer3d,
    dim: [u32; 3],
}

impl World {
    /// Construye el mundo: terreno procedural (de `llimphi-voxel`) subido al
    /// brick pool, atmósfera diurna, y un cubo-malla coloreado como monumento.
    /// Necesita `device`/`queue` (de ahí que se construya perezoso, en la 1ª
    /// pintada GPU).
    pub fn build(device: &wgpu::Device, queue: &wgpu::Queue, dim_xz: u32, seed: u32) -> Self {
        let dy = (dim_xz * 4 / 10).max(48);
        let dim = [dim_xz, dy, dim_xz];

        let grid = llimphi_voxel::terrain(dim, seed);
        let mut voxel = VoxelRenderer::new(device, queue, FMT, &grid);
        voxel.sun_dir = [0.55, 0.5, 0.32];
        voxel.atmosphere = Atmosphere {
            sky_zenith: [64, 118, 196],
            sky_horizon: [202, 218, 236],
            fog_density: 0.5 / dim_xz as f32,
        };

        let monument = Renderer3d::new(device, FMT);

        Self {
            scene: Scene3d::new(),
            voxel,
            monument,
            dim,
        }
    }

    /// Centro de órbita sugerido (un poco sobre el nivel medio del mundo).
    pub fn focus(&self) -> Vec3 {
        Vec3::new(0.0, self.dim[1] as f32 * 0.10, 0.0)
    }

    /// Radio sugerido de cámara (para encuadrar el continente).
    pub fn orbit_dist(&self) -> f32 {
        self.dim[0] as f32 * 1.5
    }

    /// Posiciona el monumento: flota sobre el centro del mundo y gira con
    /// `angle` (rad). El mundo voxel ocupa `y ∈ [-dy/2, dy/2]`; lo dejamos a
    /// ~`0.45·dy` para que asome entre los picos (y se ocluya con ellos — prueba
    /// la convivencia voxel+malla en vivo).
    pub fn animate(&mut self, angle: f32) {
        let dy = self.dim[1] as f32;
        let size = self.dim[0] as f32 * 0.11;
        let y = dy * 0.45;
        let m = Mat4::from_translation(Vec3::new(0.0, y, 0.0))
            * Mat4::from_rotation_y(angle)
            * Mat4::from_scale(Vec3::splat(size));
        self.monument.set_model(m);
    }

    /// Compone terreno + monumento sobre `target` (depth compartido). Firma
    /// compatible con la closure de `gpu_paint_with` (más la cámara).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: (u32, u32),
        camera: &Camera3d,
    ) {
        self.scene.render(
            device,
            queue,
            encoder,
            target,
            viewport,
            camera,
            Some(&self.voxel),
            &[&self.monument],
        );
    }
}
