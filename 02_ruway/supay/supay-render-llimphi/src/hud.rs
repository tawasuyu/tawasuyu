use super::*;

/// Pinta cielo arriba + tinte del piso del sector del jugador abajo.
/// El sector del jugador se infiere del primer sprite del jugador (el
/// snapshot no expone explícitamente sector del player en 3.1; el sprite
/// con índice 0 suele ser el avatar). Si no hay sectores, fallback gris.
pub(crate) fn draw_backdrop(scene: &mut Scene, rect: PaintRect, snap: &SceneSnapshot, cfg: &RenderConfig) {
    // Horizonte = línea donde z_cam=0 cae en pantalla. Con pitch sumamos
    // `focal · tan(pitch)` al centro vertical para que el sky/floor
    // backdrop se mueva con la mirada (mouse-look). Clampeamos a los
    // bordes del rect para no pintar fuera.
    let focal = (rect.h * 0.5) / (cfg.fov_y_deg.to_radians() * 0.5).tan();
    let pitch = snap.player.view_pitch.clamp(-PITCH_MAX, PITCH_MAX);
    let pitch_offset_px = (focal * pitch.tan()) as f64;
    let mid_y_unclamped = rect.y as f64 + (rect.h as f64) * 0.5 + pitch_offset_px;
    let mid_y = mid_y_unclamped.clamp(rect.y as f64, (rect.y + rect.h) as f64);
    let sky_rect = Rect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        mid_y,
    );
    let floor_rect = Rect::new(
        rect.x as f64,
        mid_y,
        (rect.x + rect.w) as f64,
        (rect.y + rect.h) as f64,
    );

    // Sky con textura real si el atlas la tiene (SKY1 en E1, SKY2 en
    // E2, SKY3 en E3). Scrolling horizontal según player.angle —
    // convención Doom: 360° = 4 × sky_width = 1024 pixels en panorama.
    let sky_drawn = (|| -> bool {
        let Some(atlas) = cfg.atlas.as_ref() else {
            return false;
        };
        let Some(tex) = atlas.wall_texture("SKY1") else {
            return false;
        };
        use llimphi_ui::llimphi_raster::peniko::{Blob, Extend, Image, ImageFormat};
        let tex_w = tex.width as f64;
        let tex_h = tex.height as f64;
        let panorama_px = tex_w * 4.0; // 360° = 4 × tex.width
        let px_per_rad = panorama_px / std::f64::consts::TAU;
        // Scroll: player.angle aumenta antihorario; el sky debe
        // moverse en el sentido opuesto (cuando giro a la izquierda,
        // el sky parece moverse a la derecha en pantalla).
        let scroll_x = (-snap.player.angle as f64) * px_per_rad;
        // FOV horizontal aproximada (asumimos rect 4:3-ish, fov_y=75°).
        // pixels image por pixel pantalla en horizontal:
        // ancho de sky panorama visible = (fov_x_rad / 2π) × panorama_px
        // Aproximación: tomamos fov_x = fov_y · aspect_ratio.
        let aspect = rect.w as f64 / rect.h.max(1.0) as f64;
        let fov_x_rad = (cfg.fov_y_deg as f64).to_radians() * aspect;
        let pixels_to_show = fov_x_rad / std::f64::consts::TAU * panorama_px;
        let scale_x = pixels_to_show / rect.w as f64;
        // Mantenemos el alto visual del sky constante (= mitad del rect)
        // para que el panorama no se estire al hacer pitch. El offset
        // vertical de la textura sigue al horizonte: `sky_top_y` es la
        // posición Y de la fila iy=0 del lump, calculada para que la
        // fila iy=tex_h (el bottom del panorama) caiga sobre el horizonte
        // virtual `mid_y_unclamped` (puede estar fuera del viewport
        // cuando el pitch es agresivo; vello clipea con `sky_rect`).
        let sky_visual_h = (rect.h as f64) * 0.5;
        let scale_y = tex_h / sky_visual_h;
        let sky_top_y = mid_y_unclamped - sky_visual_h;
        // Affine: image(ix, iy) → screen((ix - scroll_x) / scale_x, iy / scale_y).
        // Vello forward affine a/b/c/d/e/f donde sx = a·ix + c·iy + e,
        // sy = b·ix + d·iy + f.
        let xform = Affine::new([
            1.0 / scale_x,
            0.0,
            0.0,
            1.0 / scale_y,
            -scroll_x / scale_x + rect.x as f64,
            sky_top_y,
        ]);
        let img = Image::new(
            Blob::from(tex.rgba.clone()),
            ImageFormat::Rgba8,
            tex.width as u32,
            tex.height as u32,
        )
        .with_x_extend(Extend::Repeat)
        .with_y_extend(Extend::Pad);
        scene.fill(Fill::NonZero, Affine::IDENTITY, &img, Some(xform), &sky_rect);
        true
    })();

    if !sky_drawn {
        scene.fill(Fill::NonZero, Affine::IDENTITY, SKY_BAND_TOP, None, &sky_rect);
    }
    let _ = SKY_BAND_BOT;

    // Floor backdrop: si tenemos al menos un sector, usá su paleta.
    // Como heurística pickeamos el sector con más light_level (la
    // habitación más iluminada — suele ser donde el jugador está
    // cuando arranca el nivel). No es exacto pero quita el "gris muerto"
    // de la 3.0 cuando mirás al vacío.
    let brightest = snap.sectors.iter().max_by_key(|s| s.light_level);
    let floor_rgb = brightest
        .and_then(|s| {
            cfg.atlas
                .as_ref()
                .and_then(|a| a.flat_color(s.floor_pic))
                .or_else(|| Some(FLOOR_PALETTE[(s.floor_pic as usize) % FLOOR_PALETTE.len()]))
        })
        .unwrap_or((0x32, 0x2E, 0x28));
    let backdrop_shade = 0.45;
    let bg = Color::from_rgba8(
        ((floor_rgb.0 as f32) * backdrop_shade) as u8,
        ((floor_rgb.1 as f32) * backdrop_shade) as u8,
        ((floor_rgb.2 as f32) * backdrop_shade) as u8,
        255,
    );
    scene.fill(Fill::NonZero, Affine::IDENTITY, bg, None, &floor_rect);
}

