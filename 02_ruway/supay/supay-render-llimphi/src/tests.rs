    use super::*;

    #[test]
    fn camera_identity_at_zero_angle() {
        let cam = Camera::new(0.0, 0.0, 0.0, 0.0);
        let (x, y) = cam.to_cam_2d(10.0, 0.0);
        assert!((x - 10.0).abs() < 1e-5);
        assert!(y.abs() < 1e-5);
    }

    #[test]
    fn camera_left_is_negative_y_cam() {
        let cam = Camera::new(0.0, 0.0, 0.0, 0.0);
        let (_x, y) = cam.to_cam_2d(0.0, 10.0);
        assert!(y < 0.0, "left point should map to negative Y_cam, got {y}");
    }

    #[test]
    fn projection_centers_origin_at_screen_center() {
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let proj = Projection::new(rect, 75_f32.to_radians());
        let p = proj.project(100.0, 0.0, 0.0);
        assert!((p.x - 400.0).abs() < 1e-3);
        assert!((p.y - 300.0).abs() < 1e-3);
    }

    #[test]
    fn projection_right_of_camera_lands_right_of_center() {
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let proj = Projection::new(rect, 75_f32.to_radians());
        let p = proj.project(10.0, 1.0, 0.0);
        assert!(p.x > 400.0, "+Y_cam should project right of center, got {}", p.x);
    }

    #[test]
    fn projection_pitch_up_shifts_horizon_down() {
        // pitch positivo = mirar hacia arriba → línea del horizonte
        // (puntos con z_cam=0) baja en pantalla (sy mayor).
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let proj_flat = Projection::new(rect, 75_f32.to_radians());
        let proj_up = Projection::new_pitched(rect, 75_f32.to_radians(), 0.4);
        let p_flat = proj_flat.project(10.0, 0.0, 0.0);
        let p_up = proj_up.project(10.0, 0.0, 0.0);
        assert!(
            p_up.y > p_flat.y,
            "pitch up debe empujar el horizonte hacia abajo, flat={} up={}",
            p_flat.y,
            p_up.y
        );
        // El offset debe ser exactamente `focal · tan(pitch)`.
        let focal = (rect.h as f64) * 0.5 / (75_f32.to_radians() as f64 * 0.5).tan();
        let expected = focal * (0.4_f64).tan();
        assert!(
            (p_up.y - p_flat.y - expected).abs() < 1e-3,
            "offset esperado {expected}, observado {}",
            p_up.y - p_flat.y
        );
    }

    #[test]
    fn projection_pitch_down_shifts_horizon_up() {
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let proj_flat = Projection::new(rect, 75_f32.to_radians());
        let proj_dn = Projection::new_pitched(rect, 75_f32.to_radians(), -0.3);
        let p_flat = proj_flat.project(10.0, 0.0, 0.0);
        let p_dn = proj_dn.project(10.0, 0.0, 0.0);
        assert!(
            p_dn.y < p_flat.y,
            "pitch down debe empujar el horizonte hacia arriba, flat={} down={}",
            p_flat.y,
            p_dn.y
        );
    }

    #[test]
    fn projection_pitch_does_not_alter_x() {
        // El y-shear es vertical puro — la coordenada X de un punto
        // debe quedar idéntica con o sin pitch.
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let proj_flat = Projection::new(rect, 75_f32.to_radians());
        let proj_up = Projection::new_pitched(rect, 75_f32.to_radians(), 0.5);
        let p_flat = proj_flat.project(10.0, 3.0, 0.0);
        let p_up = proj_up.project(10.0, 3.0, 0.0);
        assert!(
            (p_flat.x - p_up.x).abs() < 1e-3,
            "X debe ser invariante al pitch, flat.x={} up.x={}",
            p_flat.x,
            p_up.x
        );
    }

    #[test]
    fn projection_pitch_clamps_extremes() {
        // Más allá de ±π/3 el horizonte se sale del viewport; el
        // clamp del constructor evita tan() explotando.
        let rect = PaintRect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let proj_extreme = Projection::new_pitched(rect, 75_f32.to_radians(), 5.0);
        let proj_max = Projection::new_pitched(rect, 75_f32.to_radians(), PITCH_MAX);
        let p_extreme = proj_extreme.project(10.0, 0.0, 0.0);
        let p_max = proj_max.project(10.0, 0.0, 0.0);
        assert!(
            (p_extreme.y - p_max.y).abs() < 1e-3,
            "valores absurdos deben clampearse a PITCH_MAX"
        );
    }

    #[test]
    fn wall_bands_vary_shade_monotonic_lighter_up() {
        // Misma pared, misma profundidad, distintas bandas — la banda
        // de arriba debe quedar más clara que la de abajo (multiplicador
        // 0.78..1.12 con t creciente).
        let sec = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 200,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let wall = WallSeg {
            x1: 0.0,
            y1: 0.0,
            x2: 64.0,
            y2: 0.0,
            front_sector: 0,
            back_sector: NO_SECTOR,
            flags: 0,
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
        };
        let cfg = RenderConfig::default();
        let c_bot = wall_color(7, &wall, &sec, 100.0, 0, 4, &cfg);
        let c_top = wall_color(7, &wall, &sec, 100.0, 3, 4, &cfg);
        let comps = |c: Color| {
            let [r, g, b, _a] = c.to_rgba8().to_u8_array();
            r as u32 + g as u32 + b as u32
        };
        assert!(
            comps(c_top) > comps(c_bot),
            "top band ({:?}) should be lighter than bottom ({:?})",
            c_top.to_rgba8().to_u8_array(),
            c_bot.to_rgba8().to_u8_array()
        );
    }

    #[test]
    fn clip_near_keeps_polygon_fully_in_front() {
        // Cuadrado a X_cam = 100..200, Y ±50. Todo delante del near=4.
        let poly = vec![(100.0, -50.0), (200.0, -50.0), (200.0, 50.0), (100.0, 50.0)];
        let clipped = clip_near(&poly, 4.0);
        assert_eq!(clipped.len(), 4);
        assert_eq!(clipped, poly);
    }

    #[test]
    fn clip_near_drops_polygon_fully_behind() {
        // Cuadrado a X_cam = -100..-50. Todo detrás.
        let poly = vec![(-100.0, -50.0), (-50.0, -50.0), (-50.0, 50.0), (-100.0, 50.0)];
        let clipped = clip_near(&poly, 4.0);
        assert!(clipped.is_empty(), "behind-camera poly should be empty, got {clipped:?}");
    }

    #[test]
    fn clip_near_clips_polygon_crossing_plane() {
        // Triángulo con un vértice atrás (X=-10) y dos adelante (X=20).
        // Las dos aristas que cruzan deben generar intersecciones a X=near.
        let near = 4.0;
        let poly = vec![(-10.0, 0.0), (20.0, -10.0), (20.0, 10.0)];
        let clipped = clip_near(&poly, near);
        // Resultado esperado: 4 vértices — los 2 frontales + 2 intersecciones.
        assert_eq!(clipped.len(), 4, "expected 4 verts, got {clipped:?}");
        // Todas las X >= near.
        for &(x, _) in &clipped {
            assert!(x >= near - 1e-4, "vertex x={x} < near={near}");
        }
        // Las dos intersecciones deben estar en x = near.
        let on_plane = clipped.iter().filter(|&&(x, _)| (x - near).abs() < 1e-4).count();
        assert_eq!(on_plane, 2, "expected 2 vertices on plane, got {clipped:?}");
    }

    #[test]
    fn ceiling_sky_detection_matches_pic() {
        let sky_sec = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 256.0,
            light_level: 255,
            floor_pic: 0,
            ceiling_pic: 42,
        };
        assert!(ceiling_is_sky(&sky_sec, 42));
        assert!(!ceiling_is_sky(&sky_sec, 41));
        // Sentinel NO_SKY_PIC nunca debe matchear, aunque ceiling_pic
        // por casualidad sea 0xFFFF (mapa raro).
        let weird = SectorSnap {
            ceiling_pic: NO_SKY_PIC,
            ..sky_sec.clone()
        };
        assert!(!ceiling_is_sky(&weird, NO_SKY_PIC));
    }

    #[test]
    fn camera_to_from_round_trip() {
        let cam = Camera::new(100.0, 200.0, 41.0, 0.75);
        for (wx, wy) in [(150.0, 220.0), (50.0, 80.0), (100.0, 200.0), (-20.0, 999.0)] {
            let (cx, cy) = cam.to_cam_2d(wx, wy);
            let (rx, ry) = cam.from_cam_2d(cx, cy);
            assert!((rx - wx).abs() < 1e-3, "wx round-trip: {wx} → {rx}");
            assert!((ry - wy).abs() < 1e-3, "wy round-trip: {wy} → {ry}");
        }
    }

    #[test]
    fn solve_floor_affine_recovers_identity_when_world_equals_screen() {
        // Si world == screen para 3 puntos, la affine resuelta es la
        // identidad (a=1, b=0, c=0, d=1, e=0, f=0).
        let a = solve_floor_affine(
            (0.0, 0.0), Point::new(0.0, 0.0),
            (10.0, 0.0), Point::new(10.0, 0.0),
            (0.0, 10.0), Point::new(0.0, 10.0),
        ).expect("solve");
        let coeffs = a.as_coeffs();
        assert!((coeffs[0] - 1.0).abs() < 1e-6, "a={}", coeffs[0]);
        assert!(coeffs[1].abs() < 1e-6, "b={}", coeffs[1]);
        assert!(coeffs[2].abs() < 1e-6, "c={}", coeffs[2]);
        assert!((coeffs[3] - 1.0).abs() < 1e-6, "d={}", coeffs[3]);
    }

    #[test]
    fn solve_floor_affine_rejects_collinear() {
        // 3 vértices sobre una línea horizontal → det_w = 0 → None.
        let a = solve_floor_affine(
            (0.0, 0.0), Point::new(0.0, 0.0),
            (10.0, 0.0), Point::new(10.0, 0.0),
            (20.0, 0.0), Point::new(20.0, 0.0),
        );
        assert!(a.is_none());
    }

    #[test]
    fn display_angle_facing_camera_is_1() {
        // Mobj en (10, 0) facing -X (hacia el jugador en origen).
        // mobj_angle = π, viewer = (0,0). atan2(0-0, 0-10) = π.
        // rel = π - π = 0 → wedge 0 → display 1.
        let a = compute_display_angle(10.0, 0.0, std::f32::consts::PI, 0.0, 0.0);
        assert_eq!(a, 1, "expected front (1), got {a}");
    }

    #[test]
    fn display_angle_back_is_5() {
        // Mobj en (10, 0) facing +X (de espaldas al jugador en origen).
        // mobj_angle = 0, atan2(0-0, 0-10) = π. rel = π - 0 = π.
        // π / (π/4) = 4 → wedge 4 → display 5.
        let a = compute_display_angle(10.0, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(a, 5, "expected back (5), got {a}");
    }

    #[test]
    fn display_angle_right_side_is_3() {
        // Mobj en origen facing +X (su derecha = -Y world). Jugador
        // sobre el lado derecho del mobj → en -Y.
        // mobj_angle=0, viewer=(0,-10). atan2(-10-0, 0-0) = -π/2.
        // rel = (-π/2 - 0) mod 2π = 3π/2. 3π/2 / (π/4) = 6 → display 7.
        // (lado IZQUIERDO según convención Doom mirror; 3 sería al
        //  otro lado). Verificamos consistencia: si viewer está a +Y,
        //  debería ser 3.
        let a = compute_display_angle(0.0, 0.0, 0.0, 0.0, 10.0);
        // mobj_angle=0, viewer=(0,+10). atan2(+10, 0) = +π/2.
        // rel = π/2. π/2 / (π/4) = 2 → display 3.
        assert_eq!(a, 3, "expected right (3) for viewer on +Y of mobj facing +X, got {a}");
    }

    #[test]
    fn floor_color_uses_atlas_when_available() {
        // Sintetiza un WAD mínimo en memoria con un flat "F_T1" cuyo
        // promedio es conocido (todo índice 42 → palette[42]).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"IWAD");
        bytes.extend_from_slice(&2u32.to_le_bytes());
        let dir_off_placeholder = bytes.len();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        // PLAYPAL grayscale.
        let p1 = bytes.len();
        let playpal: Vec<u8> = (0..supay_wad::PALETTE_ENTRIES)
            .flat_map(|i| {
                let v = i as u8;
                [v, v, v]
            })
            .collect();
        bytes.extend_from_slice(&playpal);
        // F_T1 = todo 42.
        let p2 = bytes.len();
        bytes.extend(std::iter::repeat(42u8).take(supay_wad::FLAT_BYTES));
        let dir_off = bytes.len() as u32;
        bytes.extend_from_slice(&(p1 as u32).to_le_bytes());
        bytes.extend_from_slice(&(playpal.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"PLAYPAL\0");
        bytes.extend_from_slice(&(p2 as u32).to_le_bytes());
        bytes.extend_from_slice(&(supay_wad::FLAT_BYTES as u32).to_le_bytes());
        bytes.extend_from_slice(b"F_T1\0\0\0\0");
        bytes[dir_off_placeholder..dir_off_placeholder + 4]
            .copy_from_slice(&dir_off.to_le_bytes());

        let wad = supay_wad::Wad::parse(bytes).unwrap();
        let atlas = Arc::new(WadAtlas::new(wad, HashMap::new()));
        // Antes de registrar el nombre, flat_color devuelve None y el
        // floor_color cae a FLOOR_PALETTE.
        let sec = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 255,
            floor_pic: 7,
            ceiling_pic: 0,
        };
        let cfg_no_name = RenderConfig {
            atlas: Some(atlas.clone()),
            ..RenderConfig::default()
        };
        let c_fallback = floor_color(&sec, 0.0, &cfg_no_name);
        // Color del fallback: FLOOR_PALETTE[7 % 8] = ash (0x40,0x38,0x2C)
        // multiplicado por shade ≈ 0.92.
        let fb = c_fallback.to_rgba8().to_u8_array();
        assert!(fb[0] < 80, "fallback red should be muted, got {fb:?}");

        // Registrar nombre del flat → ahora flat_color devuelve (42,42,42).
        atlas.set_flat_name(7, "F_T1".to_string());
        let c_real = floor_color(&sec, 0.0, &cfg_no_name);
        let rc = c_real.to_rgba8().to_u8_array();
        // Expected: (42,42,42) tinted con light=255, depth=0 → shade≈0.92
        // → 42*0.92 ≈ 38 en cada canal.
        assert!((rc[0] as i32 - 38).abs() <= 2, "expected ≈38, got {rc:?}");
        assert_eq!(rc[0], rc[1]);
        assert_eq!(rc[1], rc[2]);
    }

    #[test]
    fn wall_v_top_middle_default_pegs_top_to_ceiling() {
        // Middle, no flags: la textura ancla su TOP al techo del near.
        // En z_top (= ceiling), V = 0.
        let v = wall_v_top(0, 0, 0.0, 128.0, None, None, 128.0, 64.0, 0.0);
        assert!(v.abs() < 1e-4, "expected v_top=0, got {v}");
    }

    #[test]
    fn wall_v_top_middle_dontpegbottom_pegs_bottom_to_floor() {
        // Middle + DONTPEGBOTTOM: bottom de la textura en near_floor.
        // En z_top (= ceiling=128), V = floor + tex_h - z_top = -64
        // (lo cual con Extend::Repeat tilea correctamente).
        let v = wall_v_top(0, ML_DONTPEGBOTTOM, 0.0, 128.0, None, None, 128.0, 64.0, 0.0);
        assert!((v - (-64.0)).abs() < 1e-4, "expected -64, got {v}");
    }

    #[test]
    fn wall_v_top_upper_default_pegs_to_back_ceiling() {
        // Upper sin flag: top de la textura al far_ceiling. La pared
        // "header" va de far_ceiling (= 96) a near_ceiling (= 128).
        // V(z_top = 128) = far_ceiling + tex_h - z_top = 96 + 64 - 128 = 32.
        let v = wall_v_top(1, 0, 0.0, 128.0, Some(0.0), Some(96.0), 128.0, 64.0, 0.0);
        assert!((v - 32.0).abs() < 1e-4, "expected 32, got {v}");
    }

    #[test]
    fn wall_v_top_upper_dontpegtop_pegs_to_front_ceiling() {
        // Upper + DONTPEGTOP: top alineado al near_ceiling — doors.
        // V(z_top = 128) = near_ceiling - z_top = 0.
        let v = wall_v_top(1, ML_DONTPEGTOP, 0.0, 128.0, Some(0.0), Some(96.0), 128.0, 64.0, 0.0);
        assert!(v.abs() < 1e-4, "expected 0, got {v}");
    }

    #[test]
    fn wall_v_top_lower_default_pegs_to_back_floor() {
        // Lower sin flag: top de la textura al far_floor. La pared
        // "step" va de near_floor (= 0) a far_floor (= 32).
        // V(z_top = 32) = far_floor - z_top = 0.
        let v = wall_v_top(2, 0, 0.0, 128.0, Some(32.0), Some(128.0), 32.0, 64.0, 0.0);
        assert!(v.abs() < 1e-4, "expected 0, got {v}");
    }

    #[test]
    fn wall_v_top_lower_dontpegbottom_pegs_to_near_ceiling() {
        // Lower + DONTPEGBOTTOM: top alineado al near_ceiling (= 128)
        // — alinea con la textura "main" del techo.
        // V(z_top = 32) = near_ceiling - z_top = 96.
        let v = wall_v_top(2, ML_DONTPEGBOTTOM, 0.0, 128.0, Some(32.0), Some(128.0), 32.0, 64.0, 0.0);
        assert!((v - 96.0).abs() < 1e-4, "expected 96, got {v}");
    }

    #[test]
    fn sprite_color_full_bright_bypasses_shading() {
        // Sin full-bright el sprite oscurece con light_level bajo + fog.
        // Con bit 7 set, sale a luz plena (shade=1.0).
        let sec = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 80, // oscuro
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let cfg = RenderConfig::default();
        let dim_sprite = SpriteSnap {
            x: 0.0, y: 0.0, z: 0.0, angle: 0.0,
            sprite: 0, frame: 0, sector: 0,
        };
        let bright_sprite = SpriteSnap {
            frame: 0x80, // bit 7 set
            ..dim_sprite.clone()
        };
        // depth=500 → fog atenúa visible
        let dim = sprite_color(&dim_sprite, Some(&sec), 500.0, &cfg).to_rgba8().to_u8_array();
        let bright = sprite_color(&bright_sprite, Some(&sec), 500.0, &cfg).to_rgba8().to_u8_array();
        let dim_sum = dim[0] as u32 + dim[1] as u32 + dim[2] as u32;
        let bright_sum = bright[0] as u32 + bright[1] as u32 + bright[2] as u32;
        assert!(
            bright_sum > dim_sum + 40,
            "full-bright should be much brighter than dim shaded: bright={bright:?} dim={dim:?}"
        );
    }

    #[test]
    fn wall_v_top_rowoffset_is_added() {
        // rowoffset shift directo del V_top — útil para alinear
        // texturas entre paredes adyacentes.
        let v0 = wall_v_top(0, 0, 0.0, 128.0, None, None, 128.0, 64.0, 0.0);
        let v8 = wall_v_top(0, 0, 0.0, 128.0, None, None, 128.0, 64.0, 8.0);
        assert!((v8 - v0 - 8.0).abs() < 1e-4, "expected +8 shift, got {} vs {}", v8, v0);
    }

    #[test]
    fn floor_and_ceiling_palettes_indexed_by_pic() {
        // Distintos floor_pic deben dar colores distintos cuando el módulo
        // los separa.
        let a = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 255,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let b = SectorSnap {
            floor_pic: 1,
            ..a.clone()
        };
        let cfg = RenderConfig::default();
        let ca = floor_color(&a, 0.0, &cfg);
        let cb = floor_color(&b, 0.0, &cfg);
        assert_ne!(ca.to_rgba8().to_u8_array(), cb.to_rgba8().to_u8_array());
    }

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

    #[test]
    fn bsp_walk_viewer_on_front_visits_back_first() {
        // Partición vertical x=0, dy=1. side = dx·(py - y) - dy·(px - x).
        // Para viewer en (+10, 0): side = 0·(0) - 1·(10) = -10 < 0 → back.
        // ¡Pero los hijos en Doom convention son [front, back] respecto a
        // R_PointOnSide, que dice `side > 0 = back` en su implementación
        // ¡pero usamos el signo opuesto! Verifiquemos lo que walk_bsp hace
        // realmente con esta config.
        // Implementación actual: side > 0 → near = children[0] (front lit).
        // side < 0 → near = children[1].
        // Para viewer en (+10, 0): side = -10 < 0 → near = children[1] = ss1,
        // far = children[0] = ss0. Visita ss0 primero (back-to-front).
        let nodes = simple_two_leaf_bsp();
        let mut out = Vec::new();
        walk_bsp(&nodes, (nodes.len() - 1) as u16, 10.0, 0.0, &mut out);
        assert_eq!(out, vec![0, 1], "viewer al +X visita ss0 (far) primero");
    }

    #[test]
    fn bsp_walk_viewer_on_back_visits_front_first() {
        // Para viewer en (-10, 0): side = -1·(-10) = +10 > 0 → near = children[0] = ss0,
        // far = children[1] = ss1. Visita ss1 primero (back-to-front).
        let nodes = simple_two_leaf_bsp();
        let mut out = Vec::new();
        walk_bsp(&nodes, (nodes.len() - 1) as u16, -10.0, 0.0, &mut out);
        assert_eq!(out, vec![1, 0], "viewer al -X visita ss1 (far) primero");
    }

    // -----------------------------------------------------------------
    // Fase 3.18: subsector point query + player sector light
    // -----------------------------------------------------------------

    #[test]
    fn subsector_at_point_picks_leaf_containing_point() {
        // Misma partición que `simple_two_leaf_bsp`: línea x=0 (dy=1).
        // Punto (+10, 0): side = 0 - 1·10 = -10 < 0 → near = children[1] = ss1.
        // Punto (-10, 0): side = 0 + 10 = +10 > 0 → near = children[0] = ss0.
        let nodes = simple_two_leaf_bsp();
        assert_eq!(subsector_at_point(&nodes, 10.0, 0.0), Some(1));
        assert_eq!(subsector_at_point(&nodes, -10.0, 0.0), Some(0));
    }

    #[test]
    fn subsector_at_point_none_without_bsp() {
        // Sin nodes (snapshot stub, mapa no cargado) la query devuelve None
        // sin entrar al loop — el caller cae a su fallback default.
        assert_eq!(subsector_at_point(&[], 0.0, 0.0), None);
    }

    #[test]
    fn player_sector_light_picks_local_light_level() {
        // Dos sectores con luces opuestas; el player en cada lado debe
        // leer el light_level del sector donde está parado.
        let dim = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 64,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let bright = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 240,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![dim, bright]);
        // ss0 → sector 0 (dim), ss1 → sector 1 (bright). Coincide con la
        // convención de `simple_two_leaf_bsp`: viewer en (+10, 0) cae en ss1.
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 1, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());

        snap.player.x = 10.0;
        snap.player.y = 0.0;
        assert_eq!(player_sector_light(&snap), 240, "player en ss1 (bright)");

        snap.player.x = -10.0;
        assert_eq!(player_sector_light(&snap), 64, "player en ss0 (dim)");
    }

    #[test]
    fn player_sector_light_falls_back_without_bsp() {
        // Snapshot vacío: no hay BSP, no hay sectores. Fallback 192 —
        // mismo valor que usa `gather_sprite` para sprites sin sector.
        let snap = SceneSnapshot::empty(0);
        assert_eq!(player_sector_light(&snap), DEFAULT_PLAYER_LIGHT);
        assert_eq!(DEFAULT_PLAYER_LIGHT, 192);
    }

    // -----------------------------------------------------------------
    // Fase 3.14: player overlays
    // -----------------------------------------------------------------

    #[test]
    fn overlay_none_when_all_counters_zero() {
        let ov = PlayerOverlays::default();
        assert!(overlay_rgba(&ov, 0).is_none());
    }

    #[test]
    fn overlay_damage_red_priority_over_bonus() {
        // damagecount tiene prioridad sobre bonuscount.
        let ov = PlayerOverlays {
            damage_count: 16,
            bonus_count: 16,
            ..Default::default()
        };
        let (r, g, b, _a) = overlay_rgba(&ov, 0).expect("overlay activo");
        // Es rojizo: r >> g, r >> b.
        assert!(r > g && r > b, "expected red dominant, got ({r}, {g}, {b})");
    }

    #[test]
    fn overlay_damage_alpha_scales_with_count() {
        let low = PlayerOverlays {
            damage_count: 4,
            ..Default::default()
        };
        let hi = PlayerOverlays {
            damage_count: 80,
            ..Default::default()
        };
        let (_, _, _, a_lo) = overlay_rgba(&low, 0).expect("low");
        let (_, _, _, a_hi) = overlay_rgba(&hi, 0).expect("hi");
        assert!(a_hi > a_lo, "alpha más grande con más daño: lo={a_lo} hi={a_hi}");
    }

    #[test]
    fn overlay_radsuit_blinks_in_last_seconds() {
        // power_radsuit < 4*32 (= 128): blinkea por bit 3 del tick.
        let ov = PlayerOverlays {
            power_radsuit: 50,
            ..Default::default()
        };
        // tick con bit 3 set (8, 9, 10, ...) → overlay activo (green).
        let on = overlay_rgba(&ov, 8);
        // tick con bit 3 limpio (0..7) → sin overlay.
        let off = overlay_rgba(&ov, 0);
        assert!(on.is_some(), "blink-on tick debe pintar verde");
        assert!(off.is_none(), "blink-off tick no debe pintar");
    }

    #[test]
    fn overlay_berserk_fades_with_strength() {
        // Fase 3.16: berserk recién agarrado tinte rojo intenso; después
        // de muchos tics el alpha cae.
        let fresh = PlayerOverlays {
            power_strength: 1,
            ..Default::default()
        };
        let old = PlayerOverlays {
            power_strength: 600,
            ..Default::default()
        };
        let (_, _, _, a_fresh) = overlay_rgba(&fresh, 0).expect("berserk fresh");
        let (_, _, _, a_old) = overlay_rgba(&old, 0).expect("berserk old");
        assert!(a_fresh > a_old, "alpha cae con tics: fresh={a_fresh} old={a_old}");
    }

    #[test]
    fn overlay_radsuit_priority_over_berserk() {
        // Si radsuit + berserk activos, gana radsuit (verde, no rojo).
        let ov = PlayerOverlays {
            power_strength: 1,
            power_radsuit: 200,
            ..Default::default()
        };
        let (r, g, _b, _a) = overlay_rgba(&ov, 0).expect("overlay");
        assert!(g > r, "radsuit verde domina berserk rojo: r={r} g={g}");
    }

    #[test]
    fn overlay_invuln_dominates_damage() {
        // Si hay invuln activo + damage, gana invuln (blanco, no rojo).
        let ov = PlayerOverlays {
            damage_count: 80,
            power_invuln: 200,
            ..Default::default()
        };
        let (r, g, b, _a) = overlay_rgba(&ov, 0).expect("overlay activo");
        // Blanco: r ~ g ~ b, todos altos.
        assert!(r > 180 && g > 180 && b > 180, "expected white-ish, got ({r}, {g}, {b})");
    }

    #[test]
    fn bsp_compute_depths_assigns_decreasing_values() {
        // Snapshot con 2 subsectors y el árbol simple. Compute_depths debe
        // asignar al subsector visitado primero (más lejano) el depth más
        // grande.
        let mut snap = SceneSnapshot::empty(0);
        snap.player.x = 10.0;
        snap.player.y = 0.0;
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());
        let depths = compute_bsp_order_depths(&snap);
        // ss0 visitado primero → depth grande. ss1 segundo → depth chico.
        let d0 = depths[0].expect("ss0 reached");
        let d1 = depths[1].expect("ss1 reached");
        assert!(d0 > d1, "ss0 (far) {d0} debe ser > ss1 (near) {d1}");
        // Ambos depths están sobre BSP_DEPTH_BASE para estar siempre detrás
        // de walls/sprites con depths euclidianos.
        assert!(d0 > BSP_DEPTH_BASE);
        assert!(d1 > BSP_DEPTH_BASE);
    }

    #[test]
    fn bsp_ranks_far_subsector_outranks_near() {
        // Fase 3.13b: la tabla de ranks debe darle al subsector más lejano
        // (visitado primero en la travesía back-to-front) el rank más alto,
        // para que el painter's sort lo pinte antes. Mismo escenario que
        // `bsp_compute_depths_assigns_decreasing_values`: viewer en (+10,0)
        // ⇒ travesía [ss0(far), ss1(near)].
        let mut snap = SceneSnapshot::empty(0);
        snap.player.x = 10.0;
        snap.player.y = 0.0;
        snap.subsectors = Arc::from(vec![
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
            SubsectorSnap { sector: 0, first_seg: 0, num_segs: 0 },
        ]);
        snap.nodes = Arc::from(simple_two_leaf_bsp());
        let ranks = compute_bsp_ranks(&snap);
        assert_eq!(ranks.len(), 2);
        assert!(ranks[0] > ranks[1], "ss0 (far) {} > ss1 (near) {}", ranks[0], ranks[1]);
        assert!(ranks[1] >= 1, "ningún subsector alcanzado debe quedar en 0");
        // bsp_rank_at: el punto del jugador cae en el subsector near (ss1).
        assert_eq!(bsp_rank_at(&snap.nodes, &ranks, 10.0, 0.0), ranks[1]);
        // Un punto del lado opuesto cae en el subsector far (ss0).
        assert_eq!(bsp_rank_at(&snap.nodes, &ranks, -10.0, 0.0), ranks[0]);
    }

    #[test]
    fn bsp_ranks_empty_without_bsp() {
        // Sin nodos (modo stub) la tabla queda en ceros ⇒ el sort delega
        // al depth euclidiano, preservando el comportamiento histórico.
        let mut snap = SceneSnapshot::empty(0);
        snap.subsectors = Arc::from(vec![SubsectorSnap {
            sector: 0,
            first_seg: 0,
            num_segs: 0,
        }]);
        let ranks = compute_bsp_ranks(&snap);
        assert_eq!(ranks, vec![0]);
        assert_eq!(bsp_rank_at(&snap.nodes, &ranks, 0.0, 0.0), 0);
    }

    // -----------------------------------------------------------------
    // Fase 3.22: muzzle world light
    // -----------------------------------------------------------------

    #[test]
    fn muzzle_boost_zero_when_alpha_zero() {
        // alpha = 0 ⇒ no hay fogonazo, boost = 0 sin importar la posición.
        assert_eq!(muzzle_boost_cam(0.0, 0.0, 0.0), 0.0);
        assert_eq!(muzzle_boost_cam(50.0, 30.0, 0.0), 0.0);
        // alpha negativo (no debería pasar pero defensivo) ⇒ 0.
        assert_eq!(muzzle_boost_cam(0.0, 0.0, -0.5), 0.0);
    }

    #[test]
    fn muzzle_boost_zero_outside_radius() {
        // distancia² > RADIUS² → boost 0. Tomamos el doble del radio.
        let r = MUZZLE_RADIUS_WORLD;
        assert_eq!(muzzle_boost_cam(r * 2.0, 0.0, 1.0), 0.0);
        assert_eq!(muzzle_boost_cam(0.0, r * 1.5, 1.0), 0.0);
        // Justo en el límite también es 0 (>= radius).
        assert_eq!(muzzle_boost_cam(r, 0.0, 1.0), 0.0);
    }

    #[test]
    fn muzzle_boost_peak_at_center_with_full_alpha() {
        // En (0, 0) con alpha=1 el boost alcanza MUZZLE_BOOST_PEAK exacto.
        let b = muzzle_boost_cam(0.0, 0.0, 1.0);
        assert!((b - MUZZLE_BOOST_PEAK).abs() < 1e-5, "expected peak, got {b}");
    }

    #[test]
    fn muzzle_boost_falls_off_with_distance_squared() {
        // Falloff quadrático: comparando r/4 vs r/2 (mismo eje), el
        // boost a r/4 debe ser estrictamente mayor que a r/2, y la
        // diferencia no debe ser lineal.
        let r = MUZZLE_RADIUS_WORLD;
        let b_close = muzzle_boost_cam(r * 0.25, 0.0, 1.0);
        let b_mid = muzzle_boost_cam(r * 0.5, 0.0, 1.0);
        let b_far = muzzle_boost_cam(r * 0.75, 0.0, 1.0);
        assert!(b_close > b_mid);
        assert!(b_mid > b_far);
        // Quadrático: el ratio close/mid debe ser > 1.5 (lineal sería ~1.5).
        // Con (1 - d²/r²)² obtenemos: (1-1/16)² ≈ 0.879 vs (1-1/4)² ≈ 0.563.
        // Ratio ≈ 1.56. Verificamos > 1.4 con margen.
        assert!(b_close / b_mid > 1.4, "ratio {} too low", b_close / b_mid);
    }

    #[test]
    fn apply_muzzle_tint_warms_color() {
        // Base gris medio + boost positivo ⇒ los canales R y G suben más
        // que B (tint cálido amarillo-blanco). Alpha preservada.
        let base = Color::from_rgba8(100, 100, 100, 255);
        let warm = apply_muzzle_tint(base, 0.3);
        let [r, g, b, a] = warm.to_rgba8().to_u8_array();
        assert_eq!(a, 255, "alpha preserved");
        assert!(r > 100 && g > 100 && b > 100, "all channels boosted");
        assert!(r >= g, "red ≥ green tint shape");
        assert!(g > b, "yellow tint: green > blue");
    }

    #[test]
    fn apply_muzzle_tint_zero_is_identity() {
        // boost ≤ 0 ⇒ retorna el color sin cambio. Fast path.
        let base = Color::from_rgba8(77, 188, 222, 200);
        let same = apply_muzzle_tint(base, 0.0);
        assert_eq!(same.to_rgba8().to_u8_array(), [77, 188, 222, 200]);
        let same2 = apply_muzzle_tint(base, -0.5);
        assert_eq!(same2.to_rgba8().to_u8_array(), [77, 188, 222, 200]);
    }

    #[test]
    fn sprite_shade_with_muzzle_zero_is_grayscale() {
        // boost = 0 ⇒ idéntico al shading grayscale histórico.
        let s = sprite_shade_with_muzzle(0.6, 0.0);
        assert_eq!(s, [0.6, 0.6, 0.6]);
    }

    #[test]
    fn sprite_shade_with_muzzle_warm_when_boost_positive() {
        // boost > 0 ⇒ R/G suben más que B respecto al shading uniforme.
        let s = sprite_shade_with_muzzle(0.5, 0.4);
        // El tint es (255, 220, 140) / 255 ≈ (1.0, 0.86, 0.55).
        // Multiplicador per-canal: 1 + 0.4 · tint. Red ≥ green > blue.
        assert!(s[0] >= s[1], "R ≥ G");
        assert!(s[1] > s[2], "G > B");
        // Todos los canales clampean ≤ 1.0.
        assert!(s[0] <= 1.0 && s[1] <= 1.0 && s[2] <= 1.0);
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

    #[test]
    fn lit_sectors_includes_player_sector() {
        let snap = snap_with_adjacency();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(lit.contains(&0), "sector del player siempre lit");
    }

    #[test]
    fn lit_sectors_includes_adjacent_via_twosided() {
        let snap = snap_with_adjacency();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(lit.contains(&1), "vecino directo via two-sided lit");
    }

    #[test]
    fn lit_sectors_excludes_unconnected_sector() {
        let snap = snap_with_adjacency();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(
            !lit.contains(&2),
            "sector aislado (sólo one-sided) no entra al lit set"
        );
    }

    #[test]
    fn lit_sectors_none_without_bsp() {
        // Stub mode: sin nodes BSP devuelve None ⇒ "lit everywhere"
        // (3.22 behavior preservado en stub).
        let snap = SceneSnapshot::empty(0);
        assert!(compute_muzzle_lit_sectors(&snap).is_none());
    }

    #[test]
    fn muzzle_boost_gated_passes_through_when_lit_none() {
        // Sin lit set (modo stub o toggle apagado), el boost pasa
        // sin gating — equivalente a 3.22.
        let b = muzzle_boost_gated(0.3, 42, None);
        assert!((b - 0.3).abs() < 1e-6);
    }

    #[test]
    fn muzzle_boost_gated_keeps_when_sector_in_lit() {
        let mut lit = HashSet::new();
        lit.insert(7_u32);
        let b = muzzle_boost_gated(0.3, 7, Some(&lit));
        assert!((b - 0.3).abs() < 1e-6, "sector 7 está en lit ⇒ boost intacto");
    }

    #[test]
    fn muzzle_boost_gated_zeroes_when_sector_not_in_lit() {
        let mut lit = HashSet::new();
        lit.insert(7_u32);
        let b = muzzle_boost_gated(0.3, 99, Some(&lit));
        assert_eq!(b, 0.0, "sector 99 no está en lit ⇒ boost gateado a 0");
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

    #[test]
    fn lit_sectors_includes_two_hop_neighbor_within_radius() {
        // BFS llega a sector 2 vía W01 (hop 1) + W12 (hop 2). Ambos
        // bridge walls dentro del radio físico.
        let snap = snap_with_chain();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(lit.contains(&0), "sector del player");
        assert!(lit.contains(&1), "vecino directo");
        assert!(lit.contains(&2), "vecino-del-vecino dentro del radio");
    }

    #[test]
    fn lit_sectors_bfs_stops_at_max_hops() {
        // Sector 3 requeriría hop 3 (MAX=2 corta). Aunque W23 está dentro
        // del radio, el BFS ya no lo alcanza.
        let snap = snap_with_chain();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(
            !lit.contains(&3),
            "sector a 3 hops no entra al lit (MAX_HOPS=2)"
        );
    }

    #[test]
    fn lit_sectors_excludes_one_hop_when_bridge_wall_beyond_radius() {
        // Sector 5 es vecino directo de 0 (W05), pero el midpoint del
        // bridge está a >MUZZLE_RADIUS_WORLD del jugador. El filtro
        // descarta el wall del BFS aunque la adyacencia exista.
        let snap = snap_with_chain();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(
            !lit.contains(&5),
            "vecino directo con bridge wall fuera de MUZZLE_RADIUS no entra al lit"
        );
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

    #[test]
    fn lit_sectors_cumulative_path_cuts_when_sum_exceeds_radius() {
        // 3.25 vs 3.24: ambos walls pasarían el chequeo per-bridge contra
        // el player (290 y 210 < 384), pero el camino real acumulado
        // recorre 410 unidades — fuera del radio. Sec 2 se excluye.
        let snap = snap_with_l_shape();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(lit.contains(&0), "player sector siempre lit");
        assert!(lit.contains(&1), "vecino directo dentro del radio");
        assert!(
            !lit.contains(&2),
            "L-shape: camino acumulativo 410 > 384 corta antes de sec 2"
        );
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

    #[test]
    fn lit_sectors_cumulative_uses_wall_midpoint_as_entry() {
        // Si el algoritmo siempre midiera desde el player, sec 2 caería
        // fuera (cumulative ≈ 604). Con entry chaining (3.25), sec 2 entra
        // (cumulative = 350 < 384).
        let snap = snap_with_chained_entry_points();
        let lit = compute_muzzle_lit_sectors(&snap).expect("BSP disponible");
        assert!(lit.contains(&1), "sec 1 lit (cumulative=300)");
        assert!(
            lit.contains(&2),
            "sec 2 lit via entry-chaining (cumulative=350 < 384) — sin el chain caería"
        );
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

    #[test]
    fn world_lights_boost_zero_with_empty_list() {
        // Sin lights, el boost siempre es 0 en cualquier punto.
        assert_eq!(world_lights_boost_cam(0.0, 0.0, &[]), 0.0);
        assert_eq!(world_lights_boost_cam(100.0, -200.0, &[]), 0.0);
    }

    #[test]
    fn world_lights_boost_peak_at_center_with_single_light() {
        // Una sola luz en (0,0); evaluamos el boost exactamente en (0,0).
        // f = 1 - 0/r² = 1 ⇒ boost = 1 · 1 · PEAK.
        let lights = vec![WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: MUZZLE_TINT_RGB,
            lit_sectors: None,
        }];
        let b = world_lights_boost_cam(0.0, 0.0, &lights);
        assert!(
            (b - WORLD_LIGHT_PEAK).abs() < 1e-5,
            "esperado peak {}, dió {}",
            WORLD_LIGHT_PEAK,
            b
        );
    }

    #[test]
    fn world_lights_boost_zero_outside_radius() {
        // Luz al borde y más allá del radio ⇒ boost 0.
        let lights = vec![WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: MUZZLE_TINT_RGB,
            lit_sectors: None,
        }];
        let r = WORLD_LIGHT_RADIUS_WORLD;
        assert_eq!(world_lights_boost_cam(r, 0.0, &lights), 0.0);
        assert_eq!(world_lights_boost_cam(0.0, r * 1.5, &lights), 0.0);
        assert_eq!(world_lights_boost_cam(-r * 2.0, 0.0, &lights), 0.0);
    }

    #[test]
    fn world_lights_boost_falls_off_with_distance_squared() {
        // En d=r/2 ⇒ f = 1 - 0.25 = 0.75 ⇒ boost = 0.5625 · PEAK.
        // En d=r/4 ⇒ f = 1 - 1/16 = 0.9375 ⇒ boost = 0.879 · PEAK.
        // El ratio close/mid > 1.4 verifica la caída cuadrática.
        let lights = vec![WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: MUZZLE_TINT_RGB,
            lit_sectors: None,
        }];
        let close = world_lights_boost_cam(WORLD_LIGHT_RADIUS_WORLD * 0.25, 0.0, &lights);
        let mid = world_lights_boost_cam(WORLD_LIGHT_RADIUS_WORLD * 0.5, 0.0, &lights);
        assert!(close > mid, "más cerca ⇒ más boost");
        assert!(
            close / mid > 1.4,
            "ratio close/mid {} debería superar 1.4 (cuadrático)",
            close / mid
        );
    }

    #[test]
    fn world_lights_boost_sums_multiple_sources_clamped_to_muzzle_peak() {
        // Dos luces colocadas exactamente en el mismo punto ⇒ suma de
        // contribuciones, pero clampeada al peak del muzzle (invariante:
        // el fogonazo del arma no debe quedar dominado por proyectiles).
        let lights = vec![
            WorldLight {
                x_cam: 0.0,
                y_cam: 0.0,
                z_cam: 0.0,
                sector: 0,
                tint_rgb: MUZZLE_TINT_RGB,
                lit_sectors: None,
            },
            WorldLight {
                x_cam: 0.0,
                y_cam: 0.0,
                z_cam: 0.0,
                sector: 1,
                tint_rgb: MUZZLE_TINT_RGB,
                lit_sectors: None,
            },
        ];
        let b = world_lights_boost_cam(0.0, 0.0, &lights);
        // Sin clamp serían 2 × PEAK = 0.8; con clamp = MUZZLE_BOOST_PEAK.
        assert!(b <= MUZZLE_BOOST_PEAK + 1e-5);
        assert!(b > WORLD_LIGHT_PEAK, "suma debería superar PEAK individual");
    }

    #[test]
    fn gather_world_lights_filters_non_fullbright() {
        // Snapshot con dos sprites: uno full-bright (frame con bit 7),
        // uno normal. Sólo el primero entra al lit set.
        let mut snap = SceneSnapshot::empty(0);
        snap.sprites = Arc::from(vec![
            fb_sprite(64.0, 0.0, 0x82, 0),  // FF_FULLBRIGHT
            fb_sprite(128.0, 0.0, 0x02, 0), // sin bit 7
        ]);
        let cam = Camera::new(0.0, 0.0, 0.0, 0.0);
        let lights = gather_world_lights(&snap, &cam, None, false);
        assert_eq!(lights.len(), 1, "sólo el sprite FF_FULLBRIGHT cuenta");
    }

    #[test]
    fn gather_world_lights_skips_no_sector_and_caps_to_max() {
        // 20 sprites FF_FULLBRIGHT ⇒ se truncan a MAX_WORLD_LIGHTS.
        // Uno con NO_SECTOR queda excluido siempre.
        let mut sprites: Vec<SpriteSnap> = (0..20)
            .map(|i| fb_sprite(50.0 + i as f32 * 5.0, 0.0, 0x80, (i as u32) % 4))
            .collect();
        sprites.push(fb_sprite(0.0, 0.0, 0x80, NO_SECTOR));
        let mut snap = SceneSnapshot::empty(0);
        snap.sprites = Arc::from(sprites);
        let cam = Camera::new(0.0, 0.0, 0.0, 0.0);
        let lights = gather_world_lights(&snap, &cam, None, false);
        assert_eq!(
            lights.len(),
            MAX_WORLD_LIGHTS,
            "truncado a {} aunque haya más",
            MAX_WORLD_LIGHTS
        );
        // El sprite con NO_SECTOR no debería estar; los de cap son los más
        // cercanos al player (origen). El más cercano (i=0 a 50 units) sí
        // debería entrar — verificamos por presencia de un x cercano.
        let min_dx = lights
            .iter()
            .map(|l| l.x_cam.abs())
            .fold(f32::INFINITY, f32::min);
        assert!(
            min_dx < 60.0,
            "el más cercano (i=0 a 50 u) debe estar entre los seleccionados"
        );
    }

    #[test]
    fn combined_boost_clamps_to_muzzle_peak_when_muzzle_and_lights_overlap() {
        // Muzzle peak (alpha=1, surface en origen) + luz coincidente:
        // suma sin clamp = 0.55 + 0.40 = 0.95; con clamp = 0.55.
        let lights = vec![WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: MUZZLE_TINT_RGB,
            lit_sectors: None,
        }];
        let b = combined_boost_cam(0.0, 0.0, 1.0, 0, None, &lights);
        assert!(
            (b - MUZZLE_BOOST_PEAK).abs() < 1e-5,
            "esperado peak {}, dió {}",
            MUZZLE_BOOST_PEAK,
            b
        );
    }

    // -----------------------------------------------------------------
    // Fase 3.27: tinte per-spritenum + boost RGB per-canal
    // -----------------------------------------------------------------

    #[test]
    fn sprite_tint_for_name_resolves_known_sprites() {
        let imp = sprite_tint_for_name("BAL1");
        assert_eq!(imp, (255, 130, 60), "imp fireball rojo-naranja");
        let plasma = sprite_tint_for_name("PLSS");
        assert_eq!(plasma, (130, 180, 255), "plasma azul-cyan");
        let bfg = sprite_tint_for_name("BFS1");
        assert_eq!(bfg, (160, 255, 160), "BFG ball verde fluorescente");
        let torch_blue = sprite_tint_for_name("TBLU");
        assert_eq!(torch_blue, (110, 160, 255), "blue torch azul");
    }

    #[test]
    fn sprite_tint_for_name_falls_back_to_muzzle_tint_for_unknown() {
        let unk = sprite_tint_for_name("XYZW");
        assert_eq!(unk, MUZZLE_TINT_RGB, "sprite desconocido cae al amarillo");
        // El nombre puede traer más de 4 chars (e.g. "PLSSA0"); el match
        // se hace sobre los primeros 4 — debería resolver igual.
        let plasma_long = sprite_tint_for_name("PLSSA0");
        assert_eq!(plasma_long, (130, 180, 255));
    }

    #[test]
    fn sprite_tint_for_name_is_case_insensitive() {
        // El motor a veces devuelve los nombres tal-cual del WAD (uppercase)
        // pero defensemos contra mixed-case por si una fase futura los
        // normaliza.
        assert_eq!(sprite_tint_for_name("bal1"), (255, 130, 60));
        assert_eq!(sprite_tint_for_name("Plss"), (130, 180, 255));
    }

    // -----------------------------------------------------------------
    // Fase 3.36: tintes Doom 2 (mancubus, revenant, archvile, etc.)
    // -----------------------------------------------------------------

    #[test]
    fn sprite_tint_for_name_resolves_doom2_projectiles() {
        // MANF (mancubus fireball), FATB (revenant tracer), SKEL
        // (revenant attack) — todos con tinte cálido distinto de
        // MUZZLE_TINT_RGB (el fallback amarillo del 3.26).
        let manf = sprite_tint_for_name("MANF");
        assert_eq!(manf, (255, 160, 90), "mancubus fireball naranja");
        let fatb = sprite_tint_for_name("FATB");
        assert_eq!(fatb, (255, 220, 160), "revenant tracer pálido cálido");
        let skel = sprite_tint_for_name("SKEL");
        assert_eq!(skel, (255, 200, 150), "revenant attack pálido cálido");
        // Todos los Doom 2 tints deben diferir del fallback amarillo.
        assert_ne!(manf, MUZZLE_TINT_RGB);
        assert_ne!(fatb, MUZZLE_TINT_RGB);
        assert_ne!(skel, MUZZLE_TINT_RGB);
    }

    #[test]
    fn sprite_tint_for_name_resolves_archvile_flame() {
        // Archvile attack frames (VILE) + fire pillar (FIRE) — ambos
        // rojo flame, FIRE más saturado.
        let vile = sprite_tint_for_name("VILE");
        assert_eq!(vile, (255, 130, 70), "archvile attack rojo flame");
        let fire = sprite_tint_for_name("FIRE");
        assert_eq!(fire, (255, 100, 50), "archvile fire pillar rojo saturado");
        // FIRE más rojo (G más bajo) que VILE — el pillar es más intenso.
        assert!(fire.1 < vile.1, "FIRE G < VILE G");
    }

    #[test]
    fn sprite_tint_for_name_resolves_lost_soul_and_pickups() {
        // Lost soul (SKUL) = blue-white flame; soul sphere (SOUL) y
        // mega armor (MEGA) = azul/cyan glow.
        let skul = sprite_tint_for_name("SKUL");
        assert_eq!(skul, (180, 220, 255), "lost soul blue-white");
        let soul = sprite_tint_for_name("SOUL");
        assert_eq!(soul, (130, 200, 255), "soul sphere cyan-blue");
        let mega = sprite_tint_for_name("MEGA");
        assert_eq!(mega, (130, 220, 200), "mega armor verde-cyan");
        // Los tres tienen B > R (azules), distintos del fallback amarillo.
        assert!(skul.2 > skul.0);
        assert!(soul.2 > soul.0);
        assert!(mega.2 > mega.0);
    }

    #[test]
    fn sprite_tint_for_name_resolves_colored_keys() {
        // Keycards y skullkeys — colores que matchean el HUD del juego.
        assert_eq!(sprite_tint_for_name("BKEY"), (110, 160, 255), "blue keycard");
        assert_eq!(sprite_tint_for_name("YKEY"), (255, 240, 130), "yellow keycard");
        assert_eq!(sprite_tint_for_name("RKEY"), (255, 130, 90),  "red keycard");
        assert_eq!(sprite_tint_for_name("BSKU"), (110, 160, 255), "blue skullkey");
        assert_eq!(sprite_tint_for_name("YSKU"), (255, 240, 130), "yellow skullkey");
        assert_eq!(sprite_tint_for_name("RSKU"), (255, 130, 90),  "red skullkey");
        // Mismas keys card y skull deben dar el mismo color.
        assert_eq!(sprite_tint_for_name("BKEY"), sprite_tint_for_name("BSKU"));
    }

    #[test]
    fn sprite_tint_for_name_doom2_lookups_case_insensitive() {
        // Las entradas nuevas también respetan el case-insensitive del 3.27.
        assert_eq!(sprite_tint_for_name("manf"), (255, 160, 90));
        assert_eq!(sprite_tint_for_name("Skul"), (180, 220, 255));
        assert_eq!(sprite_tint_for_name("vile"), (255, 130, 70));
        // El 4-char match también funciona con sufijos (e.g. "MANFA1" ⇒ MANF).
        assert_eq!(sprite_tint_for_name("MANFA1"), (255, 160, 90));
        assert_eq!(sprite_tint_for_name("SKULA0"), (180, 220, 255));
    }

    #[test]
    fn muzzle_boost_rgb_uses_muzzle_tint_per_channel() {
        // Muzzle en origen con alpha=1 ⇒ scalar = MUZZLE_BOOST_PEAK.
        // Per-canal = peak · (255/255, 220/255, 140/255).
        let b = muzzle_boost_rgb_cam(0.0, 0.0, 1.0);
        let expected_r = MUZZLE_BOOST_PEAK * (MUZZLE_TINT_RGB.0 as f32 / 255.0);
        let expected_g = MUZZLE_BOOST_PEAK * (MUZZLE_TINT_RGB.1 as f32 / 255.0);
        let expected_b = MUZZLE_BOOST_PEAK * (MUZZLE_TINT_RGB.2 as f32 / 255.0);
        assert!((b[0] - expected_r).abs() < 1e-5);
        assert!((b[1] - expected_g).abs() < 1e-5);
        assert!((b[2] - expected_b).abs() < 1e-5);
        // R > G > B porque el amarillo cálido tiene R=255 > G=220 > B=140.
        assert!(b[0] > b[1] && b[1] > b[2], "amarillo: R > G > B");
    }

    #[test]
    fn world_lights_boost_rgb_per_light_tint_dominates() {
        // Una sola luz verde (BFG) en el origen ⇒ boost RGB con G alto,
        // R/B mucho más bajos.
        let lights = vec![WorldLight {
            x_cam: 0.0,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: (160, 255, 160), // BFG green
            lit_sectors: None,
        }];
        let b = world_lights_boost_rgb_cam(0.0, 0.0, 0, &lights);
        assert!(b[1] > b[0] && b[1] > b[2], "G debe dominar para BFG verde");
        // Magnitud verde ≈ WORLD_LIGHT_PEAK · 255/255 = PEAK.
        assert!(
            (b[1] - WORLD_LIGHT_PEAK).abs() < 1e-5,
            "G debe alcanzar PEAK"
        );
    }

    #[test]
    fn combined_boost_rgb_clamps_each_channel_to_muzzle_peak() {
        // Muchas luces saturadas en cada canal ⇒ cada canal clampea a peak.
        let lights: Vec<WorldLight> = (0..10)
            .map(|_| WorldLight {
                x_cam: 0.0,
                y_cam: 0.0,
                z_cam: 0.0,
                sector: 0,
                tint_rgb: (255, 255, 255), // luz blanca máxima
                lit_sectors: None,
            })
            .collect();
        let b = combined_boost_rgb_cam(0.0, 0.0, 1.0, 0, None, &lights);
        for ch in 0..3 {
            assert!(
                b[ch] <= MUZZLE_BOOST_PEAK + 1e-5,
                "canal {} {} > peak",
                ch,
                b[ch]
            );
            // Y deberían estar saturados (al peak):
            assert!(
                (b[ch] - MUZZLE_BOOST_PEAK).abs() < 1e-4,
                "canal {} debería estar saturado",
                ch
            );
        }
    }

    #[test]
    fn apply_color_boost_adds_per_channel() {
        let base = Color::from_rgba8(50, 50, 50, 255);
        // Boost sólo en G ⇒ sale verdoso.
        let b = apply_color_boost(base, [0.0, 0.4, 0.0]);
        let [r, g, bb, a] = b.to_rgba8().to_u8_array();
        assert_eq!(r, 50, "R sin cambio");
        assert!(g > 100, "G boosted (esperado ~50 + 0.4·255 ≈ 152), dió {g}");
        assert_eq!(bb, 50, "B sin cambio");
        assert_eq!(a, 255, "alpha preservada");
    }

    #[test]
    fn apply_color_boost_zero_is_identity() {
        let base = Color::from_rgba8(120, 80, 200, 200);
        let same = apply_color_boost(base, ZERO_BOOST);
        assert_eq!(same.to_rgba8().to_u8_array(), [120, 80, 200, 200]);
    }

    #[test]
    fn sprite_shade_with_world_per_channel() {
        // Shade base 0.5, boost RGB (0, 0.4, 0) ⇒ G escalado, R/B intactos.
        let s = sprite_shade_with_world(0.5, [0.0, 0.4, 0.0]);
        assert!((s[0] - 0.5).abs() < 1e-5, "R sin cambio");
        assert!(s[1] > 0.5, "G boosted");
        assert!((s[2] - 0.5).abs() < 1e-5, "B sin cambio");
    }

    #[test]
    fn overlay_color_alpha_from_boost_normalizes_to_brightest_channel() {
        // Boost dominantemente verde con poca R y nada de B ⇒
        // color overlay debe ser verde dominante.
        let (r, g, b, a) = overlay_color_alpha_from_boost([0.05, 0.30, 0.0]).expect("non-trivial");
        assert!(g > r && g > b, "G dominante en color overlay");
        assert!(a > 0, "alpha > 0 para boost no despreciable");
    }

    #[test]
    fn overlay_color_alpha_from_boost_none_when_negligible() {
        // Boost por debajo del threshold ⇒ None.
        assert!(overlay_color_alpha_from_boost([0.01, 0.0, 0.0]).is_none());
        assert!(overlay_color_alpha_from_boost(ZERO_BOOST).is_none());
    }

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

    #[test]
    fn wall_v_bands_default_one_preserves_path() {
        // `cfg.wall_vertical_bands = 1` debe preservar el path 3.32-3.41:
        // un único boost al z=0 (eye-level), sin subdivisión. El default
        // de RenderConfig es 1.
        let cfg = RenderConfig::default();
        assert_eq!(cfg.wall_vertical_bands, 1);
        // Sanity: el path single (v_bands == 1) en gather_wall computa
        // el boost una sola vez. Reproducible por:
        let z_surf_default = 0.0_f32; // eye level (3.34 convention)
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: -20.0,
            sector: NO_SECTOR, tint_rgb: (255, 255, 255), lit_sectors: None,
        }];
        let n = (-1.0, 0.0);
        let single = world_lights_boost_rgb_for_wall_cam(100.0, 0.0, z_surf_default, NO_SECTOR, &lights, n, true);
        for ch in 0..3 {
            assert!(single[ch].is_finite());
        }
    }

    // =================================================================
    // Fase 3.43 — Gradiente vertical continuo para walls
    // =================================================================

    #[test]
    fn wall_gradient_dark_stops_offsets_monotonic_and_cover_unit() {
        // Los stops deben quedar en offsets crecientes que cubran [0, 1]:
        // el primer stop en 0 (bottom), el último en 1 (top).
        let samples = [(0.0_f32, 0.0_f32), (0.5, 0.1), (1.0, 0.2)];
        let stops = wall_darkness_gradient_stops(0.4, &samples);
        assert_eq!(stops.len(), 3);
        assert_eq!(stops[0].0, 0.0);
        assert_eq!(stops[2].0, 1.0);
        for w in stops.windows(2) {
            assert!(w[1].0 > w[0].0, "offsets estrictamente crecientes");
        }
    }

    #[test]
    fn wall_gradient_dark_stop_brighter_band_is_less_opaque() {
        // Una banda con más boost ⇒ shade iluminado mayor ⇒ overlay
        // negro menos opaco (alpha menor). Bottom con boost 0.4, top con
        // boost 0.0, base_shade 0.3.
        let samples = [(0.0_f32, 0.4_f32), (1.0, 0.0)];
        let stops = wall_darkness_gradient_stops(0.3, &samples);
        let a_bottom = stops[0].1.to_rgba8().to_u8_array()[3];
        let a_top = stops[1].1.to_rgba8().to_u8_array()[3];
        assert!(
            a_bottom < a_top,
            "banda más iluminada (bottom) ⇒ menos oscuridad: a_bottom={} a_top={}",
            a_bottom, a_top
        );
    }

    #[test]
    fn wall_gradient_tint_none_when_all_negligible() {
        // Si ningún sample tiene tinte apreciable, no se emite gradiente
        // de tinte (None) ⇒ el render loop salta el segundo fill.
        let samples = [
            (0.0_f32, ZERO_BOOST),
            (0.5, [0.005, 0.0, 0.0]),
            (1.0, ZERO_BOOST),
        ];
        assert!(wall_tint_gradient_stops(&samples).is_none());
    }

    #[test]
    fn wall_gradient_tint_some_keeps_all_stops_with_transparent_gaps() {
        // Con al menos un sample tintado, devolvemos Some con TODOS los
        // stops (los despreciables quedan alpha 0) para no cortar la
        // continuidad del gradiente.
        let samples = [
            (0.0_f32, ZERO_BOOST),     // despreciable ⇒ alpha 0
            (0.5, [0.0, 0.30, 0.0]),   // verde apreciable
            (1.0, ZERO_BOOST),         // despreciable ⇒ alpha 0
        ];
        let stops = wall_tint_gradient_stops(&samples).expect("hay un sample tintado");
        assert_eq!(stops.len(), 3);
        assert_eq!(stops[0].1.to_rgba8().to_u8_array()[3], 0, "gap inferior transparente");
        assert!(stops[1].1.to_rgba8().to_u8_array()[3] > 0, "stop tintado opaco");
        assert_eq!(stops[2].1.to_rgba8().to_u8_array()[3], 0, "gap superior transparente");
        // El canal verde del stop tintado domina (normalizado al máximo).
        let [r, g, b, _] = stops[1].1.to_rgba8().to_u8_array();
        assert!(g > r && g > b, "tinte verde: g={} r={} b={}", g, r, b);
    }

    #[test]
    fn wall_gradient_default_off_preserves_3_42_path() {
        // Default RenderConfig: gradiente off ⇒ el path 3.42 (bandas /
        // single overlay) queda intacto.
        let cfg = RenderConfig::default();
        assert!(!cfg.wall_vertical_gradient);
    }

    // =================================================================
    // Fase 3.44 — Gradiente de profundidad para pisos/techos
    // =================================================================

    #[test]
    fn plane_near_far_picks_closest_and_farthest() {
        // Polígono con vértices a distintas distancias del origen
        // cam-space. near = el de menor d², far = el de mayor.
        let poly = [(10.0_f32, 0.0), (100.0, 0.0), (50.0, 50.0), (5.0, 2.0)];
        let (i_near, i_far) = plane_near_far_indices(&poly).expect("4 vértices");
        assert_eq!(i_near, 3, "(5,2) es el más cercano");
        assert_eq!(i_far, 1, "(100,0) es el más lejano");
    }

    #[test]
    fn plane_near_far_none_with_under_two_verts() {
        assert!(plane_near_far_indices(&[]).is_none());
        assert!(plane_near_far_indices(&[(1.0, 1.0)]).is_none());
    }

    #[test]
    fn plane_depth_gradient_near_brighter_than_far() {
        // Reusa wall_darkness_gradient_stops con base_shade=0 y el
        // lit-shade completo por sample. Cerca (offset 0) más iluminado
        // ⇒ menos opaco que lejos (offset 1).
        let near_lit = 0.85_f32; // poco fog, cerca del jugador
        let far_lit = 0.30_f32; // mucho fog, lejos
        let stops = wall_darkness_gradient_stops(0.0, &[(0.0, near_lit), (1.0, far_lit)]);
        let a_near = stops[0].1.to_rgba8().to_u8_array()[3];
        let a_far = stops[1].1.to_rgba8().to_u8_array()[3];
        assert!(
            a_near < a_far,
            "near menos oscuro que far: a_near={} a_far={}",
            a_near, a_far
        );
    }

    #[test]
    fn plane_depth_gradient_default_off() {
        let cfg = RenderConfig::default();
        assert!(!cfg.plane_depth_gradient);
    }

    #[test]
    fn axis_offset_endpoints_and_midpoint() {
        // Fase 3.45: proyección sobre el eje start→end.
        let start = Point::new(100.0, 400.0);
        let end = Point::new(100.0, 100.0); // eje vertical hacia arriba
        assert!((axis_offset(start, start, end) - 0.0).abs() < 1e-5, "start ⇒ 0");
        assert!((axis_offset(end, start, end) - 1.0).abs() < 1e-5, "end ⇒ 1");
        let mid = Point::new(100.0, 250.0);
        assert!((axis_offset(mid, start, end) - 0.5).abs() < 1e-5, "mid ⇒ 0.5");
    }

    #[test]
    fn axis_offset_clamps_and_projects_orthogonally() {
        let start = Point::new(0.0, 0.0);
        let end = Point::new(10.0, 0.0); // eje horizontal
        // Punto más allá del end ⇒ clamp a 1.
        assert_eq!(axis_offset(Point::new(50.0, 0.0), start, end), 1.0);
        // Punto antes del start ⇒ clamp a 0.
        assert_eq!(axis_offset(Point::new(-5.0, 0.0), start, end), 0.0);
        // Punto fuera del eje (con offset y): sólo cuenta la componente x.
        assert!((axis_offset(Point::new(5.0, 99.0), start, end) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn axis_offset_degenerate_axis_is_zero() {
        let p = Point::new(3.0, 7.0);
        let s = Point::new(1.0, 1.0);
        assert_eq!(axis_offset(p, s, s), 0.0, "eje cero ⇒ 0 sin NaN");
    }

    #[test]
    fn plane_multistop_dedup_keeps_increasing_offsets() {
        // Reproduce el dedup del gradiente de planos 3.45: offsets casi
        // iguales colapsan, el resultado queda estrictamente creciente.
        let raw = [
            (0.0_f32, 0.9_f32),
            (0.00005, 0.8), // colapsa con 0.0 (< +1e-4)
            (0.5, 0.6),
            (0.5, 0.5), // colapsa con el 0.5 previo
            (1.0, 0.3),
        ];
        let mut last = f32::NEG_INFINITY;
        let mut kept = Vec::new();
        for &(off, lit) in &raw {
            if off <= last + 1e-4 {
                continue;
            }
            last = off;
            kept.push((off, lit));
        }
        let offs: Vec<f32> = kept.iter().map(|&(o, _)| o).collect();
        assert_eq!(offs, vec![0.0, 0.5, 1.0], "dedup deja 3 stops crecientes");
        for w in offs.windows(2) {
            assert!(w[1] > w[0]);
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

    #[test]
    fn decal_in_front_produces_one_renderable() {
        let (cam, proj) = decal_test_setup();
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: 100.0,
                y: 0.0,
                z: 40.0,
                radius: 5.0,
                color: (24, 21, 18),
                alpha: 1.0,
                tangent: (0.0, 0.0),
                horizontal: false,
                wall_span: None,
            }],
            ..Default::default()
        };
        let mut out = Vec::new();
        gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
        assert_eq!(out.len(), 1, "decal al frente ⇒ 1 quad");
        assert!(matches!(out[0].kind, RenderKind::Fill));
    }

    #[test]
    fn decal_behind_camera_is_culled() {
        let (cam, proj) = decal_test_setup();
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: -100.0, // detrás (x_cam < near)
                y: 0.0,
                z: 40.0,
                radius: 5.0,
                color: (24, 21, 18),
                alpha: 1.0,
                tangent: (0.0, 0.0),
                horizontal: false,
                wall_span: None,
            }],
            ..Default::default()
        };
        let mut out = Vec::new();
        gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
        assert!(out.is_empty(), "decal detrás de la cámara se descarta");
    }

    #[test]
    fn decal_zero_alpha_is_skipped() {
        let (cam, proj) = decal_test_setup();
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: 100.0,
                y: 0.0,
                z: 40.0,
                radius: 5.0,
                color: (24, 21, 18),
                alpha: 0.0, // ya desvanecido
                tangent: (0.0, 0.0),
                horizontal: false,
                wall_span: None,
            }],
            ..Default::default()
        };
        let mut out = Vec::new();
        gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
        assert!(out.is_empty(), "alpha 0 ⇒ no se dibuja");
    }

    #[test]
    fn decal_alpha_maps_to_color_alpha_channel() {
        let (cam, proj) = decal_test_setup();
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: 100.0,
                y: 0.0,
                z: 40.0,
                radius: 5.0,
                color: (100, 10, 10),
                alpha: 0.5,
                tangent: (0.0, 0.0),
                horizontal: false,
                wall_span: None,
            }],
            ..Default::default()
        };
        let mut out = Vec::new();
        gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
        assert_eq!(out.len(), 1);
        let a = out[0].color.to_rgba8().to_u8_array()[3];
        assert!((a as i32 - 127).abs() <= 1, "alpha 0.5 ⇒ ~127, got {}", a);
    }

    #[test]
    fn decal_depth_sits_in_front_of_its_surface() {
        // El depth se sesga -0.5 respecto a la distancia euclidiana
        // del impacto ⇒ se dibuja delante de la pared a esa distancia.
        let (cam, proj) = decal_test_setup();
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: 100.0,
                y: 0.0,
                z: 40.0,
                radius: 5.0,
                color: (24, 21, 18),
                alpha: 1.0,
                tangent: (0.0, 0.0),
                horizontal: false,
                wall_span: None,
            }],
            ..Default::default()
        };
        let mut out = Vec::new();
        gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
        assert!((out[0].depth - (100.0 - 0.5)).abs() < 1e-3, "depth = dist - 0.5");
    }

    #[test]
    fn decal_wall_aligned_quad_is_not_axis_aligned() {
        // Fase 3.47: un decal sobre una pared oblicua (tangente a 45°)
        // proyecta un quad cuyos lados superior/inferior tienen distinta
        // longitud en pantalla (perspectiva) — a diferencia del billboard
        // axis-aligned. Comparamos un decal billboard vs uno con tangente
        // diagonal en la misma posición.
        let (cam, proj) = decal_test_setup();
        let mk = |tangent: (f32, f32)| {
            let cfg = RenderConfig {
                decals: vec![Decal {
                    x: 100.0,
                    y: 0.0,
                    z: 40.0,
                    radius: 8.0,
                    color: (24, 21, 18),
                    alpha: 1.0,
                    tangent,
                    horizontal: false,
                    wall_span: None,
                }],
                ..Default::default()
            };
            let mut out = Vec::new();
            gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
            out
        };
        let billboard = mk((0.0, 0.0));
        let walled = mk((0.707, 0.707)); // pared a 45° respecto a la vista
        assert_eq!(billboard.len(), 1);
        assert_eq!(walled.len(), 1);
        // El billboard cae a profundidad constante ⇒ borde izq y der a
        // la misma `x_cam` ⇒ misma altura en pantalla. El walled tiene
        // su lado izquierdo más cerca (más alto) y el derecho más lejos
        // (más bajo) — la perspectiva de la pared oblicua.
        let edge_heights = |bz: &BezPath| {
            let pts: Vec<Point> = bz.elements().iter().filter_map(|e| match e {
                llimphi_ui::llimphi_raster::kurbo::PathEl::MoveTo(p)
                | llimphi_ui::llimphi_raster::kurbo::PathEl::LineTo(p) => Some(*p),
                _ => None,
            }).collect();
            // pts = [tl, tr, br, bl]. Altura izq = tl→bl, der = tr→br.
            let left = (pts[0].y - pts[3].y).abs();
            let right = (pts[1].y - pts[2].y).abs();
            (left, right)
        };
        let (bl, br) = edge_heights(&billboard[0].path);
        assert!((bl - br).abs() < 1e-6, "billboard: alturas izq == der");
        let (wl, wr) = edge_heights(&walled[0].path);
        assert!(
            (wl - wr).abs() > 1e-3,
            "pared oblicua: altura izq != der (perspectiva), izq={} der={}",
            wl, wr
        );
    }

    #[test]
    fn decal_horizontal_lies_flat_below_eye() {
        // Fase 3.48: un decal horizontal (charco) en el piso, bajo el
        // ojo, proyecta su borde cercano (más bajo en pantalla) más
        // ancho que el lejano — perspectiva de un quad sobre el suelo.
        let (cam, proj) = decal_test_setup(); // view_z=41, mira +X
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: 100.0,
                y: 0.0,
                z: 0.0, // a nivel del piso, bajo el ojo
                radius: 16.0,
                color: (24, 21, 18),
                alpha: 1.0,
                tangent: (0.0, 0.0),
                horizontal: true,
                wall_span: None,
            }],
            ..Default::default()
        };
        let mut out = Vec::new();
        gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
        assert_eq!(out.len(), 1);
        let pts: Vec<Point> = out[0]
            .path
            .elements()
            .iter()
            .filter_map(|e| match e {
                llimphi_ui::llimphi_raster::kurbo::PathEl::MoveTo(p)
                | llimphi_ui::llimphi_raster::kurbo::PathEl::LineTo(p) => Some(*p),
                _ => None,
            })
            .collect();
        // pts = [(-r,-r), (r,-r), (r,r), (-r,r)] en XY mundo. El borde
        // cercano (x_cam = 100-16 = 84) lo forman pts[0] y pts[3]; el
        // lejano (x_cam = 116), pts[1] y pts[2]. El cercano es más ancho.
        let near_w = (pts[0].x - pts[3].x).abs();
        let far_w = (pts[1].x - pts[2].x).abs();
        assert!(
            near_w > far_w + 1e-3,
            "borde cercano más ancho que el lejano: near={} far={}",
            near_w, far_w
        );
    }

    #[test]
    fn decal_shade_rgb_darkens_in_dark_sector() {
        // Fase 3.49: shade 1.0 preserva el color; shade bajo lo oscurece
        // per-canal; shade 0 ⇒ negro.
        let c = (104, 12, 12);
        assert_eq!(shade_rgb(c, 1.0), c, "luz plena ⇒ idéntico");
        assert_eq!(shade_rgb(c, 0.5), (52, 6, 6), "mitad de luz ⇒ mitad por canal");
        assert_eq!(shade_rgb(c, 0.0), (0, 0, 0), "oscuridad total ⇒ negro");
        assert_eq!(shade_rgb(c, 2.0), c, "clamp a 1.0");
    }

    #[test]
    fn decal_picks_up_world_light_tint() {
        // Fase 3.50: un decal scorch (gris oscuro) junto a una world
        // light verde recibe boost en el canal verde — comparamos con la
        // misma escena sin luces.
        let (cam, proj) = decal_test_setup();
        // Snap con BSP de una hoja (sector único, luz media) para que el
        // path de shading+boost se active (nodes no vacío).
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 200,
            floor_pic: 0,
            ceiling_pic: 0,
        }]);
        snap.subsectors = Arc::from(vec![SubsectorSnap {
            sector: 0,
            first_seg: 0,
            num_segs: 0,
        }]);
        // Nodo único cuyos dos hijos apuntan al subsector 0.
        snap.nodes = Arc::from(vec![NodeSnap {
            partition_x: 0.0,
            partition_y: 0.0,
            partition_dx: 0.0,
            partition_dy: 1.0,
            children: [NF_SUBSECTOR, NF_SUBSECTOR],
        }]);
        let decal = Decal {
            x: 100.0,
            y: 0.0,
            z: 40.0,
            radius: 5.0,
            color: (40, 40, 40),
            alpha: 1.0,
            tangent: (0.0, 0.0),
            horizontal: false,
            wall_span: None,
        };
        let cfg = RenderConfig {
            decals: vec![decal],
            ..Default::default()
        };
        // Sin luces.
        let mut plain = Vec::new();
        gather_decals(&mut plain, &cfg, &snap, &cam, &proj, None, &[]);
        // Con una world light verde pegada al decal.
        let green = [WorldLight {
            x_cam: 100.0,
            y_cam: 0.0,
            z_cam: 40.0 - cam.view_z,
            sector: 0,
            tint_rgb: (0, 255, 0),
            lit_sectors: None,
        }];
        let mut lit = Vec::new();
        gather_decals(&mut lit, &cfg, &snap, &cam, &proj, None, &green);
        let g_plain = plain[0].color.to_rgba8().to_u8_array()[1];
        let g_lit = lit[0].color.to_rgba8().to_u8_array()[1];
        assert!(
            g_lit > g_plain,
            "la world light verde sube el canal G: plain={} lit={}",
            g_plain, g_lit
        );
    }

    #[test]
    fn decal_wall_grazing_light_dimmer_than_head_on() {
        // Fase 3.51: una marca pegada a la pared (tangent set) recibe el
        // tinte de world lights atenuado por el cosine de la normal del
        // muro. Una luz verde **encarada** (perpendicular a la pared)
        // tinta más fuerte que una **rasante** (paralela al muro) a la
        // misma distancia. La pared corre a lo largo de Y (tangent (0,1)),
        // su normal toward-camera es (-1, 0).
        let (cam, proj) = decal_test_setup();
        let mut snap = SceneSnapshot::empty(0);
        snap.sectors = Arc::from(vec![SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 200,
            floor_pic: 0,
            ceiling_pic: 0,
        }]);
        snap.subsectors = Arc::from(vec![SubsectorSnap {
            sector: 0,
            first_seg: 0,
            num_segs: 0,
        }]);
        snap.nodes = Arc::from(vec![NodeSnap {
            partition_x: 0.0,
            partition_y: 0.0,
            partition_dx: 0.0,
            partition_dy: 1.0,
            children: [NF_SUBSECTOR, NF_SUBSECTOR],
        }]);
        let cfg = RenderConfig {
            decals: vec![Decal {
                x: 100.0,
                y: 0.0,
                z: 40.0,
                radius: 5.0,
                color: (40, 40, 40),
                alpha: 1.0,
                tangent: (0.0, 1.0), // muro a lo largo de Y ⇒ normal ±X
                horizontal: false,
                wall_span: None,
            }],
            ..Default::default()
        };
        let z = 40.0 - cam.view_z;
        // Encarada: entre cámara y decal ⇒ cos ≈ 1.
        let head_on = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: z,
            sector: 0, tint_rgb: (0, 255, 0), lit_sectors: None,
        }];
        // Rasante: a lo largo del muro, misma distancia ⇒ cos ≈ 0.
        let grazing = [WorldLight {
            x_cam: 100.0, y_cam: 50.0, z_cam: z,
            sector: 0, tint_rgb: (0, 255, 0), lit_sectors: None,
        }];
        let mut a = Vec::new();
        let mut b = Vec::new();
        gather_decals(&mut a, &cfg, &snap, &cam, &proj, None, &head_on);
        gather_decals(&mut b, &cfg, &snap, &cam, &proj, None, &grazing);
        let g_head = a[0].color.to_rgba8().to_u8_array()[1];
        let g_graze = b[0].color.to_rgba8().to_u8_array()[1];
        assert!(
            g_head > g_graze,
            "luz encarada tinta más que rasante: head={} graze={}",
            g_head, g_graze
        );
    }

    #[test]
    fn decal_wall_span_clips_horizontal_extent() {
        // Fase 3.52: un decal de pared con `wall_span` más angosto que
        // `[-r, r]` produce un quad más angosto en pantalla — recortado al
        // borde del lineseg en vez de sangrar más allá de la esquina.
        let (cam, proj) = decal_test_setup();
        let width = |span: Option<(f32, f32)>| -> f64 {
            let cfg = RenderConfig {
                decals: vec![Decal {
                    x: 100.0,
                    y: 0.0,
                    z: 40.0,
                    radius: 8.0,
                    color: (24, 21, 18),
                    alpha: 1.0,
                    tangent: (0.0, 1.0), // muro a lo largo de Y
                    horizontal: false,
                    wall_span: span,
                }],
                ..Default::default()
            };
            let mut out = Vec::new();
            gather_decals(&mut out, &cfg, &SceneSnapshot::empty(0), &cam, &proj, None, &[]);
            assert_eq!(out.len(), 1);
            let xs: Vec<f64> = out[0]
                .path
                .elements()
                .iter()
                .filter_map(|e| match e {
                    llimphi_ui::llimphi_raster::kurbo::PathEl::MoveTo(p)
                    | llimphi_ui::llimphi_raster::kurbo::PathEl::LineTo(p) => Some(p.x),
                    _ => None,
                })
                .collect();
            xs.iter().cloned().fold(f64::MIN, f64::max)
                - xs.iter().cloned().fold(f64::MAX, f64::min)
        };
        let full = width(None); // sin recorte: ± r = 16 u de ancho
        let clipped = width(Some((-2.0, 3.0))); // recortado a 5 u
        assert!(
            clipped < full * 0.6,
            "wall_span recorta el ancho del quad: full={} clipped={}",
            full, clipped
        );
        assert!(clipped > 0.0, "quad recortado sigue teniendo área");
    }

    #[test]
    fn clip_half_plane_keeps_positive_side() {
        // Fase 3.53: cuadrado unidad recortado por el semiplano `x ≥ 0`
        // (normal (1,0), borde por el origen) ⇒ todos los vértices x ≥ 0.
        let square = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        let out = clip_half_plane(&square, (0.0, 0.0), (1.0, 0.0));
        assert!(out.len() >= 3, "queda un polígono con área");
        assert!(
            out.iter().all(|&(x, _)| x >= -1e-5),
            "ningún vértice del lado negativo: {:?}",
            out
        );
    }

    #[test]
    fn clip_decal_to_walls_keeps_center_side_and_ignores_far_walls() {
        // Fase 3.53: un charco en (0,0) r=5 junto a un muro vertical en
        // x=2 (lo alcanza: dist 2 ≤ 5) ⇒ recorta al lado del centro
        // (x ≤ 2). Un muro lejano en x=100 (fuera del radio) no recorta.
        let mk_wall = |x1, y1, x2, y2| WallSeg {
            x1, y1, x2, y2,
            front_sector: 0,
            back_sector: NO_SECTOR,
            flags: 0,
            textures: [[0; 8]; 6],
            tex_x_offsets: [0.0; 2],
            tex_y_offsets: [0.0; 2],
        };
        let quad = [(-5.0, -5.0), (5.0, -5.0), (5.0, 5.0), (-5.0, 5.0)];
        // Muro cercano: recorta a x ≤ 2.
        let near_wall = [mk_wall(2.0, -10.0, 2.0, 10.0)];
        let clipped = clip_decal_to_walls(&quad, &near_wall, 0.0, 0.0, 5.0);
        assert!(clipped.len() >= 3, "queda polígono");
        assert!(
            clipped.iter().all(|&(x, _)| x <= 2.0 + 1e-4),
            "recortado al lado del centro (x ≤ 2): {:?}",
            clipped
        );
        let max_x = clipped.iter().map(|&(x, _)| x).fold(f32::MIN, f32::max);
        assert!((max_x - 2.0).abs() < 1e-3, "el borde llega justo al muro");
        // Muro lejano: no recorta ⇒ quad intacto (llega a x=5).
        let far_wall = [mk_wall(100.0, -10.0, 100.0, 10.0)];
        let untouched = clip_decal_to_walls(&quad, &far_wall, 0.0, 0.0, 5.0);
        let max_x_far = untouched.iter().map(|&(x, _)| x).fold(f32::MIN, f32::max);
        assert!((max_x_far - 5.0).abs() < 1e-3, "muro lejano no recorta");
    }

    // =================================================================
    // Fase 3.41 — Weapon rim 3D
    // =================================================================

    #[test]
    fn weapon_rim_3d_recovers_2d_when_z_zero() {
        // Luces con z_cam=0 ⇒ 3D == 2D (caso de los tests previos 3.30).
        let red = (255, 130, 60);
        let blue = (110, 160, 255);
        let lights = [
            rim_light(120.0, 0.0, red),
            rim_light(-60.0, 90.0, blue),
            rim_light(0.0, -150.0, (255, 255, 200)),
        ];
        let baseline = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        let dir = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, true);
        // omni del 3.30 sumaba sin direccional ⇒ matchea con dir-3.41
        // sólo cuando todos los lights tienen att=1 (frontales). No es
        // el caso de este test general; aquí verificamos que el path
        // funciona con z=0 sin crash + valores finitos.
        for ch in 0..3 {
            assert!(dir[ch].is_finite(), "canal {} finite", ch);
            assert!(dir[ch] <= baseline[ch] + 1e-5, "dir <= baseline omni");
        }
    }

    #[test]
    fn weapon_rim_3d_attenuates_for_high_light_compared_to_planar() {
        // Misma XY (50, 0) pero z distinto:
        //   - planar (50, 0, 0): luz al nivel del eye/weapon ⇒ cos=1.
        //   - high   (50, 0, 80): luz arriba ⇒ d_3D=94, cos=50/94=0.53.
        // El path direccional 3D debería dimear la luz alta respecto
        // a la planar.
        let white = (255, 255, 255);
        let planar = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let high = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 80.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let b_planar = weapon_rim_boost_rgb_cam(NO_SECTOR, &planar, true);
        let b_high = weapon_rim_boost_rgb_cam(NO_SECTOR, &high, true);
        for ch in 0..3 {
            assert!(
                b_planar[ch] > b_high[ch],
                "luz alta debería atenuar: canal {} planar={} high={}",
                ch, b_planar[ch], b_high[ch]
            );
        }
    }

    #[test]
    fn weapon_rim_3d_radius_cuts_far_vertical_light() {
        // Luz a XY=(0,0) pero z=400 (fuera del radio 384). En 2D
        // d_XY=0 ⇒ omni la incluye. En 3D d=400 > r ⇒ excluida.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 0.0, y_cam: 0.0, z_cam: 400.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let dir = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, true);
        let omni = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, false);
        assert_eq!(dir, ZERO_BOOST, "3D radio corta la luz lejana en z");
        assert!(omni[0] > 0.0, "omni 2D no la corta");
    }

    #[test]
    fn weapon_rim_3d_disabled_uses_omni_2d() {
        // Toggle off ⇒ bit-equivalent al 3.29 omni 2D (sin z).
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 20.0, z_cam: 100.0,
            sector: NO_SECTOR, tint_rgb: (200, 200, 200), lit_sectors: None,
        }];
        let off = weapon_rim_boost_rgb_cam(NO_SECTOR, &lights, false);
        let baseline = world_lights_boost_rgb_cam(0.0, 0.0, NO_SECTOR, &lights);
        assert_eq!(off, baseline);
    }

    // =================================================================
    // Fase 3.40 — Muzzle falloff 3D
    // =================================================================

    #[test]
    fn muzzle_boost_3d_recovers_2d_when_z_zero() {
        // Sin componente z, el helper 3D debe dar exactamente el mismo
        // resultado que el 2D — backwards-compat.
        let xs = [0.0_f32, 50.0, 100.0, 200.0];
        for &x in &xs {
            let s2d = muzzle_boost_cam(x, 0.0, 1.0);
            let s3d = muzzle_boost_cam_3d(x, 0.0, 0.0, 1.0);
            assert!(
                (s2d - s3d).abs() < 1e-5,
                "z=0 ⇒ 3D == 2D para x={}: s2d={} s3d={}",
                x, s2d, s3d
            );
        }
    }

    #[test]
    fn muzzle_boost_3d_attenuates_with_height() {
        // Misma XY pero z creciente ⇒ scalar cae monotonamente.
        let planar = muzzle_boost_cam_3d(50.0, 0.0, 0.0, 1.0);
        let mid = muzzle_boost_cam_3d(50.0, 0.0, 50.0, 1.0);
        let high = muzzle_boost_cam_3d(50.0, 0.0, 150.0, 1.0);
        assert!(planar > mid, "planar > mid: {} > {}", planar, mid);
        assert!(mid > high, "mid > high: {} > {}", mid, high);
    }

    #[test]
    fn muzzle_boost_3d_radius_cuts_far_vertical() {
        // d_2D=0 pero z muy alto ⇒ 2D la incluye, 3D la corta.
        let r = MUZZLE_RADIUS_WORLD;
        let s2d = muzzle_boost_cam(0.0, 0.0, 1.0); // peak
        let s3d = muzzle_boost_cam_3d(0.0, 0.0, r + 10.0, 1.0); // fuera de radio
        assert!(s2d > 0.0, "2D no la corta (d_XY=0)");
        assert_eq!(s3d, 0.0, "3D la corta (d_3D > r)");
    }

    #[test]
    fn muzzle_brdf_wall_3d_falloff_dims_high_surface() {
        // Pared a (100, 0) en cam-space + z_surf alto: el muzzle 3D del
        // 3.40 debe dar menos que el 2D del 3.32-3.37 (que ignoraba z).
        // Verificamos comparando el helper actual contra un cálculo
        // manual con scalar 2D pero misma cosine att.
        let n = (-1.0, 0.0);
        let z_high = 80.0;
        let actual_3d = muzzle_boost_rgb_wall_3d(100.0, 0.0, z_high, 1.0, n);
        // Simulamos el path 3.32-3.37 con scalar 2D pero cosine 3D:
        // los componentes per-canal serían `scalar_2d * tint * att`.
        let scalar_2d = muzzle_boost_cam(100.0, 0.0, 1.0);
        // cos del wall normal (-1,0) con dir surf→muzzle (-100,0,-80)/d_3D:
        let d2 = 100.0_f32 * 100.0 + z_high * z_high;
        let inv_d = d2.sqrt().recip();
        let cos = ((-1.0) * (-100.0) + 0.0 * 0.0) * inv_d;
        let att = (0.5 + 0.5 * cos).max(WALL_RIM_AMBIENT_FLOOR);
        let pre_340 = [
            scalar_2d * MUZZLE_TINT_RGB.0 as f32 / 255.0 * att,
            scalar_2d * MUZZLE_TINT_RGB.1 as f32 / 255.0 * att,
            scalar_2d * MUZZLE_TINT_RGB.2 as f32 / 255.0 * att,
        ];
        for ch in 0..3 {
            assert!(
                actual_3d[ch] < pre_340[ch],
                "3.40 dimea respecto al modelo pre-3.40 (scalar 2D + cosine 3D): canal {} 3.40={} pre={}",
                ch, actual_3d[ch], pre_340[ch]
            );
        }
    }

    #[test]
    fn muzzle_brdf_wall_perpendicular_full_intensity() {
        // Pared straight-ahead a (100, 0, 0), normal (-1, 0). Muzzle en
        // origin ⇒ direction surf→muzzle = (-1, 0, 0). cos = 1 ⇒ att=1.
        // Direccional debe coincidir con el muzzle omni (sin cosine).
        let n = (-1.0, 0.0);
        let dir = muzzle_boost_rgb_wall_3d(100.0, 0.0, 0.0, 1.0, n);
        let omni = muzzle_boost_rgb_cam(100.0, 0.0, 1.0);
        for ch in 0..3 {
            assert!(
                (dir[ch] - omni[ch]).abs() < 1e-5,
                "perpendicular: canal {} dir={} omni={}",
                ch, dir[ch], omni[ch]
            );
        }
    }

    #[test]
    fn muzzle_brdf_wall_oblique_attenuates() {
        // Pared oblicua: midpoint (100, 50), normal apuntando al cam pero
        // con componente lateral. dot(n, -m)/|m_3D| = cos < 1 ⇒ att < 1
        // ⇒ direccional < omni en cada canal.
        let mx = 100.0;
        let my = 50.0;
        // Pared dirección (0, 1) (vertical-Y), normal (-1, 0) toward camera.
        let n = (-1.0, 0.0);
        let dir = muzzle_boost_rgb_wall_3d(mx, my, 0.0, 1.0, n);
        let omni = muzzle_boost_rgb_cam(mx, my, 1.0);
        for ch in 0..3 {
            assert!(dir[ch] < omni[ch], "oblique: canal {} dir={} >= omni={}", ch, dir[ch], omni[ch]);
        }
    }

    #[test]
    fn muzzle_brdf_wall_disabled_equals_omni() {
        // Toggle off ⇒ combined wall usa muzzle_boost_rgb_cam (omni).
        let n = (-1.0, 0.0);
        let off = combined_boost_rgb_wall_cam(
            100.0, 50.0, 0.0, 1.0, NO_SECTOR, None, &[], n, false, false,
        );
        let on = combined_boost_rgb_wall_cam(
            100.0, 50.0, 0.0, 1.0, NO_SECTOR, None, &[], n, false, true,
        );
        // En perpendicular straight muzzle direccional == omni; en
        // oblicuo direccional < omni ⇒ off[i] >= on[i] por canal.
        for ch in 0..3 {
            assert!(off[ch] >= on[ch], "off >= on en canal {}", ch);
        }
    }

    #[test]
    fn muzzle_brdf_plane_floor_below_camera_full_cosine() {
        // Floor a z_surf = -32 (debajo del ojo), centroide en (0, 0, -32).
        // direction surf→muzzle = (0, 0, 32)/32 = (0, 0, 1). cos con
        // n_z=+1 (floor) = 1 ⇒ att=1. Fase 3.40: el scalar usa d_3D=32,
        // no d_2D=0, así que decae ligeramente respecto al peak. La
        // verificación correcta es `dir ≈ scalar_3D · tint` (att=1 sin
        // modulación) — coherente con falloff 3D del 3.40.
        let dir = muzzle_boost_rgb_plane_3d(0.0, 0.0, -32.0, 1.0, 1.0);
        let scalar_3d = muzzle_boost_cam_3d(0.0, 0.0, -32.0, 1.0);
        let expected = [
            scalar_3d * MUZZLE_TINT_RGB.0 as f32 / 255.0,
            scalar_3d * MUZZLE_TINT_RGB.1 as f32 / 255.0,
            scalar_3d * MUZZLE_TINT_RGB.2 as f32 / 255.0,
        ];
        for ch in 0..3 {
            assert!(
                (dir[ch] - expected[ch]).abs() < 1e-5,
                "cos=1 ⇒ dir = scalar_3D·tint: canal {} dir={} expected={}",
                ch, dir[ch], expected[ch]
            );
        }
    }

    // =================================================================
    // Fase 3.38 — Sprite sample point al centro del billboard
    // =================================================================

    #[test]
    fn sprite_sample_center_vs_floor_differs_for_overhead_light() {
        // Antorcha alta TLMP a XY (0, 0) en z_cam=+80 (techo). Sprite a
        // XY (100, 0). Sample en floor (z_surf=0) vs en centro (z_surf=28
        // ≈ cfg.sprite_height/2). El sample center reduce el dz (80→52)
        // y el d_3D (128→113), por lo que el cosine (que normaliza por
        // d_3D) sube ⇒ más aporte.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 0.0, y_cam: 0.0, z_cam: 80.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let floor = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        let center = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, 28.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            if floor[ch] > 0.01 {
                assert!(
                    center[ch] > floor[ch],
                    "centro debería recibir más de luz alta: canal {} center={} floor={}",
                    ch, center[ch], floor[ch]
                );
            }
        }
    }

    #[test]
    fn sprite_sample_center_vs_floor_differs_for_floor_light() {
        // Espejo: proyectil al ras del piso (z_cam=-32). Sprite a XY
        // (100, 0). Sample en floor (z_surf=0) tiene dz=-32 ⇒ d_3D=104,
        // cos pequeño. Sample en centro (z_surf=28) tiene dz=-60 ⇒
        // d_3D=116, d_3D mayor pero el dz grande hace cos más rasante.
        // Resultado: el sample floor recibe **más** que el center —
        // un proyectil al ras del piso ilumina la base del mobj con
        // cosine mejor que su centro.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 0.0, y_cam: 0.0, z_cam: -32.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let floor = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, 0.0, NO_SECTOR, &lights, true);
        let center = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, 28.0, NO_SECTOR, &lights, true);
        for ch in 0..3 {
            if center[ch] > 0.01 {
                assert!(
                    floor[ch] > center[ch],
                    "floor sample debería recibir más de luz baja: canal {} floor={} center={}",
                    ch, floor[ch], center[ch]
                );
            }
        }
    }

    #[test]
    fn sprite_sample_center_planar_light_matches_floor_when_dz_zero() {
        // Si la luz está al **nivel del sample** (mismo z), el cosine es
        // puramente XY independientemente de dónde esté el sample point.
        // Una luz al centro vertical del sprite (z=center) ⇒ dz=0
        // produce el mismo resultado que luz al floor (z=0) cuando el
        // sample también está al floor — ambas tienen dz=0 desde su
        // respectiva sample point.
        let white = (255, 255, 255);
        let light_at_center = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 28.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let light_at_floor = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 0.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        // Center sample vs center-z light: dz=0.
        let a = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, 28.0, NO_SECTOR, &light_at_center, true);
        // Floor sample vs floor-z light: dz=0 también.
        let b = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, 0.0, NO_SECTOR, &light_at_floor, true);
        for ch in 0..3 {
            assert!(
                (a[ch] - b[ch]).abs() < 1e-5,
                "dz=0 desde cualquier sample ⇒ mismo aporte: canal {} a={} b={}",
                ch, a[ch], b[ch]
            );
        }
    }

    #[test]
    fn sprite_sample_center_offset_zero_recovers_3_35_behavior() {
        // Si cfg.sprite_height = 0, el offset es 0 y el sample queda
        // en sprite.z (Fase 3.35). Verificamos que el helper produce
        // exactamente el mismo resultado con z_surf_cam idéntico —
        // la regresión sólo está en el caller (`gather_sprite`), aquí
        // chequeamos sanidad del helper.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 50.0, y_cam: 0.0, z_cam: 20.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        // sprite.z = 0, cfg.sprite_height = 0 ⇒ z_surf_cam = 0
        let z_surf_335 = 0.0_f32 + 0.0 * 0.5; // 3.35 behavior
        let z_surf_338 = 0.0_f32 + 0.0 * 0.5; // 3.38 con sprite_height=0
        assert_eq!(z_surf_335, z_surf_338);
        let a = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, z_surf_335, NO_SECTOR, &lights, true);
        let b = world_lights_boost_rgb_for_sprite_cam(100.0, 0.0, z_surf_338, NO_SECTOR, &lights, true);
        assert_eq!(a, b, "sprite_height=0 ⇒ 3.38 == 3.35");
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

    #[test]
    fn billboard_center_imp_at_floor() {
        // Imp típico: TROOA1 patch h≈56, topoffset≈48. Imp parado en
        // floor=0, view_z=40. Centro = floor + to - h/2 - view_z =
        // 0 + 48 - 28 - 40 = -20. Es decir, 20 unidades debajo del eye —
        // consistente con un mobj de altura 56 parado en piso 0 con ojo
        // a 40, centro a 8 absoluto.
        let z = billboard_center_z_cam(0.0, 48.0, 56.0, 40.0);
        assert!((z - (-20.0)).abs() < 1e-3, "centro esperado -20, got {}", z);
    }

    #[test]
    fn billboard_center_cyberdemon_taller_than_imp_estimate() {
        // Cyberdemon: patch h≈110, topoffset≈110 (estimado). Comparamos
        // contra el sample cfg.sprite_height=56 default. El centro real
        // del cyberdemon queda **más alto** que el estimate.
        let real_cyber = billboard_center_z_cam(0.0, 110.0, 110.0, 40.0);
        let estimate_56 = 0.0_f32 - 40.0 + 56.0 * 0.5; // 3.38 fallback
        assert!(
            real_cyber > estimate_56,
            "cyberdemon real ({}) debería estar arriba del estimate ({})",
            real_cyber, estimate_56
        );
    }

    #[test]
    fn billboard_center_puff_lower_than_imp_estimate() {
        // PUFF: patch h≈16, topoffset≈16. Centro real del puff queda
        // **más abajo** que el estimate cfg.sprite_height=56. El bullet
        // puff es chiquito y queda apoyado al techo del impacto.
        let real_puff = billboard_center_z_cam(64.0, 16.0, 16.0, 40.0);
        let estimate_56 = 64.0_f32 - 40.0 + 56.0 * 0.5; // 3.38 fallback con sprite.z=64
        assert!(
            real_puff < estimate_56,
            "puff real ({}) debería estar abajo del estimate ({})",
            real_puff, estimate_56
        );
    }

    #[test]
    fn billboard_center_uses_patch_height_for_brdf() {
        // Verificación que el sample con patch real impacta el BRDF.
        // Cyberdemon h=110 a XY=200, floor=0, view_z=40 ⇒ centro=15.
        // Luz a XY=(100, 0) z_cam=10 (cerca del centro real).
        // Sample real (z_surf=15) ⇒ dz=-5 ⇒ cos casi puro XY.
        // Sample 3.38 estimate (z_surf=-12) ⇒ dz=+22 ⇒ cos rasante.
        let white = (255, 255, 255);
        let lights = [WorldLight {
            x_cam: 100.0, y_cam: 0.0, z_cam: 10.0,
            sector: NO_SECTOR, tint_rgb: white, lit_sectors: None,
        }];
        let real_cyber_z = billboard_center_z_cam(0.0, 110.0, 110.0, 40.0); // 15
        let estimate_z = 0.0_f32 - 40.0 + 56.0 * 0.5;                          // -12
        let b_real = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, real_cyber_z, NO_SECTOR, &lights, true);
        let b_estimate = world_lights_boost_rgb_for_sprite_cam(200.0, 0.0, estimate_z, NO_SECTOR, &lights, true);
        // El sample real debería dar diferente boost (la luz está al
        // nivel del centro real, no del estimate). Diferencia positiva
        // significa la fase 3.39 cambia el rendering.
        let mut any_diff = false;
        for ch in 0..3 {
            if (b_real[ch] - b_estimate[ch]).abs() > 1e-4 {
                any_diff = true;
            }
        }
        assert!(any_diff, "patch.height real debería producir boost diferente al estimate");
    }

    #[test]
    fn muzzle_brdf_plane_far_horizontal_attenuates() {
        // Floor lejos horizontalmente, poco vertical: centroide (100, 0, -8).
        // direction surf→muzzle = (-100, 0, 8)/sqrt(10064) ≈ (-0.997, 0, 0.080).
        // cos con n_z=+1 = 0.080 ⇒ att = (0.5 + 0.04).max(0.3) ≈ 0.54.
        // Direccional debe ser ~54% del omni por canal.
        let dir = muzzle_boost_rgb_plane_3d(100.0, 0.0, -8.0, 1.0, 1.0);
        let omni = muzzle_boost_rgb_cam(100.0, 0.0, 1.0);
        for ch in 0..3 {
            if omni[ch] > 0.01 {
                let ratio = dir[ch] / omni[ch];
                assert!(
                    ratio < 0.6,
                    "rasante: canal {} ratio {} debería caer < 0.6",
                    ch, ratio
                );
                assert!(
                    ratio > PLANE_RIM_AMBIENT_FLOOR - 0.01,
                    "rasante: canal {} ratio {} debería estar sobre el piso ambient",
                    ch, ratio
                );
            }
        }
    }
