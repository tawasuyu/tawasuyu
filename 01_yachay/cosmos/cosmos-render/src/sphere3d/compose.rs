use super::*;

// =====================================================================
// Composición
// =====================================================================

/// Compone la esfera celeste como una lista de [`DrawCommand`]s, ya
/// ordenada de atrás hacia adelante (algoritmo del pintor). El canvas
/// nativo y el cliente web la consumen igual que la rueda 2D.
pub fn compose_sphere(
    model: &RenderModel,
    view: &SphereView,
    opts: &SphereOpts,
) -> Vec<DrawCommand> {
    let pal = &opts.palette;
    let size = opts.size;
    let center = size * 0.5;
    let rad = size * 0.36;
    let proj = Projector::new(view, center, center, rad);
    let eps = opts.obliquity_deg.to_radians();
    // El cénit del observador — disponible cuando se pide el horizonte.
    // Lo usan tanto la sección del horizonte como el día/noche de los
    // cuerpos.
    let zenith = if opts.show_horizon {
        Some(zenith_ecliptic(model.geo_latitude_deg, model.midheaven_deg, eps))
    } else {
        None
    };

    // (profundidad, comando) — se ordena al final.
    let mut items: Vec<(f32, DrawCommand)> = Vec::new();

    // --- Cuerpo de la esfera: sombreado con volumen ---
    add_sphere_shading(&mut items, pal, center, rad);

    // --- Cielo de fondo: Vía Láctea + estrellas. En modo claro se pintan
    // oscuras (negras) para verse sobre el fondo claro. ---
    if opts.show_sky {
        add_milky_way_glow(&mut items, &proj, eps, size, zenith, pal.is_dark);
        add_starfield(&mut items, &proj, size, eps, pal.is_dark);
    }

    // --- Figuras de las constelaciones ---
    if opts.show_constellations {
        add_constellations(&mut items, &proj, eps, size, pal);
    }

    // --- Rejilla: meridianos + paralelos de la eclíptica ---
    if opts.show_grid {
        let grid = pal.fg_muted.with_alpha(0.16);
        for k in 0..6 {
            add_loop(&mut items, &proj, &meridian_points((k as f32) * 30.0, 64), grid, 0.5);
        }
        for &beta in &[-60.0_f32, -30.0, 30.0, 60.0] {
            add_loop(&mut items, &proj, &parallel_points(beta, 64), grid, 0.5);
        }
    }

    // --- Ecuador celeste + eje de la Tierra ---
    if opts.show_equator {
        let equator: Vec<Vec3> = ring_points(96).iter().map(|p| rot_x(*p, eps)).collect();
        add_loop(&mut items, &proj, &equator, pal.uranus.with_alpha(0.85), 1.3);
        let n = proj.project(rot_x(Vec3::new(0.0, 0.0, 1.0), eps));
        let s = proj.project(rot_x(Vec3::new(0.0, 0.0, -1.0), eps));
        items.push((
            (n.depth + s.depth) * 0.5,
            DrawCommand::Line {
                x1: s.x,
                y1: s.y,
                x2: n.x,
                y2: n.y,
                color: pal.uranus.with_alpha(0.45),
                width: 0.8,
                dash: Some((4.0, 4.0)),
            },
        ));
    }

    // --- Eclíptica: el camino del zodíaco, el aro prominente ---
    add_loop(&mut items, &proj, &ring_points(96), pal.dial_ring, 2.0);
    {
        // Eje polar de la eclíptica, tenue.
        let n = proj.project(Vec3::new(0.0, 0.0, 1.0));
        let s = proj.project(Vec3::new(0.0, 0.0, -1.0));
        items.push((
            (n.depth + s.depth) * 0.5,
            DrawCommand::Line {
                x1: s.x,
                y1: s.y,
                x2: n.x,
                y2: n.y,
                color: pal.fg_muted.with_alpha(0.30),
                width: 0.6,
                dash: None,
            },
        ));
    }

    // --- Polos: eclípticos (punto dorado) y celestes (anillo + cruz) ---
    for z in [1.0_f32, -1.0] {
        let p = proj.project(Vec3::new(0.0, 0.0, z));
        items.push((
            p.depth + 0.001,
            DrawCommand::Circle {
                cx: p.x,
                cy: p.y,
                r: size * 0.009,
                stroke: None,
                fill: Some(dim(pal.dial_ring, p.depth)),
                stroke_w: 0.0,
            },
        ));
    }
    for (z, label) in [(1.0_f32, "PN"), (-1.0, "PS")] {
        let pole = rot_x(Vec3::new(0.0, 0.0, z), eps);
        let p = proj.project(pole);
        let col = dim(pal.uranus, p.depth);
        let arm = size * 0.013;
        items.push((
            p.depth + 0.001,
            DrawCommand::Circle {
                cx: p.x,
                cy: p.y,
                r: size * 0.012,
                stroke: Some(col),
                fill: None,
                stroke_w: 1.2,
            },
        ));
        items.push((
            p.depth + 0.001,
            DrawCommand::Line {
                x1: p.x - arm,
                y1: p.y,
                x2: p.x + arm,
                y2: p.y,
                color: col,
                width: 1.0,
                dash: None,
            },
        ));
        items.push((
            p.depth + 0.001,
            DrawCommand::Line {
                x1: p.x,
                y1: p.y - arm,
                x2: p.x,
                y2: p.y + arm,
                color: col,
                width: 1.0,
                dash: None,
            },
        ));
        let lp = proj.project(pole.scale(1.13));
        items.push((
            lp.depth + 0.002,
            DrawCommand::Text {
                x: lp.x,
                y: lp.y,
                content: label.into(),
                color: dim(pal.uranus, lp.depth),
                size: size * 0.018,
                anchor: TextAnchor::Middle,
            },
        ));
    }

    // --- Horizonte local, cénit del observador y meridiano ---
    if let Some(z) = zenith {
        let horiz_color = if pal.is_dark {
            Rgba::opaque(0.90, 0.58, 0.32)
        } else {
            Rgba::opaque(0.66, 0.38, 0.14)
        };
        add_loop(
            &mut items,
            &proj,
            &great_circle_perp(z, 96),
            horiz_color.with_alpha(0.90),
            1.7,
        );
        // El meridiano local: círculo máximo por el cénit y el polo
        // celeste — su normal es `z × NCP`.
        let ncp = rot_x(Vec3::new(0.0, 0.0, 1.0), eps);
        add_loop(
            &mut items,
            &proj,
            &great_circle_perp(z.cross(ncp), 96),
            pal.fg_muted.with_alpha(0.28),
            0.7,
        );
        // Cénit — el punto geográfico del observador — y nadir.
        add_point_marker(&mut items, &proj, z, pal.sun, size, "Cénit", true);
        add_point_marker(
            &mut items,
            &proj,
            z.scale(-1.0),
            pal.fg_muted,
            size,
            "Nadir",
            false,
        );
    }

    // --- Signos: espolón en cada borde + glifo en el centro ---
    if opts.show_signs {
        for i in 0..12 {
            let boundary = (i as f32) * 30.0;
            let a = proj.project(eclip(boundary));
            let b = proj.project(eclip(boundary).scale(1.09));
            let d = (a.depth + b.depth) * 0.5;
            items.push((
                d,
                DrawCommand::Line {
                    x1: a.x,
                    y1: a.y,
                    x2: b.x,
                    y2: b.y,
                    color: dim(pal.dial_ring, d),
                    width: 1.0,
                    dash: None,
                },
            ));
            let mid = boundary + 15.0;
            let g = proj.project(eclip(mid).scale(1.17));
            let name = SIGN_NAMES[i];
            items.push((
                g.depth + 0.002,
                DrawCommand::Text {
                    x: g.x,
                    y: g.y,
                    content: sign_unicode(name).into(),
                    color: dim(pal.sign(name), g.depth),
                    size: size * 0.030,
                    anchor: TextAnchor::Middle,
                },
            ));
        }
    }

    // --- Ángulos ASC / MC / DSC / IC ---
    for (deg, label) in [
        (model.ascendant_deg, "Asc"),
        (model.midheaven_deg, "MC"),
        (model.descendant_deg, "Dsc"),
        (model.imum_coeli_deg, "IC"),
    ] {
        let a = proj.project(eclip(deg));
        let b = proj.project(eclip(deg).scale(1.14));
        let d = (a.depth + b.depth) * 0.5;
        items.push((
            d,
            DrawCommand::Line {
                x1: a.x,
                y1: a.y,
                x2: b.x,
                y2: b.y,
                color: dim(pal.angle_highlight, d),
                width: 1.6,
                dash: None,
            },
        ));
        let lbl = proj.project(eclip(deg).scale(1.30));
        items.push((
            lbl.depth + 0.002,
            DrawCommand::Text {
                x: lbl.x,
                y: lbl.y,
                content: label.into(),
                color: dim(pal.angle_highlight, lbl.depth),
                size: size * 0.021,
                anchor: TextAnchor::Middle,
            },
        ));
    }

    // --- Cuerpos: natales (disco lleno) y topocéntricos (disco hueco
    //     + conector a su par geocéntrico) ---
    if opts.show_bodies {
        let halo = if pal.is_dark {
            pal.bg_panel.with_alpha(0.92)
        } else {
            Rgba::opaque(1.0, 1.0, 1.0).with_alpha(0.92)
        };
        // 1) Cuerpos natales (geocéntricos). Se recuerdan sus posiciones
        //    para poder tender el conector hacia los topocéntricos.
        let mut natal_pos: Vec<(String, Vec3)> = Vec::new();
        for layer in &model.layers {
            if !matches!(layer.kind, LayerKind::Bodies) || layer.module_id != "natal" {
                continue;
            }
            for g in &layer.glyphs {
                let pos = eclip(g.deg);
                natal_pos.push((g.symbol.clone(), pos));
                let p = proj.project(pos);
                let mut color = pal.planet(&g.symbol);
                // Día/noche: un cuerpo bajo el horizonte se atenúa — de
                // un vistazo se ve qué planetas estaban sobre la tierra
                // en el momento de la carta.
                if let Some(z) = zenith {
                    if pos.dot(z) < 0.0 {
                        color = color.with_alpha(color.a * 0.40);
                    }
                }
                items.push((
                    p.depth,
                    DrawCommand::Circle {
                        cx: p.x,
                        cy: p.y,
                        r: size * 0.020,
                        stroke: Some(dim(color, p.depth)),
                        fill: Some(halo),
                        stroke_w: 1.3,
                    },
                ));
                items.push((
                    p.depth + 0.003,
                    DrawCommand::Text {
                        x: p.x,
                        y: p.y,
                        content: planet_unicode_with_retro(&g.symbol, g.retrograde),
                        color: dim(color, p.depth),
                        size: size * 0.026,
                        anchor: TextAnchor::Middle,
                    },
                ));
            }
        }
        // 2) Cuerpos topocéntricos — si la capa está activa. Disco hueco
        //    (sin relleno, lo distingue del natal) + un conector hasta
        //    su par geocéntrico: el LARGO del conector es la paralaje,
        //    así no se miente sobre su magnitud (un cinturón aparte la
        //    exageraría — la diferencia es sub-grado salvo la Luna).
        for layer in &model.layers {
            if !matches!(layer.kind, LayerKind::Bodies) || layer.module_id != "topocentric" {
                continue;
            }
            for g in &layer.glyphs {
                let pos = eclip(g.deg);
                let p = proj.project(pos);
                let color = dim(pal.planet(&g.symbol), p.depth);
                if let Some((_, npos)) = natal_pos.iter().find(|(s, _)| s == &g.symbol) {
                    let np = proj.project(*npos);
                    items.push((
                        p.depth - 0.001,
                        DrawCommand::Line {
                            x1: np.x,
                            y1: np.y,
                            x2: p.x,
                            y2: p.y,
                            color: color.with_alpha(color.a * 0.70),
                            width: 1.0,
                            dash: None,
                        },
                    ));
                }
                items.push((
                    p.depth + 0.002,
                    DrawCommand::Circle {
                        cx: p.x,
                        cy: p.y,
                        r: size * 0.014,
                        stroke: Some(color),
                        fill: None,
                        stroke_w: 1.3,
                    },
                ));
            }
        }
    }

    // --- Estrellas fijas notables (capa del motor, si está activa) ---
    // El motor emite la capa `FixedStars` con la longitud eclíptica ya
    // precesionada; aquí se le suma la latitud para situarla en su
    // lugar real de la esfera, no aplastada sobre la eclíptica.
    for layer in &model.layers {
        if !matches!(layer.kind, LayerKind::FixedStars) {
            continue;
        }
        for g in &layer.glyphs {
            let name = g.annotation.as_deref().unwrap_or("");
            let pos = eclip_latlon(g.deg, fixed_star_latitude(name));
            add_fixed_star(&mut items, &proj, pos, size, name, pal);
        }
    }

    // --- Tierra interior: globo esquemático con el observador ---
    if opts.show_earth {
        add_inner_earth(&mut items, &proj, model, eps, size, center, rad, pal);
    }

    // Algoritmo del pintor: de la profundidad menor (fondo) a la mayor.
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
    items.into_iter().map(|(_, cmd)| cmd).collect()
}

