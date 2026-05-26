//! Carta armónica — transforma un `RenderModel` natal a su armónico
//! de orden N.
//!
//! La carta armónica multiplica cada longitud eclíptica por N (mod
//! 360). Es la herramienta de John Addey / David Cochrane para
//! revelar patrones de aspecto: dos cuerpos que forman el aspecto de
//! la N-ésima armónica natal (p. ej. un quintil en N=5) caen
//! conjuntos en la carta armónica N — los clusters armónicos saltan
//! a la vista.
//!
//! Lógica pura, agnóstica de surface: el engine produce el
//! `RenderModel` natal y delega aquí la transformación. Reutilizable
//! por el canvas Llimphi y por el cliente web.

use crate::{AspectSummary, Geometry, LayerKind, LineSeg, RenderModel};

/// Máxima armónica que cubre el espectro de fuerza.
pub const HARMONIC_SPECTRUM_MAX: u32 = 32;

/// Aspectos que se buscan en la carta armónica: `(id, ángulo, orbe)`.
/// Conjunción y oposición llevan orbe más amplio, como es convención.
const HARMONIC_ASPECTS: &[(&str, f32, f32)] = &[
    ("conjunction", 0.0, 8.0),
    ("opposition", 180.0, 7.0),
    ("trine", 120.0, 6.0),
    ("square", 90.0, 6.0),
    ("sextile", 60.0, 4.0),
];

/// Transforma `model` —una carta natal ya compuesta— en su carta
/// armónica de orden `n`. `n <= 1` la deja intacta.
///
/// Sólo afecta las capas `module_id == "natal"`: los cuerpos pasan a
/// `(lon · n) mod 360` y la capa de aspectos se recomputa sobre las
/// posiciones armónicas. Las casas y los ángulos natales se conservan
/// como marco espacial de referencia (variante "armónicos en casas
/// radicales"); los overlays, si los hubiera, quedan intactos.
pub fn apply_harmonic(model: &mut RenderModel, n: u32) {
    if n <= 1 {
        return;
    }
    let nf = n as f32;

    // 0. Longitudes natales (pre-transformación) para el espectro.
    let natal_longitudes: Vec<f32> = model
        .layers
        .iter()
        .filter(|l| l.module_id == "natal" && l.kind == LayerKind::Bodies)
        .flat_map(|l| l.glyphs.iter().map(|g| g.deg))
        .collect();

    // 1. Transformar los cuerpos natales; recolectar `(símbolo, lon)`.
    let mut bodies: Vec<(String, f32)> = Vec::new();
    for layer in &mut model.layers {
        if layer.module_id != "natal" || layer.kind != LayerKind::Bodies {
            continue;
        }
        for g in &mut layer.glyphs {
            g.deg = (g.deg * nf).rem_euclid(360.0);
            bodies.push((g.symbol.clone(), g.deg));
        }
        if let Geometry::Points(points) = &mut layer.geometry {
            for p in points {
                p.deg = (p.deg * nf).rem_euclid(360.0);
            }
        }
    }

    // 2. Recomputar la capa de aspectos natal sobre las posiciones
    //    armónicas.
    let lines = harmonic_aspect_lines(&bodies);
    for layer in &mut model.layers {
        if layer.module_id == "natal" && layer.kind == LayerKind::Aspects {
            layer.geometry = Geometry::Lines(lines.clone());
        }
    }

    // 3. Rehacer el `aspect_summary`. En este punto del pipeline sólo
    //    contiene aspectos natales (los overlays agregan los suyos
    //    después de esta transformación).
    model.aspect_summary = lines
        .iter()
        .map(|l| AspectSummary {
            module_id: "natal".into(),
            from_body: l.from_body.clone(),
            to_body: l.to_body.clone(),
            kind: l.kind.clone(),
            orb_deg: l.orb_deg as f64,
            applying: None,
        })
        .collect();

    // 4. Espectro de fuerza armónica + armónico activo + título.
    model.harmonic = n;
    model.harmonic_spectrum = harmonic_spectrum(&natal_longitudes, HARMONIC_SPECTRUM_MAX);
    model.title = format!("{} · H{}", model.title, n);
}

