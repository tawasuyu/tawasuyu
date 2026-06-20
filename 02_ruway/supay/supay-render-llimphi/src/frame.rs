use super::*;

pub fn scene_view<Msg: Clone + Send + Sync + 'static>(
    pair: &SnapshotPair,
    last_tick_at: Instant,
    tick_period: Duration,
    config: RenderConfig,
) -> View<Msg> {
    let prev = pair.prev().cloned();
    let next = pair.next().cloned();
    let tick_period_secs = tick_period.as_secs_f32().max(1.0 / 1000.0);
    let config = Arc::new(config);
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .clip(true)
    .paint_with(move |scene, ts, rect: PaintRect| {
        let alpha = (last_tick_at.elapsed().as_secs_f32() / tick_period_secs).clamp(0.0, 1.0);
        let snap = make_frame(prev.as_ref(), next.as_ref(), alpha);
        render_frame(scene, ts, rect, &snap, &config);
    })
}

pub(crate) fn make_frame(
    prev: Option<&SceneSnapshot>,
    next: Option<&SceneSnapshot>,
    alpha: f32,
) -> SceneSnapshot {
    match (prev, next) {
        (Some(p), Some(n)) => interpolate(p, n, alpha),
        (None, Some(n)) | (Some(n), None) => n.clone(),
        (None, None) => SceneSnapshot::empty(0),
    }
}

// =====================================================================
// Render por frame
// =====================================================================

/// Renderiza un `SceneSnapshot` directamente a una `Scene` de vello, sin
/// montar el bucle Elm de [`scene_view`] ni interpolar. Entry point público
/// para herramientas de captura headless (volcado de frames a PNG, golden
/// tests visuales) que necesitan ver exactamente lo que el renderer produce.
/// `width`/`height` definen el viewport (origen en 0,0).
pub fn render_snapshot(
    scene: &mut Scene,
    ts: &mut Typesetter,
    width: f32,
    height: f32,
    snap: &SceneSnapshot,
    cfg: &RenderConfig,
) {
    let rect = PaintRect {
        x: 0.0,
        y: 0.0,
        w: width,
        h: height,
    };
    render_frame(scene, ts, rect, snap, cfg);
}

