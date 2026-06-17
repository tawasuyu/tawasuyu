//! Contenido de la escena: terreno voxel + atmósfera + un "monumento" malla
//! flotante. Aísla el *qué se muestra* del *cómo se maneja* (`main.rs`), así la
//! app puede ganar personalidad (más estructuras, entidades, reglas) tocando
//! sólo acá, sin reescribir el bucle.

use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{
    Atmosphere, Camera3d, Hud, HudQuad, Renderer3d, Scene3d, VoxelGrid, VoxelRenderer,
};
use llimphi_ui::llimphi_hal::wgpu;
use llimphi_voxel::{Critter, Player};

/// Paleta de bichos (ovejas, marrones, terrosos, celestes…).
const HERD_PALETTE: [[u8; 3]; 5] = [
    [236, 236, 232],
    [205, 170, 125],
    [120, 92, 70],
    [96, 132, 176],
    [206, 198, 96],
];

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
    /// Bichos que deambulan el terreno (misma física que el jugador). Se
    /// dibujan como cajas en el pase de ray-march (`VoxelRenderer::entities`).
    critters: Vec<Critter>,
    /// Overlay screen-space para la mira de primera persona.
    hud: Hud,
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

        // Manada esparcida por el interior, en una grilla con jitter
        // determinista (cada bicho su semilla → rumbos distintos).
        let margin = 16u32;
        let (cols, rows) = (7u32, 5u32);
        let mut critters = Vec::with_capacity((cols * rows) as usize);
        let mut k: u32 = 0x9e3779b9;
        let span_x = dim[0].saturating_sub(2 * margin).max(1);
        let span_z = dim[2].saturating_sub(2 * margin).max(1);
        for r in 0..rows {
            for c in 0..cols {
                // Jitter LCG por celda para que no quede una grilla perfecta.
                k = k.wrapping_mul(1664525).wrapping_add(1013904223);
                let jx = (k >> 16) % 9;
                k = k.wrapping_mul(1664525).wrapping_add(1013904223);
                let jz = (k >> 16) % 9;
                let x = margin + span_x * c / (cols - 1) + jx;
                let z = margin + span_z * r / (rows - 1) + jz;
                let color = HERD_PALETTE[(r * cols + c) as usize % HERD_PALETTE.len()];
                critters.push(Critter::spawn_on(&grid, x.min(dim[0] - 1), z.min(dim[2] - 1), color, k));
            }
        }

        let hud = Hud::new(device, FMT);

        let mut world = Self {
            scene: Scene3d::new(),
            voxel,
            monument,
            grid,
            dim,
            player,
            critters,
            hud,
        };
        // Calentar la manada para que arranque desparramada (no en grilla).
        for _ in 0..150 {
            world.tick(1.0 / 30.0);
        }
        world
    }

    /// Avanza un frame de vida: deambula la manada y vuelca sus cajas al
    /// renderer (la capa de entidades del ray-march). Barato (instancing
    /// analítico) — llamar cada frame, en cualquier modo.
    pub fn tick(&mut self, dt: f32) {
        for c in &mut self.critters {
            c.step(&self.grid, dt);
        }
        self.voxel.entities.clear();
        self.voxel.entities.extend(self.critters.iter().map(|c| c.entity()));
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

    /// Centro (en **mundo**) del bicho más cercano a `eye_world`, o `None` si no
    /// hay manada. Para encuadrar un bicho en una toma de primera persona.
    pub fn nearest_critter(&self, eye_world: Vec3) -> Option<Vec3> {
        let center = self.world_center();
        self.critters
            .iter()
            .map(|c| {
                let e = c.entity();
                Vec3::new(e.pos[0], e.pos[1], e.pos[2]) - center
            })
            .min_by(|a, b| {
                a.distance_squared(eye_world)
                    .total_cmp(&b.distance_squared(eye_world))
            })
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

    /// Pinta el HUD de primera persona encima de la escena (pase screen-space):
    /// la **mira** centrada + un panel con el **modo** y las **coordenadas** del
    /// jugador (lectura del propio `Player`, espacio de grilla). Llamar *después*
    /// de [`render`](Self::render), en modo explorar.
    pub fn draw_hud(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: (u32, u32),
    ) {
        let mut quads = HudQuad::crosshair(viewport, 9.0, 2.0, [1.0, 1.0, 1.0, 0.85]).to_vec();

        // Texto: modo + coordenadas. La grilla es entera, redondeamos a voxel.
        let p = self.player.pos;
        let coords = format!("X {} Y {} Z {}", p.x as i32, p.y as i32, p.z as i32);
        let px = 2.0; // pixel de glifo (10×14 por carácter)
        let (tx, ty) = (16.0, 14.0);
        let lh = 7.0 * px + 6.0; // alto de línea (glifo + interlínea)

        // Panel de fondo translúcido para legibilidad sobre cualquier terreno.
        let pw = HudQuad::text_width(&coords, px).max(HudQuad::text_width("EXPLORAR", px));
        quads.push(HudQuad {
            x: tx - 8.0,
            y: ty - 8.0,
            w: pw + 16.0,
            h: lh * 2.0 + 8.0,
            color: [0.0, 0.0, 0.0, 0.45],
        });
        quads.extend(HudQuad::text("EXPLORAR", tx, ty, px, [0.62, 0.95, 0.78, 0.95]));
        quads.extend(HudQuad::text(&coords, tx, ty + lh, px, [0.95, 0.97, 1.0, 0.95]));

        self.hud.render(device, queue, encoder, target, viewport, &quads);
    }
}
