//! Vista voxel 3D de la app — el puente `dominium-voxel` → `llimphi-3d` montado
//! en el bucle Elm vía `gpu_paint_with`.
//!
//! [`View3d`] sostiene el estado GPU que **persiste entre frames** (el
//! `VoxelRenderer` no se puede clonar ni vive en el `Model` clonable): se guarda
//! en un `Arc<Mutex<View3d>>` que la closure de pintura captura. El renderer se
//! crea **perezosamente** en el primer paint (cuando el compositor entrega el
//! `device`), y de ahí en más se reusa; las actualizaciones del mundo se suben
//! incrementalmente con `sync` sobre una `VoxelGrid` persistente (patrón
//! `clear_all` del motor), sin reconstruir texturas.

use dominium_core::World;
use dominium_iso::ZWeights;
use dominium_voxel::{lemming_entities, voxelize_into, VoxelConfig};
use llimphi_3d::wgpu;
use llimphi_3d::{Atmosphere, Camera3d, Entity3d, VoxelGrid, VoxelRenderer};

/// Formato del target que el compositor de Llimphi entrega a `gpu_paint_with`
/// (el default del stack, igual que los ejemplos de `llimphi-3d`). El pipeline
/// del `VoxelRenderer` debe construirse con este mismo formato.
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Estado GPU persistente de la vista 3D. Vive en un `Arc<Mutex<_>>` fuera del
/// `Model` clonable.
pub(crate) struct View3d {
    /// Grilla voxel persistente — se re-rellena in-place al cambiar el mundo.
    grid: VoxelGrid,
    /// Renderer ray-march; `None` hasta el primer paint (necesita `device`).
    vr: Option<VoxelRenderer>,
    /// Entidades (lemmings) del último `revoxelize`, subidas en cada render.
    entities: Vec<Entity3d>,
    /// Hay cambios de terreno sin subir (la grilla quedó dirty tras `revoxelize`).
    needs_sync: bool,
}

impl View3d {
    /// Crea la vista con una grilla vacía de `[ancho, alto_max, alto]`. El
    /// renderer se difiere al primer paint.
    pub(crate) fn new(grid_w: u32, grid_h: u32, max_height: u32) -> Self {
        Self {
            grid: VoxelGrid::new([grid_w, max_height.max(2), grid_h]),
            vr: None,
            entities: Vec::new(),
            needs_sync: false,
        }
    }

    /// Re-voxeliza el terreno y recalcula las entidades desde `world`. No toca
    /// la GPU (eso ocurre en el próximo `render`): rellena la grilla persistente
    /// (que queda dirty) y guarda las entidades. Lo llama el `update` cuando el
    /// mundo cambió (al activar el 3D y, con throttle, mientras la sim corre).
    pub(crate) fn revoxelize(&mut self, world: &World, zw: &ZWeights, cfg: &VoxelConfig) {
        voxelize_into(&mut self.grid, world, zw, cfg);
        let (ents, _dropped) = lemming_entities(world, zw, cfg);
        self.entities = ents;
        self.needs_sync = true;
    }

    /// Pinta la escena voxel sobre `target`. Crea el renderer la primera vez,
    /// sube el delta de terreno si lo hay, refresca las entidades y ray-marchea.
    /// Firma compatible con la closure `gpu_paint_with` (más la cámara).
    pub(crate) fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: (u32, u32),
        camera: &Camera3d,
    ) {
        if self.vr.is_none() {
            // Primer paint: el `new` sube la grilla ya rellena → no hace falta
            // un sync extra este frame.
            let mut vr = VoxelRenderer::new(device, queue, FMT, &self.grid);
            vr.atmosphere = Atmosphere {
                sky_zenith: [44, 62, 104],
                sky_horizon: [150, 168, 196],
                fog_density: 0.004,
                god_rays: 0.0,
            };
            vr.sun_dir = normalize([0.45, 0.82, 0.35]);
            self.vr = Some(vr);
            self.grid.reset_dirty();
            self.needs_sync = false;
        } else if self.needs_sync {
            // Disjoint: `vr` toma prestado el campo `vr`; `grid` el campo `grid`.
            let vr = self.vr.as_mut().unwrap();
            vr.sync(queue, &mut self.grid);
            self.needs_sync = false;
        }
        let vr = self.vr.as_mut().unwrap();
        vr.entities = self.entities.clone();
        vr.render(device, queue, encoder, target, viewport, camera);
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / l, v[1] / l, v[2] / l]
}
