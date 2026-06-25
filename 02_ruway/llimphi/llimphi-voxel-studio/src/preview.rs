//! Preview 3D en vivo de un [`WorldRecipe`]: genera el `VoxelGrid` de la receta y
//! lo compone con [`Scene3d`] sobre el target del canvas, **confinado** al rect del
//! panel. Se **regenera** cuando cambia la "generación" (un contador que el editor
//! incrementa al tocar un parámetro / cambiar de mundo) o el `dim`.
//!
//! Además del terreno, sabe **posar actores**: guarda el grid para consultar la
//! altura del suelo ([`ground_at`](WorldPreview::ground_at)) y mantiene un pool de
//! [`Renderer3d`] para dibujar las mallas de los actores de una escena en vivo.

use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{Atmosphere, Camera3d, Renderer3d, Scene3d, Vertex3d, VoxelGrid, VoxelRenderer};
use llimphi_ui::llimphi_hal::wgpu;
use llimphi_voxel::{WorldRecipe, SCENE_SUN};

/// Formato de la textura intermedia de Llimphi (target de `gpu_paint_with`).
pub const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// El mundo de preview: terreno de la receta + atmósfera + actores posados.
pub struct WorldPreview {
    scene: Scene3d,
    voxel: VoxelRenderer,
    /// El grid generado (se conserva para consultar la altura del suelo al posar
    /// actores).
    grid: VoxelGrid,
    dim: [u32; 3],
    /// Generación de la receta con la que se construyó el grid actual.
    built_gen: u64,
    /// Columna de **mundo** `[wx, wz]` donde cae la esquina local `(0,0)` de la
    /// ventana actual. `[0, 0]` = mundo centrado en el origen (preview de un mundo /
    /// personaje); distinto cuando la ventana **sigue al reparto** de una escena
    /// (mundo infinito). Ver [`Self::ensure_window`] / [`Self::ground_at_world`].
    origin: [i32; 2],
    /// Pool de renderers de actor (uno por actor; la malla se re-sube por frame).
    actor_r: Vec<Renderer3d>,
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
        let grid = recipe.generate(dim);
        let voxel = Self::make_voxel(device, queue, &grid, dim);
        Self {
            scene: Scene3d::new(),
            voxel,
            grid,
            dim,
            built_gen: gen,
            origin: [0, 0],
            actor_r: Vec::new(),
        }
    }

    /// Arma el `VoxelRenderer` de un grid con la atmósfera diurna del editor.
    fn make_voxel(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: &VoxelGrid,
        dim: [u32; 3],
    ) -> VoxelRenderer {
        let mut voxel = VoxelRenderer::new(device, queue, FMT, grid);
        voxel.sun_dir = SCENE_SUN; // sol bajo: luz rasante, claroscuro; el plano Backlight mira hacia acá
        voxel.atmosphere = Atmosphere {
            sky_zenith: [64, 118, 196],
            sky_horizon: [202, 218, 236],
            fog_density: 1.1 / dim[0] as f32, // bruma de desierto: medio que dispersa → god rays legibles
            god_rays: 0.9, // haces de sol cruzando la niebla — sello anti-Minecraft
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
            self.grid = recipe.generate(dim);
            self.voxel = Self::make_voxel(device, queue, &self.grid, dim);
            self.dim = dim;
            self.built_gen = gen;
            self.origin = [0, 0]; // `generate` es centrado en el origen
        }
    }

    /// **Asegura la ventana de una escena en un mundo infinito**: regenera el grid
    /// si cambió la receta (`gen`) o el `origin` de ventana. A diferencia de
    /// [`Self::rebuild_if`] (mundo finito centrado), acá el `origin` lo fija el
    /// caller para que la ventana **siga al reparto** ([`window_origin_for_cast`]
    /// (llimphi_voxel::window_origin_for_cast)); como sólo regenera al cruzar un
    /// paso, es barato por cuadro. Para posar actores sobre el relieve usá
    /// [`Self::ground_at_world`] (toma coords de mundo, no de ventana).
    pub fn ensure_window(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        recipe: &WorldRecipe,
        gen: u64,
        origin: [i32; 2],
    ) {
        if gen != self.built_gen || origin != self.origin {
            self.grid = recipe.generate_window(self.dim, origin);
            self.voxel = Self::make_voxel(device, queue, &self.grid, self.dim);
            self.built_gen = gen;
            self.origin = origin;
        }
    }

    /// Regenera la ventana del mundo en un **origen** de grilla `[wx, wz]` (para
    /// volar un mundo infinito: el terreno es función pura de mundo, así que mover
    /// el origen scrollea relieve nuevo de forma continua). `fog` ajusta la niebla
    /// (más densa esconde los bordes de la ventana en un flythrough).
    pub fn set_window(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        recipe: &WorldRecipe,
        origin: [i32; 2],
        fog: f32,
    ) {
        self.grid = recipe.generate_window(self.dim, origin);
        self.voxel = Self::make_voxel(device, queue, &self.grid, self.dim);
        self.voxel.atmosphere.fog_density = fog;
        self.origin = origin;
    }

    /// Posición (espacio de grilla, igual que el render del voxel) del **suelo**
    /// sobre la columna `(gx, gz)`: pies un voxel por encima del terreno (o `y=0` si
    /// la columna está vacía). Para parar un actor sobre el relieve.
    pub fn ground_at(&self, gx: u32, gz: u32) -> Vec3 {
        let gx = gx.min(self.dim[0] - 1);
        let gz = gz.min(self.dim[2] - 1);
        let top = self.grid.height_at(gx, gz).map(|y| y as f32 + 1.0).unwrap_or(0.0);
        Vec3::new(gx as f32 + 0.5, top, gz as f32 + 0.5)
    }

    /// Igual que [`Self::ground_at`] pero tomando una columna de **mundo** `(wx,
    /// wz)`: la mapea a la ventana actual restándole el [`origin`](Self) y devuelve
    /// la posición en **espacio de ventana** (grilla `[0, dim]`, igual que
    /// `ground_at`) — el caller le resta `dim/2` para llevarla al espacio centrado
    /// del shader. Para posar actores de una escena en un mundo infinito: la columna
    /// se busca donde realmente cae en la ventana que sigue al reparto. Fuera de la
    /// ventana, se clampa al borde.
    pub fn ground_at_world(&self, wx: i32, wz: i32) -> Vec3 {
        let lx = (wx - self.origin[0]).clamp(0, self.dim[0] as i32 - 1) as u32;
        let lz = (wz - self.origin[1]).clamp(0, self.dim[2] as i32 - 1) as u32;
        let top = self.grid.height_at(lx, lz).map(|y| y as f32 + 1.0).unwrap_or(0.0);
        Vec3::new(lx as f32 + 0.5, top, lz as f32 + 0.5)
    }

    /// Compone **sólo el terreno** sobre `target`, confinado al rect del canvas.
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
            device, queue, encoder, target, viewport, rect, camera, Some(&self.voxel), &[],
        );
    }

    /// Compone terreno + **actores** (mallas `(model, vértices, índices)`) en el
    /// mismo depth compartido — para reproducir una escena. Mantiene el pool de
    /// renderers al tamaño del reparto.
    #[allow(clippy::too_many_arguments)]
    pub fn render_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: (u32, u32),
        rect: (f32, f32, f32, f32),
        camera: &Camera3d,
        actors: &[(Mat4, Vec<Vertex3d>, Vec<u16>)],
    ) {
        while self.actor_r.len() < actors.len() {
            self.actor_r.push(Renderer3d::new(device, FMT));
        }
        for (r, (model, v, i)) in self.actor_r.iter_mut().zip(actors) {
            r.set_geometry(device, v, i);
            r.set_model(*model);
        }
        let refs: Vec<&Renderer3d> = self.actor_r.iter().take(actors.len()).collect();
        self.scene.render_in(
            device, queue, encoder, target, viewport, rect, camera, Some(&self.voxel), &refs,
        );
    }
}
