#[allow(unused_imports)] use super::*;
#[allow(unused_imports)] use super::super::*;



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
        // Si hay invuln activo + damage, gana invuln: `draw_player_overlays`
        // toma el camino de inversión real de color (blend Difference) en
        // cuanto `invuln_active` es true, sin llegar a los tintes planos de
        // `overlay_rgba`. Verificamos esa dominancia.
        let ov = PlayerOverlays {
            damage_count: 80,
            power_invuln: 200,
            ..Default::default()
        };
        assert!(invuln_active(&ov, 0), "invuln debe dominar y disparar el invert");
    }

    #[test]
    fn invuln_blinks_in_final_tics() {
        // En los últimos 32 tics la invulnerabilidad parpadea con bit 3 del
        // tick; fuera de esa ventana está siempre activa.
        let long = PlayerOverlays { power_invuln: 500, ..Default::default() };
        assert!(invuln_active(&long, 0), "lejos del final: siempre on");
        assert!(invuln_active(&long, 0xF), "lejos del final: on aun con bit3");
        let ending = PlayerOverlays { power_invuln: 10, ..Default::default() };
        assert!(invuln_active(&ending, 0x8), "fin + bit3 set → visible");
        assert!(!invuln_active(&ending, 0x0), "fin + bit3 clear → invisible (blink)");
        let none = PlayerOverlays::default();
        assert!(!invuln_active(&none, 0x8), "sin invuln nunca activo");
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