// =====================================================================
// Weapon sprite overlay (Fase 3.15)
// =====================================================================
//
// Doom pinta `psprites[ps_weapon]` (la animación del arma en mano) como
// overlay 2D sobre la vista. Las coordenadas vienen en el viewport
// nominal 320×200; escalamos al rect real preservando aspect-fit
// (Doom 4:3, igual que el FB original).

/// Constante nominal del viewport Doom — el motor produce sx/sy
/// asumiendo esta resolución base.
pub(crate) const DOOM_VIEW_W: f32 = 320.0;
pub(crate) const DOOM_VIEW_H: f32 = 200.0;
/// Constante de psprite del motor: el counter `psp->sy` arranca en 32
/// (WEAPONTOP) en idle, sube hasta 128 (WEAPONBOTTOM) cuando el arma se
/// guarda. La diferencia `sy - WEAPONTOP` es cuánto cae el arma desde
/// la posición "lista para disparar".
pub(crate) const DOOM_WEAPON_TOP: f32 = 32.0;

pub(crate) fn draw_weapon_sprite(
    scene: &mut Scene,
    rect: PaintRect,
    weap: &WeaponSpriteSnap,
    player_light: u8,
    rim_boost: BoostRgb,
    cfg: &RenderConfig,
) {
    if !weap.active {
        return;
    }
    let Some(atlas) = cfg.atlas.as_ref() else {
        return;
    };
    // Las armas en Doom son sprites no-rotacionales con lump `<NAME><F>0`.
    // Nuestra `sprite_patch` con angle=1 cae automáticamente al fallback
    // omnidireccional vía `sprite_lump`.
    let Some((patch, mirror)) = atlas.sprite_patch(weap.sprite, weap.frame, 1) else {
        return;
    };

    // Escalado uniforme: usamos la altura del rect como referencia (Doom
    // standard 320×200 = 1.6:1, mismo aspect que nuestra ventana 1280×800).
    // Aspectos más altos letterboxean horizontalmente.
    let scale = (rect.w / DOOM_VIEW_W).min(rect.h / DOOM_VIEW_H);
    let patch_w_s = patch.width as f32 * scale;
    let patch_h_s = patch.height as f32 * scale;

    // Horizontal: psp->sx defaultea 0 = centrado. Cuando hay weapon bob
    // o switch animation, sx oscila ±N pixels. Centramos el patch +
    // offset horizontal de sx.
    let screen_x_center = rect.x + rect.w * 0.5 + weap.sx * scale;
    let screen_x = screen_x_center - patch_w_s * 0.5;

    // Vertical: psp->sy es la coord top-of-patch en el viewport nominal
    // 200px de Doom. WEAPONTOP=32 = arma totalmente levantada (visible);
    // sy crece hasta WEAPONBOTTOM=128 cuando el arma baja (al cambiar
    // de arma, por ejemplo). Anchor: con sy=32, el patch queda anclado
    // al bottom del rect; subir sy lo hunde por debajo (offscreen).
    let bottom = rect.y + rect.h;
    let screen_y = bottom - patch_h_s + (weap.sy - DOOM_WEAPON_TOP) * scale;

    // Fase 3.18: el arma se tinta por la luz del sector donde está
    // parado el jugador. Si el frame tiene `FF_FULLBRIGHT` (bit 7) —
    // muzzle flash, plasma idle frame, etc. — saltamos el shade y va a
    // luz plena (igual que `gather_sprite`). Depth = 0: el arma está
    // "en la mano", no debería atenuarse por niebla aunque el cuarto
    // sí lo esté.
    let full_bright = (weap.frame & 0x80) != 0;
    let shade = if full_bright {
        1.0
    } else {
        shade_for(player_light, 0.0, cfg)
    };
    // Fase 3.28: rim-light desde world lights cercanas. El arma recoge
    // tinte ambiente per-canal (torch azul → arma azulada; fireball
    // cerca → rim rojizo). Bypass en full_bright: el destello del
    // propio fogonazo domina y subsume el ambiente.
    let tint_rgb = if full_bright {
        [shade, shade, shade]
    } else {
        sprite_shade_with_world(shade, rim_boost)
    };
    let img = make_tinted_sprite_image_rgb(&patch, tint_rgb);
    // Affine: image(ix, iy) → screen(screen_x + ix·scale, screen_y + iy·scale).
    // Para mirror, X negativo + offset al borde derecho.
    let xform = if mirror {
        Affine::new([
            -(scale as f64),
            0.0,
            0.0,
            scale as f64,
            (screen_x + patch_w_s) as f64,
            screen_y as f64,
        ])
    } else {
        Affine::new([
            scale as f64,
            0.0,
            0.0,
            scale as f64,
            screen_x as f64,
            screen_y as f64,
        ])
    };
    scene.draw_image(&img, xform);
}

