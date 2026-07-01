//! Preview 3D en vivo de un [`WorldRecipe`]: genera el `VoxelGrid` de la receta y
//! lo compone con [`Scene3d`] sobre el target del canvas, **confinado** al rect del
//! panel. Se **regenera** cuando cambia la "generación" (un contador que el editor
//! incrementa al tocar un parámetro / cambiar de mundo) o el `dim`.
//!
//! Además del terreno, sabe **posar actores**: guarda el grid para consultar la
//! altura del suelo ([`ground_at`](WorldPreview::ground_at)) y mantiene un pool de
//! [`Renderer3d`] para dibujar las mallas de los actores de una escena en vivo.

use llimphi_3d::glam::{Mat4, Vec3, Vec4};
use llimphi_3d::{Atmosphere, Camera3d, Renderer3d, Scene3d, Vertex3d, VoxelGrid, VoxelRenderer};
use llimphi_ui::llimphi_hal::wgpu;
use llimphi_voxel::{
    CharSpec, Clip, EcuacionSim, FieldDef, GrowthSim, Habitante, MundoRender, Program, WaterSim,
    CELL_WATER, SCENE_SUN,
};

/// Formato de la textura intermedia de Llimphi (target de `gpu_paint_with`).
pub const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// **Densidad de bruma de una escena** (numerador; la densidad real es
/// `SCENE_FOG / dim[0]`). Más alta que la del preview de un mundo: la ventana
/// voxel sigue siendo una caja finita de `dim`, así que sin bruma su **borde** se
/// ve como una isla cuadrada flotando en el cielo. Con esto el borde (a ~`dim/2`
/// del reparto centrado) se funde con el horizonte → la escena se lee infinita,
/// mientras el reparto (cerca de la cámara) queda nítido. Es el mismo truco del
/// `--flythrough`.
pub const SCENE_FOG: f32 = 2.6;

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
    /// Simulación de agua en curso (ley Fluir), si está activa. `None` = terreno
    /// estático. Ver [`Self::ensure_sim`] / [`Self::sim_step`].
    sim: Option<WaterSim>,
    /// Crecimiento de plantas en curso (ley Crecer), si está activo.
    growth: Option<GrowthSim>,
    /// Ley autorada (Ecuacion) corriendo sobre un material del mundo, si está activa:
    /// recolorea sus celdas por el campo. Ver [`Self::ensure_ecuacion`] / [`Self::ecuacion_step`].
    ecuacion: Option<EcuacionSim>,
    /// Bandada de habitantes (conducta) deambulando sobre el grid, cada uno con el
    /// cuerpo del Ser que representa. Vacía = sin vida.
    manada: Vec<(Habitante, CharSpec)>,
    /// Contexto de *picking* (al dirigir): `(inv_view_proj, ojo, altura_del_plano)` en
    /// espacio centrado, para resolver un click de pantalla a una columna de mundo.
    pick: Option<(Mat4, Vec3, f32)>,
    /// Pool de renderers de actor (uno por actor; la malla se re-sube por frame).
    actor_r: Vec<Renderer3d>,
}

