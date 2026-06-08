use super::*;

/// Pinta un disco oscuro en el plano del piso bajo el sprite. Lo
/// aproximamos con un dodecágono CCW en world-space, transformamos a
/// cam-space, clipeamos al near plane (2D) y proyectamos cada vértice
/// con la cámara perspectiva. El resultado es una elipse natural en
/// pantalla — más alargada cuanto más cerca del jugador, casi línea
/// en la distancia.
///
/// El radio en world units viene del atlas si está disponible (mitad
/// del width del patch del frame actual, escalado por 0.55 para que la
/// sombra no exceda el ancho del sprite). Sin atlas usa
/// `cfg.sprite_half_width`.
///
/// La depth se pone `sprite_depth + 0.5` para que el shadow se pinte
/// **justo antes** del sprite en el orden back-to-front (painter's),
/// quedando bajo los pies del mobj pero encima del piso del sector.
#[allow(clippy::too_many_arguments)]
pub(crate) fn gather_sprite_shadow(
    out: &mut Vec<Renderable>,
    sprite: &SpriteSnap,
    sec: Option<&SectorSnap>,
    cam: &Camera,
    proj: &Projection,
    cfg: &RenderConfig,
    sprite_x_cam: f32,
    floor: f32,
    sprite_depth: f32,
    bsp_rank: u32,
) {
    // Radio en world units. Si tenemos el patch decodificado del atlas
    // usamos su mitad de width — así un enemigo grande (caco/baron) tira
    // sombra más ancha que un imp.
    let radius = if let Some(atlas) = cfg.atlas.as_ref() {
        let angle = compute_display_angle(sprite.x, sprite.y, sprite.angle, cam.px, cam.py);
        atlas
            .sprite_patch(sprite.sprite, sprite.frame, angle)
            .map(|(p, _)| (p.width as f32) * 0.55 * 0.5)
            .unwrap_or(cfg.sprite_half_width)
    } else {
        cfg.sprite_half_width
    };
    if radius <= 0.0 {
        return;
    }
    // Dodecágono en world space alrededor de (sprite.x, sprite.y).
    // CCW; los puntos viven todos en Z = floor.
    const N: usize = 12;
    let z_cam = floor - cam.view_z;
    let mut poly_cam: [(f32, f32); N] = [(0.0, 0.0); N];
    let twopi = std::f32::consts::TAU;
    // Pequeño achatamiento: la sombra es 100% radius en eje view-perpendicular
    // y 60% en eje view-paralelo (eje X_cam). Doom-monsters paran sobre
    // sus pies redondos, pero al verlos *desde* el jugador la huella
    // visual queda más como elipse — quedan más naturales así.
    let rx = radius * 0.6;
    let ry = radius;
    for i in 0..N {
        let theta = (i as f32) / (N as f32) * twopi;
        // Generamos en world coords con orientación alineada al world XY.
        let wx = sprite.x + theta.cos() * rx;
        let wy = sprite.y + theta.sin() * ry;
        poly_cam[i] = cam.to_cam_2d(wx, wy);
    }
    let clipped = clip_near(&poly_cam, cfg.near);
    if clipped.len() < 3 {
        return;
    }
    let mut path = BezPath::new();
    let mut first = true;
    for (xc, yc) in &clipped {
        let p = proj.project(*xc, *yc, z_cam);
        if !p.x.is_finite() || !p.y.is_finite() {
            return;
        }
        if first {
            path.move_to(p);
            first = false;
        } else {
            path.line_to(p);
        }
    }
    path.close_path();
    // Tinte: negro con alpha modulado por la luz del sector. Sectores
    // muy oscuros (cuartos sin iluminar) atenúan la sombra — no tiene
    // sentido pintar una mancha negra sobre piso ya casi negro. Fog
    // distante también la diluye.
    let light = sec.map(|s| s.light_level).unwrap_or(192) as f32 / 255.0;
    let fog = 1.0 - (sprite_x_cam / cfg.far_fog).clamp(0.0, 1.0);
    let alpha = (0.42 * light * fog).clamp(0.0, 0.55);
    let a = (alpha * 255.0) as u8;
    if a < 4 {
        return;
    }
    out.push(Renderable {
        bsp_rank,
        depth: sprite_depth + 0.5,
        color: Color::from_rgba8(0, 0, 0, a),
        path,
        kind: RenderKind::Fill,
    });
}

