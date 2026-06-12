// Tests partidos en grupos por tamaño (Regla 1). Sin reordenar lógica.
mod grupo_01;
mod grupo_02;
mod grupo_03;
mod grupo_04;
mod grupo_05;

    use super::*;

    // -----------------------------------------------------------------
    // Fase 3.13: BSP back-to-front traversal
    // -----------------------------------------------------------------

    /// Construye un BSP de 2 hojas con partición a X=0 y dx=0, dy=1
    /// (línea vertical). Front (children[0]) = subsector 0 (lado +X).
    /// Back (children[1]) = subsector 1 (lado -X).
    fn simple_two_leaf_bsp() -> Vec<NodeSnap> {
        vec![NodeSnap {
            partition_x: 0.0,
            partition_y: 0.0,
            partition_dx: 0.0,
            partition_dy: 1.0,
            children: [NF_SUBSECTOR | 0, NF_SUBSECTOR | 1],
        }]
    }

    // -----------------------------------------------------------------
    // Fase 3.23: oclusión sectorial del muzzle boost
    // -----------------------------------------------------------------

    /// Construye un snapshot con el BSP de 2 hojas y un set de paredes
    /// que conectan el sector 0 (player room) al 1 vía two-sided, y
    /// dejan el sector 2 aislado (sólo paredes one-sided).
    fn snap_with_adjacency() -> SceneSnapshot {
        let mk_sector = || SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 160,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![mk_sector(), mk_sector(), mk_sector()]);
        // 2 subsectores: ss0 → sector 0 (player), ss1 → sector 1.
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 1, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());
        // Player en (-10, 0) ⇒ subsector 0 ⇒ sector 0 (ver
        // `subsector_at_point_picks_leaf_containing_point`).
        snap.player.x = -10.0;
        snap.player.y = 0.0;

        let wall = |fs: u32, bs: u32| WallSeg {
            x1: 0.0,
            y1: 0.0,
            x2: 0.0,
            y2: 0.0,
            front_sector: fs,
            back_sector: bs,
            flags: 0,
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
        };
        snap.walls = Arc::from(vec![
            // 0↔1 two-sided: el muzzle del player en 0 ilumina al 1.
            wall(0, 1),
            // Sector 2: sólo paredes one-sided ⇒ no conecta con player.
            wall(2, NO_SECTOR),
        ]);
        snap
    }

    // -----------------------------------------------------------------
    // Fase 3.24: BFS multi-hop + filtro por radio del bridge wall
    // -----------------------------------------------------------------

    /// Snap con una cadena de sectores 0→1→2→3 vía paredes two-sided
    /// + sector 5 colgado al jugador por un bridge wall lejano.
    /// Player en (-10, 0) ⇒ subsector 0 ⇒ sector 0.
    fn snap_with_chain() -> SceneSnapshot {
        let mk_sector = || SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 160,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![
            mk_sector(),
            mk_sector(),
            mk_sector(),
            mk_sector(),
            mk_sector(),
            mk_sector(),
        ]);
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 1, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());
        snap.player.x = -10.0;
        snap.player.y = 0.0;

        // Pared con midpoint en `(mx, my)` (segmento `[mx, my]→[mx, my]`
        // → midpoint trivial). Suficiente para el test del radius filter
        // del BFS — la geometría real no importa.
        let wall = |fs: u32, bs: u32, mx: f32, my: f32| WallSeg {
            x1: mx,
            y1: my,
            x2: mx,
            y2: my,
            front_sector: fs,
            back_sector: bs,
            flags: 0,
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
        };
        // Cadena 0↔1↔2↔3 con midpoints crecientes en X. Todos dentro
        // del radio salvo el último W23 a 200 unidades (aún dentro de
        // 384 desde player=-10 → distancia 210 < 384). El sector 3
        // queda fuera del lit por hops>MAX (2), no por radio.
        //
        // Bridge wall lejano 0↔5 con midpoint a 500 — fuera del radio
        // desde player=-10 (distancia 510 > 384). Sector 5 no entra al
        // lit pese a ser vecino directo.
        snap.walls = Arc::from(vec![
            wall(0, 1, 0.0, 0.0),     // hop 1: dist 10 → ✓
            wall(1, 2, 50.0, 0.0),    // hop 2: dist 60 → ✓
            wall(2, 3, 200.0, 0.0),   // hop 3 (no se llega por MAX=2)
            wall(0, 5, 500.0, 0.0),   // hop 1 pero bridge fuera del radio
        ]);
        snap
    }

    // -----------------------------------------------------------------
    // Fase 3.25: radio cumulativo por hop (Dijkstra-lite)
    // -----------------------------------------------------------------

    /// L-shape: dos paredes alineadas en codo donde el chequeo
    /// per-bridge contra el player (3.24) aprobaría ambas, pero el
    /// camino acumulativo player→W01→W12 supera el radio.
    ///
    /// - Player en (-10, 0) ⇒ subsector 0 ⇒ sector 0.
    /// - W01 midpoint (200, 0): dist desde player = 210 < 384.
    /// - W12 midpoint (200, 200): dist desde player ≈ 290 < 384 (3.24 lo aceptaba).
    /// - Cumulativo: 210 (player→W01) + 200 (W01→W12) = 410 > 384.
    ///   3.25 corta el camino y deja sec 2 fuera del lit set.
    fn snap_with_l_shape() -> SceneSnapshot {
        let mk_sector = || SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 160,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![mk_sector(), mk_sector(), mk_sector()]);
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 1, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());
        snap.player.x = -10.0;
        snap.player.y = 0.0;

        let wall = |fs: u32, bs: u32, mx: f32, my: f32| WallSeg {
            x1: mx,
            y1: my,
            x2: mx,
            y2: my,
            front_sector: fs,
            back_sector: bs,
            flags: 0,
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
        };
        snap.walls = Arc::from(vec![
            wall(0, 1, 200.0, 0.0),   // hop1 cumulative = 210
            wall(1, 2, 200.0, 200.0), // hop2 cumulative = 410 > 384
        ]);
        snap
    }

    /// Cadena donde cada hop suma poco al anterior aunque los midpoints
    /// estén lejos del jugador. Sólo es alcanzable correctamente si el
    /// algoritmo usa el midpoint del bridge previo como entry point del
    /// siguiente hop (no la posición del player).
    fn snap_with_chained_entry_points() -> SceneSnapshot {
        let mk_sector = || SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 160,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![mk_sector(), mk_sector(), mk_sector()]);
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 1, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());
        snap.player.x = 0.0;
        snap.player.y = 0.0;

        let wall = |fs: u32, bs: u32, mx: f32, my: f32| WallSeg {
            x1: mx,
            y1: my,
            x2: mx,
            y2: my,
            front_sector: fs,
            back_sector: bs,
            flags: 0,
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
        };
        // W01 mid (300, 0). hop_d = 300.
        // W12 mid (300, 50).
        //   - Si entry = (300, 0) (W01 mid): hop_d = 50. cumulativo sec2 = 350 < 384.
        //   - Si entry = (0, 0) (player): hop_d ≈ 304. cumulativo sec2 ≈ 604 > 384.
        snap.walls = Arc::from(vec![
            wall(0, 1, 300.0, 0.0),
            wall(1, 2, 300.0, 50.0),
        ]);
        snap
    }

    // -----------------------------------------------------------------
    // Fase 3.26: world point lights desde FF_FULLBRIGHT mobjs
    // -----------------------------------------------------------------

    /// Sprite helper para los tests de world lights.
    fn fb_sprite(x: f32, y: f32, frame: u8, sector: u32) -> SpriteSnap {
        SpriteSnap {
            x,
            y,
            z: 0.0,
            angle: 0.0,
            sprite: 0,
            frame,
            sector,
        }
    }

    // =================================================================
    // Fase 3.28 — Weapon rim-light desde world lights
    // =================================================================

    /// Helper: una `WorldLight` en `(x_cam, y_cam)` con el tinte dado.
    /// `lit_sectors: None` ⇒ aporta sin gating sectorial (path 3.27).
    fn rim_light(x: f32, y: f32, tint: (u8, u8, u8)) -> WorldLight {
        WorldLight {
            x_cam: x,
            y_cam: y,
            z_cam: 0.0,
            sector: NO_SECTOR,
            tint_rgb: tint,
            lit_sectors: None,
        }
    }

    // =================================================================
    // Fase 3.33 — BRDF para pisos y techos con z exportado
    // =================================================================

    /// Helper: luz con z_cam dado.
    fn plane_light(x: f32, y: f32, z: f32, tint: (u8, u8, u8)) -> WorldLight {
        WorldLight {
            x_cam: x,
            y_cam: y,
            z_cam: z,
            sector: NO_SECTOR,
            tint_rgb: tint,
            lit_sectors: None,
        }
    }

    // =================================================================
    // Fase 3.46 — Decals efímeros de impacto
    // =================================================================

    fn decal_test_setup() -> (Camera, Projection) {
        let cam = Camera::new(0.0, 0.0, 41.0, 0.0); // mira hacia +X
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        (cam, Projection::new(rect, 75_f32.to_radians()))
    }

    // =================================================================
    // Fase 3.39 — Sprite sample con patch.height real (textured path)
    // =================================================================

    /// Centro vertical del billboard en cam-space dado el `floor` (z del
    /// sector), `topoffset` del patch, su altura `h`, y `view_z` del
    /// jugador. Equivale a `((z_top + z_bot) * 0.5)` que usa el path
    /// texturizado (Fase 3.39).
    fn billboard_center_z_cam(floor: f32, topoffset: f32, h: f32, view_z: f32) -> f32 {
        let z_top = floor + topoffset - view_z;
        let z_bot = floor + topoffset - h - view_z;
        (z_top + z_bot) * 0.5
    }