use super::*;

// --- Cielo de fondo: estrellas decorativas + Vía Láctea --------------

/// Polo norte galáctico (J2000): AR 192.859°, Dec +27.128° — constante
/// estándar IAU que fija el plano de la Vía Láctea.
pub(crate) const GAL_POLE_RA: f32 = 192.859;
pub(crate) const GAL_POLE_DEC: f32 = 27.128;
/// Centro galáctico (Sgr A*, J2000): AR 266.405°, Dec −28.936°. Hacia
/// ahí la Vía Láctea es más brillante.
pub(crate) const GAL_CENTER_RA: f32 = 266.405;
pub(crate) const GAL_CENTER_DEC: f32 = -28.936;

/// Hash entero → f32 en [0,1). Determinista (variante de splitmix32):
/// la misma entrada da siempre el mismo valor, así el campo de
/// estrellas no titila ni salta entre frames.
pub(crate) fn hash01(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    (x as f32) / (u32::MAX as f32)
}

/// Punto uniforme sobre la esfera unidad a partir de dos uniformes.
pub(crate) fn sphere_point(u1: f32, u2: f32) -> Vec3 {
    let z = 2.0 * u1 - 1.0;
    let rho = (1.0 - z * z).max(0.0).sqrt();
    let theta = std::f32::consts::TAU * u2;
    Vec3::new(rho * theta.cos(), rho * theta.sin(), z)
}

/// Vector unitario de una dirección ecuatorial (AR, Dec en grados).
pub(crate) fn equatorial_dir(ra_deg: f32, dec_deg: f32) -> Vec3 {
    let (sr, cr) = ra_deg.to_radians().sin_cos();
    let (sd, cd) = dec_deg.to_radians().sin_cos();
    Vec3::new(cd * cr, cd * sr, sd)
}

/// Empuja una estrella: un disco diminuto con brillo y un leve tinte
/// (azulado o cálido). Va detrás de la rejilla pero delante del
/// sombreado — un fondo de planetario.
pub(crate) fn push_star(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    size: f32,
    pos: Vec3,
    brightness: f32,
    tint: f32,
    dark: bool,
) {
    let p = proj.project(pos);
    let bright = brightness * brightness; // sesga hacia las tenues
    let r = size * (0.0011 + 0.0026 * bright);
    let alpha = (0.20 + 0.62 * bright) * depth_alpha(p.depth);
    // En modo claro las estrellas se pintan oscuras (negras) para ser
    // visibles sobre el fondo claro; en oscuro conservan su tinte.
    let col = if !dark {
        Rgba { r: 0.06, g: 0.08, b: 0.14, a: alpha }
    } else if tint < 0.22 {
        Rgba { r: 0.74, g: 0.81, b: 1.0, a: alpha }
    } else if tint > 0.86 {
        Rgba { r: 1.0, g: 0.86, b: 0.72, a: alpha }
    } else {
        Rgba { r: 0.95, g: 0.96, b: 1.0, a: alpha }
    };
    items.push((
        p.depth - 3.0,
        DrawCommand::Circle {
            cx: p.x,
            cy: p.y,
            r,
            stroke: None,
            fill: Some(col),
            stroke_w: 0.0,
        },
    ));
}

/// El cielo de fondo: un campo de estrellas isótropo —decorativo, no un
/// catálogo real— más una sobredensidad de estrellas tenues a lo largo
/// del plano galáctico, que dibuja la Vía Láctea. Ambos giran con la
/// esfera, así que delatan su rotación de un vistazo.
pub(crate) fn add_starfield(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    size: f32,
    eps: f32,
    dark: bool,
) {
    const FONDO: u32 = 210;
    for i in 0..FONDO {
        let pos = sphere_point(hash01(i * 3), hash01(i * 3 + 1));
        push_star(items, proj, size, pos, hash01(i * 3 + 2), hash01(i * 7 + 1), dark);
    }
    // Vía Láctea — el plano galáctico ubicado con el polo galáctico real.
    let gpole = rot_x(equatorial_dir(GAL_POLE_RA, GAL_POLE_DEC), eps);
    let geq = great_circle_perp(gpole, 256);
    const VIA: u32 = 240;
    for i in 0..VIA {
        let s = 9001 + i;
        let idx = (hash01(s * 5) * geq.len() as f32) as usize % geq.len();
        let on_eq = geq[idx];
        // Latitud galáctica pequeña, concentrada cerca de 0 — producto
        // de dos uniformes centrados → densa en el plano.
        let u = hash01(s * 5 + 1) - 0.5;
        let v = hash01(s * 5 + 2) - 0.5;
        let b = (u * v * 4.0 * 13.0).to_radians();
        let (sb, cb) = b.sin_cos();
        let pos = Vec3::new(
            on_eq.x * cb + gpole.x * sb,
            on_eq.y * cb + gpole.y * sb,
            on_eq.z * cb + gpole.z * sb,
        );
        push_star(items, proj, size, pos, hash01(s * 5 + 3) * 0.55, hash01(s * 5 + 4), dark);
    }
}

