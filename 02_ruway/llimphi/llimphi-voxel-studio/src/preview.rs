//! Preview 3D en vivo de un [`WorldRecipe`]: genera el `VoxelGrid` de la receta y
//! lo compone con [`Scene3d`] sobre el target del canvas. Se **regenera** cuando
//! cambia la "generación" (un contador que el editor incrementa al tocar un
//! parámetro) o el `dim` — así mover un slider repinta el mundo nuevo.
//!
//! Liviano y deliberadamente tonto: reconstruye el `VoxelRenderer` entero al
//! regenerar (un editor edita a ritmo humano; a lo sumo un rebuild por frame). No
//! hay jugador, manada ni monumento: sólo el terreno de la receta.

use llimphi_3d::{Atmosphere, Camera3d, Scene3d, VoxelRenderer};
use llimphi_ui::llimphi_hal::wgpu;
use llimphi_voxel::WorldRecipe;

/// Formato de la textura intermedia de Llimphi (target de `gpu_paint_with`).
pub const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// El mundo de preview: terreno de la receta + atmósfera, listo para componer.
pub struct WorldPreview {
    scene: Scene3d,
    voxel: VoxelRenderer,
    dim: [u32; 3],
    /// Generación de la receta con la que se construyó el grid actual.
    built_gen: u64,
}

impl WorldPreview {
    /// Construye el preview generando el grid de `recipe` a tamaño `dim`.
    pub fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        recipe: &WorldRecipe,
        dim: [u32; 3],
        gen: u64,
    ) -> Self {
        let voxel = Self::make_voxel(device, queue, recipe, dim);
        Self { scene: Scene3d::new(), voxel, dim, built_gen: gen }
    }

    /// Genera el grid de la receta y arma su `VoxelRenderer` con la atmósfera
    /// diurna del editor.
    fn make_voxel(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        recipe: &WorldRecipe,
        dim: [u32; 3],
    ) -> VoxelRenderer {
        let grid = recipe.generate(dim);
        let mut voxel = VoxelRenderer::new(device, queue, FMT, &grid);
        voxel.sun_dir = [0.55, 0.5, 0.32];
        voxel.atmosphere = Atmosphere {
            sky_zenith: [64, 118, 196],
            sky_horizon: [202, 218, 236],
            fog_density: 0.5 / dim[0] as f32,
        };
        voxel
    }

    /// Si `gen`/`dim` cambiaron desde el último build, **regenera** el mundo.
    pub fn rebuild_if(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        recipe: &WorldRecipe,
        dim: [u32; 3],
        gen: u64,
    ) {
        if gen != self.built_gen || dim != self.dim {
            self.voxel = Self::make_voxel(device, queue, recipe, dim);
            self.dim = dim;
            self.built_gen = gen;
        }
    }

    /// Compone el terreno sobre `target`, **confinado** al rect `(x, y, w, h)` del
    /// canvas (px del target) — así no pisa el chrome del editor alrededor.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: (u32, u32),
        rect: (f32, f32, f32, f32),
        camera: &Camera3d,
    ) {
        self.scene.render_in(
            device,
            queue,
            encoder,
            target,
            viewport,
            rect,
            camera,
            Some(&self.voxel),
            &[],
        );
    }
}
