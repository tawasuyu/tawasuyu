#[allow(unused_imports)] use super::*;
#[allow(unused_imports)] use super::super::*;



    #[test]
    fn gather_world_lights_uses_default_tint_without_atlas() {
        // Sin atlas (modo stub), los lights caen al amarillo cálido.
        let mut snap = SceneSnapshot::empty(0);
        snap.sprites = Arc::from(vec![fb_sprite(64.0, 0.0, 0x80, 0)]);
        let cam = Camera::new(0.0, 0.0, 0.0, 0.0);
        let lights = gather_world_lights(&snap, &cam, None, false);
        assert_eq!(lights.len(), 1);
        assert_eq!(lights[0].tint_rgb, MUZZLE_TINT_RGB);
    }

    #[test]
    fn weapon_rim_boost_zero_at_player_with_no_world_lights() {
        // Sin world lights el arma no recibe tinte ambiente: identity.
        let boost = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &[]);
        assert_eq!(boost, ZERO_BOOST);
        let tint = sprite_shade_with_world(0.7, boost);
        assert!((tint[0] - 0.7).abs() < 1e-5);
        assert!((tint[1] - 0.7).abs() < 1e-5);
        assert!((tint[2] - 0.7).abs() < 1e-5);
    }

    #[test]
    fn weapon_rim_boost_blue_torch_skews_blue_at_player() {
        // Antorcha azul a 120 u del jugador (dentro de WORLD_LIGHT_RADIUS=192):
        // el boost en (0,0) tiene B > R y B > G — el arma se tinta azulada.
        let blue = (110, 160, 255);
        let lights = [rim_light(120.0, 0.0, blue)];
        let boost = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        assert!(
            boost[2] > boost[0] && boost[2] > boost[1],
            "blue torch debería skewear B: got [{}, {}, {}]",
            boost[0], boost[1], boost[2]
        );
        // Con shade=0.5 (cuarto oscuro, donde el rim importa) el tinte
        // final preserva la asimetría: el canal B queda por encima del R.
        // En shade=1.0 todos los canales saturan a 1.0 — el rim sólo
        // se ve cuando el arma está apagada por luz baja.
        let tint = sprite_shade_with_world(0.5, boost);
        assert!(tint[2] > tint[0], "tint[B] > tint[R] con shade bajo");
    }

    #[test]
    fn weapon_rim_boost_red_fireball_skews_red_at_player() {
        // BAL1 imp fireball a 80 u del jugador: el boost tiene R > G > B.
        let red = (255, 130, 60);
        let lights = [rim_light(80.0, 0.0, red)];
        let boost = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        assert!(
            boost[0] > boost[1] && boost[1] > boost[2],
            "fireball debería skewear R > G > B: got [{}, {}, {}]",
            boost[0], boost[1], boost[2]
        );
    }

    #[test]
    fn weapon_rim_boost_zero_when_light_beyond_radius() {
        // Una luz fuera del radio (`WORLD_LIGHT_RADIUS_WORLD`) no aporta
        // boost al arma — el rim queda neutro aunque haya antorchas
        // lejanas en línea de vista.
        let blue = (110, 160, 255);
        let r = WORLD_LIGHT_RADIUS_WORLD + 1.0;
        let lights = [rim_light(r, 0.0, blue)];
        let boost = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(boost, ZERO_BOOST);
    }

    #[test]
    fn weapon_full_bright_bypasses_rim_boost() {
        // Cuando el frame del arma tiene FF_FULLBRIGHT, el render usa
        // `[shade, shade, shade]` y *no* sprite_shade_with_world — el
        // destello del fogonazo domina y subsume el ambiente. Validamos
        // que el path normal en cuarto oscuro (shade=0.5) sí preserva
        // la asimetría per-canal, mientras el path full_bright es
        // grayscale: `[1, 1, 1]` independiente del boost.
        let blue = (110, 160, 255);
        let lights = [rim_light(120.0, 0.0, blue)];
        let boost = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        // Path normal: tint asimétrico per-canal en shade bajo.
        let normal_tint = sprite_shade_with_world(0.5, boost);
        assert!(
            normal_tint[2] > normal_tint[0],
            "path normal debería tener B>R con boost azul + shade bajo"
        );
        // Path full_bright: el render *no* llama a sprite_shade_with_world,
        // usa `[shade, shade, shade]` directo — grayscale.
        let full_bright_tint = [1.0_f32, 1.0, 1.0];
        assert_eq!(full_bright_tint[0], full_bright_tint[1]);
        assert_eq!(full_bright_tint[1], full_bright_tint[2]);
    }

    // =================================================================
    // Fase 3.29 — Oclusión sectorial de world lights
    // =================================================================

    #[test]
    fn lit_sectors_from_arbitrary_source_includes_source_sector() {
        // Generalización: arrancar la BFS desde un sector arbitrario
        // (p. ej. el sector que aloja a un proyectil FF_FULLBRIGHT)
        // siempre incluye al sector origen, y al vecino conectado por
        // two-sided. El sector 2 (sólo one-sided) queda excluido.
        let snap = snap_with_adjacency();
        let lit = compute_lit_sectors_from(&snap, 0.0, 0.0, 1, WORLD_LIGHT_RADIUS_WORLD);
        assert!(lit.contains(&1), "sector origen siempre en el set");
        assert!(lit.contains(&0), "vecino directo via two-sided incluido");
        assert!(!lit.contains(&2), "sector aislado fuera del set");
    }

    #[test]
    fn world_lights_boost_rgb_skips_light_when_surf_not_in_lit_sectors() {
        // Luz con lit_sectors restringido a {1}. Superficie en sector 2 ⇒
        // la luz no aporta. Misma luz evaluada con surf_sector=1 sí aporta.
        let mut lit = HashSet::new();
        lit.insert(1_u32);
        let light = WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 1,
            tint_rgb: (255, 255, 255),
            lit_sectors: Some(Arc::new(lit)),
        };
        let lights = [light];
        let blocked = world_lights_boost_rgb_cam(0.0, 0.0, 2, &lights);
        assert_eq!(blocked, ZERO_BOOST, "sector no listado ⇒ luz oculta");
        let visible = world_lights_boost_rgb_cam(0.0, 0.0, 1, &lights);
        assert!(visible[0] > 0.0, "sector listado ⇒ luz aporta");
    }

    #[test]
    fn world_lights_boost_rgb_passes_light_when_lit_sectors_is_none() {
        // Backward-compat 3.27: lit_sectors=None ⇒ surf_sector ignorado.
        // Una luz sin gating aporta en cualquier sector.
        let light = WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: (255, 255, 255),
            lit_sectors: None,
        };
        let lights = [light];
        let b0 = world_lights_boost_rgb_cam(0.0, 0.0, 0, &lights);
        let b9 = world_lights_boost_rgb_cam(0.0, 0.0, 999, &lights);
        assert_eq!(b0, b9, "sin gating, surf_sector no cambia el boost");
        assert!(b0[0] > 0.0);
    }

    #[test]
    fn gather_world_lights_computes_lit_sectors_when_occlusion_enabled() {
        // Con BSP + un sprite FF_FULLBRIGHT en sector 1 + oclusión on,
        // la luz cachea un set que incluye al menos su sector origen.
        let mut snap = snap_with_adjacency();
        // Sprite en (0, 0): cae sobre el seam pero el sector lo fijamos
        // explícitamente a 1 (igual al snap_with_adjacency wall 0↔1).
        snap.sprites = Arc::from(vec![fb_sprite(0.0, 0.0, 0x80, 1)]);
        let cam = Camera::new(snap.player.x, snap.player.y, 0.0, 0.0);
        let lights = gather_world_lights(&snap, &cam, None, true);
        assert_eq!(lights.len(), 1);
        let set = lights[0]
            .lit_sectors
            .as_ref()
            .expect("oclusión on con BSP ⇒ Some(set)");
        assert!(set.contains(&1), "set incluye sector origen");
    }

    #[test]
    fn gather_world_lights_skips_occlusion_when_disabled_or_no_bsp() {
        // (a) oclusión off ⇒ lit_sectors = None para todas.
        let mut snap = snap_with_adjacency();
        snap.sprites = Arc::from(vec![fb_sprite(0.0, 0.0, 0x80, 1)]);
        let cam = Camera::new(snap.player.x, snap.player.y, 0.0, 0.0);
        let off = gather_world_lights(&snap, &cam, None, false);
        assert_eq!(off.len(), 1);
        assert!(off[0].lit_sectors.is_none(), "oclusión off ⇒ None");
        // (b) oclusión on pero sin BSP (snapshot sintético sin nodes)
        // ⇒ lit_sectors = None (el caller cae al comportamiento 3.27).
        let mut bare = SceneSnapshot::empty(0);
        bare.sprites = Arc::from(vec![fb_sprite(20.0, 0.0, 0x80, 0)]);
        let cam2 = Camera::new(0.0, 0.0, 0.0, 0.0);
        let no_bsp = gather_world_lights(&bare, &cam2, None, true);
        assert_eq!(no_bsp.len(), 1);
        assert!(no_bsp[0].lit_sectors.is_none(), "sin BSP ⇒ None");
    }

    // =================================================================
    // Fase 3.30 — Rim direccional del arma
    // =================================================================

    #[test]
    fn weapon_rim_directional_full_intensity_in_front() {
        // Luz a 80u en +X_cam (frente al jugador). Sin tinte real (luz
        // blanca pura) ⇒ cos(0)=1 ⇒ att=1.0 ⇒ boost igual al omni.
        let white = (255, 255, 255);
        let lights = [rim_light(80.0, 0.0, white)];
        let omni = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, false);
        let dir = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, true);
        // Diferencia despreciable ⇒ ambos paths coinciden al frente.
        for ch in 0..3 {
            assert!(
                (omni[ch] - dir[ch]).abs() < 1e-5,
                "frente debería igualar omni: canal {} omni={} dir={}",
                ch,
                omni[ch],
                dir[ch]
            );
        }
    }

    #[test]
    fn weapon_rim_directional_attenuates_lights_behind() {
        // Luz a 80u en -X_cam (detrás del jugador). cos=-1 ⇒
        // att=(0.5-0.5).max(0.3)=0.3. Boost direccional debería ser
        // estrictamente menor que omni (que ignora dirección).
        let white = (255, 255, 255);
        let lights = [rim_light(-80.0, 0.0, white)];
        let omni = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, false);
        let dir = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, true);
        // Cada canal direccional debería ser ~0.3 del omni (el piso).
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                assert!(dir[ch] < omni[ch], "canal {} debería atenuar atrás", ch);
                let ratio = dir[ch] / omni[ch];
                assert!(
                    (ratio - WEAPON_RIM_AMBIENT_FLOOR).abs() < 1e-4,
                    "ratio canal {} = {} debería ser ≈ piso {}",
                    ch,
                    ratio,
                    WEAPON_RIM_AMBIENT_FLOOR
                );
            }
        }
    }

    #[test]
    fn weapon_rim_directional_side_lights_use_half() {
        // Luz a 80u en +Y_cam (lateral derecho). cos=0 ⇒ att=0.5.
        // El boost lateral debe quedar ~ a mitad del omni.
        let white = (255, 255, 255);
        let lights = [rim_light(0.0, 80.0, white)];
        let omni = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, false);
        let dir = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, true);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                let ratio = dir[ch] / omni[ch];
                assert!(
                    (ratio - 0.5).abs() < 1e-4,
                    "lateral debería ser 0.5 del omni: canal {} ratio {}",
                    ch,
                    ratio
                );
            }
        }
    }

    #[test]
    fn weapon_rim_directional_disabled_equals_omni() {
        // Toggle off ⇒ direccional==omni para cualquier configuración.
        let red = (255, 130, 60);
        let blue = (110, 160, 255);
        let lights = [
            rim_light(120.0, 0.0, red),
            rim_light(-60.0, 90.0, blue),
            rim_light(0.0, -150.0, (255, 255, 200)),
        ];
        let omni = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, false);
        let baseline = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(
            omni, baseline,
            "directional=false debe ser bit-identical al path 3.29"
        );
    }

    #[test]
    fn weapon_rim_directional_handles_zero_distance() {
        // Luz exactamente en el jugador (raro pero posible: psprite
        // FF_FULLBRIGHT del propio fogonazo si entrara por error). El
        // cos no está definido; degradamos a att=1.0 y evitamos NaN.
        let white = (255, 255, 255);
        let lights = [rim_light(0.0, 0.0, white)];
        let dir = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, true);
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} no NaN/Inf", ch);
            assert!(dir[ch] > 0.0, "luz pegada al player aporta full");
        }
    }

    // =================================================================
    // Fase 3.31 — Rim direccional de mobj sprites
    // =================================================================

    #[test]
    fn sprite_rim_directional_front_light_matches_omni() {
        // Sprite a (200, 0) en cam-space (frente al jugador). Una luz
        // a (100, 0) está entre el jugador y el sprite — desde el
        // sprite, la luz queda en dirección -X (hacia la cámara), que
        // es exactamente su fake-normal. cos(0)=1 ⇒ att=1.0 ⇒ el path
        // direccional debería coincidir bit-a-bit con el omni.
        let white = (255, 255, 255);
        let lights = [rim_light(100.0, 0.0, white)];
        let omni = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        let dir = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            assert!(
                (omni[ch] - dir[ch]).abs() < 1e-5,
                "luz front al sprite debería igualar omni: canal {} omni={} dir={}",
                ch, omni[ch], dir[ch]
            );
        }
    }

    #[test]
    fn sprite_rim_directional_back_light_falls_to_floor() {
        // Sprite a (200, 0), luz a (260, 0) (detrás del sprite desde
        // la cámara). Desde el sprite la luz está en +X (lejos de la
        // cámara), opuesto a la fake-normal (-1, 0). cos=-1 ⇒ att=floor.
        let white = (255, 255, 255);
        let lights = [rim_light(260.0, 0.0, white)];
        let omni = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        let dir = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                let ratio = dir[ch] / omni[ch];
                assert!(
                    (ratio - SPRITE_RIM_AMBIENT_FLOOR).abs() < 1e-4,
                    "back-light debería caer al piso ambient: canal {} ratio {}",
                    ch, ratio
                );
            }
        }
    }

    #[test]
    fn sprite_rim_directional_side_light_uses_half() {
        // Sprite a (200, 0), luz a (200, 60) (al costado del sprite,
        // perpendicular al eje player→sprite). Desde el sprite la
        // dirección a la luz es (0, 1) — perpendicular a la normal
        // (-1, 0). cos=0 ⇒ att=0.5.
        let white = (255, 255, 255);
        let lights = [rim_light(200.0, 60.0, white)];
        let omni = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        let dir = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                let ratio = dir[ch] / omni[ch];
                assert!(
                    (ratio - 0.5).abs() < 1e-4,
                    "lateral debería ser 0.5 del omni: canal {} ratio {}",
                    ch, ratio
                );
            }
        }
    }

    #[test]
    fn sprite_rim_directional_disabled_equals_omni_for_arbitrary_lights() {
        // Toggle off ⇒ direccional debe coincidir con `world_lights_boost_rgb_cam`
        // para cualquier configuración de luces (tres luces, tintes
        // distintos, posiciones mezcladas alrededor del sprite).
        let red = (255, 130, 60);
        let blue = (110, 160, 255);
        let warm = (255, 220, 140);
        let lights = [
            rim_light(180.0, 30.0, red),
            rim_light(120.0, -40.0, blue),
            rim_light(240.0, 80.0, warm),
        ];
        let omni = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        let baseline = world_lights_boost_rgb_cam(200.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(omni, baseline, "directional=false debe ser bit-identical al path 3.29");
    }

    #[test]
    fn sprite_rim_directional_degenerates_safely_at_camera() {
        // Sprite exactamente en el origen del cam-space (degenerado:
        // billboard sin normal definida). Caemos al path omni dentro
        // del helper direccional para evitar NaN. Resultado finito y
        // ≥ 0 por canal.
        let white = (255, 255, 255);
        let lights = [rim_light(50.0, 0.0, white)];
        let dir = world_lights_boost_rgb_for_sprite_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} no NaN/Inf", ch);
        }
        // Y debería coincidir con el omni (porque caemos al fallback).
        let omni = world_lights_boost_rgb_for_sprite_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        assert_eq!(dir, omni, "degenerado ⇒ fallback omni");
    }

    // =================================================================
    // Fase 3.32 — Rim direccional para paredes
    // =================================================================

    #[test]
    fn wall_normal_cam_orients_toward_camera() {
        // Pared horizontal a la derecha del player: endpoints (100, -50)
        // y (100, 50). Midpoint (100, 0). Normal candidates ±(1, 0) (no
        // ±(0, 1) — ojo: perpendicular a la dirección (0, 100)).
        // La que apunta toward camera (origen) es (-1, 0).
        let n = wall_normal_cam(100.0, -50.0, 100.0, 50.0, 100.0, 0.0);
        assert!((n.0 - (-1.0)).abs() < 1e-5, "nx debe ser -1: {}", n.0);
        assert!(n.1.abs() < 1e-5, "ny debe ser ~0: {}", n.1);
    }

    #[test]
    fn wall_normal_cam_degenerate_zero_length() {
        // Pared degenerada (endpoints idénticos) ⇒ (0, 0). El caller
        // debería caer al path omni.
        let n = wall_normal_cam(50.0, 50.0, 50.0, 50.0, 50.0, 50.0);
        assert_eq!(n, (0.0, 0.0));
    }

    #[test]
    fn wall_rim_directional_perpendicular_light_full_intensity() {
        // Pared a x=100, normal toward camera = (-1, 0). Luz frente a la
        // pared sobre el eje normal: cam-space (50, 0) — perpendicular
        // directo al plano. Direction surf→light = (-50, 0)/50 = (-1, 0).
        // cos(theta) = dot(normal, dir) = (-1)·(-1) + 0 = 1 ⇒ att=1.
        let white = (255, 255, 255);
        let lights = [rim_light(50.0, 0.0, white)];
        let n = (-1.0, 0.0);
        let omni = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, false);
        let dir = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            assert!(
                (omni[ch] - dir[ch]).abs() < 1e-5,
                "luz perpendicular ⇒ direccional ≈ omni: canal {}", ch
            );
        }
    }

    #[test]
    fn wall_rim_directional_grazing_uses_half() {
        // Pared a x=100, normal (-1, 0). Luz sobre el plano de la
        // pared: cam-space (100, 30) — paralela al lineseg. Direction
        // surf→light = (0, 30)/30 = (0, 1). cos = (-1)·0 + 0·1 = 0
        // ⇒ att = 0.5.
        let white = (255, 255, 255);
        let lights = [rim_light(100.0, 30.0, white)];
        let n = (-1.0, 0.0);
        let omni = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, false);
        let dir = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                let ratio = dir[ch] / omni[ch];
                assert!(
                    (ratio - 0.5).abs() < 1e-4,
                    "rasante debería ser 0.5: canal {} ratio {}", ch, ratio
                );
            }
        }
    }

    #[test]
    fn wall_rim_directional_back_light_falls_to_floor() {
        // Pared a x=100, normal (-1, 0). Luz "detrás" de la pared
        // (lejos de la cámara): cam-space (150, 0). Direction surf→light
        // = (50, 0)/50 = (1, 0). cos = (-1)·1 = -1 ⇒ att=floor (0.3).
        let white = (255, 255, 255);
        let lights = [rim_light(150.0, 0.0, white)];
        let n = (-1.0, 0.0);
        let omni = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, false);
        let dir = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                let ratio = dir[ch] / omni[ch];
                assert!(
                    (ratio - WALL_RIM_AMBIENT_FLOOR).abs() < 1e-4,
                    "back-light ⇒ piso ambient: canal {} ratio {}", ch, ratio
                );
            }
        }
    }

    #[test]
    fn wall_rim_directional_disabled_equals_omni() {
        // Toggle off ⇒ direccional debe coincidir con `world_lights_boost_rgb_cam`
        // para múltiples luces en distintas direcciones.
        let red = (255, 130, 60);
        let blue = (110, 160, 255);
        let lights = [
            rim_light(50.0, 0.0, red),
            rim_light(100.0, 40.0, blue),
            rim_light(150.0, -20.0, (255, 240, 200)),
        ];
        let n = (-1.0, 0.0);
        let omni = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, false);
        let baseline = world_lights_boost_rgb_cam(100.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(omni, baseline, "directional=false ⇒ bit-identical al 3.29");
    }

    #[test]
    fn plane_rim_directional_floor_strongest_when_light_above() {
        // Floor centroide en el origen. Dos luces a igual d_3D=50 pero
        // distinta dirección — el cosine es la única variable:
        // - above (0, 30, 40): dz = +40 ⇒ cos = 40/50 = 0.8 ⇒ att = 0.9.
        // - level (50, 0, 0): dz = 0 ⇒ cos = 0 ⇒ att = 0.5.
        // Ratio esperado ≈ 1.8 (=0.9/0.5) por canal — el plano "ve"
        // mejor la luz por arriba (su cara mira a +Z).
        let white = (255, 255, 255);
        let above = [plane_light(0.0, 30.0, 40.0, white)];
        let level = [plane_light(50.0, 0.0, 0.0, white)];
        let b_above = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &above, 1.0, true);
        let b_level = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &level, 1.0, true);
        for ch in 0..3 {
            assert!(
                b_above[ch] > b_level[ch],
                "luz por arriba del floor debería iluminar más: canal {} above={} level={}",
                ch, b_above[ch], b_level[ch]
            );
        }
    }

    #[test]
    fn plane_rim_directional_ceiling_strongest_when_light_below() {
        // Espejo del test del floor con normal `-Z`. d_3D=50 fijo:
        // - below (0, 30, -40): dz = -40 ⇒ cos = -(-40)/50 = 0.8.
        // - level (50, 0, 0): dz = 0 ⇒ cos = 0.
        let white = (255, 255, 255);
        let below = [plane_light(0.0, 30.0, -40.0, white)];
        let level = [plane_light(50.0, 0.0, 0.0, white)];
        let b_below = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &below, -1.0, true);
        let b_level = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &level, -1.0, true);
        for ch in 0..3 {
            assert!(
                b_below[ch] > b_level[ch],
                "luz por debajo del ceiling debería iluminar más: canal {} below={} level={}",
                ch, b_below[ch], b_level[ch]
            );
        }
    }

    #[test]
    fn plane_rim_directional_3d_radius_cuts_far_vertical() {
        // Luz a 0 XY pero z_cam = 250 — fuera del radio (192). El
        // path 2D omni la incluiría (distancia horizontal = 0); el
        // 3D direccional la rechaza por d_3D = 250 > 192. Result =
        // ZERO_BOOST en direccional, > 0 en omni.
        let white = (255, 255, 255);
        let lights = [plane_light(0.0, 0.0, 250.0, white)];
        let dir = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, 1.0, true);
        let omni = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, 1.0, false);
        assert_eq!(dir, ZERO_BOOST, "3D radio corta la luz lejana en z");
        assert!(omni[0] > 0.0, "omni 2D no la corta (distancia XY=0)");
    }

    #[test]
    fn plane_rim_directional_disabled_equals_omni_2d() {
        // Toggle off ⇒ el helper de plano debe coincidir bit-a-bit con
        // `world_lights_boost_rgb_cam` (path omni 2D del 3.29).
        let lights = [
            plane_light(50.0, 30.0, 20.0, (255, 130, 60)),
            plane_light(80.0, -20.0, -50.0, (110, 160, 255)),
        ];
        let off = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, 1.0, false);
        let baseline = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(off, baseline);
    }

    #[test]
    fn plane_rim_directional_floor_back_lit_from_below_falls_to_floor() {
        // Floor con normal +Z. Una luz "por debajo" del piso (dz < 0)
        // back-lightea: cos = +1 * dz/d_3D < 0 ⇒ att = floor (raro en
        // Doom — los mobjs FF_FULLBRIGHT van por arriba de los pisos —
        // pero el caso límite debe atenuarse al piso ambient).
        let white = (255, 255, 255);
        // Floor a z_cam = 0; luz a z_cam = -50 (50 abajo del piso).
        let lights = [plane_light(0.0, 0.0, -50.0, white)];
        let dir = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, 1.0, true);
        // Omni-2D: distancia 2D = 0 ⇒ full peak.
        let omni = world_lights_boost_rgb_for_plane_cam(0.0, 0.0, 0.0, NO_SECTOR, &lights, 1.0, false);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                // Direccional debería ser bastante menor que omni
                // (att ≈ floor = 0.3, modulado además por el radio
                // 3D que en este test sigue dentro del rango).
                assert!(
                    dir[ch] < omni[ch] * (PLANE_RIM_AMBIENT_FLOOR + 0.1),
                    "back-lit floor: canal {} dir={} no clampea cerca del floor ambient",
                    ch, dir[ch]
                );
            }
        }
    }

    // =================================================================
    // Fase 3.34 — BRDF 3D para paredes
    // =================================================================

    #[test]
    fn wall_rim_3d_high_light_attenuates_compared_to_planar() {
        // Pared en x=100, normal toward-camera (-1, 0). Dos luces a la
        // **misma XY** (50, 0) pero distinta z_cam:
        //   - planar: (50, 0, 0) — al nivel del eye / surface sample.
        //   - high:   (50, 0, 60) — 60 unidades por encima.
        // El path 3D usa d² 3D y cos(θ) = (nx·dx + ny·dy)/d_3D — la
        // luz alta tiene d_3D > d_2D y cos < cos_2D, por lo que su
        // aporte cae respecto a la planar.
        let white = (255, 255, 255);
        let planar = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let high = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 60.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let b_planar = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &planar, n, true);
        let b_high = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &high, n, true);
        for ch in 0..3 {
            assert!(
                b_planar[ch] > b_high[ch],
                "luz alta debería atenuar más vía 3D: canal {} planar={} high={}",
                ch, b_planar[ch], b_high[ch]
            );
        }
    }

    #[test]
    fn wall_rim_3d_radius_cuts_far_vertical_light() {
        // Pared en x=100, normal (-1, 0). Luz a XY (100, 0) pero
        // z=250 (muy arriba). En 2D d=0 ⇒ omni la incluye. En 3D
        // d=250 > r=192 ⇒ direccional la excluye.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 100.0, y_cam: 0.0, z_cam: 250.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let dir = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, true);
        let omni = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, false);
        assert_eq!(dir, ZERO_BOOST, "3D radio corta la luz lejana en z");
        assert!(omni[0] > 0.0, "omni 2D no la corta (d_XY=0)");
    }

    #[test]
    fn wall_rim_3d_planar_light_finite_and_positive() {
        // Luz con z_cam=0 (planar al eye level). El path 3D con dz=0
        // colapsa al cálculo 2D del 3.32 — verificamos sanidad
        // numérica para una geometría no-trivial.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 20.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let dir = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} finite", ch);
            assert!(dir[ch] > 0.0, "canal {} positivo", ch);
        }
    }

    #[test]
    fn wall_rim_3d_disabled_uses_omni_2d() {
        // Toggle off ⇒ debería seguir usando `world_lights_boost_rgb_cam`
        // omni 2D del 3.29 — bit-equivalente aún con z_cam alto.
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 20.0, z_cam: 100.0,
            sector: NO_SECTOR, tint_rgb: (200, 200, 200), lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let off = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, false);
        let baseline = world_lights_boost_rgb_cam(100.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(off, baseline, "directional=false ⇒ bit-equivalente al 3.29");
    }

    #[test]
    fn wall_rim_3d_handles_zero_distance_safely() {
        // Luz coincidente con la superficie en 3D (XY + z) ⇒ d² ≈ 0,
        // fast path att=1.0, sin NaN.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 100.0, y_cam: 0.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let dir = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} finite", ch);
            assert!(dir[ch] > 0.0, "luz pegada aporta full");
        }
    }

    // =================================================================
    // Fase 3.35 — BRDF 3D para mobj sprites
    // =================================================================

    #[test]
    fn sprite_rim_3d_high_light_attenuates_compared_to_planar() {
        // Sprite (mobj) en (200, 0, 0). Dos luces a misma XY (100, 0)
        // pero distinto z_cam:
        //   - planar (100, 0, 0): al nivel del eye/sprite.
        //   - high   (100, 0, 60): 60 unidades arriba.
        // 3D BRDF: d² incluye dz, cos = (nx·dx + ny·dy)/d_3D — la alta
        // queda con d_3D > d_2D y cos < cos_2D ⇒ menor aporte.
        let white = (255, 255, 255);
        let planar = [WorldLight {
            x_cam: 100.0, y_cam: 0.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let high = [WorldLight {
            x_cam: 100.0, y_cam: 0.0, z_cam: 60.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let b_planar = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &planar, true);
        let b_high = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &high, true);
        for ch in 0..3 {
            assert!(
                b_planar[ch] > b_high[ch],
                "luz alta debería atenuar más vía 3D: canal {} planar={} high={}",
                ch, b_planar[ch], b_high[ch]
            );
        }
    }

    #[test]
    fn sprite_rim_3d_radius_cuts_far_vertical_light() {
        // Sprite a (200, 0, 0). Luz a XY (200, 0) pero z=250. En 2D
        // d_XY=0 ⇒ omni la incluye; en 3D d=250 > r=192 ⇒ direccional
        // la excluye.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 200.0, y_cam: 0.0, z_cam: 250.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let dir = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        let omni = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        assert_eq!(dir, ZERO_BOOST, "3D radio corta la luz lejana en z");
        assert!(omni[0] > 0.0, "omni 2D no la corta (d_XY=0)");
    }

    #[test]
    fn sprite_rim_3d_planar_light_finite_and_positive() {
        // Luz con z_cam=0 (planar al sprite). Sanity check del path 3D
        // colapsando a 2D cuando dz=0.
        let red = (255, 130, 60);
        let lights = [WorldLight {
            x_cam: 100.0, y_cam: 30.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: red, lit_sectors: None,
        }];
        let dir = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} finite", ch);
        }
        assert!(dir[0] > 0.0, "tinte rojo presente");
    }

    #[test]
    fn sprite_rim_3d_disabled_uses_omni_2d() {
        // Toggle off ⇒ bit-equivalente al `world_lights_boost_rgb_cam`
        // omni 2D del 3.29 incluso con z_cam alto.
        let lights = [WorldLight {
            x_cam: 100.0, y_cam: 20.0, z_cam: 80.0,
            sector: NO_SECTOR, tint_rgb: (200, 200, 200), lit_sectors: None,
        }];
        let off = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, 0.0, NO_SECTOR, &lights, false);
        let baseline = world_lights_boost_rgb_cam(200.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(off, baseline);
    }

    #[test]
    fn sprite_rim_3d_handles_sprite_below_eye_level() {
        // Sprite en (200, 0, -32) (mobj parado sobre piso 32 u debajo
        // del ojo) + luz al ras del piso a la izquierda (100, 50, -32).
        // dz = 0 ⇒ ratio 3D/2D ≈ 1 (luz al nivel del sprite). El
        // direccional debería seguir siendo finito y positivo.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 100.0, y_cam: 50.0, z_cam: -32.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let dir = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, -32.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} finite", ch);
            assert!(dir[ch] > 0.0, "luz al nivel del sprite aporta canal {}", ch);
        }
    }

    // =================================================================
    // Fase 3.37 — Muzzle direccional sobre walls y planes
    // =================================================================

    // =================================================================
    // Fase 3.42 — Bandas verticales para BRDF de walls
    // =================================================================

    #[test]
    fn wall_v_band_centers_split_slab_uniformly() {
        // Verifica el cálculo de los centros verticales de N bandas
        // sobre un slab `[z_bot, z_top]`. Reproduce la fórmula del
        // loop de gather_wall: `z_band_center = z_bot + (z_top - z_bot)
        // * (t0 + t1) * 0.5` con `t0 = b/N`, `t1 = (b+1)/N`.
        let z_bot = 0.0_f32;
        let z_top = 128.0_f32;
        let v_bands: u32 = 4;
        let mut centers = Vec::new();
        for b in 0..v_bands {
            let t0 = b as f32 / v_bands as f32;
            let t1 = (b + 1) as f32 / v_bands as f32;
            centers.push(z_bot + (z_top - z_bot) * (t0 + t1) * 0.5);
        }
        // Esperado: 16, 48, 80, 112 (centros de cada cuarto).
        assert_eq!(centers, vec![16.0, 48.0, 80.0, 112.0]);
    }

    #[test]
    fn wall_v_band_bottom_band_receives_more_from_floor_light() {
        // Pared a x=100 con normal toward-camera (-1, 0). Luz al ras del
        // piso (z_cam = -50). Comparamos boost al centro de la banda
        // inferior (z_band_cam=-32) vs banda superior (z_band_cam=+96).
        // La luz baja tiene dz pequeño con la banda inferior ⇒ d_3D
        // menor ⇒ más boost.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: -50.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let band_low = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, -32.0, NO_SECTOR, &lights, n, true);
        let band_high = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 96.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            assert!(
                band_low[ch] > band_high[ch],
                "luz al piso ⇒ banda inferior recibe más: canal {} low={} high={}",
                ch, band_low[ch], band_high[ch]
            );
        }
    }

    #[test]
    fn wall_v_band_top_band_receives_more_from_ceiling_light() {
        // Espejo: luz a la altura del techo (z_cam=+90) ⇒ la banda
        // superior recibe más.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 90.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let band_low = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, -32.0, NO_SECTOR, &lights, n, true);
        let band_high = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, 96.0, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            assert!(
                band_high[ch] > band_low[ch],
                "luz al techo ⇒ banda superior recibe más: canal {} high={} low={}",
                ch, band_high[ch], band_low[ch]
            );
        }
    }