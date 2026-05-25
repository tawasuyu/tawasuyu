//! Export del `RenderModel` a SVG.
//!
//! Genera un documento SVG standalone con la misma geometría que pinta
//! el canvas: anillos zodiacales, cusps, planetas, aspectos. El
//! resultado es escalable (imprimible a cualquier tamaño) y no requiere
//! la app GPUI para verse — cualquier visor de SVG sirve.
//!
//! Convención de coordenadas idéntica al canvas:
//! `screen_angle_deg = 180 - (longitude - ascendant)` con +y para abajo.

use std::f64::consts::PI;
use std::fmt::Write;

use crate::{Geometry, LayerKind, RenderModel};

/// Dimensiones default del viewport. Aspect ratio cuadrada.
const VIEWBOX: f64 = 800.0;
const MARGIN: f64 = 40.0;

/// Radios normalizados — espejan los de `cosmos_app-canvas`.
const R_SIGN_OUTER: f64 = 1.00;
const R_SIGN_INNER: f64 = 0.88;
const R_TRANSITS: f64 = 0.82;
const R_HOUSES_OUTER: f64 = 0.78;
const R_HOUSES_INNER: f64 = 0.66;
const R_BODIES: f64 = 0.58;
const R_PROGRESSION: f64 = 0.48;
const R_SOLAR_ARC: f64 = 0.40;
const R_ASPECTS: f64 = 0.32;

/// Convierte el `RenderModel` a un documento SVG completo.
pub fn render_to_svg(render: &RenderModel) -> String {
    let mut out = String::with_capacity(8192);
    let r_outer = (VIEWBOX - MARGIN * 2.0) / 2.0;
    let cx = VIEWBOX / 2.0;
    let cy = VIEWBOX / 2.0;
    let asc = render.ascendant_deg as f64;

    writeln!(
        out,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {0} {1}" width="{0}" height="{1}" font-family="serif" text-anchor="middle" dominant-baseline="central">"#,
        VIEWBOX, VIEWBOX
    )
    .unwrap();

    // Fondo + título.
    writeln!(
        out,
        r##"  <rect x="0" y="0" width="{0}" height="{0}" fill="#fdfaf3"/>
  <text x="{cx}" y="20" font-size="14" fill="#2a2620">{title}</text>"##,
        VIEWBOX,
        cx = cx,
        title = escape_xml(&render.title)
    )
    .unwrap();

    // Anillos base.
    for r in [R_SIGN_OUTER, R_SIGN_INNER, R_HOUSES_OUTER, R_HOUSES_INNER] {
        writeln!(
            out,
            r##"  <circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="#a89572" stroke-width="0.6"/>"##,
            r = r * r_outer
        )
        .unwrap();
    }

    // Cusps del zodíaco cada 30°.
    for i in 0..12 {
        let lon = (i as f64) * 30.0;
        let (x1, y1) = polar(lon, asc, R_SIGN_INNER * r_outer, cx, cy);
        let (x2, y2) = polar(lon, asc, R_SIGN_OUTER * r_outer, cx, cy);
        writeln!(
            out,
            r##"  <line x1="{x1:.2}" y1="{y1:.2}" x2="{x2:.2}" y2="{y2:.2}" stroke="#a89572" stroke-width="0.5"/>"##,
        )
        .unwrap();
    }

    // Glifos de signos a media-altura del dial.
    let sign_mid = (R_SIGN_OUTER + R_SIGN_INNER) / 2.0;
    for layer in &render.layers {
        if matches!(layer.kind, LayerKind::SignDial) {
            for g in &layer.glyphs {
                let (x, y) = polar(g.deg as f64, asc, sign_mid * r_outer, cx, cy);
                writeln!(
                    out,
                    r##"  <text x="{x:.2}" y="{y:.2}" font-size="16" fill="#5a4830">{}</text>"##,
                    sign_unicode(&g.symbol)
                )
                .unwrap();
            }
        }
    }

    // Cusps de casas + énfasis Asc/IC/Desc/MC.
    for layer in &render.layers {
        if matches!(layer.kind, LayerKind::Houses) {
            if let Geometry::Ring { cusps_deg } = &layer.geometry {
                for (i, c) in cusps_deg.iter().enumerate() {
                    let is_angle = i == 0 || i == 3 || i == 6 || i == 9;
                    let (color, w) = if is_angle {
                        ("#b8862e", 1.6)
                    } else {
                        ("#9b8460", 0.5)
                    };
                    let (x1, y1) =
                        polar(*c as f64, asc, R_HOUSES_INNER * r_outer, cx, cy);
                    let (x2, y2) =
                        polar(*c as f64, asc, R_HOUSES_OUTER * r_outer, cx, cy);
                    writeln!(
                        out,
                        r##"  <line x1="{x1:.2}" y1="{y1:.2}" x2="{x2:.2}" y2="{y2:.2}" stroke="{color}" stroke-width="{w}"/>"##,
                    )
                    .unwrap();
                }
            }
        }
    }

    // Líneas de aspectos. Para natal usamos un solo ring; para
    // cross-aspects (transit/synastry/progression/solar_arc/...) los
    // extremos van en rings distintos según el `module_id`.
    for layer in &render.layers {
        if !matches!(layer.kind, LayerKind::Aspects) {
            continue;
        }
        if let Geometry::Lines(segs) = &layer.geometry {
            let (r_from, r_to) = aspect_radii(&layer.module_id);
            for seg in segs {
                let color = aspect_color_hex(&seg.kind);
                let (x1, y1) = polar(seg.from_deg as f64, asc, r_from * r_outer, cx, cy);
                let (x2, y2) = polar(seg.to_deg as f64, asc, r_to * r_outer, cx, cy);
                writeln!(
                    out,
                    r##"  <line x1="{x1:.2}" y1="{y1:.2}" x2="{x2:.2}" y2="{y2:.2}" stroke="{color}" stroke-width="0.6" stroke-opacity="{op:.2}"/>"##,
                    op = seg.opacity
                )
                .unwrap();
            }
        }
    }

    // Glifos planetarios (natal + overlays). Cada uno en su ring.
    for layer in &render.layers {
        if !matches!(layer.kind, LayerKind::Bodies | LayerKind::Outer) {
            continue;
        }
        let ring = body_ring_radius(&layer.module_id);
        let size = if layer.module_id == "natal" { 18 } else { 14 };
        for g in &layer.glyphs {
            let (x, y) = polar(g.deg as f64, asc, ring * r_outer, cx, cy);
            let glyph = planet_unicode(&g.symbol);
            let suffix = match (g.retrograde, g.dignity_marker.as_deref()) {
                (true, Some(m)) => format!("ᴿ{}", m),
                (true, None) => "ᴿ".into(),
                (false, Some(m)) => m.to_string(),
                (false, None) => String::new(),
            };
            writeln!(
                out,
                r##"  <text x="{x:.2}" y="{y:.2}" font-size="{size}" fill="#1f1812">{glyph}{suffix}</text>"##
            )
            .unwrap();
        }
    }

    // Etiquetas ASC / MC / DESC / IC en el perímetro.
    for (deg, label) in [
        (asc, "ASC"),
        (render.midheaven_deg as f64, "MC"),
        (render.descendant_deg as f64, "DESC"),
        (render.imum_coeli_deg as f64, "IC"),
    ] {
        let (x, y) = polar(deg, asc, 1.06 * r_outer, cx, cy);
        writeln!(
            out,
            r##"  <text x="{x:.2}" y="{y:.2}" font-size="10" fill="#b8862e">{label}</text>"##
        )
        .unwrap();
    }

    writeln!(out, "</svg>").unwrap();
    out
}

