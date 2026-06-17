//! `WorldStream` — ventana voxel acotada que se desliza por un mundo procedural
//! **ilimitado** (M6: streaming). En vez de un grid fijo centrado en el origen,
//! mantenemos una ventana de `dim` voxels cuya esquina cae en una columna de
//! **mundo** `origin`; al caminar, [`WorldStream::follow`] reubica la ventana
//! (en pasos de `step`) y regenera el terreno con [`fill_terrain_window`]. Como
//! el relieve es función pura de mundo ([`column_height`]), las costuras encajan
//! y se puede caminar indefinidamente sin "muro" ni repetición.
//!
//! **Alcance (MVP):** la regeneración es de la ventana **entera** por cada paso
//! cruzado (no un *shift* parcial que sólo rellene el borde nuevo). Es O(ventana)
//! por reubicación, no por frame — sólo al cruzar un múltiplo de `step`. El grid
//! queda **dirty completo** para que `VoxelRenderer::sync` re-suba (o se
//! reconstruya el renderer). El *shift* incremental (copiar la zona común y
//! generar sólo la franja nueva) y el LOD del horizonte quedan como optimización
//! futura — ver `MOTOR-VOXEL.md` §7.

use llimphi_3d::VoxelGrid;

use std::collections::HashMap;

use crate::terrain::fill_terrain_window;

/// Ventana de mundo que sigue a un foco (la cámara/jugador) por un terreno
/// procedural ilimitado.
pub struct WorldStream {
    grid: VoxelGrid,
    dim: [u32; 3],
    seed: u32,
    /// Columna de mundo `[wx, wz]` donde cae la esquina local `(0,0)`.
    origin: [i32; 2],
    /// Granularidad de reubicación (voxels). Recentrar sólo cuando el foco se
    /// aleja del centro más de medio paso evita regenerar cada voxel caminado.
    step: i32,
    /// **Ediciones persistentes** por coordenada de **mundo** (`[wx, wy, wz]` →
    /// RGBA; `a = 0` = voxel cavado/aire). Como el terreno se regenera desde la
    /// semilla cada vez que la ventana vuelve a una zona, sin esto los cambios del
    /// jugador se perderían al alejarse y volver. Se re-aplican sobre el terreno
    /// fresco en cada `follow` (overlay). Es el estado a serializar para la
    /// persistencia CAS a disco (futuro): `mundo → BLAKE3(postcard(patch))`.
    edits: HashMap<[i32; 3], [u8; 4]>,
}

impl WorldStream {
    /// Crea una ventana de `dim` centrada en la columna de mundo `(center_x,
    /// center_z)` y la genera. `step` = granularidad de reubicación en voxels
    /// (típicamente el lado de un *brick*, p.ej. 8).
    pub fn new(dim: [u32; 3], seed: u32, center_x: i32, center_z: i32, step: u32) -> Self {
        let step = step.max(1) as i32;
        let origin = Self::origin_for(dim, center_x, center_z, step);
        let mut grid = VoxelGrid::new(dim);
        fill_terrain_window(&mut grid, origin, seed);
        Self { grid, dim, seed, origin, step, edits: HashMap::new() }
    }

    /// Origen (esquina) de ventana que centra la columna de mundo `(cx, cz)`,
    /// **snappeado** a `step` para que sólo cambie a saltos discretos.
    fn origin_for(dim: [u32; 3], cx: i32, cz: i32, step: i32) -> [i32; 2] {
        let half_x = dim[0] as i32 / 2;
        let half_z = dim[2] as i32 / 2;
        [
            snap(cx - half_x, step),
            snap(cz - half_z, step),
        ]
    }

    /// Reubica la ventana para centrar la columna de mundo `(cx, cz)` y, si el
    /// origen snappeado cambió, **regenera** el terreno. Devuelve `true` si hubo
    /// regeneración (el grid quedó dirty → el caller debe `sync`/reconstruir).
    pub fn follow(&mut self, cx: i32, cz: i32) -> bool {
        let want = Self::origin_for(self.dim, cx, cz, self.step);
        if want == self.origin {
            return false;
        }
        self.origin = want;
        fill_terrain_window(&mut self.grid, want, self.seed);
        // Re-aplicar las ediciones persistentes que caen en la ventana nueva
        // (overlay sobre el terreno fresco): así un cráter/estructura sobrevive
        // alejarse y volver.
        self.reapply_edits();
        true
    }

