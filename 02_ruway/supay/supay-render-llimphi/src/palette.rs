use super::*;

pub(crate) const WALL_PALETTE: &[(u8, u8, u8)] = &[
    (0xB0, 0x88, 0x66), // BROVINE — marrón cálido
    (0x88, 0x80, 0x70), // BLAKWAL — gris piedra
    (0x68, 0x58, 0x4C), // BROWN1  — marrón oscuro
    (0x8C, 0x74, 0x5C), // BROVINE alt
    (0x9C, 0x9C, 0x9C), // GRAYBIG — gris claro
    (0x6C, 0x6C, 0x6C), // GRAY1   — gris medio
    (0xA8, 0x84, 0x54), // STARTAN — tan UAC
    (0x74, 0x5C, 0x44), // BROWN2  — marrón quemado
    (0x84, 0x6C, 0x54), // marrón medio
    (0x5C, 0x4C, 0x40), // marrón profundo
    (0xB8, 0xA0, 0x80), // sand
    (0x4C, 0x54, 0x60), // slate
    (0x80, 0x70, 0x58), // tech tan
    (0x68, 0x64, 0x60), // dust gray
    (0x90, 0x80, 0x68), // cardboard
    (0xA0, 0x70, 0x4C), // rust
];

/// Pisos: marrones tierra, gris piedra, slime verde, marble azulado.
/// Indexed por `floor_pic % len`.
pub(crate) const FLOOR_PALETTE: &[(u8, u8, u8)] = &[
    (0x54, 0x44, 0x34), // FLAT5_5 — dirt
    (0x4C, 0x48, 0x44), // FLAT5_1 — stone
    (0x3C, 0x54, 0x38), // SLIME — slime green
    (0x38, 0x40, 0x50), // marble blue
    (0x5C, 0x50, 0x3C), // wood
    (0x44, 0x3C, 0x34), // tech dark
    (0x6C, 0x58, 0x40), // sand floor
    (0x40, 0x38, 0x2C), // ash
];

/// Techos: típicamente más oscuros + un blue-noche que reemplaza a F_SKY1.
pub(crate) const CEIL_PALETTE: &[(u8, u8, u8)] = &[
    (0x38, 0x34, 0x30), // CEIL3_1 — dark slate
    (0x44, 0x40, 0x38), // CEIL5_2 — light slate
    (0x2C, 0x28, 0x24), // RROCK04 — black rock
    (0x4C, 0x44, 0x38), // tech panel
];

/// "Cielo" en 3.2 se detecta comparando `sector.ceiling_pic` contra el
/// `sky_pic` del snapshot (el motor lo resuelve vía `skyflatnum` al
/// cargar el mapa). Cuando coincide, los pisos/techos por subsector
/// directamente NO emiten polígono y el backdrop se ve por ahí.
pub(crate) const SKY_BAND_TOP: Color = Color::from_rgba8(8, 10, 18, 255);
pub(crate) const SKY_BAND_BOT: Color = Color::from_rgba8(20, 22, 32, 255);

pub(crate) fn ceiling_is_sky(sec: &SectorSnap, sky_pic: u16) -> bool {
    sky_pic != NO_SKY_PIC && sec.ceiling_pic == sky_pic
}

// =====================================================================
// Shading
// =====================================================================

/// Gamma de la luz del sector (Fase 3.58). Doom **no** atenúa la luz
/// linealmente: su colormap mantiene la imagen casi a tope hasta luces
/// medias (~L≥160) y sólo cae rápido en la mitad baja. Una curva `< 1`
/// levanta los tonos medios/altos para igualar el renderer software de
/// Doom (ground truth) — sin ella, `light/255` lineal dejaba todo el nivel
/// notablemente más apagado que el original (L=192 daba 0.75 en vez de
/// ~0.86). Los sectores realmente oscuros (L<96) siguen oscuros.
pub(crate) const LIGHT_GAMMA: f32 = 0.62;

pub(crate) fn shade_for(light_level: u8, depth: f32, cfg: &RenderConfig) -> f32 {
    let light = (light_level as f32 / 255.0).powf(LIGHT_GAMMA);
    let fog = 1.0 - (depth / cfg.far_fog).clamp(0.0, 0.85);
    (light * fog).clamp(0.05, 1.0)
}

pub(crate) fn tint(rgb: (u8, u8, u8), shade: f32) -> Color {
    Color::from_rgba8(
        ((rgb.0 as f32) * shade) as u8,
        ((rgb.1 as f32) * shade) as u8,
        ((rgb.2 as f32) * shade) as u8,
        255,
    )
}

