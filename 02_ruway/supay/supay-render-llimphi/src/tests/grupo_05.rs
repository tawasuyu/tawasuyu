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