fn polar(longitude_deg: f64, ascendant_deg: f64, radius: f64, cx: f64, cy: f64) -> (f64, f64) {
    let deg = 180.0 - (longitude_deg - ascendant_deg);
    let rad = deg * PI / 180.0;
    (cx + radius * rad.cos(), cy + radius * rad.sin())
}

fn aspect_radii(module_id: &str) -> (f64, f64) {
    if crate::OUTER_RING_MODULES.contains(&module_id) {
        return (R_BODIES, R_TRANSITS);
    }
    match module_id {
        "progression" => (R_BODIES, R_PROGRESSION),
        "solar_arc" => (R_BODIES, R_SOLAR_ARC),
        _ => (R_ASPECTS, R_ASPECTS),
    }
}

fn body_ring_radius(module_id: &str) -> f64 {
    if crate::OUTER_RING_MODULES.contains(&module_id) {
        return R_TRANSITS;
    }
    match module_id {
        "progression" => R_PROGRESSION,
        "solar_arc" => R_SOLAR_ARC,
        _ => R_BODIES,
    }
}

fn sign_unicode(name: &str) -> &'static str {
    match name {
        "aries" => "♈",
        "taurus" => "♉",
        "gemini" => "♊",
        "cancer" => "♋",
        "leo" => "♌",
        "virgo" => "♍",
        "libra" => "♎",
        "scorpio" => "♏",
        "sagittarius" => "♐",
        "capricorn" => "♑",
        "aquarius" => "♒",
        "pisces" => "♓",
        _ => "?",
    }
}

fn planet_unicode(name: &str) -> &'static str {
    match name {
        "sun" => "☉",
        "moon" => "☽",
        "mercury" => "☿",
        "venus" => "♀",
        "mars" => "♂",
        "jupiter" => "♃",
        "saturn" => "♄",
        "uranus" => "♅",
        "neptune" => "♆",
        "pluto" => "♇",
        "north_node" => "☊",
        "south_node" => "☋",
        "chiron" => "⚷",
        "lilith" => "⚸",
        "ceres" => "⚳",
        "pallas" => "⚴",
        "juno" => "⚵",
        "vesta" => "⚶",
        _ => "•",
    }
}

fn aspect_color_hex(kind: &str) -> &'static str {
    match kind {
        "conjunction" => "#b8862e",
        "opposition" => "#a64a8a",
        "trine" => "#3f7d57",
        "square" => "#c64b2a",
        "sextile" => "#3a6db5",
        _ => "#8a7660",
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compute_mock, ChartKind};
    use cosmos_model::{Chart, ContactId, StoredBirthData, StoredChartConfig};

    fn sample_chart() -> Chart {
        Chart {
            id: cosmos_model::ChartId::new(),
            contact_id: ContactId::new(),
            kind: ChartKind::Natal,
            label: "Test".into(),
            birth_data: StoredBirthData {
                year: 1987,
                month: 3,
                day: 14,
                hour: 5,
                minute: 22,
                second: 0.0,
                tz_offset_minutes: -240,
                latitude_deg: 10.0,
                longitude_deg: -66.0,
                altitude_m: 0.0,
                time_certainty: Default::default(),
                subject_name: None,
                birthplace_label: None,
            },
            config: StoredChartConfig::default(),
            related_chart_id: None,
            created_at_ms: 0,
        }
    }

    #[test]
    fn svg_well_formed_minimal() {
        let render = compute_mock(&sample_chart());
        let svg = render_to_svg(&render);
        assert!(svg.starts_with("<?xml"));
        assert!(svg.contains("<svg"));
        assert!(svg.ends_with("</svg>\n"));
        // Debe traer al menos un círculo de los rings base.
        assert!(svg.contains("<circle "));
    }
}