// =====================================================================
// Player overlays (Fase 3.14)
// =====================================================================
//
// Doom intercala PLAYPAL[1..13] cuando algo le pasa al jugador:
//   - [1..8]   = damage red flash (intensidad ∝ damagecount)
//   - [9..12]  = bonus yellow flash (intensidad ∝ bonuscount)
//   - [13]     = radiation suit green tint
//   - invuln   = inversión de colores via colormap (más caro de emular)
//
// Como sampleamos siempre con PLAYPAL[0] desde el renderer 3D, los
// overlays no aparecen "gratis" — los pintamos como rect full-screen
// semi-transparente al final del frame.

/// Pinta el overlay del jugador (damage/pickup/radsuit/invuln) sobre
/// todo el viewport. No-op si no hay overlays activos.
pub(crate) fn draw_player_overlays(scene: &mut Scene, rect: PaintRect, ov: &PlayerOverlays, tick: u64) {
    let Some((r, g, b, a)) = overlay_rgba(ov, tick) else {
        return;
    };
    let path = Rect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        (rect.y + rect.h) as f64,
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgba8(r, g, b, a),
        None,
        &path,
    );
}

/// Resuelve el overlay activo + su color RGBA. Prioridad Doom:
///   damage > bonus > radsuit. Invuln se superpone con tinte propio.
///
/// Acepta `tick` para el blink de los últimos 4 segundos (32 tics =
/// ~0.9 s a 35 Hz) de invuln/radsuit — bit 3 del tick controla on/off.
pub(crate) fn overlay_rgba(ov: &PlayerOverlays, tick: u64) -> Option<(u8, u8, u8, u8)> {
    use PlayerOverlays as O;
    let _ = std::mem::size_of::<O>();
    // Invulnerability: blink 32 tics finales, blanco brillante.
    let invuln_active = ov.power_invuln > 0
        && (ov.power_invuln > 4 * 32 || (tick & 0x8) != 0);
    if invuln_active {
        // Blanco semi-translúcido — aproximación cheap del invert colors
        // de Doom. Subir alpha hace que la escena "se desature".
        return Some((220, 220, 232, 110));
    }
    // Damage: red flash 8 niveles, alpha cada 8 pts de damagecount.
    if ov.damage_count > 0 {
        // Doom: (dc + 7) >> 3 → niveles 1..8. NUMREDPALS=8.
        let level = (((ov.damage_count + 7) >> 3).min(8)) as u8;
        // Alpha ramp 40..200 sobre los 8 niveles (más fuerte = más opaco).
        let alpha = 24 + level * 24;
        return Some((220, 30, 30, alpha));
    }
    // Bonus pickup: yellow flash 4 niveles.
    if ov.bonus_count > 0 {
        // Doom: (bc + 7) >> 3, NUMBONUSPALS=4.
        let level = (((ov.bonus_count + 7) >> 3).min(4)) as u8;
        let alpha = 24 + level * 18;
        return Some((215, 180, 70, alpha));
    }
    // Radsuit: green tint constante mientras el power > 4*32 (≈3.6 s),
    // luego blinkea con bit 3 del tick.
    if ov.power_radsuit > 0 {
        let active = ov.power_radsuit > 4 * 32 || (tick & 0x8) != 0;
        if active {
            return Some((45, 140, 60, 64));
        }
    }
    // Berserk (`pw_strength`): tinte rojo que fade-out lento. Doom:
    // `palette_idx = 12 - (strength >> 6)`, clampado a 0..7 = paletas
    // STARTREDPALS+0..7. Nosotros mapeamos a alpha directo: recién
    // agarrado el berserk strength=1 → idx=12 (max), después de muchos
    // tics strength sube y el alpha cae. `strength >> 6` empieza en 0
    // y crece a ~16+ en pocos minutos.
    if ov.power_strength > 0 {
        let shift = (ov.power_strength >> 6) as i32;
        let level = (12 - shift).clamp(1, 8) as u8;
        let alpha = (level * 10).min(90); // ramp 10..80
        return Some((180, 40, 30, alpha));
    }
    None
}

