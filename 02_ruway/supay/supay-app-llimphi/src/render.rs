use super::*;

// =====================================================================
// Raycaster (DDA estilo Lode Vandevenne)
// =====================================================================

pub(crate) struct RayHit {
    /// Distancia perpendicular al plano de cámara (no euclidean — evita
    /// fish-eye en la altura del slice).
    pub(crate) perp_dist: f32,
    pub(crate) material: u8,
    /// `true` si la pared golpeada es E/W (vertical grid edge);
    /// `false` si N/S. Se usa para el sombreado tipo Doom.
    pub(crate) side_ew: bool,
    /// Posición horizontal del hit dentro de la pared, en `[0, 1)`.
    /// Las texturas procedurales por slice la usan para variar el
    /// patrón a lo largo de la pared (ladrillos, paneles).
    pub(crate) wall_x: f32,
}

pub(crate) fn cast_ray(px: f32, py: f32, ray_angle: f32) -> RayHit {
    let (sin, cos) = ray_angle.sin_cos();
    let dir_x = cos;
    let dir_y = sin;

    let delta_x = if dir_x.abs() < 1e-6 { 1e6 } else { (1.0_f32 / dir_x).abs() };
    let delta_y = if dir_y.abs() < 1e-6 { 1e6 } else { (1.0_f32 / dir_y).abs() };

    let mut map_x = px.floor() as i32;
    let mut map_y = py.floor() as i32;

    let (step_x, mut side_x) = if dir_x < 0.0 {
        (-1, (px - map_x as f32) * delta_x)
    } else {
        (1, (map_x as f32 + 1.0 - px) * delta_x)
    };
    let (step_y, mut side_y) = if dir_y < 0.0 {
        (-1, (py - map_y as f32) * delta_y)
    } else {
        (1, (map_y as f32 + 1.0 - py) * delta_y)
    };

    let mut side_ew = false;
    let mut hit = 0_u8;
    // Loop con tope alto por seguridad — el mapa está cerrado.
    for _ in 0..256 {
        if side_x < side_y {
            side_x += delta_x;
            map_x += step_x;
            side_ew = true;
        } else {
            side_y += delta_y;
            map_y += step_y;
            side_ew = false;
        }
        let t = tile(map_x, map_y);
        if t != 0 {
            hit = t;
            break;
        }
    }

    // Distancia perpendicular: una de las dos componentes según el lado.
    let perp = if side_ew {
        (map_x as f32 - px + (1 - step_x) as f32 * 0.5) / dir_x
    } else {
        (map_y as f32 - py + (1 - step_y) as f32 * 0.5) / dir_y
    };
    let perp = perp.max(0.0001);

    // wall_x: posición en la pared donde golpeó el rayo, normalizada
    // a [0, 1). Para paredes E/W (lado vertical) viene de Y; para N/S
    // de X. Es la coordenada que usan las texturas procedurales.
    let wall_x_raw = if side_ew {
        py + perp * dir_y
    } else {
        px + perp * dir_x
    };
    let wall_x = wall_x_raw - wall_x_raw.floor();

    RayHit {
        perp_dist: perp,
        material: hit,
        side_ew,
        wall_x,
    }
}

// =====================================================================
// Texturas procedurales — sin bitmaps. Cada material define un
// `texture_mul(wall_x, wall_y, tick)` que devuelve un multiplicador
// en `[0.6, 1.15]` aproximadamente. El renderer divide cada slice en
// SLICE_SEGMENTS bandas verticales y pinta cada una con su shade.
// =====================================================================

/// Cantidad de segmentos verticales por slice. Más = más detalle de
/// textura, más rects. 8 es buen compromiso visual/costo a 960×600
/// con COL_STRIDE = 3 (~320 cols × 8 segs = ~2560 rects/frame).
pub(crate) const SLICE_SEGMENTS: usize = 8;

/// Multiplicador de detalle textural. `wall_x ∈ [0, 1)` posición
/// horizontal en la pared, `wall_y ∈ [0, 1)` posición vertical del
/// segmento (0 = arriba), `tick` para texturas animadas (slime).
pub(crate) fn texture_mul(material: u8, wall_x: f32, wall_y: f32, tick: u64) -> f32 {
    match material {
        1 => techbase_mul(wall_x, wall_y),
        2 => brick_mul(wall_x, wall_y),
        3 => metal_mul(wall_x, wall_y),
        4 => slime_mul(wall_x, wall_y, tick),
        _ => 1.0,
    }
}

