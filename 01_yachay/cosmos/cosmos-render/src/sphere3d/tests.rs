    use super::*;
    use crate::{ChartId, ChartKind, Geometry, Glyph, Layer};

    #[test]
    fn vernal_point_y_cuadratura_sobre_la_eclyptica() {
        let v = eclip(0.0);
        assert!((v.x - 1.0).abs() < 1e-5 && v.y.abs() < 1e-5 && v.z.abs() < 1e-5);
        let q = eclip(90.0);
        assert!(q.x.abs() < 1e-5 && (q.y - 1.0).abs() < 1e-5 && q.z.abs() < 1e-5);
    }

    #[test]
    fn la_oblicuidad_inclina_el_polo_celeste() {
        // El polo norte celeste = polo eclíptico rotado por ε. El
        // ángulo entre ambos debe ser exactamente ε.
        let ncp = rot_x(Vec3::new(0.0, 0.0, 1.0), OBLICUIDAD_DEG.to_radians());
        let cos_ang = ncp.z; // producto punto con (0,0,1).
        let ang = cos_ang.acos().to_degrees();
        assert!((ang - OBLICUIDAD_DEG).abs() < 1e-3, "ángulo {ang}");
    }

    #[test]
    fn la_proyeccion_no_se_sale_del_cuadro() {
        let view = SphereView::default();
        let proj = Projector::new(&view, 300.0, 300.0, 108.0);
        for i in 0..360 {
            let p = proj.project(eclip(i as f32));
            assert!(p.x >= 300.0 - 109.0 && p.x <= 300.0 + 109.0);
            assert!(p.y >= 300.0 - 109.0 && p.y <= 300.0 + 109.0);
        }
    }

    fn modelo_demo() -> RenderModel {
        RenderModel {
            chart_id: ChartId::default(),
            chart_kind: ChartKind::Natal,
            title: "demo".into(),
            subtitle: None,
            compute_ms: 0,
            ascendant_deg: 100.0,
            midheaven_deg: 10.0,
            descendant_deg: 280.0,
            imum_coeli_deg: 190.0,
            geo_latitude_deg: -34.6,
            geo_longitude_deg: -58.4,
            layers: vec![Layer {
                module_id: "natal".into(),
                kind: LayerKind::Bodies,
                ring: 0.0,
                z: 0,
                geometry: Geometry::GlyphsOnly,
                glyphs: vec![
                    Glyph { deg: 12.0, symbol: "sun".into(), ..Default::default() },
                    Glyph { deg: 200.0, symbol: "moon".into(), ..Default::default() },
                ],
            }],
            overlays: vec![],
            aspect_summary: vec![],
            uranian_groups: vec![],
            gr_triggers: vec![],
            harmonic: 1,
            harmonic_spectrum: vec![],
        }
    }

    #[test]
    fn compose_sphere_emite_esqueleto_y_cuerpos() {
        // Sin constelaciones, para contar solo el esqueleto base.
        let cmds = compose_sphere(
            &modelo_demo(),
            &SphereView::default(),
            &SphereOpts { show_constellations: false, ..Default::default() },
        );
        assert!(!cmds.is_empty(), "la esfera produce comandos");
        let lineas = cmds.iter().filter(|c| matches!(c, DrawCommand::Line { .. })).count();
        let textos = cmds.iter().filter(|c| matches!(c, DrawCommand::Text { .. })).count();
        assert!(lineas > 100, "círculos máximos como polilíneas: {lineas}");
        // 12 signos + 4 ángulos + 2 polos celestes + cénit + nadir + 2
        // cuerpos = 22 etiquetas de texto.
        assert_eq!(textos, 22, "glifos de signos, ángulos, polos y cuerpos: {textos}");
    }

    #[test]
    fn las_constelaciones_dibujan_sus_figuras() {
        assert!(
            crate::constellations_data::FIGURAS.len() > 80,
            "el catálogo trae las 88 constelaciones"
        );
        let modelo = modelo_demo();
        let lineas = |c: &[DrawCommand]| {
            c.iter().filter(|d| matches!(d, DrawCommand::Line { .. })).count()
        };
        let con = compose_sphere(&modelo, &SphereView::default(), &SphereOpts::default());
        let sin = compose_sphere(
            &modelo,
            &SphereView::default(),
            &SphereOpts { show_constellations: false, ..Default::default() },
        );
        assert!(
            lineas(&con) > lineas(&sin) + 500,
            "las figuras agregan cientos de trazos: {} vs {}",
            lineas(&con),
            lineas(&sin),
        );
    }

    #[test]
    fn el_cenit_esta_a_la_colatitud_del_polo_celeste() {
        let eps = OBLICUIDAD_DEG.to_radians();
        for &(lat, mc) in &[(-34.6_f32, 10.0_f32), (40.0, 200.0), (0.0, 95.0), (60.0, 300.0)] {
            let z = zenith_ecliptic(lat, mc, eps);
            let ncp = rot_x(Vec3::new(0.0, 0.0, 1.0), eps);
            // El ángulo cénit↔polo celeste es la colatitud (90°−φ): su
            // coseno —el producto punto de dos unitarios— es sin φ.
            assert!(
                (z.dot(ncp) - lat.to_radians().sin()).abs() < 1e-4,
                "lat {lat}: z·NCP = {} vs sin φ = {}",
                z.dot(ncp),
                lat.to_radians().sin(),
            );
        }
    }

    #[test]
    fn el_cielo_dibuja_un_campo_de_estrellas() {
        let modelo = modelo_demo();
        let con = compose_sphere(
            &modelo,
            &SphereView::default(),
            &SphereOpts { show_sky: true, ..Default::default() },
        );
        let sin = compose_sphere(
            &modelo,
            &SphereView::default(),
            &SphereOpts { show_sky: false, ..Default::default() },
        );
        let discos = |c: &[DrawCommand]| {
            c.iter().filter(|d| matches!(d, DrawCommand::Circle { .. })).count()
        };
        assert!(
            discos(&con) > discos(&sin) + 300,
            "el cielo agrega cientos de estrellas: {} vs {}",
            discos(&con),
            discos(&sin),
        );
    }

    #[test]
    fn eclip_latlon_respeta_la_latitud() {
        let sobre = eclip_latlon(123.0, 0.0);
        assert!(sobre.z.abs() < 1e-5, "latitud 0 → sobre la eclíptica");
        let polo = eclip_latlon(45.0, 90.0);
        assert!((polo.z - 1.0).abs() < 1e-5, "latitud 90 → polo eclíptico");
        let sirio = eclip_latlon(200.0, -39.61);
        assert!((sirio.z - (-39.61_f32).to_radians().sin()).abs() < 1e-5);
    }

    #[test]
    fn las_latitudes_de_estrellas_fijas_son_coherentes() {
        // Sirio es la más austral; Vega la más boreal; Régulo casi
        // sobre la eclíptica; una desconocida cae a latitud 0.
        assert!(fixed_star_latitude("Sirius") < -30.0);
        assert!(fixed_star_latitude("Vega") > 55.0);
        assert!(fixed_star_latitude("Regulus").abs() < 1.0);
        assert_eq!(fixed_star_latitude("Inexistente"), 0.0);
    }

    #[test]
    fn compose_sphere_dibuja_las_estrellas_fijas_de_la_capa() {
        let mut modelo = modelo_demo();
        modelo.layers.push(Layer {
            module_id: "fixed_stars".into(),
            kind: LayerKind::FixedStars,
            ring: 1.04,
            z: 16,
            geometry: Geometry::GlyphsOnly,
            glyphs: vec![Glyph {
                deg: 104.0,
                symbol: "✦Sir".into(),
                annotation: Some("Sirius".into()),
                ..Default::default()
            }],
        });
        let cmds = compose_sphere(&modelo, &SphereView::default(), &SphereOpts::default());
        assert!(
            cmds.iter().any(|c| matches!(
                c,
                DrawCommand::Text { content, .. } if content == "Sirius"
            )),
            "la estrella fija de la capa aparece etiquetada en la esfera"
        );
    }

    #[test]
    fn el_observador_sobre_la_tierra_coincide_con_el_cenit() {
        let eps = OBLICUIDAD_DEG.to_radians();
        for &(lat, lon, mc) in &[(-34.6_f32, -58.4, 10.0), (40.0, 14.0, 200.0), (51.5, 0.0, 280.0)] {
            let ramc = ramc_deg(mc, eps);
            // El punto geográfico del observador mira exactamente al
            // cénit — eso ancla la orientación de la Tierra interior.
            let obs = geo_to_ecliptic(lat, lon, lon, ramc, eps);
            let zen = zenith_ecliptic(lat, mc, eps);
            assert!(obs.dot(zen) > 0.9999, "obs·cénit = {}", obs.dot(zen));
        }
    }

    #[test]
    fn la_tierra_interior_dibuja_continentes_rellenos() {
        let modelo = modelo_demo();
        let poligonos = |c: &[DrawCommand]| {
            c.iter().filter(|d| matches!(d, DrawCommand::Polygon { .. })).count()
        };
        let con = compose_sphere(&modelo, &SphereView::default(), &SphereOpts::default());
        let sin = compose_sphere(
            &modelo,
            &SphereView::default(),
            &SphereOpts { show_earth: false, ..Default::default() },
        );
        assert_eq!(poligonos(&sin), 0, "sin Tierra no hay continentes");
        assert!(
            poligonos(&con) >= 6,
            "la Tierra interior rellena cada continente como polígono"
        );
    }

    #[test]
    fn el_meridiano_contiene_cenit_polo_y_medio_cielo() {
        let eps = OBLICUIDAD_DEG.to_radians();
        for &(lat, mc) in &[(-34.6_f32, 10.0_f32), (40.0, 200.0), (51.5, 280.0)] {
            let z = zenith_ecliptic(lat, mc, eps);
            let ncp = rot_x(Vec3::new(0.0, 0.0, 1.0), eps);
            // Cénit, polo celeste y MC son coplanares (el plano del
            // meridiano) → su producto mixto se anula. Esto verifica
            // que el RAMC se derivó bien del Medio Cielo.
            let triple = z.cross(ncp).dot(eclip(mc));
            assert!(triple.abs() < 1e-4, "lat {lat}, mc {mc}: triple = {triple}");
        }
    }

    #[test]
    fn el_primer_comando_es_el_limbo_de_fondo() {
        let cmds = compose_sphere(&modelo_demo(), &SphereView::default(), &SphereOpts::default());
        assert!(
            matches!(cmds.first(), Some(DrawCommand::Circle { .. })),
            "el limbo (profundidad −100) se pinta primero"
        );
    }