/// **Fase 3.46** — proyecta cada decal del host como un quad pequeño
/// camera-facing. Mismo modelo de billboard que los sprites: a una
/// profundidad `x_cam` constante el quad es axis-aligned en pantalla.
/// `+y_cam` = izquierda, `+z` = arriba (convención de `gather_sprite`).
/// El depth se sesga `-0.5` para que la marca quede apenas delante de la
/// pared/piso donde impactó, sin z-fight.
#[allow(clippy::too_many_arguments)]
pub(crate) fn gather_decals(
    out: &mut Vec<Renderable>,
    cfg: &RenderConfig,
    snap: &SceneSnapshot,
    cam: &Camera,
    proj: &Projection,
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
) {
    // Fase 3.13b: tabla de ranks BSP por subsector — clave primaria del
    // painter's sort. La calculamos una vez por llamada (los decals son
    // pocos); vacía/cero en modo stub → orden euclidiano histórico.
    let ranks = compute_bsp_ranks(snap);
    for d in &cfg.decals {
        let a = (d.alpha.clamp(0.0, 1.0) * 255.0) as u8;
        if a == 0 {
            continue;
        }
        // Cull por el centro: si el impacto está detrás del near-plane,
        // descartamos el decal entero.
        let (cx_cam, cy_cam) = cam.to_cam_2d(d.x, d.y);
        if cx_cam < cfg.near {
            continue;
        }
        let r = d.radius;
        let cz = d.z - cam.view_z;
        // Fase 3.49/3.52: resolvemos el sector del decal una sola vez (BSP
        // point query) — lo reusan el shading + boost (3.49-3.51) y el
        // recorte vertical al rango [floor, ceiling] del sector (3.52).
        let sector = if snap.nodes.is_empty() {
            None
        } else {
            subsector_at_point(&snap.nodes, d.x, d.y)
                .and_then(|ss| snap.subsectors.get(ss as usize))
                .map(|ss| ss.sector)
        };
        let sector_snap = sector.and_then(|s| snap.sectors.get(s as usize));
        let bsp_rank = bsp_rank_at(&snap.nodes, &ranks, d.x, d.y);
        // Fase 3.47: si hay tangente de pared, el quad yace **plano**
        // sobre el lineseg (eje horizontal = tangente mundo, vertical =
        // +Z) — se ve en perspectiva, no de cara a la cámara. Sin
        // tangente, billboard 3.46 (a `cx_cam` constante el quad es
        // axis-aligned en pantalla).
        let (tx, ty) = d.tangent;
        let corners: Vec<Point> = if d.horizontal {
            // Fase 3.48: charco horizontal — ejes en el plano XY mundo a
            // `z` constante. El quad se ve en perspectiva sobre el piso
            // (o bajo el techo).
            //
            // Fase 3.53: recortamos el quad a las paredes que lo bordean
            // (`clip_decal_to_walls`) — una mancha junto a un muro deja de
            // treparlo o cruzar al cuarto vecino. Sin paredes (modo stub) o
            // con el toggle off ⇒ quad completo como en 3.48.
            let quad = [
                (d.x - r, d.y - r),
                (d.x + r, d.y - r),
                (d.x + r, d.y + r),
                (d.x - r, d.y + r),
            ];
            let world = if cfg.decal_clip_walls && !snap.walls.is_empty() {
                let clipped = clip_decal_to_walls(&quad, &snap.walls, d.x, d.y, r);
                if clipped.len() < 3 {
                    continue;
                }
                clipped
            } else {
                quad.to_vec()
            };
            world
                .iter()
                .map(|&(wx, wy)| {
                    let (wcx, wcy) = cam.to_cam_2d(wx, wy);
                    proj.project(wcx, wcy, cz)
                })
                .collect()
        } else if tx != 0.0 || ty != 0.0 {
            // Esquinas en mundo: centro ± tangente (horizontal) y ± Z
            // (vertical). Cada una se transforma a cámara y se proyecta —
            // el quad queda con la inclinación de la pared.
            //
            // Fase 3.52: recortamos la extensión horizontal al span del
            // lineseg (`wall_span`) y la vertical al rango [floor, ceiling]
            // del sector, para que el decal no sangre más allá del borde
            // de la pared (la esquina) ni del piso/techo. Sin span / sin
            // sector ⇒ ± r como en 3.51.
            let (s_lo, s_hi) = match d.wall_span {
                Some((mn, mx)) => (mn.max(-r), mx.min(r)),
                None => (-r, r),
            };
            let (dz_lo, dz_hi) = match sector_snap {
                Some(s) => {
                    let floor_cam = s.floor_height - cam.view_z;
                    let ceil_cam = s.ceiling_height - cam.view_z;
                    ((floor_cam - cz).max(-r), (ceil_cam - cz).min(r))
                }
                None => (-r, r),
            };
            // Recorte completo ⇒ quad vacío: lo saltamos.
            if s_hi <= s_lo || dz_hi <= dz_lo {
                continue;
            }
            let project_world = |sx: f32, dz: f32| -> Point {
                let (wcx, wcy) = cam.to_cam_2d(d.x + tx * sx, d.y + ty * sx);
                proj.project(wcx, wcy, cz + dz)
            };
            vec![
                project_world(s_lo, dz_hi),
                project_world(s_hi, dz_hi),
                project_world(s_hi, dz_lo),
                project_world(s_lo, dz_lo),
            ]
        } else {
            vec![
                proj.project(cx_cam, cy_cam + r, cz + r),
                proj.project(cx_cam, cy_cam - r, cz + r),
                proj.project(cx_cam, cy_cam - r, cz - r),
                proj.project(cx_cam, cy_cam + r, cz - r),
            ]
        };
        if !corners.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
            continue;
        }
        let depth = (cx_cam * cx_cam + cy_cam * cy_cam).sqrt();
        // Fase 3.49: shadeamos el color por la luz del sector donde cae
        // el decal (+ fog por distancia). Fase 3.50: además sumamos el
        // tinte RGB de world lights + muzzle en esa posición — un charco
        // junto a un fireball se enrojece, el fogonazo lo ilumina. En
        // modo stub (sin BSP) queda full-bright como en 3.46-3.48.
        let col = if snap.nodes.is_empty() {
            Color::from_rgba8(d.color.0, d.color.1, d.color.2, a)
        } else {
            let light = sector_snap
                .map(|s| s.light_level)
                .unwrap_or(DEFAULT_PLAYER_LIGHT);
            let (sr, sg, sb) = shade_rgb(d.color, shade_for(light, depth, cfg));
            let base = Color::from_rgba8(sr, sg, sb, a);
            let surf_sector = sector.unwrap_or(NO_SECTOR);
            let z_surf_cam = d.z - cam.view_z;
            // Fase 3.51: el boost se direcciona por la normal de la
            // superficie donde yace el decal — un scorch en pared rasante
            // a la luz recibe menos tinte que uno encarado, un charco bajo
            // un fireball alto recoge el cosine vertical. Charco
            // (`horizontal`) ⇒ BRDF de plano; marca de pared (`tangent`)
            // ⇒ BRDF de pared; billboard flotante ⇒ omni (no tiene normal
            // estable). Con `decal_rim_directional=false` todo cae al omni
            // 3.50 bit-equivalente.
            let boost = if cfg.decal_rim_directional && d.horizontal {
                // Charco: normal +Z (piso) o -Z (techo) según a qué plano
                // del sector está más pegado el decal.
                let n_z = sector_snap
                    .map(|s| {
                        if (d.z - s.floor_height).abs() <= (d.z - s.ceiling_height).abs() {
                            1.0
                        } else {
                            -1.0
                        }
                    })
                    .unwrap_or(1.0);
                combined_boost_rgb_plane_cam(
                    cx_cam,
                    cy_cam,
                    z_surf_cam,
                    cfg.muzzle_glow_alpha,
                    surf_sector,
                    lit_sectors,
                    world_lights,
                    n_z,
                    true,
                    cfg.muzzle_brdf,
                )
            } else if cfg.decal_rim_directional && (tx != 0.0 || ty != 0.0) {
                // Marca de pared: la normal es perpendicular a la tangente
                // mundo. Transformamos dos puntos a lo largo de la tangente
                // a cam-space y resolvemos la perpendicular toward-camera
                // (misma maquinaria que los slabs de pared, 3.32).
                let (ax, ay) = cam.to_cam_2d(d.x - tx, d.y - ty);
                let (bx, by) = cam.to_cam_2d(d.x + tx, d.y + ty);
                let normal = wall_normal_cam(ax, ay, bx, by, cx_cam, cy_cam);
                combined_boost_rgb_wall_cam(
                    cx_cam,
                    cy_cam,
                    z_surf_cam,
                    cfg.muzzle_glow_alpha,
                    surf_sector,
                    lit_sectors,
                    world_lights,
                    normal,
                    normal != (0.0, 0.0),
                    cfg.muzzle_brdf,
                )
            } else {
                // Billboard flotante (sangre en el aire) o direccional
                // off: omni toward-camera (3.50).
                combined_boost_rgb_sprite_cam(
                    cx_cam,
                    cy_cam,
                    z_surf_cam,
                    cfg.muzzle_glow_alpha,
                    surf_sector,
                    lit_sectors,
                    world_lights,
                    false,
                )
            };
            apply_color_boost(base, boost)
        };
        let mut path = BezPath::new();
        path.move_to(corners[0]);
        for p in &corners[1..] {
            path.line_to(*p);
        }
        path.close_path();
        out.push(Renderable {
        bsp_rank,
            depth: depth - 0.5,
            color: col,
            path,
            kind: RenderKind::Fill,
        });
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn gather_sprite(
    out: &mut Vec<Renderable>,
    sprite: &SpriteSnap,
    snap: &SceneSnapshot,
    cam: &Camera,
    proj: &Projection,
    cfg: &RenderConfig,
    bsp_ranks: &[u32],
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
) {
    let (x_cam, y_cam) = cam.to_cam_2d(sprite.x, sprite.y);
    if x_cam < cfg.near {
        return;
    }
    let sec = snap.sectors.get(sprite.sector as usize);
    let floor = sec.map(|s| s.floor_height).unwrap_or(0.0);
    let depth = (x_cam * x_cam + y_cam * y_cam).sqrt();
    // Fase 3.13b: rank BSP del subsector donde está parado el sprite —
    // clave primaria del painter's sort. Un sprite es un punto, así que
    // su subsector es inequívoco (a diferencia de una pared en el borde).
    // Esto corrige el bug conocido: un sprite cercano en distancia
    // euclidiana dejaba de atravesar una pared que el BSP pone delante.
    // Sin BSP (`bsp_ranks` vacío) cae a 0 → orden euclidiano histórico.
    let bsp_rank = bsp_rank_at(&snap.nodes, bsp_ranks, sprite.x, sprite.y);
    // Fase 3.35: punto de muestreo vertical para BRDF 3D — base del
    // billboard relativo al ojo del jugador.
    // Fase 3.38: subimos el sample al **centro** vertical del billboard
    // (`+ cfg.sprite_height * 0.5`). Default usado por el path fallback
    // (sin atlas / patch missing) — el cfg.sprite_height es estimado.
    // Fase 3.39: el path texturizado override este sample con la altura
    // **real** del patch del WAD (`(z_top + z_bot) * 0.5`), por mobj.
    // Un cyberdemon (~110 u) y un PUFF (~16 u) ahora tienen sample
    // points distintos — más fiel a su geometría real.
    let z_surf_cam = sprite.z - cam.view_z + cfg.sprite_height * 0.5;

    // Fase 3.21: sombra circular en el plano del piso bajo el sprite.
    // Va siempre — texturizado o fallback — antes de pushear el sprite
    // mismo. `gather_sprite_shadow` decide su tamaño usando el patch
    // del atlas (si está) o `cfg.sprite_half_width` como fallback.
    if cfg.sprite_shadows {
        gather_sprite_shadow(out, sprite, sec, cam, proj, cfg, x_cam, floor, depth, bsp_rank);
    }

    // ---- Camino texturizado: hay atlas + patch decodificado ----
    if let Some(atlas) = cfg.atlas.as_ref() {
        // Ángulo de display 1..8 según la convención Doom:
        // R_PointToAngle2(thing, viewer) − thing.angle, redondeado al
        // wedge de π/4 más cercano. 1 = facing camera, 5 = back,
        // 3 = right side, 7 = left.
        let display_angle = compute_display_angle(sprite.x, sprite.y, sprite.angle, cam.px, cam.py);
        if let Some((patch, mirror)) =
            atlas.sprite_patch(sprite.sprite, sprite.frame, display_angle)
        {
            let w = patch.width as f32;
            let h = patch.height as f32;
            let lo = patch.leftoffset as f32;
            let to = patch.topoffset as f32;
            let y_left = y_cam + lo;
            let y_right = y_cam + lo - w;
            let z_top = floor + to - cam.view_z;
            let z_bot = floor + to - h - cam.view_z;
            // Billboard axis-aligned → affine exacto.
            let tl = proj.project(x_cam, y_left, z_top);
            let br = proj.project(x_cam, y_right, z_bot);
            let sx = (br.x - tl.x) / w as f64;
            let sy = (br.y - tl.y) / h as f64;
            if !(sx.is_finite() && sy.is_finite() && sx > 0.01 && sy > 0.01) {
                return;
            }
            // Shading: tinte multiplicativo al RGBA cacheado, según
            // light_level del sector + fog distance. Construimos un
            // Image nuevo con la versión tinted — cada draw cuesta
            // un Vec::with_capacity + iter de width·height pixels;
            // para sprites típicos (≈2300 px) ronda 10 KB/draw,
            // ~30 sprites/frame ≈ 300 KB/frame, asumible a 60 fps.
            //
            // Full-bright (bit 7 = FF_FULLBRIGHT_BYTE): si el estado
            // del mobj tiene este flag (proyectiles, muzzle flashes,
            // frames de "fire" de imps/cacos), saltamos shade y fog —
            // el sprite se ve a luz plena como en Doom original.
            let full_bright = (sprite.frame & 0x80) != 0;
            let light = sec.map(|s| s.light_level).unwrap_or(192);
            let shade = if full_bright {
                1.0
            } else {
                shade_for(light, depth, cfg)
            };
            // Fase 3.22: si el muzzle flash está activo y el sprite está
            // dentro del radio, sumamos un tinte cálido per-canal. Sprites
            // full-bright (proyectiles, fire frames) ya estaban a luz plena
            // y reciben el tinte amarillo sin saturarse — `sprite_shade_with_muzzle`
            // clampea ≤ 1.0 por canal.
            // Fase 3.23: gateado por `sprite.sector` — un imp atrás de una
            // pared sólida no se ilumina aunque la distancia euclidiana
            // del player lo alcance.
            // Fase 3.26: el sprite también recibe boost de las world
            // lights (mobjs FF_FULLBRIGHT cercanos), sumado al muzzle.
            // Fase 3.27: boost RGB per-canal — un sprite cerca de una
            // bola BFG se tinta verdoso; cerca de plasma, azulado.
            // Fase 3.31: opcionalmente direccional — luces detrás del
            // sprite back-lightean (cara visible apagada con piso
            // ambient), luces frontales tintan al 100 %.
            // Fase 3.39: sample point en el **centro real** del billboard
            // texturizado usando `(z_top + z_bot) * 0.5`. Reemplaza al
            // estimate basado en `cfg.sprite_height` que sigue vigente
            // para el fallback. Mobj alto (cyberdemon h=110) ⇒ centro
            // a 55 u sobre floor; PUFF (h=16) ⇒ 8 u. Cada uno recibe el
            // cosine BRDF apropiado para su tamaño.
            let z_surf_cam_textured = ((z_top + z_bot) * 0.5) as f32;
            let boost_rgb = combined_boost_rgb_sprite_cam(
                x_cam,
                y_cam,
                z_surf_cam_textured,
                cfg.muzzle_glow_alpha,
                sprite.sector,
                lit_sectors,
                world_lights,
                cfg.sprite_rim_directional,
            );
            let shade_rgb = sprite_shade_with_world(shade, boost_rgb);
            let img = make_tinted_sprite_image_rgb(&patch, shade_rgb);
            // Mirror = pintamos espejado: scale_x negativo + corrimiento.
            let xform = if mirror {
                Affine::translate((br.x, tl.y)) * Affine::scale_non_uniform(-sx, sy)
            } else {
                Affine::translate((tl.x, tl.y)) * Affine::scale_non_uniform(sx, sy)
            };
            out.push(Renderable {
        bsp_rank,
                depth,
                color: Color::WHITE,
                path: BezPath::new(),
                kind: RenderKind::Sprite { image: img, xform },
            });
            return;
        }
    }

    // ---- Fallback 3.1: rectángulo coloreado ----
    let z_bot = floor - cam.view_z;
    let z_top = z_bot + cfg.sprite_height;
    let hw = cfg.sprite_half_width;
    let bl = proj.project(x_cam, y_cam + hw, z_bot);
    let tl = proj.project(x_cam, y_cam + hw, z_top);
    let tr = proj.project(x_cam, y_cam - hw, z_top);
    let br = proj.project(x_cam, y_cam - hw, z_bot);
    let mut path = BezPath::new();
    path.move_to(bl);
    path.line_to(tl);
    path.line_to(tr);
    path.line_to(br);
    path.close_path();
    // Fase 3.26: fallback (sin patch del WAD) también combina muzzle + world lights.
    // Fase 3.27: boost RGB per-canal.
    // Fase 3.31: idem rim direccional (fake-normal toward camera) si
    // el toggle está on. Fase 3.35: distancia 3D usando `z_surf_cam`.
    let boost = combined_boost_rgb_sprite_cam(
        x_cam,
        y_cam,
        z_surf_cam,
        cfg.muzzle_glow_alpha,
        sprite.sector,
        lit_sectors,
        world_lights,
        cfg.sprite_rim_directional,
    );
    out.push(Renderable {
        bsp_rank,
        depth,
        color: apply_color_boost(sprite_color(sprite, sec, depth, cfg), boost),
        path,
        kind: RenderKind::Fill,
    });
}

/// Variante per-canal: cada componente RGB se multiplica por su tint
/// individual. Usada por el muzzle flash (Fase 3.22) para tintar
/// amarillo cálido los sprites cercanos al destello del arma. Default
/// equivalente a `[shade, shade, shade]` = grayscale shading.
pub(crate) fn make_tinted_sprite_image_rgb(
    patch: &supay_wad::Patch,
    tint: [f32; 3],
) -> llimphi_ui::llimphi_raster::peniko::ImageBrush {
    use llimphi_ui::llimphi_raster::peniko::{
        Blob, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
    };
    let tr = tint[0].clamp(0.05, 1.0);
    let tg = tint[1].clamp(0.05, 1.0);
    let tb = tint[2].clamp(0.05, 1.0);
    let identity = (tr - 1.0).abs() < 1e-3 && (tg - 1.0).abs() < 1e-3 && (tb - 1.0).abs() < 1e-3;
    let tinted: Vec<u8> = if identity {
        patch.rgba.clone()
    } else {
        let mut out = Vec::with_capacity(patch.rgba.len());
        for chunk in patch.rgba.chunks_exact(4) {
            out.push(((chunk[0] as f32) * tr) as u8);
            out.push(((chunk[1] as f32) * tg) as u8);
            out.push(((chunk[2] as f32) * tb) as u8);
            out.push(chunk[3]);
        }
        out
    };
    let blob = Blob::from(tinted);
    Image::new(ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: patch.width as u32, height: patch.height as u32 })
}