// =====================================================================
// Crosshair + viñeta de cabina (Fase 3.19)
// =====================================================================
//
// Dos capas cosméticas post-3D:
//
//   - **Viñeta**: gradient radial transparente→crimson_deep, oscurece
//     las esquinas para que el viewport se sienta como mirar por la
//     visera de un casco. Multiplica el rango de luz percibido: el
//     foco visual queda en el centro de la acción.
//   - **Crosshair**: cruz fina centrada de 4 chevrons + dot, con halo
//     crimson_deep abajo para legibilidad sobre cualquier fondo (paredes
//     claras, cielo, sprites). 7 px de marca, 1 px de ancho.

/// Pinta una viñeta radial muy sutil sobre todo el rect. `cfg.vignette`
/// controla la fuerza global (0..1+). Sin allocar paths: un único fill
/// del rect con el gradient como brush.
pub(crate) fn draw_vignette(scene: &mut Scene, rect: PaintRect, cfg: &RenderConfig) {
    use llimphi_ui::llimphi_raster::peniko::{color::AlphaColor, Gradient};
    if cfg.vignette <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let cx = rect.x as f64 + rect.w as f64 * 0.5;
    let cy = rect.y as f64 + rect.h as f64 * 0.5;
    // Radio = mitad de la diagonal — el último stop alcanza justo a las
    // esquinas. El centro queda transparente; el final, crimson_deep
    // tinted con alpha proporcional a `cfg.vignette`.
    let diag_half = (((rect.w as f64).powi(2) + (rect.h as f64).powi(2)).sqrt() * 0.5) as f32;
    let strength = cfg.vignette.clamp(0.0, 1.5);
    // crimson_deep ≈ rgba(90,14,14) — mismo tono del marco del header.
    let inner: Color = AlphaColor::new([0.35, 0.05, 0.05, 0.0]);
    let mid: Color = AlphaColor::new([0.35, 0.05, 0.05, 0.05 * strength]);
    let outer: Color = AlphaColor::new([0.35, 0.05, 0.05, 0.30 * strength]);
    // Tres stops: el segundo en 0.6 evita que la transición sea lineal
    // (que se ve falsa) y mantiene el centro limpio. La curva resultante
    // es casi quadrática — el oscurecimiento empieza recién en el último
    // tercio del radio.
    let gradient = Gradient::new_radial(Point::new(cx, cy), diag_half)
        .with_stops([(0.0, inner), (0.6, mid), (1.0, outer)].as_slice());
    let full = Rect::new(
        rect.x as f64,
        rect.y as f64,
        (rect.x + rect.w) as f64,
        (rect.y + rect.h) as f64,
    );
    scene.fill(Fill::NonZero, Affine::IDENTITY, &gradient, None, &full);
}