    /// Edita un voxel en coordenadas de **mundo** y lo **persiste**: `Some(rgb)`
    /// coloca un bloque sólido, `None` lo cava (aire). El cambio se registra en
    /// `edits` (sobrevive el regen del streaming) y, si el voxel cae en la ventana
    /// actual, se aplica al grid (que queda dirty → el caller hace `sync`/scroll).
    pub fn edit(&mut self, wx: i32, wy: i32, wz: i32, block: Option<[u8; 3]>) {
        let v = match block {
            Some(rgb) => [rgb[0], rgb[1], rgb[2], 255],
            None => [0, 0, 0, 0],
        };
        self.edits.insert([wx, wy, wz], v);
        self.apply_voxel(wx, wy, wz, v);
    }

    /// Cantidad de voxels editados persistidos (para reportar/serializar).
    pub fn edit_count(&self) -> usize {
        self.edits.len()
    }

    /// Serializa las ediciones a un blob **postcard** (lista de `([wx,wy,wz],
    /// RGBA)`), apto para guardar en la CAS de tawasuyu direccionado por su
    /// **BLAKE3** (`mundo → BLAKE3(blob)`) y recargar entre ejecuciones. Es la
    /// **persistencia a disco** del estado in-memory de [`Self::edit`].
    pub fn export_edits(&self) -> Vec<u8> {
        let mut v: Vec<([i32; 3], [u8; 4])> = self.edits.iter().map(|(&k, &val)| (k, val)).collect();
        // Orden canónico (el HashMap no es determinista) → mismas ediciones dan el
        // mismo blob y, por ende, la misma dirección BLAKE3 (dedup/integridad CAS).
        v.sort_unstable_by_key(|(k, _)| *k);
        postcard::to_allocvec(&v).expect("postcard ediciones")
    }

    /// Carga ediciones desde un blob de [`Self::export_edits`], las fusiona en el
    /// mapa persistente y las **re-aplica** sobre la ventana actual. Devuelve la
    /// cantidad cargada, o `None` si el blob no decodifica.
    pub fn import_edits(&mut self, bytes: &[u8]) -> Option<usize> {
        let v: Vec<([i32; 3], [u8; 4])> = postcard::from_bytes(bytes).ok()?;
        let n = v.len();
        for (k, val) in v {
            self.edits.insert(k, val);
        }
        self.reapply_edits();
        Some(n)
    }

    /// Aplica un voxel `v` (RGBA, `a=0`=aire) al grid si su coordenada de mundo
    /// cae en la ventana actual. No-op si está afuera (ya quedó en `edits`).
    fn apply_voxel(&mut self, wx: i32, wy: i32, wz: i32, v: [u8; 4]) {
        let lx = wx - self.origin[0];
        let lz = wz - self.origin[1];
        if lx < 0 || wy < 0 || lz < 0 {
            return;
        }
        let (lx, ly, lz) = (lx as u32, wy as u32, lz as u32);
        if lx < self.dim[0] && ly < self.dim[1] && lz < self.dim[2] {
            if v[3] > 0 {
                self.grid.set(lx, ly, lz, [v[0], v[1], v[2]]);
            } else {
                self.grid.clear(lx, ly, lz);
            }
        }
    }

    /// Re-aplica todas las ediciones que caen en la ventana actual sobre el grid.
    fn reapply_edits(&mut self) {
        // Split de borrows: iterar `edits` (inmutable) y mutar `grid`.
        let Self { edits, grid, origin, dim, .. } = self;
        for (&[wx, wy, wz], &v) in edits.iter() {
            let lx = wx - origin[0];
            let lz = wz - origin[1];
            if lx < 0 || wy < 0 || lz < 0 {
                continue;
            }
            let (lx, ly, lz) = (lx as u32, wy as u32, lz as u32);
            if lx < dim[0] && ly < dim[1] && lz < dim[2] {
                if v[3] > 0 {
                    grid.set(lx, ly, lz, [v[0], v[1], v[2]]);
                } else {
                    grid.clear(lx, ly, lz);
                }
            }
        }
    }

    /// Grid actual (para renderizar / `VoxelRenderer::new`).
    pub fn grid(&self) -> &VoxelGrid {
        &self.grid
    }

    /// Grid mutable (para `VoxelRenderer::sync`, que toma `&mut`).
    pub fn grid_mut(&mut self) -> &mut VoxelGrid {
        &mut self.grid
    }