/// Hash determinístico ligero para variar tonos por linedef. xorshift
/// de 32 bits sembrado con el índice — la idea es que paredes adyacentes
/// no tengan exactamente el mismo color base.
pub(crate) fn wall_hash(wall_idx: u32) -> u32 {
    let mut x = wall_idx.wrapping_add(0x9E37_79B9);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

pub(crate) fn wall_color(
    wall_idx: u32,
    wall: &WallSeg,
    sec: &SectorSnap,
    depth: f32,
    band: u32,
    bands: u32,
    cfg: &RenderConfig,
) -> Color {
    // Base color por linedef hash + nudge por front_sector (para que
    // cada habitación tienda a una familia de tonos sin ser uniforme).
    let h = wall_hash(wall_idx).wrapping_add(wall.front_sector.wrapping_mul(7));
    let base = WALL_PALETTE[(h as usize) % WALL_PALETTE.len()];

    // Banda 0 = piso → más oscura. Banda top = techo → más clara. Curva
    // levemente positiva: simulación cheap de iluminación cenital.
    let band_t = if bands <= 1 {
        0.5
    } else {
        band as f32 / (bands - 1) as f32
    };
    // Factor en [0.78, 1.12] — bajo abajo, alto arriba, con sutil curva.
    let band_mul = 0.78 + 0.34 * band_t;

    // Variación pseudo-aleatoria por banda (ladrillo / panel feel).
    let band_jitter = {
        let hj = wall_hash(wall_idx ^ band.wrapping_mul(0x1234_5));
        let n = ((hj as f32) / (u32::MAX as f32)) * 2.0 - 1.0; // -1..1
        1.0 + n * 0.08 // ±8%
    };

    let base_shade = shade_for(sec.light_level, depth, cfg);
    let shade = (base_shade * band_mul * band_jitter).clamp(0.05, 1.0);
    tint(base, shade)
}

/// Colores `(abajo, arriba)` de una pared **sin textura** para el camino de
/// gradiente vertical continuo (Fase 3.56). Mismo color base + shading por
/// distancia que [`wall_color`], pero con el multiplicador zenital evaluado
/// en los extremos `band_t = 0` (piso, más oscuro) y `band_t = 1` (techo,
/// más claro) en vez de en `bands` escalones discretos — el renderer
/// interpola entre ambos con un solo `GradientFill`, sin las costuras de
/// banda. Se omite el jitter por-banda (era un truco de textura falsa para
/// los escalones; con gradiente suave no aporta). Reproduce el rango
/// `[0.78, 1.12]·base_shade` de `wall_color` en sus dos puntas.
pub(crate) fn wall_gradient_colors(
    wall_idx: u32,
    wall: &WallSeg,
    sec: &SectorSnap,
    depth: f32,
    cfg: &RenderConfig,
) -> (Color, Color) {
    let h = wall_hash(wall_idx).wrapping_add(wall.front_sector.wrapping_mul(7));
    let base = WALL_PALETTE[(h as usize) % WALL_PALETTE.len()];
    let base_shade = shade_for(sec.light_level, depth, cfg);
    let at = |band_t: f32| {
        let band_mul = 0.78 + 0.34 * band_t;
        tint(base, (base_shade * band_mul).clamp(0.05, 1.0))
    };
    (at(0.0), at(1.0))
}

pub(crate) fn floor_color(sec: &SectorSnap, depth: f32, cfg: &RenderConfig) -> Color {
    let rgb = cfg
        .atlas
        .as_ref()
        .and_then(|a| a.flat_color(sec.floor_pic))
        .unwrap_or_else(|| FLOOR_PALETTE[(sec.floor_pic as usize) % FLOOR_PALETTE.len()]);
    let shade = shade_for(sec.light_level, depth, cfg) * 0.92;
    tint(rgb, shade.clamp(0.05, 1.0))
}

pub(crate) fn ceiling_color(sec: &SectorSnap, depth: f32, cfg: &RenderConfig, sky_pic: u16) -> Color {
    if ceiling_is_sky(sec, sky_pic) {
        return SKY_BAND_BOT;
    }
    let rgb = cfg
        .atlas
        .as_ref()
        .and_then(|a| a.flat_color(sec.ceiling_pic))
        .unwrap_or_else(|| CEIL_PALETTE[(sec.ceiling_pic as usize) % CEIL_PALETTE.len()]);
    let shade = shade_for(sec.light_level, depth, cfg) * 0.85;
    tint(rgb, shade.clamp(0.05, 1.0))
}

/// Paleta minimal por tipo de sprite. spritenum_t de Doom shareware
/// (subset): SPR_TROO=imp marrón, SPR_POSS=zombi verdoso, SPR_BAR1=barril,
/// SPR_BKEY/RKEY/YKEY=llaves, SPR_BFUG/SHOT/PLAS=armas, SPR_TLMP=lámpara.
/// Como Fase 3.1 no tiene tabla de spritenum_t expandida, usamos
/// `sprite_idx % len` directo — los colores quedan estables por tipo
/// pero no correspondem a la semántica real hasta Fase 3.2.
pub(crate) const SPRITE_PALETTE: &[(u8, u8, u8)] = &[
    (0xB4, 0x5C, 0x3C), // imp red-brown
    (0x6C, 0x84, 0x4C), // zombi verde
    (0x88, 0x70, 0x54), // barril marrón
    (0xC4, 0xA8, 0x4C), // amarillo (llave / munición)
    (0x5C, 0x80, 0xB4), // azul (llave azul / plasma)
    (0xB4, 0x44, 0x44), // rojo (llave roja / sangre)
    (0xD4, 0xC0, 0x88), // hueso / cráneo
    (0xE0, 0xA8, 0x4C), // antorcha cálida
    (0x9C, 0x9C, 0xA8), // armadura plateada
    (0x44, 0x6C, 0x44), // verde oscuro
    (0xC4, 0x80, 0x40), // naranja
    (0xA0, 0xA0, 0xB4), // gris claro
];

pub(crate) fn sprite_color(
    sprite: &SpriteSnap,
    sec: Option<&SectorSnap>,
    depth: f32,
    cfg: &RenderConfig,
) -> Color {
    let rgb = SPRITE_PALETTE[(sprite.sprite as usize) % SPRITE_PALETTE.len()];
    let full_bright = (sprite.frame & 0x80) != 0;
    let shade = if full_bright {
        1.0
    } else {
        let light = sec.map(|s| s.light_level).unwrap_or(192);
        shade_for(light, depth, cfg)
    };
    tint(rgb, shade)
}

// =====================================================================
// Backdrop (cuando paredes no cubren)
// =====================================================================