/// El resplandor difuso de la Vía Láctea — una luminosidad repartida a
/// lo largo del plano galáctico, no un brillo fijo a la pantalla. Gira
/// con la esfera. Es más intensa hacia el centro galáctico (en
/// Sagitario, como en el cielo real) y, si hay horizonte, se atenúa en
/// la parte que queda bajo tierra esa noche — la franja como se ve
/// desde la Tierra ese día.
pub(crate) fn add_milky_way_glow(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    eps: f32,
    size: f32,
    zenith: Option<Vec3>,
    dark: bool,
) {
    let gpole = rot_x(equatorial_dir(GAL_POLE_RA, GAL_POLE_DEC), eps);
    let gcenter = rot_x(equatorial_dir(GAL_CENTER_RA, GAL_CENTER_DEC), eps);
    let band = if dark {
        Rgba::opaque(0.78, 0.82, 0.96)
    } else {
        Rgba::opaque(0.20, 0.24, 0.34)
    };
    for p3 in great_circle_perp(gpole, 54) {
        // Más brillo hacia el centro galáctico.
        let toward = (p3.dot(gcenter) * 0.5 + 0.5).clamp(0.0, 1.0);
        let bright = 0.28 + 0.72 * toward * toward;
        // Atenuada bajo el horizonte local (no se ve esa noche).
        let vis = match zenith {
            Some(z) if p3.dot(z) < 0.0 => 0.40,
            _ => 1.0,
        };
        let p = proj.project(p3);
        items.push((
            p.depth - 4.0,
            DrawCommand::Circle {
                cx: p.x,
                cy: p.y,
                r: size * 0.045,
                stroke: None,
                fill: Some(band.with_alpha(0.030 * bright * vis * depth_alpha(p.depth))),
                stroke_w: 0.0,
            },
        ));
    }
}

// --- Estrellas fijas notables ----------------------------------------

/// Latitud eclíptica (grados, J2000) de las estrellas fijas notables
/// que emite el motor. La latitud apenas cambia con la precesión, así
/// que se fija aquí; la **longitud** —la coordenada astrológicamente
/// viva, que sí precesiona— la calcula el motor
/// (`build_fixed_stars_overlay`) y llega en el `Glyph`. Valores de
/// catálogo estándar, precisión ~0.5° (de sobra para el alambre).
pub(crate) fn fixed_star_latitude(name: &str) -> f32 {
    match name {
        "Regulus" => 0.47,
        "Spica" => -2.06,
        "Antares" => -4.57,
        "Aldebaran" => -5.47,
        "Pollux" => 6.68,
        "Algol" => 22.43,
        "Fomalhaut" => -21.14,
        "Sirius" => -39.61,
        "Vega" => 61.73,
        _ => 0.0,
    }
}

/// Dibuja una estrella fija: un disco brillante con destello de cuatro
/// rayos y su nombre.
pub(crate) fn add_fixed_star(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    pos: Vec3,
    size: f32,
    name: &str,
    pal: &Palette,
) {
    let p = proj.project(pos);
    let glow = Rgba::opaque(1.0, 0.96, 0.84);
    let c = dim(glow, p.depth);
    items.push((
        p.depth + 0.004,
        DrawCommand::Circle {
            cx: p.x,
            cy: p.y,
            r: size * 0.006,
            stroke: None,
            fill: Some(c),
            stroke_w: 0.0,
        },
    ));
    let ray = size * 0.018;
    let thin = c.with_alpha(c.a * 0.8);
    for (dx, dy) in [(ray, 0.0), (-ray, 0.0), (0.0, ray), (0.0, -ray)] {
        items.push((
            p.depth + 0.004,
            DrawCommand::Line {
                x1: p.x,
                y1: p.y,
                x2: p.x + dx,
                y2: p.y + dy,
                color: thin,
                width: 0.9,
                dash: None,
            },
        ));
    }
    let lp = proj.project(pos.scale(1.10));
    items.push((
        lp.depth + 0.005,
        DrawCommand::Text {
            x: lp.x,
            y: lp.y,
            content: name.into(),
            color: dim(pal.fg_text, lp.depth),
            size: size * 0.017,
            anchor: TextAnchor::Middle,
        },
    ));
}