pub(crate) fn render_frame(
    scene: &mut Scene,
    ts: &mut Typesetter,
    rect: PaintRect,
    snap: &SceneSnapshot,
    cfg: &RenderConfig,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    draw_backdrop(scene, rect, snap, cfg);

    let view_z = snap.player.z + snap.player.view_height;
    let cam = Camera::new(snap.player.x, snap.player.y, view_z, snap.player.angle);
    let proj = Projection::new_pitched(
        rect,
        cfg.fov_y_deg.to_radians(),
        snap.player.view_pitch,
    );

    // Si el snapshot trae BSP (motor real con mapa cargado), pintamos
    // pisos/techos reales por subsector. Si no, los walls hacen
    // "fake-floor" como fallback de 3.1.
    let use_subsectors = !snap.subsectors.is_empty() && !snap.segs.is_empty();

    // Fase 3.13: si tenemos el árbol BSP, calculamos un orden
    // back-to-front desde la posición del jugador para asignar depth
    // de painter's correcto a los planos de subsector. Walls y sprites
    // siguen usando depth euclidiano (su orden relativo entre ellos no
    // depende del BSP y el ordenamiento por distancia funciona en Doom
    // para cualquier viewpoint plausible).
    //
    // `bsp_order_depths[ss_id]` = depth para los planos de ese subsector.
    // Grande = pintado primero. Vacío si no hay BSP — fallback al cálculo
    // euclidiano viejo dentro de gather_subsector_planes.
    let bsp_order_depths: Vec<Option<f32>> = if use_subsectors && !snap.nodes.is_empty() {
        compute_bsp_order_depths(snap)
    } else {
        Vec::new()
    };

    // Fase 3.13b: rank back-to-front por subsector — clave PRIMARIA del
    // painter's sort para TODAS las primitivas (planos, paredes, sprites,
    // decals), no sólo los planos. Indexado por subsector. Vacío sin BSP
    // ⇒ todas las primitivas quedan en rank 0 y el sort delega al `depth`
    // euclidiano, reproduciendo exactamente el comportamiento histórico.
    let bsp_ranks: Vec<u32> = if use_subsectors && !snap.nodes.is_empty() {
        compute_bsp_ranks(snap)
    } else {
        Vec::new()
    };

    // Fase 3.23: si la oclusión sectorial está activa y hay BSP, calculamos
    // el conjunto de sectores iluminables por el muzzle flash una sola vez
    // por frame. `None` ⇒ "iluminar todo" (modo stub o toggle apagado),
    // que reproduce el comportamiento 3.22.
    let lit_sectors: Option<HashSet<u32>> =
        if cfg.muzzle_occlusion && cfg.muzzle_glow_alpha > 0.0 {
            compute_muzzle_lit_sectors(snap)
        } else {
            None
        };
    let lit_ref = lit_sectors.as_ref();

    // Fase 3.26: recolectamos las luces puntuales del mundo desde sprites
    // FF_FULLBRIGHT. Lista cacheada por frame, hasta MAX_WORLD_LIGHTS
    // ordenados por cercanía al jugador. Si el toggle está apagado, queda
    // vacía y el plumbing pasa a no-op (rama temprana en `world_lights_boost_cam`).
    let world_lights: Vec<WorldLight> = if cfg.world_lights_enabled {
        gather_world_lights(snap, &cam, cfg.atlas.as_ref(), cfg.world_lights_occlusion)
    } else {
        Vec::new()
    };
    let world_lights_ref: &[WorldLight] = &world_lights;

    // Fase 3.54 + 3.55: occlusion culling. Caminamos el BSP front-to-back
    // acumulando los rangos angulares tapados por paredes sólidas. El
    // resultado dice qué subsectores (planos/sprites) y qué paredes
    // (linedefs) quedan tapadas por muros sólidos más cercanos. `None` ⇒
    // stub sin BSP o toggle apagado → todo visible (comportamiento histórico).
    let visibility: Option<Visibility> = if cfg.occlusion_cull && use_subsectors {
        compute_visibility(snap, &cam, cfg.near)
    } else {
        None
    };
    let visible_subs = visibility.as_ref().map(|v| &v.subs);
    let visible_walls = visibility.as_ref().map(|v| &v.walls);

    let cap = snap.walls.len() * (cfg.wall_bands as usize * 2 + 2)
        + snap.subsectors.len() * 2
        + snap.sprites.len();
    let mut renderables: Vec<Renderable> = Vec::with_capacity(cap);

    if use_subsectors {
        for (idx, sub) in snap.subsectors.iter().enumerate() {
            // Fase 3.54: subsector tapado por paredes sólidas → no emitir
            // sus planos de piso/techo.
            if let Some(vis) = visible_subs {
                if !vis.get(idx).copied().unwrap_or(true) {
                    continue;
                }
            }
            let bsp_depth = bsp_order_depths.get(idx).copied().flatten();
            let bsp_rank = bsp_ranks.get(idx).copied().unwrap_or(0);
            gather_subsector_planes(
                &mut renderables,
                sub,
                snap,
                &cam,
                &proj,
                &rect,
                cfg,
                bsp_depth,
                bsp_rank,
                lit_ref,
                world_lights_ref,
            );
        }
    }
    for (idx, wall) in snap.walls.iter().enumerate() {
        // Fase 3.55: pared (linedef) íntegramente tapada por muros sólidos
        // más cercanos → no emitir sus slabs ni strips. Todos sus segs
        // quedaron angularmente ocluidos.
        if let Some(vw) = visible_walls {
            if !vw.get(idx).copied().unwrap_or(true) {
                continue;
            }
        }
        gather_wall(
            &mut renderables,
            wall,
            idx as u32,
            snap,
            &cam,
            &proj,
            &rect,
            cfg,
            use_subsectors,
            &bsp_ranks,
            lit_ref,
            world_lights_ref,
        );
    }
    for sprite in snap.sprites.iter() {
        // Fase 3.54: sprite cuyo punto cae en un subsector tapado por
        // paredes sólidas → descartar (queda íntegramente detrás de un
        // muro opaco de piso a techo).
        if let Some(vis) = visible_subs {
            if let Some(ss) = subsector_at_point(&snap.nodes, sprite.x, sprite.y) {
                if !vis.get(ss as usize).copied().unwrap_or(true) {
                    continue;
                }
            }
        }
        gather_sprite(
            &mut renderables,
            sprite,
            snap,
            &cam,
            &proj,
            cfg,
            &bsp_ranks,
            lit_ref,
            world_lights_ref,
        );
    }
    // Fase 3.46: decals de impacto (host state). Camera-facing quads
    // pequeños, z-ordenados con el resto de la escena.
    if !cfg.decals.is_empty() {
        gather_decals(
            &mut renderables, cfg, snap, &cam, &proj, lit_ref, world_lights_ref,
        );
    }
    // Painter's sort unificado (Fase 3.13b): clave primaria = rank BSP
    // back-to-front del subsector (alto = más lejano = pintado primero),
    // desempate por `depth` euclidiano descendente dentro de un mismo
    // subsector. Sin BSP todos comparten rank 0 ⇒ desempata sólo por
    // `depth`, exactamente como antes. Esto ordena correctamente el cruce
    // pared↔sprite↔plano entre subsectores que el euclidiano puro fallaba.
    renderables.sort_by(|a, b| {
        b.bsp_rank
            .cmp(&a.bsp_rank)
            .then_with(|| {
                b.depth
                    .partial_cmp(&a.depth)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    for r in &renderables {
        match &r.kind {
            RenderKind::Fill => {
                scene.fill(Fill::NonZero, Affine::IDENTITY, r.color, None, &r.path);
            }
            RenderKind::Sprite { image, xform } => {
                scene.draw_image(image, *xform);
            }
            RenderKind::TexturedWall { image, brush_xform } => {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    image,
                    Some(*brush_xform),
                    &r.path,
                );
            }
            RenderKind::GradientFill { gradient } => {
                scene.fill(Fill::NonZero, Affine::IDENTITY, gradient, None, &r.path);
            }
        }
    }

    // Fase 3.15: sprite del arma del jugador (pistol/shotgun/etc.) —
    // pintado *encima* de la escena 3D pero *debajo* del overlay de
    // PLAYPAL (porque los damage flashes en Doom tintan el arma también).
    // Fase 3.18: el arma se tinta por la luz del sector del jugador
    // (cuarto oscuro = arma oscura). Resolvemos el sector vía BSP point
    // query una sola vez para ambos psprites — el muzzle flash usa el
    // mismo player_light pero, gracias a su flag FF_FULLBRIGHT, igual
    // sale a luz plena.
    let player_light = player_sector_light(snap);
    // Fase 3.28: boost RGB del ambiente evaluado en la posición del
    // jugador (origen del cam-space). Reutiliza la lista cacheada de
    // world lights del frame; sin alocaciones extra. El muzzle del
    // propio jugador *no* se incluye (consistente con 3.22 — el
    // fogonazo sale de la pistola, no la ilumina a ella).
    // Fase 3.29: el rim del arma se evalúa en la posición del player
    // (origen cam-space). Para que la oclusión per-light corte luces
    // separadas por paredes, resolvemos el sector del player vía BSP
    // point query. Sin BSP cae a `NO_SECTOR`, que ninguna luz incluye
    // en su lit set ⇒ ZERO_BOOST salvo lights con `lit_sectors = None`
    // (toggle off), preservando el comportamiento 3.28.
    let player_sec = subsector_at_point(&snap.nodes, snap.player.x, snap.player.y)
        .and_then(|ss| snap.subsectors.get(ss as usize))
        .map(|ss| ss.sector)
        .unwrap_or(NO_SECTOR);
    // Fase 3.30: el rim del arma se atenúa por dirección a la luz
    // cuando `weapon_rim_directional` está on — una antorcha frente al
    // jugador tinta más fuerte que una atrás. Caso `false` cae al
    // path omnidireccional 3.28/3.29.
    let weapon_rim_boost = if cfg.weapon_rim_light {
        weapon_rim_boost_rgb_cam(player_sec, world_lights_ref, cfg.weapon_rim_directional)
    } else {
        ZERO_BOOST
    };
    draw_weapon_sprite(scene, rect, &snap.weapon, player_light, weapon_rim_boost, cfg);
    // Fase 3.16: muzzle flash (`ps_flash`) sobrepuesto al weapon.
    // Doom usa este slot para el destello brillante de BFG, plasma,
    // chaingun, etc. Mismo helper, mismo z-order layer apenas encima.
    draw_weapon_sprite(
        scene,
        rect,
        &snap.weapon_flash,
        player_light,
        weapon_rim_boost,
        cfg,
    );

    // Fase 3.19: viñeta de cabina (gradient radial muy sutil). Va antes
    // que el overlay de PLAYPAL para que un damage flash rojo intenso
    // cubra la viñeta sin contaminarse con ella. `cfg.vignette == 0.0`
    // ⇒ no-op.
    draw_vignette(scene, rect, cfg);

    // Fase 3.14: overlay full-screen al final del frame (damage red,
    // pickup yellow, radsuit green, invuln white). Modernización pura
    // de la lógica de Doom de palette swapping a PLAYPAL[1..13].
    draw_player_overlays(scene, rect, &snap.player_overlays, snap.tick);

    // Fase 3.19: crosshair central encima de todo — incluso de los
    // overlays. Si el jugador está dañado y la pantalla se tinta de
    // rojo, el crosshair sigue siendo legible. Toggleable desde el host
    // con `cfg.crosshair = false`.
    if cfg.crosshair {
        draw_crosshair(scene, rect);
    }

    // Fase 3.20: HUD inferior modernista. Va al final, encima de todo,
    // para que la barra slim al pie con health/armor/ammo/keys quede
    // siempre legible. El HUD se desactiva en stub mode (sin jugador
    // real → stats hueco) y cuando el caller pone `cfg.hud = false`.
    if cfg.hud && snap.player_stats.health > 0 {
        draw_hud(scene, ts, rect, &snap.player_stats);
    }
}