    /// Columna de mundo de la esquina local `(0,0)`.
    pub fn origin(&self) -> [i32; 2] {
        self.origin
    }

    /// Origen en **voxels 3D** (`y = 0`: el mundo no scrollea en vertical), para
    /// pasar a [`VoxelRenderer::scroll_to`](llimphi_3d::VoxelRenderer::scroll_to).
    /// Si `step` es múltiplo de [`llimphi_3d::VOXEL_BRICK`] queda alineado a brick.
    pub fn origin_voxel(&self) -> [i32; 3] {
        [self.origin[0], 0, self.origin[1]]
    }

    /// Mapea una columna de **mundo** a coordenada **local** de ventana, o `None`
    /// si cae afuera. Para ubicar al jugador/entidades dentro del grid actual.
    pub fn world_to_local(&self, wx: i32, wz: i32) -> Option<(u32, u32)> {
        let lx = wx - self.origin[0];
        let lz = wz - self.origin[1];
        if lx >= 0 && lz >= 0 && lx < self.dim[0] as i32 && lz < self.dim[2] as i32 {
            Some((lx as u32, lz as u32))
        } else {
            None
        }
    }

    /// Mapea una coordenada **local** de ventana a su columna de **mundo**.
    pub fn local_to_world(&self, lx: u32, lz: u32) -> (i32, i32) {
        (self.origin[0] + lx as i32, self.origin[1] + lz as i32)
    }
}

