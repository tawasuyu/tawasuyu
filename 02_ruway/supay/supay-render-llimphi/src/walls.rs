use super::*;

pub(crate) fn gather_wall(
    out: &mut Vec<Renderable>,
    wall: &WallSeg,
    wall_idx: u32,
    snap: &SceneSnapshot,
    cam: &Camera,
    proj: &Projection,
    rect: &PaintRect,
    cfg: &RenderConfig,
    skip_fake_floor: bool,
    bsp_ranks: &[u32],
    lit_sectors: Option<&HashSet<u32>>,
    world_lights: &[WorldLight],
) {
    // Front/back side por convención Doom.
    let cross = (wall.x2 - wall.x1) * (cam.py - wall.y1)
        - (wall.y2 - wall.y1) * (cam.px - wall.x1);
    let on_front = cross < 0.0;

    let (near_idx, far_idx) = if on_front {
        (wall.front_sector, wall.back_sector)
    } else {
        (wall.back_sector, wall.front_sector)
    };

    if near_idx == NO_SECTOR {
        return;
    }
    let Some(near_sec) = snap.sectors.get(near_idx as usize) else {
        return;
    };
    let far_sec = if far_idx != NO_SECTOR {
        snap.sectors.get(far_idx as usize)
    } else {
        None
    };

    let (mut x1, mut y1) = cam.to_cam_2d(wall.x1, wall.y1);
    let (mut x2, mut y2) = cam.to_cam_2d(wall.x2, wall.y2);

    let near = cfg.near;
    if x1 < near && x2 < near {
        return;
    }
    if x1 < near {
        let t = (near - x1) / (x2 - x1);
        y1 += (y2 - y1) * t;
        x1 = near;
    } else if x2 < near {
        let t = (near - x2) / (x1 - x2);
        y2 += (y1 - y2) * t;
        x2 = near;
    }

    // Determinamos las slabs visibles + alturas para floor/ceiling strips.
    let near_floor = near_sec.floor_height;
    let near_ceiling = near_sec.ceiling_height;
    let mut slabs: [(f32, f32, &SectorSnap); 2] = [
        (0.0, 0.0, near_sec),
        (0.0, 0.0, near_sec),
    ];
    let mut n_slabs = 0_usize;
    let (floor_strip_z, ceiling_strip_z) = match far_sec {
        Some(far) => {
            // Lower (step up).
            if far.floor_height > near_floor {
                slabs[n_slabs] = (near_floor, far.floor_height, near_sec);
                n_slabs += 1;
            }
            // Upper (header).
            if far.ceiling_height < near_ceiling {
                slabs[n_slabs] = (far.ceiling_height, near_ceiling, near_sec);
                n_slabs += 1;
            }
            // Para floor/ceiling visibles del lado del jugador:
            // si el step sube, vemos el floor del near; si el step baja
            // (far más bajo) ya no hay slab pero el floor del far asoma.
            let visible_floor = near_floor.min(far.floor_height);
            let visible_ceil = near_ceiling.max(far.ceiling_height);
            (visible_floor, visible_ceil)
        }
        None => {
            slabs[0] = (near_floor, near_ceiling, near_sec);
            n_slabs = 1;
            (near_floor, near_ceiling)
        }
    };

    if n_slabs == 0 && far_sec.is_none() {
        return;
    }

    // Depth para sort: distancia euclidiana del midpoint en cámara.
    let mid_x = (x1 + x2) * 0.5;
    let mid_y = (y1 + y2) * 0.5;
    let depth = (mid_x * mid_x + mid_y * mid_y).sqrt();
    // Fase 3.13b: rank BSP del subsector del lado que mira el jugador —
    // clave primaria del painter's sort. Nudge del midpoint mundo 1 unidad
    // hacia la cámara para caer dentro del subsector "near" (no en el borde
    // ambiguo entre los dos). Sin BSP (`bsp_ranks` vacío) cae a 0 → el sort
    // delega al `depth` euclidiano como en la convención histórica.
    let bsp_rank = {
        let wmx = (wall.x1 + wall.x2) * 0.5;
        let wmy = (wall.y1 + wall.y2) * 0.5;
        let ddx = cam.px - wmx;
        let ddy = cam.py - wmy;
        let dlen = (ddx * ddx + ddy * ddy).sqrt().max(1e-3);
        bsp_rank_at(&snap.nodes, bsp_ranks, wmx + ddx / dlen, wmy + ddy / dlen)
    };
    // Fase 3.22: boost del muzzle flash en el midpoint de la pared.
    // Cae con distancia² desde el jugador (en cam-space player = origen).
    // Fase 3.23: gateado por el sector "near" del wall (el lado del que
    // miramos). Si ese sector no está en el lit set (cuarto inalcanzable
    // desde el player por linedef two-sided directo), el boost se anula
    // — la pared queda como en escena base, sin tinte cálido.
    // Fase 3.26: sumamos también las world lights de mobjs FF_FULLBRIGHT
    // cercanos al midpoint. Fase 3.27: el boost ahora es per-canal RGB
    // — cada luz emite su tinte (BFG verde, plasma azul, fireball rojo,
    // antorcha teñida). El scalar `boost_scalar = max(boost_rgb)` se
    // usa donde necesitamos una magnitud única (overlay alpha del
    // shading darkness).
    // Fase 3.32: rim direccional. La normal cam-space de la pared
    // (perpendicular al lineseg, toward camera) modula el aporte de
    // cada world light por cos(θ). Muzzle queda omni.
    // Fase 3.34: distancia y cosine en 3D, sample point en eye level
    // (`z_surf_cam = 0.0`). El radio 3D excluye luces remotas en
    // vertical aunque XY caiga dentro del rango.
    let wall_normal = wall_normal_cam(x1, y1, x2, y2, mid_x, mid_y);
    let boost_rgb = combined_boost_rgb_wall_cam(
        mid_x,
        mid_y,
        0.0,
        cfg.muzzle_glow_alpha,
        near_idx,
        lit_sectors,
        world_lights,
        wall_normal,
        cfg.wall_rim_directional,
        cfg.muzzle_brdf,
    );
    let boost_scalar = boost_max(boost_rgb);

    // -----------------------------------------------------------------
    // Floor & ceiling strips ("fake floor") — fallback de 3.1 cuando no
    // hay BSP. Si el snapshot trae subsectors, los pisos/techos los
    // dibuja `gather_subsector_planes` con polígonos reales y este
    // bloque se salta entero.
    // -----------------------------------------------------------------
    if !skip_fake_floor {
        let zf = floor_strip_z - cam.view_z;
        let zc = ceiling_strip_z - cam.view_z;
        let bl_floor = proj.project(x1, y1, zf);
        let br_floor = proj.project(x2, y2, zf);
        let bl_ceil = proj.project(x1, y1, zc);
        let br_ceil = proj.project(x2, y2, zc);

        let screen_top = rect.y as f64;
        let screen_bot = (rect.y + rect.h) as f64;

        if bl_floor.y < screen_bot || br_floor.y < screen_bot {
            let mut path = BezPath::new();
            path.move_to(Point::new(bl_floor.x, screen_bot));
            path.line_to(bl_floor);
            path.line_to(br_floor);
            path.line_to(Point::new(br_floor.x, screen_bot));
            path.close_path();
            out.push(Renderable {
                bsp_rank,
                depth: depth + 0.5,
                color: apply_color_boost(floor_color(near_sec, depth, cfg), boost_rgb),
                path,
                kind: RenderKind::Fill,
            });
        }

        if bl_ceil.y > screen_top || br_ceil.y > screen_top {
            let mut path = BezPath::new();
            path.move_to(Point::new(bl_ceil.x, screen_top));
            path.line_to(Point::new(br_ceil.x, screen_top));
            path.line_to(br_ceil);
            path.line_to(bl_ceil);
            path.close_path();
            out.push(Renderable {
                bsp_rank,
                depth: depth + 0.5,
                color: apply_color_boost(
                    ceiling_color(near_sec, depth, cfg, snap.sky_pic),
                    boost_rgb,
                ),
                path,
                kind: RenderKind::Fill,
            });
        }
    }

    // -----------------------------------------------------------------
    // Wall slabs: texturizadas si hay textura asignada + atlas; sino
    // fallback a bandas horizontales con shading procedural.
    // -----------------------------------------------------------------
    // Index del slab actual en `slabs`: i=0 puede ser lower o solid,
    // i=1 (si existe) es upper. `slab_kind_for(i, n_slabs, far_sec)`
    // resuelve cuál sidedef-kind aplica (0=mid, 1=upper, 2=lower).
    let bands = cfg.wall_bands.max(1);
    let wall_len = ((wall.x2 - wall.x1).powi(2) + (wall.y2 - wall.y1).powi(2)).sqrt().max(1e-3);
    for (slab_i, &(z_bot, z_top, sec)) in (&slabs[..n_slabs]).iter().enumerate() {
        if z_top <= z_bot {
            continue;
        }
        let zb = z_bot - cam.view_z;
        let zt = z_top - cam.view_z;
        let bl = proj.project(x1, y1, zb);
        let tl = proj.project(x1, y1, zt);
        let tr = proj.project(x2, y2, zt);
        let br = proj.project(x2, y2, zb);

        // ¿Hay textura asignada? Front side (0) o back side (1) según
        // qué lado del linedef ve el jugador. kind según slab_i.
        let side_idx = if on_front { 0usize } else { 1usize };
        let kind = wall_slab_kind(slab_i, n_slabs, far_sec.is_some());
        let tex_slot = wall.textures.get(side_idx * 3 + kind);
        let tex_name = tex_slot.and_then(|s| supay_scene::texture_name(s));
        let tex = tex_name.and_then(|n| cfg.atlas.as_ref().and_then(|a| a.wall_texture(n)));

        let mut path = BezPath::new();
        path.move_to(bl);
        path.line_to(tl);
        path.line_to(tr);
        path.line_to(br);
        path.close_path();

        if let Some(tex) = tex {
            // Per-strip rendering: subdividimos la pared en N strips a
            // lo largo del linedef. Cada strip se proyecta y resuelve
            // su propia affine — el error de perspectiva queda 1/N.
            use llimphi_ui::llimphi_raster::peniko::{
                Blob, Extend, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
            };
            let strips = cfg.wall_strips.max(1);
            let slab_h = (z_top - z_bot).max(1e-3);
            // Offsets de textura del sidedef + convención de pegging
            // de Doom (ML_DONTPEGTOP / ML_DONTPEGBOTTOM). v_top es la
            // coord V del image en el borde superior del slab — el
            // affine V de cada strip arranca ahí.
            let tex_x_offset = wall.tex_x_offsets[side_idx];
            let row_offset = wall.tex_y_offsets[side_idx];
            let far_floor = far_sec.map(|f| f.floor_height);
            let far_ceiling = far_sec.map(|f| f.ceiling_height);
            let v_top = wall_v_top(
                kind,
                wall.flags,
                near_floor,
                near_ceiling,
                far_floor,
                far_ceiling,
                z_top,
                tex.height as f32,
                row_offset,
            );
            let img = Image::new(ImageData { data: Blob::from(tex.rgba.clone()), format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: tex.width as u32, height: tex.height as u32 })
            .with_extend(Extend::Repeat);
            // Para cada strip: lerp world a lo largo de v1→v2, proyectar
            // y emitir quad con su propio affine. Reuso el `img` clonado
            // por refcount (Blob).
            for s in 0..strips {
                let t0 = s as f32 / strips as f32;
                let t1 = (s + 1) as f32 / strips as f32;
                // World start/end del strip (después del near-clip,
                // que ya está reflejado en x1/y1/x2/y2 cam-space).
                // Trabajamos en cam space directamente: lerp entre los
                // dos extremos cam del slab.
                let cx0 = x1 + (x2 - x1) * t0;
                let cy0 = y1 + (y2 - y1) * t0;
                let cx1 = x1 + (x2 - x1) * t1;
                let cy1 = y1 + (y2 - y1) * t1;
                let zb_c = z_bot - cam.view_z;
                let zt_c = z_top - cam.view_z;
                let s_bl = proj.project(cx0, cy0, zb_c);
                let s_tl = proj.project(cx0, cy0, zt_c);
                let s_tr = proj.project(cx1, cy1, zt_c);
                let s_br = proj.project(cx1, cy1, zb_c);
                // U coord en image space del strip:
                //   [tex_x_offset + t0·wall_len, tex_x_offset + t1·wall_len].
                // V coord: [v_top, v_top + slab_h]. El affine mapea
                // image(u, v) → screen.
                let strip_w = wall_len * (t1 - t0);
                let strip_u_base = tex_x_offset + wall_len * t0;
                let step_ux = (s_tr.x - s_tl.x) / strip_w.max(1e-3) as f64;
                let step_uy = (s_tr.y - s_tl.y) / strip_w.max(1e-3) as f64;
                let step_vx = (s_bl.x - s_tl.x) / slab_h as f64;
                let step_vy = (s_bl.y - s_tl.y) / slab_h as f64;
                let xform = Affine::new([
                    step_ux,
                    step_uy,
                    step_vx,
                    step_vy,
                    s_tl.x - strip_u_base as f64 * step_ux - v_top as f64 * step_vx,
                    s_tl.y - strip_u_base as f64 * step_uy - v_top as f64 * step_vy,
                ]);
                let mut s_path = BezPath::new();
                s_path.move_to(s_bl);
                s_path.line_to(s_tl);
                s_path.line_to(s_tr);
                s_path.line_to(s_br);
                s_path.close_path();
                out.push(Renderable {
                bsp_rank,
                    depth,
                    color: Color::WHITE,
                    path: s_path,
                    kind: RenderKind::TexturedWall {
                        image: img.clone(),
                        brush_xform: xform,
                    },
                });
            }
            // Overlay de shade y tinte. Fase 3.32-3.41: una sola fill
            // por slab con boost computado al eye-level.
            // Fase 3.42: si `wall_vertical_bands > 1`, subdividimos el
            // slab en N bandas horizontales y computamos el boost al
            // centro vertical de cada una. Una antorcha al ras del piso
            // ilumina más la parte baja, una a la altura del techo más
            // la parte alta — gradient discreto vertical.
            let base_shade = shade_for(sec.light_level, depth, cfg);
            let v_bands = cfg.wall_vertical_bands.max(1) as u32;
            if cfg.wall_vertical_gradient {
                // Path 3.43: gradiente lineal continuo bottom→top. Dos
                // fills por slab (oscuridad + tinte) en lugar de 2N.
                use llimphi_ui::llimphi_raster::peniko::Gradient;
                let nstops = (cfg.wall_vertical_bands as usize).max(2) + 1;
                // Geometría: bottom-center (t=0) → top-center (t=1) en
                // pantalla. Proyectamos las cuatro esquinas del slab.
                let zb_c = z_bot - cam.view_z;
                let zt_c = z_top - cam.view_z;
                let g_bl = proj.project(x1, y1, zb_c);
                let g_br = proj.project(x2, y2, zb_c);
                let g_tl = proj.project(x1, y1, zt_c);
                let g_tr = proj.project(x2, y2, zt_c);
                let start = Point::new((g_bl.x + g_br.x) * 0.5, (g_bl.y + g_br.y) * 0.5);
                let end = Point::new((g_tl.x + g_tr.x) * 0.5, (g_tl.y + g_tr.y) * 0.5);
                // Muestreo del boost a `nstops` alturas (igual normal de
                // pared, distinto z_surf_cam).
                let mut dark_samples = Vec::with_capacity(nstops);
                let mut tint_samples = Vec::with_capacity(nstops);
                for i in 0..nstops {
                    let t = i as f32 / (nstops - 1) as f32;
                    let z_band_cam = (z_bot + (z_top - z_bot) * t) - cam.view_z;
                    let band_boost = combined_boost_rgb_wall_cam(
                        mid_x,
                        mid_y,
                        z_band_cam,
                        cfg.muzzle_glow_alpha,
                        near_idx,
                        lit_sectors,
                        world_lights,
                        wall_normal,
                        cfg.wall_rim_directional,
                        cfg.muzzle_brdf,
                    );
                    dark_samples.push((t, boost_max(band_boost)));
                    tint_samples.push((t, band_boost));
                }
                let dark_stops = wall_darkness_gradient_stops(base_shade, &dark_samples);
                let dark_grad =
                    Gradient::new_linear(start, end).with_stops(dark_stops.as_slice());
                out.push(Renderable {
                bsp_rank,
                    depth: depth - 0.001,
                    color: Color::WHITE,
                    path: path.clone(),
                    kind: RenderKind::GradientFill {
                        gradient: dark_grad,
                    },
                });
                if let Some(tint_stops) = wall_tint_gradient_stops(&tint_samples) {
                    let tint_grad =
                        Gradient::new_linear(start, end).with_stops(tint_stops.as_slice());
                    out.push(Renderable {
                bsp_rank,
                        depth: depth - 0.002,
                        color: Color::WHITE,
                        path,
                        kind: RenderKind::GradientFill {
                            gradient: tint_grad,
                        },
                    });
                }
            } else if v_bands == 1 {
                // Path 3.32-3.41: single overlay sobre todo el slab.
                let lit_shade = (base_shade + boost_scalar).clamp(0.0, 1.0);
                if lit_shade < 0.95 {
                    let alpha = ((1.0 - lit_shade) * 255.0) as u8;
                    out.push(Renderable {
                bsp_rank,
                        depth: depth - 0.001,
                        color: Color::from_rgba8(0, 0, 0, alpha),
                        path: path.clone(),
                        kind: RenderKind::Fill,
                    });
                }
                if let Some((or, og, ob, oa)) = overlay_color_alpha_from_boost(boost_rgb) {
                    out.push(Renderable {
                bsp_rank,
                        depth: depth - 0.002,
                        color: Color::from_rgba8(or, og, ob, oa),
                        path,
                        kind: RenderKind::Fill,
                    });
                }
            } else {
                // Path 3.42: N bandas verticales, cada una con su boost.
                for b in 0..v_bands {
                    let t0 = b as f32 / v_bands as f32;
                    let t1 = (b + 1) as f32 / v_bands as f32;
                    // Centro vertical de la banda en world z.
                    let z_band_center =
                        z_bot + (z_top - z_bot) * (t0 + t1) * 0.5;
                    let z_band_cam = z_band_center - cam.view_z;
                    // Boost específico de la banda (mismo wall_normal,
                    // distinto z_surf_cam).
                    let band_boost = combined_boost_rgb_wall_cam(
                        mid_x,
                        mid_y,
                        z_band_cam,
                        cfg.muzzle_glow_alpha,
                        near_idx,
                        lit_sectors,
                        world_lights,
                        wall_normal,
                        cfg.wall_rim_directional,
                        cfg.muzzle_brdf,
                    );
                    let band_scalar = boost_max(band_boost);
                    // Path de la banda: clip vertical del slab.
                    let zb_b = (z_bot + (z_top - z_bot) * t0) - cam.view_z;
                    let zt_b = (z_bot + (z_top - z_bot) * t1) - cam.view_z;
                    let bl_b = proj.project(x1, y1, zb_b);
                    let tl_b = proj.project(x1, y1, zt_b);
                    let tr_b = proj.project(x2, y2, zt_b);
                    let br_b = proj.project(x2, y2, zb_b);
                    let mut band_path = BezPath::new();
                    band_path.move_to(bl_b);
                    band_path.line_to(tl_b);
                    band_path.line_to(tr_b);
                    band_path.line_to(br_b);
                    band_path.close_path();
                    let lit_band = (base_shade + band_scalar).clamp(0.0, 1.0);
                    if lit_band < 0.95 {
                        let alpha = ((1.0 - lit_band) * 255.0) as u8;
                        out.push(Renderable {
                bsp_rank,
                            depth: depth - 0.001,
                            color: Color::from_rgba8(0, 0, 0, alpha),
                            path: band_path.clone(),
                            kind: RenderKind::Fill,
                        });
                    }
                    if let Some((or, og, ob, oa)) =
                        overlay_color_alpha_from_boost(band_boost)
                    {
                        out.push(Renderable {
                bsp_rank,
                            depth: depth - 0.002,
                            color: Color::from_rgba8(or, og, ob, oa),
                            path: band_path,
                            kind: RenderKind::Fill,
                        });
                    }
                }
            }
        } else if cfg.wall_vertical_gradient {
            // Fase 3.56: pared sin textura con gradiente vertical continuo
            // (un solo `GradientFill` piso→techo) en vez de `bands` fills
            // planos escalonados — mismo suavizado que el camino texturizado
            // de 3.43. Cierra el desnivel visual entre paredes con y sin
            // atlas (escena stub, app cargada temprano, texturas ausentes).
            if cfg.debug_untextured {
                out.push(Renderable {
                    bsp_rank,
                    depth,
                    color: Color::from_rgba8(255, 0, 255, 255),
                    path,
                    kind: RenderKind::Fill,
                });
                continue;
            }
            use llimphi_ui::llimphi_raster::peniko::Gradient;
            let (c_bot, c_top) = wall_gradient_colors(wall_idx, wall, sec, depth, cfg);
            let c_bot = apply_color_boost(c_bot, boost_rgb);
            let c_top = apply_color_boost(c_top, boost_rgb);
            let start = Point::new((bl.x + br.x) * 0.5, (bl.y + br.y) * 0.5);
            let end = Point::new((tl.x + tr.x) * 0.5, (tl.y + tr.y) * 0.5);
            let stops: Vec<(f32, Color)> = vec![(0.0, c_bot), (1.0, c_top)];
            out.push(Renderable {
                bsp_rank,
                depth,
                color: Color::WHITE,
                path,
                kind: RenderKind::GradientFill {
                    gradient: Gradient::new_linear(start, end).with_stops(stops.as_slice()),
                },
            });
        } else {
            // Fallback: bandas horizontales coloreadas (3.1 behavior).
            for b in 0..bands {
                let t0 = b as f32 / bands as f32;
                let t1 = (b + 1) as f32 / bands as f32;
                let zb_b = (z_bot + (z_top - z_bot) * t0) - cam.view_z;
                let zt_b = (z_bot + (z_top - z_bot) * t1) - cam.view_z;
                let bl_b = proj.project(x1, y1, zb_b);
                let tl_b = proj.project(x1, y1, zt_b);
                let tr_b = proj.project(x2, y2, zt_b);
                let br_b = proj.project(x2, y2, zb_b);
                let mut p = BezPath::new();
                p.move_to(bl_b);
                p.line_to(tl_b);
                p.line_to(tr_b);
                p.line_to(br_b);
                p.close_path();
                let fill_color = if cfg.debug_untextured {
                    Color::from_rgba8(255, 0, 255, 255) // magenta = pared sin textura
                } else {
                    apply_color_boost(
                        wall_color(wall_idx, wall, sec, depth, b, bands, cfg),
                        boost_rgb,
                    )
                };
                out.push(Renderable {
                bsp_rank,
                    depth,
                    color: fill_color,
                    path: p,
                    kind: RenderKind::Fill,
                });
            }
        }
    }
}