/// Techbase beige: junta horizontal sutil cada 0.25 unidades + leve
/// shade gradiente vertical. Plano y limpio.
pub(crate) fn techbase_mul(wall_x: f32, wall_y: f32) -> f32 {
    let _ = wall_x;
    // Junta cada 0.25 con grosor ~0.04.
    let row_pos = (wall_y * 4.0).fract();
    let joint = if row_pos < 0.05 || row_pos > 0.95 { 0.78 } else { 1.0 };
    // Gradiente vertical sutil (más oscuro abajo).
    let grad = 0.92 + 0.10 * (1.0 - wall_y);
    joint * grad
}

/// Ladrillo: filas alternadas con offset 0.5 (running bond típico),
/// juntas horizontales más oscuras + juntas verticales en cada
/// ladrillo. Visualmente "Doom HELL ladrillo".
pub(crate) fn brick_mul(wall_x: f32, wall_y: f32) -> f32 {
    // Filas de 0.25 de alto. Cada fila desplaza wall_x medio ladrillo.
    let row = (wall_y * 4.0).floor() as i32;
    let row_offset = if row % 2 == 0 { 0.0 } else { 0.5 };
    let bx = (wall_x + row_offset).fract();
    let by = (wall_y * 4.0).fract();
    // Junta horizontal (gruesa, oscura).
    let h_joint = if by < 0.10 { 0.55 } else { 1.0 };
    // Junta vertical cada 0.5 (ladrillos de medio metro).
    let v_pos = (bx * 2.0).fract();
    let v_joint = if v_pos < 0.06 || v_pos > 0.94 { 0.62 } else { 1.0 };
    // Variación interna por ladrillo (pseudo-random pero determinístico).
    let brick_id = ((bx * 2.0).floor() as i32 + row * 7) as u32;
    let variation = 0.96 + ((brick_id.wrapping_mul(2_654_435_761) >> 24) & 0xF) as f32 / 200.0;
    h_joint * v_joint * variation
}

/// Metal: paneles verticales (0.25 unidades) con bordes oscuros y
/// pequeños "tornillos" en las esquinas (puntos más oscuros).
pub(crate) fn metal_mul(wall_x: f32, wall_y: f32) -> f32 {
    let panel_x = (wall_x * 4.0).fract();
    // Bordes verticales del panel.
    let edge_v = if panel_x < 0.06 || panel_x > 0.94 { 0.72 } else { 1.0 };
    // Tornillos en esquinas (intersección de bordes).
    let near_top = wall_y < 0.06 || (wall_y - 0.5).abs() < 0.03;
    let near_edge = panel_x < 0.10 || panel_x > 0.90;
    let bolt = if near_top && near_edge { 0.55 } else { 1.0 };
    // Sutil highlight central por panel.
    let center_glow = 1.0 + 0.05 * (1.0 - (panel_x - 0.5).abs() * 2.0);
    edge_v * bolt * center_glow
}

/// Slime: patrón orgánico que ondula con el tick. Las celdas brillan
/// y se atenúan en olas — el efecto "fluido vivo" de Doom.
pub(crate) fn slime_mul(wall_x: f32, wall_y: f32, tick: u64) -> f32 {
    let t = tick as f32 * 0.08;
    let wave1 = (wall_y * 7.0 + t).sin() * 0.10;
    let wave2 = (wall_x * 5.0 - t * 0.7).sin() * 0.06;
    let speckle_phase = (wall_y * 17.0 + wall_x * 13.0 + t * 0.4).sin();
    let speckle = if speckle_phase > 0.85 { 0.15 } else { 0.0 };
    (1.0 + wave1 + wave2 + speckle).clamp(0.75, 1.20)
}

// =====================================================================
// Render — paint_with custom dentro del rect del nodo
// =====================================================================