/// Pinta un crosshair central minimalista: 4 chevrons + dot, con sombra
/// crimson_deep debajo para destacar sobre fondos claros. Tamaño fijo
/// en pixels (no escala con el viewport — un crosshair que crece se
/// siente raro). Diseño:
///
/// ```text
///        ▌
///        ▌
///   ▬▬     ▬▬
///       ·
///        ▌
///        ▌
/// ```
///
/// Distancia del centro al inicio de cada marca = `GAP` (6 px).
/// Largo de cada marca = `LEN` (7 px). Ancho = 1 px (line cap square).
pub(crate) fn draw_crosshair(scene: &mut Scene, rect: PaintRect) {
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    const GAP: f64 = 6.0;
    const LEN: f64 = 7.0;
    const W: f64 = 1.0;
    const DOT: f64 = 1.0;
    let cx = rect.x as f64 + rect.w as f64 * 0.5;
    let cy = rect.y as f64 + rect.h as f64 * 0.5;
    // Color de tinta + sombra. La sombra va 1 px abajo-derecha para
    // que el ojo lea las marcas aún sobre cielo claro o paredes
    // texturizadas brillantes.
    let ink: Color = AlphaColor::new([0.96, 0.92, 0.84, 0.95]); // bone ~232,216,192
    let halo: Color = AlphaColor::new([0.05, 0.02, 0.02, 0.45]); // crimson_deep darker
    // Build cada chevron como rect de 1×LEN o LEN×1.
    let arms: [Rect; 4] = [
        // top
        Rect::new(cx - W * 0.5, cy - GAP - LEN, cx + W * 0.5, cy - GAP),
        // bottom
        Rect::new(cx - W * 0.5, cy + GAP, cx + W * 0.5, cy + GAP + LEN),
        // left
        Rect::new(cx - GAP - LEN, cy - W * 0.5, cx - GAP, cy + W * 0.5),
        // right
        Rect::new(cx + GAP, cy - W * 0.5, cx + GAP + LEN, cy + W * 0.5),
    ];
    let dot = Rect::new(cx - DOT, cy - DOT, cx + DOT, cy + DOT);
    // Sombra (offset 1px abajo-derecha): se pinta primero para quedar
    // debajo de la tinta.
    let shadow_xform = Affine::translate((1.0, 1.0));
    for arm in &arms {
        scene.fill(Fill::NonZero, shadow_xform, halo, None, arm);
    }
    scene.fill(Fill::NonZero, shadow_xform, halo, None, &dot);
    // Tinta principal.
    for arm in &arms {
        scene.fill(Fill::NonZero, Affine::IDENTITY, ink, None, arm);
    }
    scene.fill(Fill::NonZero, Affine::IDENTITY, ink, None, &dot);
}

// =====================================================================
// HUD inferior modernista (Fase 3.20)
// =====================================================================
//
// Banda slim al pie del viewport 3D con los stats vitales del jugador:
// HEALTH (% + barra), ARMOR (% + barra tinted por tipo), AMMO (current
// / max del slot del arma activa), KEYS (chips por color).
//
// Paleta espejo del header del host (crimson/amber/bone/dust) para que
// la app entera se sienta una sola pieza. Fondo COLOR_BG_PANEL con
// alpha para no ocluir totalmente la acción del piso.

