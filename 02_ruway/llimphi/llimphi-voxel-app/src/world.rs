//! Contenido de la escena: terreno voxel + atmósfera + un "monumento" malla
//! flotante. Aísla el *qué se muestra* del *cómo se maneja* (`main.rs`), así la
//! app puede ganar personalidad (más estructuras, entidades, reglas) tocando
//! sólo acá, sin reescribir el bucle.

use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{Atmosphere, Camera3d, Renderer3d, Scene3d, VoxelGrid, VoxelRenderer};
use llimphi_ui::llimphi_hal::wgpu;
use llimphi_voxel::Player;

/// Formato de la textura intermedia de Llimphi (target de `gpu_paint_with`).
pub const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Mundo render-able + editable: el motor voxel (terreno) + una malla de
/// triángulos (el monumento), compuestos por [`Scene3d`] en un depth
/// compartido. Conserva el [`VoxelGrid`] para editar (romper/colocar) por
/// raycast y subir incremental con `sync`.
pub struct World {
    scene: Scene3d,
    voxel: VoxelRenderer,
    monument: Renderer3d,
    grid: VoxelGrid,
    dim: [u32; 3],
    /// Jugador en primera persona (modo "explorar"): camina el terreno con
    /// gravedad y colisión. En modo órbita simplemente se ignora.
    player: Player,
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

        // Jugador posado sobre la columna central del terreno.
        let player = Player::spawn_on(&grid, dim[0] / 2, dim[2] / 2);

        Self {
            scene: Scene3d::new(),
            voxel,
            monument,
            grid,
            dim,
            player,
        }
    }

    /// Avanza la física del jugador `dt` segundos con la caminata deseada
    /// (`wish`, horizontal, espacio de grilla) y `jump`. Devuelve el ojo del
    /// jugador en **mundo** (centrado en el origen, listo para la cámara).
    pub fn step_player(&mut self, wish: Vec3, jump: bool, dt: f32) -> Vec3 {
        self.player.step(&self.grid, wish, jump, dt);
        self.player.eye() - self.world_center()
    }

    /// Reposa al jugador sobre la columna central (al entrar a modo explorar,
    /// por si el terreno cambió por ediciones).
    pub fn respawn_player(&mut self) {
        self.player = Player::spawn_on(&self.grid, self.dim[0] / 2, self.dim[2] / 2);
    }

    /// Reposa al jugador sobre una columna concreta `(x, z)` (clamp al grid).
    /// Útil para encuadrar una vista en primera persona desde un mirador.
    pub fn spawn_player_at(&mut self, x: u32, z: u32) {
        let x = x.min(self.dim[0] - 1);
        let z = z.min(self.dim[2] - 1);
        self.player = Player::spawn_on(&self.grid, x, z);
    }

    /// Medio-`dim`: el offset entre espacio de grilla (`[0,dim]`) y mundo
    /// (centrado en el origen). `mundo = grilla - centro`.
    fn world_center(&self) -> Vec3 {
        Vec3::new(self.dim[0] as f32, self.dim[1] as f32, self.dim[2] as f32) * 0.5
    }

    /// Edita el mundo por **raycast** desde un origen/dirección (espacio de
    /// grilla = `eye_mundo + dim/2`): `build=false` cava un cráter en el voxel
    /// golpeado; `build=true` deposita un bloque en la celda vacía adyacente.
    /// Sube sólo lo tocado (`sync` incremental). Devuelve `true` si pegó algo.
    /// Se llama desde el hilo GPU (necesita `queue`).
    pub fn apply_edit(&mut self, queue: &wgpu::Queue, origin: [f32; 3], dir: [f32; 3], build: bool) -> bool {
        let max = self.dim[0] as f32 * 3.0;
        let Some(hit) = llimphi_voxel::raycast(&self.grid, origin, dir, max) else {
            return false;
        };
        if build {
            let [px, py, pz] = hit.place;
            let r = 1i32;
            for dz in -r..=r {
                for dy in -r..=r {
                    for dx in -r..=r {
                        let (x, y, z) = (px + dx, py + dy, pz + dz);
                        if x >= 0 && y >= 0 && z >= 0 {
                            self.grid.set(x as u32, y as u32, z as u32, [222, 184, 96]);
                        }
                    }
                }
            }
        } else {
            let [cx, cy, cz] = hit.cell;
            let r = 4i32;
            for dz in -r..=r {
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx * dx + dy * dy + dz * dz <= r * r {
                            let (x, y, z) = (cx + dx, cy + dy, cz + dz);
                            if x >= 0 && y >= 0 && z >= 0 {
                                self.grid.clear(x as u32, y as u32, z as u32);
                            }
                        }
                    }
                }
            }
        }
        self.voxel.sync(queue, &mut self.grid);
        true
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