pub(crate) fn scene_pane(model: &Model) -> View<Msg> {
    // Capturamos snapshot del frame. Todo Send+Sync trivial.
    let px = model.px;
    let py = model.py;
    let pa = model.pa;
    let tick = model.tick;
    let bullets = model.bullets.clone();
    let decals = model.decals.clone();
    let static_sprites = model.static_sprites.clone();
    let enemies = model.enemies.clone();
    let pickups = model.pickups.clone();
    let temp_lights = model.temp_lights.clone();
    let game_over = model.game_over;
    let victory = model.victory;

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
        draw_scene(
            scene,
            ts,
            rect,
            px,
            py,
            pa,
            tick,
            &bullets,
            &decals,
            &static_sprites,
            &enemies,
            &pickups,
            &temp_lights,
        );
        if game_over {
            draw_overlay(scene, ts, rect, "MUERTO", "SPACE para reiniciar", (0.95, 0.30, 0.25));
        } else if victory {
            draw_overlay(scene, ts, rect, "VICTORIA", "SPACE para reiniciar", (0.50, 0.95, 0.55));
        }
    })
}

/// Resolución de raycast (columnas verticales). Sub-muestreo a ~1
/// columna por 3 px de pantalla: el costo de cada rayo + slice baja
/// 3× y el resultado se ve casi igual (paredes son superficies
/// continuas). Bajalo a 1.0 si querés calidad full.
pub(crate) const COL_STRIDE: f32 = 3.0;

/// Luz ambiental mínima — sin esto los rincones sin luz son negro
/// puro. Doom original tenía ambient sectorial; acá un escalar
/// global más el aporte de las luces puntuales.
pub(crate) const AMBIENT: f32 = 0.18;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_scene(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    _ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: PaintRect,
    px: f32,
    py: f32,
    pa: f32,
    tick: u64,
    bullets: &[Bullet],
    decals: &[Decal],
    static_sprites: &[Sprite],
    enemies: &[Enemy],
    pickups: &[Pickup],
    temp_lights: &[TempLight],
) {
    let w = rect.w as f64;
    let h = rect.h as f64;
    if w <= 0.0 || h <= 0.0 {
        return;
    }

    // Banding del cielo/piso — barato y enriquece el fondo. El
    // raycast pinta encima.
    draw_sky_and_floor(scene, rect);

    // Z-buffer por columna: perp_dist de la pared que cubre esa
    // columna. Los sprites se pintan después usando esto para
    // ocultarse detrás de paredes más cercanas.
    let total_cols = (w / COL_STRIDE as f64).max(1.0) as usize + 1;
    let mut z_buf: Vec<f32> = vec![f32::INFINITY; total_cols];

    // --- Pass 1: paredes con textura procedural por slice ---
    let mut x_pix = rect.x as f64;
    let x_end = (rect.x + rect.w) as f64;
    let mut i = 0_usize;
    while x_pix < x_end {
        let col_frac = i as f32 / total_cols as f32;
        let ray_angle = pa - FOV * 0.5 + FOV * col_frac;
        let hit = cast_ray(px, py, ray_angle);
        let cos_offset = (ray_angle - pa).cos().max(0.0001);
        let corrected = hit.perp_dist * cos_offset;

        // Hit world-point para iluminación por luces puntuales.
        let hit_x = px + hit.perp_dist * ray_angle.cos();
        let hit_y = py + hit.perp_dist * ray_angle.sin();
        let lights = lighting_contribution(hit_x, hit_y, tick, bullets, temp_lights);
        let mut light_mul = (
            (AMBIENT + lights.0).min(2.0),
            (AMBIENT + lights.1).min(2.0),
            (AMBIENT + lights.2).min(2.0),
        );
        if hit.side_ew {
            // Bias clásico de Doom para distinguir paredes E/W.
            light_mul.0 *= 0.78;
            light_mul.1 *= 0.78;
            light_mul.2 *= 0.78;
        }

        let base = material_color(hit.material);
        let lit = (base.0 * light_mul.0, base.1 * light_mul.1, base.2 * light_mul.2);

        // Altura del slice en píxeles.
        let line_h = (h / corrected as f64).min(h * 4.0);
        let y_mid = h * 0.5 + rect.y as f64;
        let y_top = y_mid - line_h * 0.5;
        let view_top = rect.y as f64;
        let view_bot = (rect.y + rect.h) as f64;
        let x_right = x_pix + COL_STRIDE as f64;

        // Subdivisión vertical en SLICE_SEGMENTS bandas: cada una con
        // su textura procedural aplicada. wall_y normalizado [0, 1).
        let seg_h_world = 1.0_f32 / SLICE_SEGMENTS as f32;
        for j in 0..SLICE_SEGMENTS {
            let wy_lo = j as f32 * seg_h_world;
            let wy_hi = (j + 1) as f32 * seg_h_world;
            let wy_mid = (wy_lo + wy_hi) * 0.5;
            let detail = texture_mul(hit.material, hit.wall_x, wy_mid, tick);
            let mut seg = (lit.0 * detail, lit.1 * detail, lit.2 * detail);
            seg = shade_by_dist(seg, hit.perp_dist);
            seg = apply_fog(seg, hit.perp_dist);

            let seg_y_top = (y_top + wy_lo as f64 * line_h).max(view_top);
            let seg_y_bot = (y_top + wy_hi as f64 * line_h).min(view_bot);
            if seg_y_bot <= seg_y_top {
                continue; // segmento entero fuera del viewport
            }
            let r = llimphi_ui::llimphi_raster::kurbo::Rect::new(
                x_pix,
                seg_y_top,
                x_right,
                seg_y_bot,
            );
            scene.fill(
                Fill::NonZero,
                llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
                rgb(seg.0, seg.1, seg.2),
                None,
                &r,
            );
        }

        // Guardamos la perp_dist (no la corregida) para z-test con sprites.
        if i < z_buf.len() {
            z_buf[i] = hit.perp_dist;
        }

        x_pix += COL_STRIDE as f64;
        i += 1;
    }

    // --- Pass 2: sprites billboarded con z-test por columna ---
    // Combinamos en una sola lista todos los sprites del frame
    // (estáticos + enemies según su estado + pickups + bullets +
    // decals) para que `draw_sprites` los ordene por distancia y
    // pinte de atrás hacia adelante.
    let mut all_sprites: Vec<Sprite> = static_sprites.to_vec();
    for e in enemies {
        let (kind, scale) = match e.state {
            EnemyState::Idle | EnemyState::Walking => (SpriteKind::Imp, 0.85),
            EnemyState::Dying(_) => (SpriteKind::DyingImp, 0.65),
            EnemyState::Dead => (SpriteKind::Corpse, 0.30),
        };
        all_sprites.push(Sprite { x: e.x, y: e.y, kind, scale });
    }
    for p in pickups {
        let kind = match p.kind {
            PickupKind::Ammo => SpriteKind::AmmoBox,
            PickupKind::Health => SpriteKind::HealthKit,
        };
        all_sprites.push(Sprite { x: p.x, y: p.y, kind, scale: 0.35 });
    }
    for b in bullets {
        all_sprites.push(Sprite {
            x: b.x,
            y: b.y,
            kind: SpriteKind::Bullet,
            scale: 0.15,
        });
    }
    for d in decals {
        all_sprites.push(Sprite {
            x: d.x,
            y: d.y,
            kind: SpriteKind::Decal,
            scale: 0.20,
        });
    }
    draw_sprites(
        scene,
        rect,
        px,
        py,
        pa,
        tick,
        &z_buf,
        total_cols,
        &all_sprites,
    );
    // Sutiles: avoid usar `temp_lights` solo para iluminación (ya
    // está) — los flashes en sí no se renderizan como sprites.
    let _ = temp_lights;

    // --- Overlay: crosshair + minimap ---
    draw_crosshair(scene, rect);
    draw_minimap(scene, rect, px, py, pa);
}