/// Paleta interna usada por el HUD — eco visual del header del host.
mod hud_color {
    use super::Color;
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    pub const PANEL: Color = Color::from_rgba8(12, 8, 8, 215);
    pub const RULE: Color = Color::from_rgba8(48, 16, 16, 255);
    pub const RULE_SOFT: Color = Color::from_rgba8(48, 16, 16, 140);
    pub const TRACK: Color = Color::from_rgba8(6, 4, 4, 255);
    pub const BONE: Color = Color::from_rgba8(216, 204, 188, 255);
    pub const DUST: Color = Color::from_rgba8(132, 124, 116, 255);
    pub const AMBER: Color = Color::from_rgba8(232, 168, 76, 255);
    pub const HEALTH_OK: Color = Color::from_rgba8(140, 188, 96, 255);
    pub const HEALTH_LOW: Color = Color::from_rgba8(232, 168, 76, 255);
    pub const HEALTH_CRIT: Color = Color::from_rgba8(220, 50, 50, 255);
    pub const ARMOR_GREEN: Color = Color::from_rgba8(140, 188, 96, 255);
    pub const ARMOR_BLUE: Color = Color::from_rgba8(96, 160, 232, 255);
    pub const KEY_BLUE: Color = Color::from_rgba8(56, 128, 224, 255);
    pub const KEY_YELLOW: Color = Color::from_rgba8(232, 200, 72, 255);
    pub const KEY_RED: Color = Color::from_rgba8(220, 60, 60, 255);
    /// Tinte para el indicador "skull" — más cálido/desaturado.
    pub fn skullize(base: Color) -> Color {
        let [r, g, b, a] = base.components;
        AlphaColor::new([r * 0.85, g * 0.85, b * 0.85, a])
    }
}

pub(crate) const HUD_HEIGHT: f64 = 38.0;
pub(crate) const HUD_PAD: f64 = 10.0;

/// Pinta la banda del HUD al pie del `rect`. Asume `stats.health > 0`
/// (caller filtra el pre-mapa para que no aparezca un HUD hueco).
pub(crate) fn draw_hud(scene: &mut Scene, ts: &mut Typesetter, rect: PaintRect, stats: &PlayerStats) {
    let view_w = rect.w as f64;
    let view_h = rect.h as f64;
    if view_w < 160.0 || view_h < HUD_HEIGHT + 32.0 {
        // Viewport demasiado chico — el HUD comería medio frame.
        return;
    }
    let bottom = rect.y as f64 + view_h;
    let top = bottom - HUD_HEIGHT;
    let left = rect.x as f64;
    let right = left + view_w;

    // Fondo + hairline crimson del borde superior.
    let panel = Rect::new(left, top, right, bottom);
    scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::PANEL, None, &panel);
    let rule = Rect::new(left, top, right, top + 1.0);
    scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::RULE, None, &rule);

    // Layout: 4 tiles. Anchos relativos al view_w restante.
    //   [ HEALTH 28% ][ ARMOR 22% ][ AMMO 26% ][ KEYS resto ]
    let usable = view_w - HUD_PAD * 2.0;
    let w_health = (usable * 0.28).floor();
    let w_armor = (usable * 0.22).floor();
    let w_ammo = (usable * 0.26).floor();
    let w_keys = usable - w_health - w_armor - w_ammo;

    let mut x = left + HUD_PAD;
    draw_hud_stat_tile(
        scene, ts, x, top, w_health,
        "HP",
        format!("{}", stats.health.max(0)),
        stats.health as f32 / 100.0,
        health_color(stats.health),
    );
    x += w_health;
    // Divider sutil entre tiles.
    draw_hud_divider(scene, x, top);
    draw_hud_stat_tile(
        scene, ts, x, top, w_armor,
        "AR",
        format!("{}", stats.armor_points.max(0)),
        stats.armor_points as f32 / 100.0,
        armor_color(stats.armor_type),
    );
    x += w_armor;
    draw_hud_divider(scene, x, top);
    draw_hud_ammo_tile(scene, ts, x, top, w_ammo, stats);
    x += w_ammo;
    draw_hud_divider(scene, x, top);
    draw_hud_keys_tile(scene, ts, x, top, w_keys, stats);
}