impl WorldPreview {
    /// Construye el preview generando el grid de `mr` (bioma+semilla+paleta) a `dim`,
    /// centrado en el origen.
    pub fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mr: &MundoRender,
        dim: [u32; 3],
        gen: u64,
    ) -> Self {
        let grid = mr.bioma.generate_window(mr.seed, &mr.palette, dim, [0, 0]);
        let voxel = Self::make_voxel(device, queue, &grid, dim);
        Self {
            scene: Scene3d::new(),
            voxel,
            grid,
            dim,
            built_gen: gen,
            origin: [0, 0],
            sim: None,
            growth: None,
            ecuacion: None,
            manada: Vec::new(),
            pick: None,
            actor_r: Vec::new(),
        }
    }

    /// Registra la cámara para *picking* (al dirigir): `inv_vp` = inversa de la
    /// `view_proj` con la que se renderiza, `eye` el ojo (centrado), `y0` la altura del
    /// plano de suelo donde caen los waypoints. Lo setea el preview de escena cada cuadro.
    pub fn set_pick(&mut self, inv_vp: Mat4, eye: Vec3, y0: f32) {
        self.pick = Some((inv_vp, eye, y0));
    }

    /// Resuelve un click (en NDC `[-1,1]`) a una **columna de mundo** `(gx, gz)`,
    /// intersectando el rayo de cámara con el plano de suelo `y0`. `None` si no hay
    /// contexto, el rayo no baja, o cae fuera del mundo.
    pub fn pick_world(&self, ndc_x: f32, ndc_y: f32) -> Option<(f32, f32)> {
        let (inv_vp, eye, y0) = self.pick?;
        // Punto en el plano lejano, desproyectado a espacio centrado.
        let p = inv_vp * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
        if p.w.abs() < 1e-6 {
            return None;
        }
        let far = p.truncate() / p.w;
        let dir = (far - eye).normalize_or_zero();
        if dir.y.abs() < 1e-4 {
            return None; // rayo casi horizontal: no corta el plano
        }
        let t = (y0 - eye.y) / dir.y;
        if t <= 0.0 {
            return None; // el plano está detrás de la cámara
        }
        let hit = eye + dir * t; // espacio centrado
        let half = Vec3::new(self.dim[0] as f32, self.dim[1] as f32, self.dim[2] as f32) * 0.5;
        let gx = hit.x + half.x;
        let gz = hit.z + half.z;
        if gx < 0.0 || gz < 0.0 || gx >= self.dim[0] as f32 || gz >= self.dim[2] as f32 {
            return None;
        }
        Some((gx, gz))
    }

    /// Asegura una **bandada** a partir de `pobladores` (`(ser, cuántos)`): spawnea
    /// `Σ cuántos` habitantes sobre el grid (esparcidos alrededor del centro), cada uno
    /// con la conducta y el cuerpo de su Ser. Si ya hay esa cantidad, sólo refresca la
    /// conducta de cada uno (para que editar los parámetros se note en vivo). Tope de
    /// 16 para no recargar el preview.
    pub fn ensure_manada(&mut self, pobladores: &[(CharSpec, usize)]) {
        let total: usize = pobladores.iter().map(|(_, n)| n).sum::<usize>().min(16);
        if self.manada.len() != total {
            self.manada.clear();
            let [dx, _, dz] = self.dim;
            let (cx, cz) = (dx as i32 / 2, dz as i32 / 2);
            let mut k = 0u32;
            'fill: for (spec, n) in pobladores {
                for _ in 0..*n {
                    if self.manada.len() >= 16 {
                        break 'fill;
                    }
                    // Esparcido determinista alrededor del centro (±~18 voxels).
                    let ox = (k.wrapping_mul(7) % 37) as i32 - 18;
                    let oz = (k.wrapping_mul(13) % 37) as i32 - 18;
                    let x = (cx + ox).clamp(1, dx as i32 - 2) as u32;
                    let z = (cz + oz).clamp(1, dz as i32 - 2) as u32;
                    let h = Habitante::spawn(&self.grid, x, z, spec.conducta.clone(), 1 + k);
                    self.manada.push((h, spec.clone()));
                    k += 1;
                }
            }
        } else {
            // Misma cantidad: refrescar el spec (conducta/cuerpo/colores) de cada uno
            // desde los pobladores, en el mismo orden de llenado, para que las ediciones
            // en vivo (parámetros e impulsos autorados) se noten sin re-spawnear.
            let mut idx = 0usize;
            'refresh: for (spec, n) in pobladores {
                for _ in 0..*n {
                    if idx >= self.manada.len() {
                        break 'refresh;
                    }
                    let (h, stored) = &mut self.manada[idx];
                    *stored = spec.clone();
                    h.set_conducta(spec.conducta.clone());
                    idx += 1;
                }
            }
        }
    }

    /// Detiene la bandada.
    pub fn clear_manada(&mut self) {
        self.manada.clear();
    }

    /// `true` si hay bandada activa.
    pub fn tiene_manada(&self) -> bool {
        !self.manada.is_empty()
    }

    /// Avanza la bandada `dt` (cada uno ve a los demás y a la `amenaza`) y arma las
    /// **metas de render** (matriz + malla) de cada habitante con el cuerpo de su Ser,
    /// en espacio centrado del shader (grilla − dim/2). Caminan según su andar.
    pub fn manada_metas(&mut self, dt: f32, amenaza: Option<Vec3>) -> Vec<(Mat4, Vec<Vertex3d>, Vec<u16>)> {
        let posiciones: Vec<Vec3> = self.manada.iter().map(|(h, _)| h.pos()).collect();
        for (h, _) in &mut self.manada {
            h.step(&self.grid, &posiciones, amenaza, dt);
        }
        let half = Vec3::new(self.dim[0] as f32, self.dim[1] as f32, self.dim[2] as f32) * 0.5;
        self.manada
            .iter()
            .map(|(h, spec)| spec.to_meta(h.pos() - half, h.heading(), Clip::Walk, h.fase, None))
            .collect()
    }

    /// Arranca el **crecimiento** (ley Crecer) del material `planta`: esconde sus
    /// celdas del grid y las irá revelando de abajo hacia arriba a `velocidad`.
    pub fn ensure_growth(&mut self, queue: &wgpu::Queue, planta: [u8; 3], velocidad: f32) {
        if self.growth.is_none() {
            let g = GrowthSim::from_grid(&self.grid, planta, velocidad);
            for (pos, _) in g.cells() {
                self.grid.clear(pos[0], pos[1], pos[2]);
            }
            self.voxel.sync(queue, &mut self.grid);
            self.growth = Some(g);
        }
    }

    /// Detiene el crecimiento (el próximo `rebuild_if` repone las plantas completas).
    pub fn clear_growth(&mut self) {
        self.growth = None;
    }

    /// Revela el siguiente lote de celdas de planta y lo sube a la GPU. No-op sin
    /// crecimiento activo.
    pub fn growth_step(&mut self, queue: &wgpu::Queue) {
        let Self { growth, grid, voxel, .. } = self;
        let Some(g) = growth else { return };
        let batch = g.step();
        if batch.is_empty() {
            return;
        }
        for (pos, color) in batch {
            grid.set(pos[0], pos[1], pos[2], color);
        }
        voxel.sync(queue, grid);
    }

    /// Arranca una **ley autorada** (Ecuacion) sobre las celdas del material `color`
    /// (± `tol`), si no hay una ya corriendo. `campos` define el estado y `vis` el campo
    /// que tiñe el color. No mueve celdas: las recolorea.
    pub fn ensure_ecuacion(&mut self, color: [u8; 3], tol: i32, campos: &[FieldDef], vis: usize) {
        if self.ecuacion.is_none() {
            self.ecuacion = Some(EcuacionSim::from_grid(&self.grid, color, tol, campos.to_vec(), vis));
        }
    }

    /// Detiene la ley autorada (el próximo `rebuild_if` repone los colores originales).
    pub fn clear_ecuacion(&mut self) {
        self.ecuacion = None;
    }

    /// Avanza un paso de la ley autorada y **sube sólo las celdas recoloreadas** a la
    /// GPU. No-op si no hay una corriendo.
    pub fn ecuacion_step(&mut self, queue: &wgpu::Queue, program: &Program, params: &[f32]) {
        let Self { ecuacion, grid, voxel, .. } = self;
        let Some(sim) = ecuacion else { return };
        let changes = sim.step(program, params);
        if changes.is_empty() {
            return;
        }
        for (pos, color) in changes {
            grid.set(pos[0], pos[1], pos[2], color);
        }
        voxel.sync(queue, grid);
    }

    /// Arranca la **simulación de agua** (ley Fluir) desde el grid actual, si no hay
    /// una ya corriendo. `agua` es el color con que se identifican/pintan las celdas;
    /// `(gravedad, horizontal)` son los parámetros de la ley del material líquido.
    pub fn ensure_sim(&mut self, agua: [u8; 3], gravedad: f32, horizontal: f32) {
        if self.sim.is_none() {
            self.sim = Some(WaterSim::with_params(&self.grid, agua, gravedad, horizontal));
        }
    }

    /// Detiene la simulación (el próximo `rebuild_if` repone el terreno estático).
    pub fn clear_sim(&mut self) {
        self.sim = None;
    }

    /// Avanza un paso de la simulación de agua y **sube sólo lo cambiado** a la GPU
    /// (`VoxelRenderer::sync` sobre la caja dirty). No-op si no hay simulación.
    pub fn sim_step(&mut self, queue: &wgpu::Queue, agua: [u8; 3]) {
        let Self { sim, grid, voxel, .. } = self;
        let Some(sim) = sim else { return };
        for (pos, state) in sim.step() {
            if state == CELL_WATER {
                grid.set(pos[0], pos[1], pos[2], agua);
            } else {
                grid.clear(pos[0], pos[1], pos[2]);
            }
        }
        voxel.sync(queue, grid);
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
        mr: &MundoRender,
        dim: [u32; 3],
        gen: u64,
    ) {
        if gen != self.built_gen || dim != self.dim {
            self.grid = mr.bioma.generate_window(mr.seed, &mr.palette, dim, [0, 0]);
            self.voxel = Self::make_voxel(device, queue, &self.grid, dim);
            self.dim = dim;
            self.built_gen = gen;
            self.origin = [0, 0]; // centrado en el origen
            self.sim = None; // el grid se rehízo: cualquier simulación queda obsoleta
            self.growth = None;
            self.ecuacion = None;
            self.manada.clear();
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
        mr: &MundoRender,
        gen: u64,
        origin: [i32; 2],
    ) {
        if gen != self.built_gen || origin != self.origin {
            self.grid = mr.bioma.generate_window(mr.seed, &mr.palette, self.dim, origin);
            self.voxel = Self::make_voxel(device, queue, &self.grid, self.dim);
            self.built_gen = gen;
            self.origin = origin;
        }
        // Bruma de escena (cada cuadro, sólo setea un campo): disuelve el BORDE de la
        // ventana finita en el horizonte → la escena se ve infinita, no una isla
        // cuadrada. El preview de Mundos (rebuild_if) queda nítido, sin esto.
        self.voxel.atmosphere.fog_density = SCENE_FOG / self.dim[0] as f32;
    }

    /// Regenera la ventana del mundo en un **origen** de grilla `[wx, wz]` (para
    /// volar un mundo infinito: el terreno es función pura de mundo, así que mover
    /// el origen scrollea relieve nuevo de forma continua). `fog` ajusta la niebla
    /// (más densa esconde los bordes de la ventana en un flythrough).
    pub fn set_window(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mr: &MundoRender,
        origin: [i32; 2],
        fog: f32,
    ) {
        self.grid = mr.bioma.generate_window(mr.seed, &mr.palette, self.dim, origin);
        self.voxel = Self::make_voxel(device, queue, &self.grid, self.dim);
        self.voxel.atmosphere.fog_density = fog;
        self.origin = origin;
    }

    /// Sobrescribe **sol + atmósfera** del render voxel (para un look cinematográfico
    /// puntual, p.ej. la hora dorada de un flythrough). Aditivo: no toca el preview en
    /// vivo salvo que se llame. Aplicar **después** de [`set_window`](Self::set_window)
    /// (que recrea el voxel y resetea estas perillas a su default).
    pub fn set_lighting(&mut self, sun_dir: [f32; 3], atmosphere: Atmosphere) {
        self.voxel.sun_dir = sun_dir;
        self.voxel.atmosphere = atmosphere;
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
