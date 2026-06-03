use super::*;

pub(crate) fn add_sphere_shading(
    items: &mut Vec<(f32, DrawCommand)>,
    pal: &Palette,
    center: f32,
    rad: f32,
) {
    let (base, glow, highlight) = if pal.is_dark {
        (
            Rgba::opaque(0.12, 0.14, 0.24),
            Rgba::opaque(0.34, 0.40, 0.60),
            Rgba::opaque(0.62, 0.68, 0.88),
        )
    } else {
        (
            Rgba::opaque(0.82, 0.86, 0.93),
            Rgba::opaque(1.0, 1.0, 1.0),
            Rgba::opaque(1.0, 1.0, 1.0),
        )
    };
    // Disco base — uniforme, le da cuerpo sólido a la esfera.
    items.push((
        -99.0,
        DrawCommand::Circle {
            cx: center,
            cy: center,
            r: rad,
            stroke: None,
            fill: Some(base.with_alpha(0.55)),
            stroke_w: 0.0,
        },
    ));
    // Degradado: anillos concéntricos que se acumulan hacia el centro.
    const GLOW: usize = 12;
    for i in 0..GLOW {
        let t = i as f32 / (GLOW - 1) as f32;
        items.push((
            -98.0 + t * 1.5,
            DrawCommand::Circle {
                cx: center,
                cy: center,
                r: rad * (0.95 - 0.95 * t),
                stroke: None,
                fill: Some(glow.with_alpha(0.028)),
                stroke_w: 0.0,
            },
        ));
    }
    // Brillo especular desplazado hacia la luz — tenue: la luminosidad
    // viva la reparte la Vía Láctea, que sí gira con la esfera.
    let hx = center - rad * 0.34;
    let hy = center - rad * 0.34;
    const HALO: usize = 6;
    for i in 0..HALO {
        let t = i as f32 / (HALO - 1) as f32;
        items.push((
            -95.0 + t * 0.5,
            DrawCommand::Circle {
                cx: hx,
                cy: hy,
                r: rad * 0.5 * (1.0 - t),
                stroke: None,
                fill: Some(highlight.with_alpha(0.018)),
                stroke_w: 0.0,
            },
        ));
    }
    // Contorno nítido del limbo, encima del sombreado.
    items.push((
        -94.0,
        DrawCommand::Circle {
            cx: center,
            cy: center,
            r: rad,
            stroke: Some(pal.fg_muted.with_alpha(0.32)),
            fill: None,
            stroke_w: 1.0,
        },
    ));
}

/// Proyecta una polilínea cerrada y empuja un `Line` por segmento, con
/// la profundidad como clave de orden y la atenuación ya aplicada.
pub(crate) fn add_loop(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    pts: &[Vec3],
    color: Rgba,
    width: f32,
) {
    let n = pts.len();
    for i in 0..n {
        let a = proj.project(pts[i]);
        let b = proj.project(pts[(i + 1) % n]);
        let d = (a.depth + b.depth) * 0.5;
        items.push((
            d,
            DrawCommand::Line {
                x1: a.x,
                y1: a.y,
                x2: b.x,
                y2: b.y,
                color: dim(color, d),
                width,
                dash: None,
            },
        ));
    }
}

/// Proyecta una polilínea ABIERTA y empuja un `Line` por segmento.
pub(crate) fn add_path(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    pts: &[Vec3],
    color: Rgba,
    width: f32,
) {
    for i in 0..pts.len().saturating_sub(1) {
        let a = proj.project(pts[i]);
        let b = proj.project(pts[i + 1]);
        let d = (a.depth + b.depth) * 0.5;
        items.push((
            d,
            DrawCommand::Line {
                x1: a.x,
                y1: a.y,
                x2: b.x,
                y2: b.y,
                color: dim(color, d),
                width,
                dash: None,
            },
        ));
    }
}