/// Calcula el ángulo de display 1..8 para un sprite direccional según
/// la convención Doom. `mobj_angle` = orientación facial del mobj en
/// world space (radianes desde +X, antihorario). `(viewer_x, viewer_y)`
/// = posición del jugador. Resultado: 1 si la cámara está en frente
/// del mobj, 3 a la derecha del mobj, 5 detrás, 7 a la izquierda.
pub(crate) fn compute_display_angle(
    mobj_x: f32,
    mobj_y: f32,
    mobj_angle: f32,
    viewer_x: f32,
    viewer_y: f32,
) -> u8 {
    use std::f32::consts::{FRAC_PI_4, TAU};
    let angle_to_viewer = (viewer_y - mobj_y).atan2(viewer_x - mobj_x);
    let rel = (angle_to_viewer - mobj_angle).rem_euclid(TAU);
    // Wedge de π/4. +π/8 = bias para que el wedge centre cada ángulo.
    let wedge = ((rel + FRAC_PI_4 / 2.0) / FRAC_PI_4).floor() as i32;
    let wedge = wedge.rem_euclid(8) as u8;
    wedge + 1
}

// =====================================================================
// Paletas — riffs sobre los flats/textures clásicos de Doom shareware
// (BROVINE/STARTAN/GRAYBIG/SLADWALL para paredes; FLAT5_5/MFLR8_1 para
// pisos; F_SKY1 para cielo). No son samples reales — son colores
// reverse-engineered del look visual de E1M1.
// =====================================================================