/// Espectro de fuerza armónica: para cada armónica `1..=max`, cuánto
/// resuena la carta — la suma de la cercanía a conjunción exacta de
/// todos los pares de cuerpos en esa armónica. Un pico en H marca que
/// la carta tiene un patrón fuerte de la N-ésima armónica; es la guía
/// para elegir qué armónico mirar.
pub fn harmonic_spectrum(natal_longitudes: &[f32], max: u32) -> Vec<f32> {
    (1..=max)
        .map(|h| harmonic_strength(natal_longitudes, h))
        .collect()
}

/// Fuerza de una sola armónica: suma sobre pares de cuerpos de
/// `1 - sep/orb` para los pares que caen a menos de `RESONANCE_ORB`
/// de la conjunción en esa armónica.
fn harmonic_strength(longitudes: &[f32], h: u32) -> f32 {
    const RESONANCE_ORB: f32 = 10.0;
    let hf = h as f32;
    let mut score = 0.0;
    for i in 0..longitudes.len() {
        for j in (i + 1)..longitudes.len() {
            let a = (longitudes[i] * hf).rem_euclid(360.0);
            let b = (longitudes[j] * hf).rem_euclid(360.0);
            let sep = circular_sep(a, b);
            if sep < RESONANCE_ORB {
                score += 1.0 - sep / RESONANCE_ORB;
            }
        }
    }
    score
}