/// Pinta todos los sprites visibles. Para cada uno:
/// 1. Transforma `(sprite - player)` al espacio cámara con la inversa
///    de la matriz `[plane | dir]`. `transformed.y` es la profundidad
///    (>0 = delante).
/// 2. `screen_x_center = (w/2) · (1 + transformed.x / transformed.y)`.
/// 3. Altura proporcional a `1/depth` escalada por `sprite.scale`.
/// 4. Pinta columna por columna en el rango horizontal; oculta la
///    columna si la pared en esa columna tiene `perp_dist <= depth`.
pub(crate) fn draw_sprites(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: PaintRect,
    px: f32,
    py: f32,
    pa: f32,
    tick: u64,
    z_buf: &[f32],
    total_cols: usize,
    sprites: &[Sprite],
) {
    let h = rect.h as f64;
    let half_fov = FOV * 0.5;
    let plane_len = half_fov.tan();
    // dir = (cos, sin); plane = perpendicular a dir · plane_len.
    let (sin_pa, cos_pa) = pa.sin_cos();
    let dir = (cos_pa, sin_pa);
    let plane = (-sin_pa * plane_len, cos_pa * plane_len);
    let inv_det = 1.0 / (plane.0 * dir.1 - dir.0 * plane.1);

    // Ordenar sprites por distancia descendente — los más lejanos
    // primero, así los cercanos pintan encima cuando se superponen.
    let mut visible: Vec<(usize, f32)> = sprites
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let dx = s.x - px;
            let dy = s.y - py;
            (i, dx * dx + dy * dy)
        })
        .collect();
    visible.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    for (idx, _) in visible {
        let s = &sprites[idx];
        let dx = s.x - px;
        let dy = s.y - py;
        // Transform al espacio cámara.
        let tx = inv_det * (dir.1 * dx - dir.0 * dy);
        let ty = inv_det * (-plane.1 * dx + plane.0 * dy);
        if ty <= 0.001 {
            continue; // detrás de la cámara
        }
        // Centro horizontal en columnas lógicas (0..total_cols).
        let screen_center_frac = 0.5 * (1.0 + tx / ty); // 0..1
        let center_col = screen_center_frac * total_cols as f32;
        // Tamaño aparente.
        let sprite_h = (h as f32 / ty * s.scale).min(h as f32 * 4.0);
        let sprite_w = sprite_h; // 1:1 aspect — los sprites Doom lo son
        let half_cols = (sprite_w * 0.5) / COL_STRIDE;
        let col_start = (center_col - half_cols).max(0.0) as usize;
        let col_end = ((center_col + half_cols).max(0.0) as usize).min(total_cols);

        let y_mid = h * 0.5 + rect.y as f64;
        // Anchor por kind:
        // - Barril/Pillar/Imp/Torch/Decal: apoyan en el "piso" del slice.
        //   Imp respira (bob vertical sinusoidal); Torch oscila sutil.
        // - Bullet: centrado a la altura del jugador (no toca piso ni
        //   techo, vuela horizontal).
        let bob = match s.kind {
            SpriteKind::Imp => (tick as f32 * 0.18).sin() * 0.05 * sprite_h,
            SpriteKind::Torch => (tick as f32 * 0.42).sin() * 0.015 * sprite_h,
            _ => 0.0,
        };
        let slice_h = (h as f32 / ty) as f64;
        let (y_top, y_bot) = match s.kind {
            SpriteKind::Bullet => {
                let half = sprite_h as f64 * 0.5;
                ((y_mid - half).max(rect.y as f64),
                 (y_mid + half).min((rect.y + rect.h) as f64))
            }
            _ => {
                let y_bot_g = (y_mid + slice_h * 0.5 + bob as f64).min((rect.y + rect.h) as f64);
                let y_top_g = (y_bot_g - sprite_h as f64).max(rect.y as f64);
                (y_top_g, y_bot_g)
            }
        };

        // Color con shading + fog + lighting puntual. Para sprites
        // dinámicos pasamos lista vacía de bullets/temp_lights (un
        // sprite no se ilumina a sí mismo; usa su color base).
        let (base, _appearance_h) = s.appearance();
        let lights = lighting_contribution(s.x, s.y, tick, &[], &[]);
        let light_mul = (
            (AMBIENT + lights.0).min(2.0),
            (AMBIENT + lights.1).min(2.0),
            (AMBIENT + lights.2).min(2.0),
        );
        let mut col = (base.0 * light_mul.0, base.1 * light_mul.1, base.2 * light_mul.2);
        col = shade_by_dist(col, ty);
        col = apply_fog(col, ty);
        let color = rgb(col.0, col.1, col.2);

        for cidx in col_start..col_end {
            if cidx >= z_buf.len() {
                break;
            }
            // Z-test: si la pared está más cerca, sprite tapado en esa col.
            if z_buf[cidx] < ty {
                continue;
            }
            let x_pix = rect.x as f64 + cidx as f64 * COL_STRIDE as f64;
            let r = llimphi_ui::llimphi_raster::kurbo::Rect::new(
                x_pix,
                y_top,
                x_pix + COL_STRIDE as f64,
                y_bot,
            );
            scene.fill(
                Fill::NonZero,
                llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
                color,
                None,
                &r,
            );
        }
    }
}