/// Redondea `v` al múltiplo de `step` más cercano hacia abajo (floor con signo).
#[inline]
fn snap(v: i32, step: i32) -> i32 {
    (v.div_euclid(step)) * step
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_redondea_con_signo() {
        assert_eq!(snap(0, 8), 0);
        assert_eq!(snap(7, 8), 0);
        assert_eq!(snap(8, 8), 8);
        assert_eq!(snap(-1, 8), -8);
        assert_eq!(snap(-8, 8), -8);
        assert_eq!(snap(-9, 8), -16);
    }

    /// Caminar menos de un paso NO regenera; cruzar un paso SÍ.
    #[test]
    fn follow_regenera_solo_al_cruzar_un_paso() {
        let dim = [48, 32, 48];
        let mut s = WorldStream::new(dim, 9, 0, 0, 8);
        let o0 = s.origin();
        // Moverse dentro del mismo bloque de 8: sin regen.
        assert!(!s.follow(3, 2));
        assert_eq!(s.origin(), o0);
        // Cruzar varios pasos: regen y el origen siguió al foco.
        assert!(s.follow(64, 0));
        assert_ne!(s.origin(), o0);
        // El centro de ventana quedó cerca del foco (dentro de un paso).
        let cx = s.origin()[0] + dim[0] as i32 / 2;
        assert!((cx - 64).abs() <= 8, "ventana centrada en el foco (cx={cx})");
    }

    /// Tras seguir el foco, el contenido coincide con generar esa ventana de cero
    /// (la regeneración es completa y determinista por origen).
    #[test]
    fn contenido_tras_follow_igual_a_generacion_directa() {
        let dim = [40, 28, 40];
        let seed = 555;
        let mut s = WorldStream::new(dim, seed, 0, 0, 8);
        s.follow(120, -80);
        let origin = s.origin();

        let mut directo = VoxelGrid::new(dim);
        fill_terrain_window(&mut directo, origin, seed);

        for z in 0..dim[2] {
            for y in 0..dim[1] {
                for x in 0..dim[0] {
                    assert_eq!(
                        s.grid().get(x, y, z),
                        directo.get(x, y, z),
                        "discrepa en ({x},{y},{z})"
                    );
                }
            }
        }
    }

    /// Una edición sobrevive alejarse y volver: el terreno se regenera desde la
    /// semilla, pero el overlay de `edits` la re-aplica.
    #[test]
    fn una_edicion_sobrevive_alejarse_y_volver() {
        let dim = [48, 40, 48];
        let seed = 7;
        let mut s = WorldStream::new(dim, seed, 0, 0, 8);

        // Bloque magenta en lo alto del aire sobre la columna de mundo (0,0):
        // el terreno nunca pone magenta ahí (es aire), así el origen del píxel es
        // inequívocamente la edición.
        let (wx, wy, wz) = (0, dim[1] as i32 - 3, 0);
        let magenta = [255, 0, 255];
        s.edit(wx, wy, wz, Some(magenta));
        assert_eq!(s.edit_count(), 1);

        let read = |s: &WorldStream| {
            let (lx, lz) = s.world_to_local(wx, wz).expect("en ventana");
            s.grid().get(lx, wy as u32, lz)
        };
        assert_eq!(read(&s), Some([255, 0, 255, 255]), "presente recién editada");

        // Alejarse MUCHO (el voxel sale de la ventana → terreno regenerado, sin él).
        assert!(s.follow(2000, 2000));
        assert!(s.world_to_local(wx, wz).is_none(), "fuera de la ventana lejana");
        assert_eq!(s.edit_count(), 1, "la edición sigue persistida");

        // Volver al origen: el terreno se regenera Y el overlay re-aplica la edición.
        assert!(s.follow(0, 0));
        assert_eq!(read(&s), Some([255, 0, 255, 255]), "sobrevivió al regen");

        // Y sin la edición, ese voxel sería aire (prueba que no es terreno).
        let mut limpio = VoxelGrid::new(dim);
        fill_terrain_window(&mut limpio, s.origin(), seed);
        let (lx, lz) = s.world_to_local(wx, wz).unwrap();
        assert_eq!(limpio.get(lx, wy as u32, lz), Some([0, 0, 0, 0]), "terreno solo = aire");
    }

    /// Cavar (edit con `None`) también persiste: un voxel sólido del terreno
    /// queda vacío tras alejarse y volver.
    #[test]
    fn cavar_persiste() {
        let dim = [48, 40, 48];
        let seed = 13;
        let mut s = WorldStream::new(dim, seed, 0, 0, 8);

        // Buscar un voxel SÓLIDO del terreno en la columna central y cavarlo.
        let (lx0, lz0) = s.world_to_local(0, 0).unwrap();
        let h = s.grid().height_at(lx0, lz0).expect("columna con terreno");
        let (wx, wy, wz) = (0, h as i32, 0);
        assert!(s.grid().is_solid(lx0 as i32, wy, lz0 as i32), "sólido antes");

        s.edit(wx, wy, wz, None); // cavar
        let local_solid = |s: &WorldStream| {
            let (lx, lz) = s.world_to_local(wx, wz).unwrap();
            s.grid().is_solid(lx as i32, wy, lz as i32)
        };
        assert!(!local_solid(&s), "cavado tras editar");

        s.follow(3000, -3000);
        s.follow(0, 0);
        assert!(!local_solid(&s), "sigue cavado tras volver");
    }

    /// CAS a disco (simulada en memoria): exportar las ediciones de un mundo y
    /// re-importarlas en otro recién creado las restaura; el blob es canónico
    /// (misma dirección BLAKE3) sin importar el orden de edición.
    #[test]
    fn ediciones_round_trip_por_blob() {
        let dim = [48, 40, 48];
        let seed = 99;

        // Mundo A: unas cuantas ediciones (en distinto orden que B).
        let mut a = WorldStream::new(dim, seed, 0, 0, 8);
        a.edit(0, 30, 0, Some([10, 20, 30]));
        a.edit(-5, 12, 7, None);
        a.edit(3, 25, -2, Some([200, 100, 50]));
        let blob_a = a.export_edits();

        // Mundo B: las MISMAS ediciones en otro orden → mismo blob canónico.
        let mut b = WorldStream::new(dim, seed, 0, 0, 8);
        b.edit(3, 25, -2, Some([200, 100, 50]));
        b.edit(0, 30, 0, Some([10, 20, 30]));
        b.edit(-5, 12, 7, None);
        assert_eq!(a.export_edits(), b.export_edits(), "blob canónico (orden-indep.)");
        assert_eq!(blake3::hash(&blob_a), blake3::hash(&b.export_edits()), "misma dirección CAS");

        // Mundo C: vacío → importa el blob de A → recupera las 3 ediciones.
        let mut c = WorldStream::new(dim, seed, 0, 0, 8);
        assert_eq!(c.edit_count(), 0);
        assert_eq!(c.import_edits(&blob_a), Some(3));
        assert_eq!(c.edit_count(), 3);
        // Y el contenido coincide con A en la ventana (la edición magenta arriba).
        let (lx, lz) = c.world_to_local(0, 0).unwrap();
        assert_eq!(c.grid().get(lx, 30, lz), Some([10, 20, 30, 255]), "edición restaurada");

        // Blob inválido → None, sin romper.
        assert_eq!(c.import_edits(&[0xff, 0xff, 0xff]), None);
    }
}