/// Separación circular mínima entre dos longitudes (rango `0..=180`).
fn circular_sep(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// Busca aspectos entre cada par de cuerpos por sus longitudes (ya
/// armónicas). Devuelve los segmentos ordenados por orbe ascendente.
fn harmonic_aspect_lines(bodies: &[(String, f32)]) -> Vec<LineSeg> {
    let mut lines = Vec::new();
    for i in 0..bodies.len() {
        for j in (i + 1)..bodies.len() {
            let (a_sym, a_deg) = &bodies[i];
            let (b_sym, b_deg) = &bodies[j];
            let sep = circular_sep(*a_deg, *b_deg);
            for (id, angle, orb) in HARMONIC_ASPECTS {
                let delta = (sep - angle).abs();
                if delta <= *orb {
                    lines.push(LineSeg {
                        from_deg: *a_deg,
                        to_deg: *b_deg,
                        kind: (*id).to_string(),
                        opacity: (1.0 - delta / orb * 0.65).clamp(0.30, 1.0),
                        from_body: a_sym.clone(),
                        to_body: b_sym.clone(),
                        orb_deg: delta,
                    });
                    break; // un par no forma dos aspectos a la vez
                }
            }
        }
    }
    lines.sort_by(|x, y| {
        x.orb_deg
            .partial_cmp(&y.orb_deg)
            .unwrap_or(core::cmp::Ordering::Equal)
    });
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChartId, ChartKind, Geometry, Glyph, Layer, LayerKind, PointMark, RenderModel};

    fn body(symbol: &str, deg: f32) -> Glyph {
        Glyph {
            deg,
            symbol: symbol.to_string(),
            ..Default::default()
        }
    }

    fn natal_model(bodies: &[(&str, f32)]) -> RenderModel {
        let glyphs: Vec<Glyph> = bodies.iter().map(|(s, d)| body(s, *d)).collect();
        let points: Vec<PointMark> = bodies
            .iter()
            .map(|(s, d)| PointMark {
                deg: *d,
                label: s.to_string(),
                tag: s.to_string(),
            })
            .collect();
        RenderModel {
            chart_id: ChartId::new(),
            chart_kind: ChartKind::Natal,
            title: "Test".to_string(),
            subtitle: None,
            compute_ms: 0,
            ascendant_deg: 0.0,
            midheaven_deg: 270.0,
            descendant_deg: 180.0,
            imum_coeli_deg: 90.0,
            geo_latitude_deg: 0.0,
            geo_longitude_deg: 0.0,
            layers: vec![
                Layer {
                    module_id: "natal".into(),
                    kind: LayerKind::Houses,
                    ring: 0.86,
                    z: 1,
                    geometry: Geometry::Ring {
                        cusps_deg: (0..12).map(|i| i as f32 * 30.0).collect(),
                    },
                    glyphs: Vec::new(),
                },
                Layer {
                    module_id: "natal".into(),
                    kind: LayerKind::Bodies,
                    ring: 0.72,
                    z: 2,
                    geometry: Geometry::Points(points),
                    glyphs,
                },
                Layer {
                    module_id: "natal".into(),
                    kind: LayerKind::Aspects,
                    ring: 0.58,
                    z: 3,
                    geometry: Geometry::Lines(Vec::new()),
                    glyphs: Vec::new(),
                },
            ],
            overlays: Vec::new(),
            aspect_summary: Vec::new(),
            uranian_groups: Vec::new(),
            gr_triggers: Vec::new(),
            harmonic: 1,
            harmonic_spectrum: Vec::new(),
        }
    }

    fn bodies_layer(model: &RenderModel) -> &Layer {
        model
            .layers
            .iter()
            .find(|l| l.module_id == "natal" && l.kind == LayerKind::Bodies)
            .expect("capa de cuerpos")
    }

    #[test]
    fn harmonic_one_is_identity() {
        let mut model = natal_model(&[("sun", 30.0), ("moon", 200.0)]);
        let before = model.clone();
        apply_harmonic(&mut model, 1);
        assert_eq!(bodies_layer(&model).glyphs[0].deg, before.layers[1].glyphs[0].deg);
        assert_eq!(model.title, "Test");
    }

    #[test]
    fn harmonic_two_doubles_longitudes_mod_360() {
        let mut model = natal_model(&[("sun", 30.0), ("moon", 200.0)]);
        apply_harmonic(&mut model, 2);
        let g = &bodies_layer(&model).glyphs;
        assert!((g[0].deg - 60.0).abs() < 1e-3, "30·2 = 60");
        assert!((g[1].deg - 40.0).abs() < 1e-3, "200·2 = 400 ≡ 40");
    }

    #[test]
    fn harmonic_two_also_transforms_point_marks() {
        let mut model = natal_model(&[("sun", 100.0)]);
        apply_harmonic(&mut model, 2);
        let Geometry::Points(points) = &bodies_layer(&model).geometry else {
            panic!("la capa de cuerpos debe seguir siendo Points");
        };
        assert!((points[0].deg - 200.0).abs() < 1e-3);
    }

    #[test]
    fn quintile_natally_becomes_conjunction_in_h5() {
        // 0° y 72° forman un quintil (72°). En H5: 0·5=0, 72·5=360≡0
        // → conjunción exacta.
        let mut model = natal_model(&[("sun", 0.0), ("venus", 72.0)]);
        apply_harmonic(&mut model, 5);
        let Geometry::Lines(lines) = &model
            .layers
            .iter()
            .find(|l| l.kind == LayerKind::Aspects)
            .unwrap()
            .geometry
        else {
            panic!("capa de aspectos");
        };
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].kind, "conjunction");
        assert!(lines[0].orb_deg < 0.01, "orbe ~0");
    }

    #[test]
    fn harmonic_annotates_title_and_summary() {
        let mut model = natal_model(&[("sun", 0.0), ("venus", 72.0)]);
        apply_harmonic(&mut model, 5);
        assert_eq!(model.title, "Test · H5");
        assert_eq!(model.aspect_summary.len(), 1);
        assert_eq!(model.aspect_summary[0].kind, "conjunction");
    }

    #[test]
    fn spectrum_peaks_at_the_resonant_harmonic() {
        // 0° y 72° son conjuntos en H5 (72·5 = 360 ≡ 0).
        let spectrum = harmonic_spectrum(&[0.0, 72.0], HARMONIC_SPECTRUM_MAX);
        assert_eq!(spectrum.len(), HARMONIC_SPECTRUM_MAX as usize);
        let h5 = spectrum[4]; // índice 4 = H5
        assert!(h5 > 0.99, "H5 resuena al máximo: {h5}");
        let max = spectrum.iter().copied().fold(0.0_f32, f32::max);
        assert!((h5 - max).abs() < 1e-4, "H5 es el pico del espectro");
    }

    #[test]
    fn apply_harmonic_populates_spectrum_and_current_order() {
        let mut model = natal_model(&[("sun", 0.0), ("venus", 72.0)]);
        apply_harmonic(&mut model, 5);
        assert_eq!(model.harmonic, 5);
        assert_eq!(
            model.harmonic_spectrum.len(),
            HARMONIC_SPECTRUM_MAX as usize
        );
    }

    #[test]
    fn houses_layer_is_preserved() {
        let mut model = natal_model(&[("sun", 10.0)]);
        apply_harmonic(&mut model, 3);
        assert!(
            model
                .layers
                .iter()
                .any(|l| l.kind == LayerKind::Houses),
            "las casas se conservan como marco de referencia"
        );
    }
}