/// Overlay full-screen para `game_over` o `victory`: rect negro
/// semi-transparente + título grande + subtítulo. Texto via parley
/// con el typesetter cacheado del runtime.
pub(crate) fn draw_overlay(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: PaintRect,
    title: &str,
    subtitle: &str,
    title_color: (f32, f32, f32),
) {
    // Rect negro semi-transparente.
    let scrim = llimphi_ui::llimphi_raster::kurbo::Rect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        (rect.y + rect.h) as f64,
    );
    scene.fill(
        Fill::NonZero,
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        Color::from_rgba8(0, 0, 0, 175),
        None,
        &scrim,
    );

    let cx = rect.x as f64 + rect.w as f64 * 0.5;
    let cy = rect.y as f64 + rect.h as f64 * 0.5;
    let tcolor = rgb(title_color.0, title_color.1, title_color.2);

    // Título grande centrado.
    let title_size = 64.0_f32;
    let title_block = llimphi_ui::llimphi_text::TextBlock {
        text: title,
        size_px: title_size,
        color: tcolor,
        origin: (cx - rect.w as f64 * 0.5, cy - title_size as f64),
        max_width: Some(rect.w),
        alignment: llimphi_ui::llimphi_text::Alignment::Center,
        line_height: 1.0,
    
        italic: false,
        font_family: None,
    };
    let layout = llimphi_ui::llimphi_text::layout_block(ts, &title_block);
    llimphi_ui::llimphi_text::draw_layout(scene, &layout, tcolor, title_block.origin);

    // Subtítulo más chico debajo.
    let sub_size = 18.0_f32;
    let sub_color = Color::from_rgba8(230, 220, 200, 220);
    let sub_block = llimphi_ui::llimphi_text::TextBlock {
        text: subtitle,
        size_px: sub_size,
        color: sub_color,
        origin: (cx - rect.w as f64 * 0.5, cy + 8.0),
        max_width: Some(rect.w),
        alignment: llimphi_ui::llimphi_text::Alignment::Center,
        line_height: 1.0,
    
        italic: false,
        font_family: None,
    };
    let layout = llimphi_ui::llimphi_text::layout_block(ts, &sub_block);
    llimphi_ui::llimphi_text::draw_layout(scene, &layout, sub_color, sub_block.origin);
}