/// Dibuja las figuras de las 88 constelaciones: cada trazo une estrellas
/// reales del catálogo (un punto por vértice), y el nombre va en el
/// centroide. Capa tenue — referencia, no protagonista.
pub(crate) fn add_constellations(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    eps: f32,
    size: f32,
    pal: &Palette,
) {
    let line_col = pal.fg_muted.with_alpha(0.42);
    // Estrellas de las figuras: claras en tema oscuro, oscuras (casi
    // negras) en tema claro para verse sobre el fondo.
    let star = if pal.is_dark {
        Rgba::opaque(0.92, 0.95, 1.0)
    } else {
        Rgba::opaque(0.08, 0.10, 0.16)
    };
    for fig in crate::constellations_data::FIGURAS {
        let (mut sx, mut sy, mut sz, mut n) = (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
        for path in fig.paths {
            let pts: Vec<Vec3> = path
                .iter()
                .map(|&(ra, dec)| rot_x(equatorial_dir(ra, dec), eps))
                .collect();
            add_path(items, proj, &pts, line_col, 0.7);
            for v in &pts {
                sx += v.x;
                sy += v.y;
                sz += v.z;
                n += 1.0;
                let p = proj.project(*v);
                items.push((
                    p.depth - 0.01,
                    DrawCommand::Circle {
                        cx: p.x,
                        cy: p.y,
                        r: size * 0.0017,
                        stroke: None,
                        fill: Some(star.with_alpha(0.70 * depth_alpha(p.depth))),
                        stroke_w: 0.0,
                    },
                ));
            }
        }
        if n > 0.0 {
            let c = Vec3::new(sx / n, sy / n, sz / n).normalized();
            let lp = proj.project(c);
            items.push((
                lp.depth + 0.001,
                DrawCommand::Text {
                    x: lp.x,
                    y: lp.y,
                    content: fig.nombre.into(),
                    color: pal.fg_muted.with_alpha(0.42 * depth_alpha(lp.depth)),
                    size: size * 0.0135,
                    anchor: TextAnchor::Middle,
                },
            ));
        }
    }
}

/// Los `n` puntos de un círculo máximo perpendicular a `normal`.
pub(crate) fn great_circle_perp(normal: Vec3, n: usize) -> Vec<Vec3> {
    let z = normal.normalized();
    // Una referencia que no sea casi-paralela a `z`.
    let r = if z.z.abs() < 0.9 {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let u = z.cross(r).normalized();
    let v = z.cross(u);
    (0..n)
        .map(|i| {
            let t = (i as f32) / (n as f32) * std::f32::consts::TAU;
            let (s, c) = t.sin_cos();
            Vec3::new(u.x * c + v.x * s, u.y * c + v.y * s, u.z * c + v.z * s)
        })
        .collect()
}

/// RAMC — ascensión recta del Medio Cielo, en grados: la AR del punto
/// eclíptico del MC (latitud eclíptica 0).
pub(crate) fn ramc_deg(mc_deg: f32, eps_rad: f32) -> f32 {
    let lmc = mc_deg.to_radians();
    (lmc.sin() * eps_rad.cos())
        .atan2(lmc.cos())
        .to_degrees()
}

/// El cénit del observador en el marco eclíptico — el punto del cielo
/// justo sobre su cabeza. Tiene declinación `φ` (la latitud geográfica)
/// y AR `RAMC`, y eso se lleva del marco ecuatorial al eclíptico
/// rotando por la oblicuidad.
pub(crate) fn zenith_ecliptic(lat_deg: f32, mc_deg: f32, eps_rad: f32) -> Vec3 {
    let phi = lat_deg.to_radians();
    let ramc = ramc_deg(mc_deg, eps_rad).to_radians();
    let (sphi, cphi) = phi.sin_cos();
    let (sr, cr) = ramc.sin_cos();
    rot_x(Vec3::new(cphi * cr, cphi * sr, sphi), eps_rad)
}

/// Marca un punto notable de la esfera: disco + etiqueta, y un anillo
/// extra si es `prominent`.
pub(crate) fn add_point_marker(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    pos: Vec3,
    color: Rgba,
    size: f32,
    label: &str,
    prominent: bool,
) {
    let p = proj.project(pos);
    let c = dim(color, p.depth);
    let r = if prominent { size * 0.013 } else { size * 0.008 };
    items.push((
        p.depth + 0.001,
        DrawCommand::Circle {
            cx: p.x,
            cy: p.y,
            r,
            stroke: Some(c),
            fill: Some(c.with_alpha(c.a * 0.40)),
            stroke_w: 1.4,
        },
    ));
    if prominent {
        items.push((
            p.depth + 0.001,
            DrawCommand::Circle {
                cx: p.x,
                cy: p.y,
                r: r * 1.95,
                stroke: Some(c.with_alpha(c.a * 0.55)),
                fill: None,
                stroke_w: 1.0,
            },
        ));
    }
    let lp = proj.project(pos.scale(1.13));
    items.push((
        lp.depth + 0.002,
        DrawCommand::Text {
            x: lp.x,
            y: lp.y,
            content: label.into(),
            color: dim(color, lp.depth),
            size: size * 0.019,
            anchor: TextAnchor::Middle,
        },
    ));
}