/// Resuelve el `kind` del sidedef (0=mid, 1=upper, 2=lower) para un
/// slab dado. Convención:
/// - Pared one-sided: hay un único slab → middle.
/// - Pared two-sided con n_slabs=1: el step expuesto → upper si
///   `far.ceiling < near.ceiling`, sino lower. (Reconstruimos del
///   orden en que `gather_wall` los emite — siempre lower primero.)
/// - Two-sided con n_slabs=2: slab_i=0 es lower, slab_i=1 es upper.
/// Coordenada V (image-space) en el borde superior del slab,
/// siguiendo la convención de pegging de Doom.
///
/// La regla general (ver `r_segs.c` de Chocolate Doom): la textura
/// queda anclada por un `v_anchor` que depende del `slab_kind` y los
/// flags `ML_DONTPEGTOP`/`ML_DONTPEGBOTTOM`. La V de un pixel a altura
/// world `z` es entonces `v(z) = v_anchor - z + rowoffset`. Acá
/// evaluamos eso en `z = z_top` — el resto del slab cae por debajo
/// con `v(z_bot) = v_top + slab_h` (1 image-pixel = 1 world-unit).
///
/// Casos:
/// - `kind=0` middle (one-sided): default → top de la textura en
///   `near_ceiling`. `DONTPEGBOTTOM` → bottom en `near_floor`.
/// - `kind=1` upper: default → top en `far_ceiling` (anclado al
///   bottom del opening); `DONTPEGTOP` → top en `near_ceiling`.
///   Esto hace que las puertas no muevan su textura al subir.
/// - `kind=2` lower: default → top en `far_floor` (el escalón);
///   `DONTPEGBOTTOM` → top en `near_ceiling` (para alinear con upper).
pub(crate) fn wall_v_top(
    slab_kind: usize,
    flags: u32,
    near_floor: f32,
    near_ceiling: f32,
    far_floor: Option<f32>,
    far_ceiling: Option<f32>,
    z_top: f32,
    tex_height: f32,
    row_offset: f32,
) -> f32 {
    let peg_top = (flags & ML_DONTPEGTOP) != 0;
    let peg_bot = (flags & ML_DONTPEGBOTTOM) != 0;
    let v_anchor = match slab_kind {
        0 => {
            if peg_bot {
                near_floor + tex_height
            } else {
                near_ceiling
            }
        }
        1 => {
            if peg_top {
                near_ceiling
            } else {
                far_ceiling.unwrap_or(near_ceiling) + tex_height
            }
        }
        2 => {
            if peg_bot {
                near_ceiling
            } else {
                far_floor.unwrap_or(near_floor)
            }
        }
        _ => near_ceiling,
    };
    (v_anchor - z_top) + row_offset
}

pub(crate) fn wall_slab_kind(slab_i: usize, n_slabs: usize, two_sided: bool) -> usize {
    if !two_sided {
        return 0; // middle
    }
    // En el path two-sided: gather_wall pushea lower primero (si visible)
    // y upper después. Sin n_slabs=1 sabemos cuál tipo. Aproximamos:
    if n_slabs == 2 {
        if slab_i == 0 { 2 } else { 1 }
    } else {
        // Un único slab two-sided: no podemos distinguir lower vs upper
        // sin más info. Default a upper (más común en mapas E1M1: techos
        // bajos sobre puertas).
        1
    }
}

// =====================================================================
// Subsector planes (floor + ceiling)
// =====================================================================