/// Tile genérico de "stat con barra": label dust arriba-izquierda,
/// número grande bone abajo-izquierda, barra slim al pie del tile.
pub(crate) fn draw_hud_stat_tile(
    scene: &mut Scene,
    ts: &mut Typesetter,
    x: f64,
    top: f64,
    w: f64,
    label: &str,
    value: String,
    pct: f32,
    bar_color: Color,
) {
    // Label "HP" / "AR" arriba.
    text::draw_block(
        scene,
        ts,
        &TextBlock {
            text: label,
            size_px: 9.0,
            color: hud_color::DUST,
            origin: (x + 4.0, top + 4.0),
            max_width: Some(w as f32 - 8.0),
            alignment: Alignment::Start,
            line_height: 1.0,
            italic: false,
            font_family: None,
        },
    );
    // Valor grande abajo del label.
    text::draw_block(
        scene,
        ts,
        &TextBlock {
            text: &value,
            size_px: 16.0,
            color: hud_color::BONE,
            origin: (x + 4.0, top + 13.0),
            max_width: Some(w as f32 - 8.0),
            alignment: Alignment::Start,
            line_height: 1.0,
            italic: false,
            font_family: None,
        },
    );
    // Barra slim al pie. 3 px de alto + track 1 px.
    let bar_y0 = top + HUD_HEIGHT - 6.0;
    let bar_y1 = bar_y0 + 3.0;
    let bar_x0 = x + 4.0;
    let bar_x1 = x + w - 6.0;
    let track = Rect::new(bar_x0, bar_y0, bar_x1, bar_y1);
    scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::TRACK, None, &track);
    let fill_w = ((bar_x1 - bar_x0) * pct.clamp(0.0, 1.0) as f64).max(0.0);
    if fill_w > 0.0 {
        let filled = Rect::new(bar_x0, bar_y0, bar_x0 + fill_w, bar_y1);
        scene.fill(Fill::NonZero, Affine::IDENTITY, bar_color, None, &filled);
    }
}

/// Tile de ammo: muestra `current / max` del slot del arma activa, o
/// "—" si la actual no consume ammo (puño, motosierra).
pub(crate) fn draw_hud_ammo_tile(
    scene: &mut Scene,
    ts: &mut Typesetter,
    x: f64,
    top: f64,
    w: f64,
    stats: &PlayerStats,
) {
    // Label "AMMO" + sufijo del slot (CLIP/SHELL/CELL/MISL).
    let slot_label = stats.weapon_ammo_slot().map(ammo_slot_name).unwrap_or("—");
    text::draw_block(
        scene,
        ts,
        &TextBlock {
            text: &format!("AMMO · {slot_label}"),
            size_px: 9.0,
            color: hud_color::DUST,
            origin: (x + 4.0, top + 4.0),
            max_width: Some(w as f32 - 8.0),
            alignment: Alignment::Start,
            line_height: 1.0,
            italic: false,
            font_family: None,
        },
    );
    // current / max — current en ámbar si está bajo (<25%).
    let (value, pct, color) = match stats.weapon_ammo_slot() {
        Some(slot) => {
            let cur = stats.ammo[slot].max(0);
            let max = stats.max_ammo[slot].max(1);
            let pct = (cur as f32) / (max as f32);
            let col = if pct < 0.25 {
                hud_color::HEALTH_CRIT
            } else if pct < 0.5 {
                hud_color::HEALTH_LOW
            } else {
                hud_color::BONE
            };
            (format!("{cur} / {max}"), pct, col)
        }
        None => ("∞".to_string(), 0.0, hud_color::DUST),
    };
    text::draw_block(
        scene,
        ts,
        &TextBlock {
            text: &value,
            size_px: 16.0,
            color,
            origin: (x + 4.0, top + 13.0),
            max_width: Some(w as f32 - 8.0),
            alignment: Alignment::Start,
            line_height: 1.0,
            italic: false,
            font_family: None,
        },
    );
    // Barra slim al pie — ammo en ámbar para distinguir de HP/AR.
    let bar_y0 = top + HUD_HEIGHT - 6.0;
    let bar_y1 = bar_y0 + 3.0;
    let bar_x0 = x + 4.0;
    let bar_x1 = x + w - 6.0;
    let track = Rect::new(bar_x0, bar_y0, bar_x1, bar_y1);
    scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::TRACK, None, &track);
    if pct > 0.0 {
        let fill_w = ((bar_x1 - bar_x0) * pct.clamp(0.0, 1.0) as f64).max(0.0);
        let filled = Rect::new(bar_x0, bar_y0, bar_x0 + fill_w, bar_y1);
        scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::AMBER, None, &filled);
    }
}