/// Crosshair central — dos rectángulos finos cruzados con un punto
/// hueco en medio. No es interactivo, sólo orienta el aim.
pub(crate) fn draw_crosshair(scene: &mut llimphi_ui::llimphi_raster::vello::Scene, rect: PaintRect) {
    let cx = rect.x as f64 + rect.w as f64 * 0.5;
    let cy = rect.y as f64 + rect.h as f64 * 0.5;
    let arm: f64 = 8.0;
    let thick: f64 = 1.5;
    let color = Color::from_rgba8(255, 240, 200, 180);
    // Horizontal.
    let h_rect = llimphi_ui::llimphi_raster::kurbo::Rect::new(
        cx - arm,
        cy - thick * 0.5,
        cx + arm,
        cy + thick * 0.5,
    );
    scene.fill(
        Fill::NonZero,
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        color,
        None,
        &h_rect,
    );
    // Vertical.
    let v_rect = llimphi_ui::llimphi_raster::kurbo::Rect::new(
        cx - thick * 0.5,
        cy - arm,
        cx + thick * 0.5,
        cy + arm,
    );
    scene.fill(
        Fill::NonZero,
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        color,
        None,
        &v_rect,
    );
    // Punto central — un pequeño cuadrado oscuro para marcar el aim.
    let dot = llimphi_ui::llimphi_raster::kurbo::Rect::new(cx - 1.0, cy - 1.0, cx + 1.0, cy + 1.0);
    scene.fill(
        Fill::NonZero,
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        Color::from_rgba8(20, 10, 10, 220),
        None,
        &dot,
    );
}

pub(crate) fn draw_sky_and_floor(scene: &mut llimphi_ui::llimphi_raster::vello::Scene, rect: PaintRect) {
    let bands = 16_usize;
    let h = rect.h as f64;
    let band_h = h / bands as f64 * 0.5; // mitad superior = cielo, mitad inferior = piso
    let mid = rect.y as f64 + h * 0.5;
    for i in 0..bands {
        let y_top = mid - (i + 1) as f64 * band_h;
        let y_bot = mid - i as f64 * band_h;
        let frac = (i as f32 + 0.5) / bands as f32;
        let r = llimphi_ui::llimphi_raster::kurbo::Rect::new(
            rect.x as f64,
            y_top,
            (rect.x + rect.w) as f64,
            y_bot,
        );
        scene.fill(
            Fill::NonZero,
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            ceiling_color(1.0 - frac),
            None,
            &r,
        );
    }
    for i in 0..bands {
        let y_top = mid + i as f64 * band_h;
        let y_bot = mid + (i + 1) as f64 * band_h;
        let frac = (i as f32 + 0.5) / bands as f32;
        let r = llimphi_ui::llimphi_raster::kurbo::Rect::new(
            rect.x as f64,
            y_top,
            (rect.x + rect.w) as f64,
            y_bot,
        );
        scene.fill(
            Fill::NonZero,
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            floor_color(frac),
            None,
            &r,
        );
    }
}

