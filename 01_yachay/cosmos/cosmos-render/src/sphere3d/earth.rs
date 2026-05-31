use super::*;

// --- Tierra interior ------------------------------------------------

/// Contornos continentales **esquemáticos** (lat, lon en grados) — solo
/// referenciales, trazos muy gruesos para la Tierra interior. NO son un
/// mapa de precisión; dan el «ahí está tu continente» y nada más.
pub(crate) const CONTINENTES: &[&[(f32, f32)]] = &[
    // África
    &[
        (35.0, -6.0), (37.0, 10.0), (33.0, 22.0), (31.0, 32.0), (12.0, 43.0),
        (11.0, 51.0), (-4.0, 40.0), (-26.0, 33.0), (-34.0, 26.0), (-34.0, 19.0),
        (-18.0, 12.0), (0.0, 9.0), (5.0, -4.0), (11.0, -15.0), (21.0, -17.0),
        (28.0, -13.0),
    ],
    // Sudamérica
    &[
        (12.0, -72.0), (11.0, -61.0), (5.0, -52.0), (-5.0, -35.0), (-23.0, -43.0),
        (-34.0, -54.0), (-52.0, -69.0), (-55.0, -67.0), (-42.0, -74.0),
        (-18.0, -70.0), (-5.0, -81.0), (2.0, -79.0), (8.0, -77.0),
    ],
    // Norteamérica
    &[
        (70.0, -160.0), (71.0, -125.0), (68.0, -95.0), (63.0, -78.0),
        (47.0, -53.0), (45.0, -67.0), (30.0, -81.0), (25.0, -81.0),
        (20.0, -97.0), (23.0, -110.0), (34.0, -120.0), (48.0, -125.0),
        (60.0, -148.0),
    ],
    // Eurasia
    &[
        (36.0, -9.0), (43.0, -9.0), (58.0, 5.0), (71.0, 26.0), (73.0, 80.0),
        (73.0, 140.0), (66.0, 180.0), (53.0, 141.0), (40.0, 130.0), (30.0, 122.0),
        (22.0, 110.0), (9.0, 105.0), (8.0, 77.0), (21.0, 72.0), (25.0, 57.0),
        (13.0, 45.0), (30.0, 33.0), (41.0, 28.0), (38.0, 15.0), (40.0, 0.0),
    ],
    // Australia
    &[
        (-11.0, 131.0), (-12.0, 142.0), (-25.0, 153.0), (-38.0, 147.0),
        (-35.0, 138.0), (-32.0, 116.0), (-22.0, 114.0), (-14.0, 127.0),
    ],
    // Antártida (casquete polar aproximado)
    &[
        (-72.0, -180.0), (-70.0, -120.0), (-73.0, -60.0), (-70.0, 0.0),
        (-73.0, 60.0), (-70.0, 120.0), (-72.0, 170.0),
    ],
];

/// Dirección (marco eclíptico, unitaria) de un punto geográfico. La
/// longitud del observador y el RAMC fijan la fase de rotación de la
/// Tierra: el observador está en AR = RAMC, así que cualquier otra
/// longitud geográfica `lon` está en AR = RAMC + (lon − lon_obs).
pub(crate) fn geo_to_ecliptic(lat: f32, lon: f32, lon_obs: f32, ramc: f32, eps_rad: f32) -> Vec3 {
    let ra = (ramc + lon - lon_obs).to_radians();
    let dec = lat.to_radians();
    let (sra, cra) = ra.sin_cos();
    let (sd, cd) = dec.sin_cos();
    rot_x(Vec3::new(cd * cra, cd * sra, sd), eps_rad)
}