/// Tile de llaves: hasta 6 chips por color (cards + skulls). Chip
/// vacío si no se tiene la llave — silueta crimson_deep.
pub(crate) fn draw_hud_keys_tile(
    scene: &mut Scene,
    ts: &mut Typesetter,
    x: f64,
    top: f64,
    w: f64,
    stats: &PlayerStats,
) {
    text::draw_block(
        scene,
        ts,
        &TextBlock {
            text: "KEYS",
            size_px: 9.0,
            color: hud_color::DUST,
            origin: (x + 4.0, top + 4.0),
            max_width: Some(w as f32 - 8.0),
            alignment: Alignment::Start,
            line_height: 1.0,
            italic: false,
            font_family: None,
        },
    );
    // 6 chips: card_blue, card_yellow, card_red, skull_blue, skull_yellow, skull_red.
    // Cards: rectángulo 12×8. Skulls: rectángulo 12×8 con borde más grueso
    // (un truco visual para distinguirlos sin pintar un sprite real).
    let colors = [
        hud_color::KEY_BLUE,
        hud_color::KEY_YELLOW,
        hud_color::KEY_RED,
    ];
    let chip_w = 13.0;
    let chip_h = 8.0;
    let gap = 4.0;
    let chips_total = chip_w * 6.0 + gap * 5.0;
    let mut cx = x + 4.0;
    // Si los chips no entran, los apretamos.
    let avail = w - 8.0;
    let scale = if chips_total > avail {
        avail / chips_total
    } else {
        1.0
    };
    let chip_w = chip_w * scale;
    let gap = gap * scale;
    let cy0 = top + 18.0;
    let cy1 = cy0 + chip_h;
    for i in 0..6 {
        let has = stats.cards[i];
        let color_idx = i % 3;
        let is_skull = i >= 3;
        let base = colors[color_idx];
        let chip = Rect::new(cx, cy0, cx + chip_w, cy1);
        if has {
            let fill = if is_skull { hud_color::skullize(base) } else { base };
            scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &chip);
            if is_skull {
                // Mini-banda crimson en el medio del chip → silueta de
                // calavera apenas evocada.
                let band = Rect::new(
                    cx + chip_w * 0.35,
                    cy0 + 2.0,
                    cx + chip_w * 0.65,
                    cy1 - 2.0,
                );
                scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::TRACK, None, &band);
            }
        } else {
            // Chip vacío: borde crimson_deep, interior transparente.
            // Lo aproximamos con 4 rects 1px (top/bottom/left/right).
            let bw = 1.0;
            for r in &[
                Rect::new(cx, cy0, cx + chip_w, cy0 + bw),
                Rect::new(cx, cy1 - bw, cx + chip_w, cy1),
                Rect::new(cx, cy0, cx + bw, cy1),
                Rect::new(cx + chip_w - bw, cy0, cx + chip_w, cy1),
            ] {
                scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::RULE_SOFT, None, r);
            }
        }
        cx += chip_w + gap;
    }
}

pub(crate) fn draw_hud_divider(scene: &mut Scene, x: f64, top: f64) {
    let r = Rect::new(x, top + 6.0, x + 1.0, top + HUD_HEIGHT - 6.0);
    scene.fill(Fill::NonZero, Affine::IDENTITY, hud_color::RULE_SOFT, None, &r);
}

pub(crate) fn health_color(hp: i32) -> Color {
    if hp >= 60 {
        hud_color::HEALTH_OK
    } else if hp >= 25 {
        hud_color::HEALTH_LOW
    } else {
        hud_color::HEALTH_CRIT
    }
}

pub(crate) fn armor_color(armor_type: u8) -> Color {
    match armor_type {
        1 => hud_color::ARMOR_GREEN,
        2 => hud_color::ARMOR_BLUE,
        _ => hud_color::DUST,
    }
}

pub(crate) fn ammo_slot_name(slot: usize) -> &'static str {
    match slot {
        0 => "CLIP",
        1 => "SHELL",
        2 => "CELL",
        3 => "MISL",
        _ => "—",
    }
}
