#[allow(unused_imports)] use super::*;
#[allow(unused_imports)] use super::super::*;



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