/// La Tierra interior: un globo pequeño en el centro de la esfera
/// celeste, con el **mar** y los **continentes** teñidos distinto, un
/// **sombreado día/noche** según la posición del Sol, y el observador
/// marcado en su lugar real. Orientada de modo que el punto geográfico
/// del observador mira al cénit — y gira con la vista.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_inner_earth(
    items: &mut Vec<(f32, DrawCommand)>,
    proj: &Projector,
    model: &RenderModel,
    eps: f32,
    size: f32,
    center: f32,
    rad: f32,
    pal: &Palette,
) {
    const R_EARTH: f32 = 0.26;
    let ramc = ramc_deg(model.midheaven_deg, eps);
    let lon_obs = model.geo_longitude_deg;
    // Dirección unitaria de un punto geográfico (sin escalar).
    let dir = |lat: f32, lon: f32| geo_to_ecliptic(lat, lon, lon_obs, ramc, eps);
    // El mismo punto, escalado al radio de la Tierra interior.
    let geo = |lat: f32, lon: f32| dir(lat, lon).scale(R_EARTH);

    // El Sol de la carta (si está) — para el día/noche. El lado de la
    // Tierra que mira al Sol es el día: un punto `d` está de día si
    // `d · sol > 0`.
    let sun_dir: Option<Vec3> = model
        .layers
        .iter()
        .filter(|l| matches!(l.kind, LayerKind::Bodies) && l.module_id == "natal")
        .flat_map(|l| l.glyphs.iter())
        .find(|g| g.symbol == "sun")
        .map(|g| eclip(g.deg));
    let es_dia = |d: Vec3| -> bool { sun_dir.map(|s| d.dot(s) > 0.0).unwrap_or(true) };

    // Mar — disco base teñido de azul.
    let sea = if pal.is_dark {
        Rgba::opaque(0.10, 0.21, 0.39)
    } else {
        Rgba::opaque(0.58, 0.72, 0.86)
    };
    items.push((
        -0.95,
        DrawCommand::Circle {
            cx: center,
            cy: center,
            r: R_EARTH * rad,
            stroke: Some(pal.fg_muted.with_alpha(0.30)),
            fill: Some(sea.with_alpha(0.55)),
            stroke_w: 0.8,
        },
    ));

    // Resplandor diurno — el hemisferio iluminado. Discos concéntricos
    // sobre el punto subsolar; se apagan si el Sol queda detrás de la
    // Tierra (entonces vemos su cara nocturna).
    if let Some(s) = sun_dir {
        let sub = proj.project(s.scale(R_EARTH));
        let face = ((sub.depth / R_EARTH) * 0.5 + 0.5).clamp(0.0, 1.0);
        let day = if pal.is_dark {
            Rgba::opaque(0.40, 0.60, 0.85)
        } else {
            Rgba::opaque(1.0, 0.98, 0.88)
        };
        for i in 0..10 {
            let t = i as f32 / 9.0;
            items.push((
                -0.93 + t * 0.04,
                DrawCommand::Circle {
                    cx: sub.x,
                    cy: sub.y,
                    r: R_EARTH * rad * (1.0 - 0.92 * t),
                    stroke: None,
                    fill: Some(day.with_alpha(0.07 * face)),
                    stroke_w: 0.0,
                },
            ));
        }
    }

    // Ecuador terrestre.
    let equator: Vec<Vec3> = (0..72)
        .map(|i| geo(0.0, (i as f32) / 72.0 * 360.0))
        .collect();
    add_loop(items, proj, &equator, pal.fg_muted.with_alpha(0.20), 0.5);

    // Terminador — la línea día/noche, círculo máximo ⊥ al Sol.
    if let Some(s) = sun_dir {
        let term: Vec<Vec3> = great_circle_perp(s, 72)
            .iter()
            .map(|p| p.scale(R_EARTH))
            .collect();
        add_loop(items, proj, &term, pal.angle_highlight.with_alpha(0.45), 0.7);
    }

    // Continentes — polígonos rellenos, teñidos de verde; el tono
    // depende de si la masa está de día o de noche.
    let land_day = if pal.is_dark {
        Rgba::opaque(0.38, 0.60, 0.34)
    } else {
        Rgba::opaque(0.52, 0.66, 0.40)
    };
    let land_night = if pal.is_dark {
        Rgba::opaque(0.13, 0.25, 0.19)
    } else {
        Rgba::opaque(0.40, 0.50, 0.40)
    };
    for outline in CONTINENTES {
        let pts3: Vec<Vec3> = outline.iter().map(|&(lat, lon)| geo(lat, lon)).collect();
        let mut cen = Vec3::new(0.0, 0.0, 0.0);
        let mut depth_sum = 0.0_f32;
        let pts2: Vec<(f32, f32)> = pts3
            .iter()
            .map(|v| {
                cen = Vec3::new(cen.x + v.x, cen.y + v.y, cen.z + v.z);
                let p = proj.project(*v);
                depth_sum += p.depth;
                (p.x, p.y)
            })
            .collect();
        let n = pts3.len().max(1) as f32;
        let depth = depth_sum / n;
        let base = if es_dia(cen.scale(1.0 / n)) {
            land_day
        } else {
            land_night
        };
        items.push((
            depth,
            DrawCommand::Polygon {
                points: pts2,
                fill: Some(dim(base, depth).with_alpha(0.62 * depth_alpha(depth))),
                stroke: Some(dim(base, depth)),
                stroke_w: 0.7,
            },
        ));
    }

    // El observador, en su lugar real sobre la Tierra.
    let obs_dir = dir(model.geo_latitude_deg, lon_obs);
    let p = proj.project(obs_dir.scale(R_EARTH));
    let oc = dim(pal.sun, p.depth);
    items.push((
        p.depth + 0.01,
        DrawCommand::Circle {
            cx: p.x,
            cy: p.y,
            r: size * 0.0075,
            stroke: Some(oc),
            fill: Some(oc.with_alpha(oc.a * if es_dia(obs_dir) { 0.6 } else { 0.15 })),
            stroke_w: 1.2,
        },
    ));
}
