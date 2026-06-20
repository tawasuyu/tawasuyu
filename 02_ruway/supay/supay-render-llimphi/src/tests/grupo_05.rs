#[allow(unused_imports)] use super::*;
#[allow(unused_imports)] use super::super::*;



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
    fn clip_poly_halfplane_keeps_correct_side() {
        // Fase 3.60/3.63 — partición vertical en x=0 (dir (0,1) ⇒ f = -x).
        // R_PointOnSide: front (children[0]) es f≤0 ⇒ x≥0; back es x≤0.
        // Recortar el cuadrado [-1,1]² debe quedarse con la mitad correcta y
        // cerrar el polígono (≥3 vértices).
        let node = NodeSnap {
            partition_x: 0.0,
            partition_y: 0.0,
            partition_dx: 0.0,
            partition_dy: 1.0,
            children: [0, 0],
        };
        let square = vec![(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        let front = clip_poly_halfplane(&square, &node, true);
        assert!(front.len() >= 3, "front cierra polígono: {front:?}");
        assert!(
            front.iter().all(|&(x, _)| x >= -1e-4),
            "front se queda con x≥0: {front:?}"
        );
        assert!(
            front.iter().any(|&(x, _)| x > 0.9),
            "front incluye el borde derecho"
        );
        let back = clip_poly_halfplane(&square, &node, false);
        assert!(back.len() >= 3);
        assert!(
            back.iter().all(|&(x, _)| x <= 1e-4),
            "back se queda con x≤0: {back:?}"
        );
    }

    #[test]
    fn godray_halo_near_brighter_and_bigger_than_far() {
        // Fase 3.57 — la luz cercana irradia un halo más grande (radio por
        // perspectiva focal/x_cam) y más brillante (atenuación por
        // distancia) que la lejana; una detrás del near plane no da halo.
        let rect = PaintRect { x: 0.0, y: 0.0, w: 960.0, h: 600.0 };
        let proj = Projection::new(rect, 75.0_f32.to_radians());
        let light = |x_cam: f32| WorldLight {
            x_cam,
            y_cam: 0.0,
            z_cam: 0.0,
            sector: 0,
            tint_rgb: (255, 220, 140),
            lit_sectors: None,
        };
        let near = godray_halo(&light(100.0), &proj, 4.0, 0.6).expect("cercana da halo");
        let far = godray_halo(&light(600.0), &proj, 4.0, 0.6).expect("lejana da halo");
        assert!(
            near.radius > far.radius,
            "radio: cercana {} > lejana {}",
            near.radius,
            far.radius
        );
        assert!(
            near.alpha > far.alpha,
            "alpha: cercana {} > lejana {}",
            near.alpha,
            far.alpha
        );
        // Detrás del near plane ⇒ sin halo.
        assert!(godray_halo(&light(0.5), &proj, 4.0, 0.6).is_none());
        // god_rays = 0 ⇒ sin halo aunque esté a la vista.
        assert!(godray_halo(&light(100.0), &proj, 4.0, 0.0).is_none());
    }

    #[test]
    fn wall_gradient_colors_top_brighter_than_bottom() {
        // Fase 3.56 — la pared sin textura, en el camino de gradiente
        // vertical, va de más oscuro abajo (piso) a más claro arriba
        // (techo), reproduciendo la curva zenital de `wall_color` en sus
        // dos puntas sin las costuras de banda.
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
        let sec = SectorSnap {
            floor_height: 0.0,
            ceiling_height: 128.0,
            light_level: 200,
            floor_pic: 0,
            ceiling_pic: 0,
        };
        let cfg = RenderConfig::default();
        let (bot, top) = wall_gradient_colors(3, &wall, &sec, 64.0, &cfg);
        let lb = bot.to_rgba8().to_u8_array();
        let lt = top.to_rgba8().to_u8_array();
        let luma = |c: [u8; 4]| c[0] as u32 + c[1] as u32 + c[2] as u32;
        assert!(
            luma(lt) > luma(lb),
            "techo ({:?}) debe ser más claro que piso ({:?})",
            lt,
            lb
        );
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