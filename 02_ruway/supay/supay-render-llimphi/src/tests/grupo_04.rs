#[allow(unused_imports)] use super::*;
#[allow(unused_imports)] use super::super::*;



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