pub(crate) fn draw_minimap(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: PaintRect,
    px: f32,
    py: f32,
    pa: f32,
) {
    let cell: f64 = 9.0;
    let pad = 12.0_f64;
    let mm_w = cell * MAP_W as f64;
    let mm_h = cell * MAP_H as f64;
    let x0 = (rect.x + rect.w) as f64 - mm_w - pad;
    let y0 = rect.y as f64 + pad;

    // Fondo translúcido del minimap.
    let bg = llimphi_ui::llimphi_raster::kurbo::Rect::new(
        x0 - 4.0,
        y0 - 4.0,
        x0 + mm_w + 4.0,
        y0 + mm_h + 4.0,
    );
    scene.fill(
        Fill::NonZero,
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        Color::from_rgba8(0, 0, 0, 170),
        None,
        &bg,
    );

    // Celdas.
    for cy in 0..MAP_H {
        for cx in 0..MAP_W {
            let t = tile(cx as i32, cy as i32);
            if t == 0 {
                continue;
            }
            let (r, g, b) = material_color(t);
            let cell_rect = llimphi_ui::llimphi_raster::kurbo::Rect::new(
                x0 + cx as f64 * cell,
                y0 + cy as f64 * cell,
                x0 + (cx + 1) as f64 * cell,
                y0 + (cy + 1) as f64 * cell,
            );
            scene.fill(
                Fill::NonZero,
                llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
                rgb(r, g, b),
                None,
                &cell_rect,
            );
        }
    }

    // Sprites estáticos como puntos coloreados según su tipo. Los
    // bullets/decals/enemies no van al minimap — son ruidosos o
    // requieren state que el minimap no recibe.
    for s in initial_static_sprites().iter() {
        let (base, _) = s.appearance();
        let dot = llimphi_ui::llimphi_raster::kurbo::Circle::new(
            (x0 + s.x as f64 * cell, y0 + s.y as f64 * cell),
            2.0,
        );
        scene.fill(
            Fill::NonZero,
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            rgb(base.0, base.1, base.2),
            None,
            &dot,
        );
    }

    // Luces como anillos suaves del color de la luz — visualizan el
    // radio de influencia aproximado en el minimap.
    for l in LIGHTS {
        let halo = llimphi_ui::llimphi_raster::kurbo::Circle::new(
            (x0 + l.x as f64 * cell, y0 + l.y as f64 * cell),
            (l.strength as f64).sqrt() * cell * 0.9,
        );
        scene.stroke(
            &Stroke::new(0.8),
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            Color::from_rgba8(
                (l.color.0 * 255.0) as u8,
                (l.color.1 * 255.0) as u8,
                (l.color.2 * 255.0) as u8,
                90,
            ),
            None,
            &halo,
        );
    }

    // Jugador + cono FOV.
    let pxc = x0 + px as f64 * cell;
    let pyc = y0 + py as f64 * cell;
    let fov_len = cell * 3.0;
    let left = pa - FOV * 0.5;
    let right = pa + FOV * 0.5;
    let mut path = BezPath::new();
    path.move_to((pxc, pyc));
    path.line_to((pxc + left.cos() as f64 * fov_len, pyc + left.sin() as f64 * fov_len));
    path.move_to((pxc, pyc));
    path.line_to((pxc + right.cos() as f64 * fov_len, pyc + right.sin() as f64 * fov_len));
    path.move_to((pxc, pyc));
    path.line_to((pxc + pa.cos() as f64 * fov_len * 1.1, pyc + pa.sin() as f64 * fov_len * 1.1));
    scene.stroke(
        &Stroke::new(1.0),
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        Color::from_rgba8(255, 200, 80, 220),
        None,
        &path,
    );

    let player_dot = llimphi_ui::llimphi_raster::kurbo::Circle::new((pxc, pyc), 2.5);
    scene.fill(
        Fill::NonZero,
        llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
        Color::from_rgba8(255, 220, 100, 255),
        None,
        &player_dot,
    );
}
