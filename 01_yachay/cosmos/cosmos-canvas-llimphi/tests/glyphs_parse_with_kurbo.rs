//! Regresión: cada path SVG emitido por `cosmos_render::glyphs` debe
//! parsear con `kurbo::BezPath::from_svg`. Si no, el canvas Llimphi
//! silenciosamente se saltea el comando (eprintln + return) y el
//! glyph aparece roto en el wheel — exactamente el bug que motivó
//! pasarse a geometría vectorial.

use cosmos_render::draw::{DrawCommand, Rgba};
use cosmos_render::glyphs::{planet_commands, retrograde_marker, sign_commands};
use llimphi_ui::llimphi_raster::kurbo::BezPath;

#[test]
fn todos_los_glyphs_parsean_con_kurbo() {
    let color = Rgba::opaque(1.0, 1.0, 1.0);
    let planets = [
        "sun", "moon", "mercury", "venus", "mars", "jupiter", "saturn", "uranus", "neptune",
        "pluto", "north_node", "south_node", "chiron", "lilith",
    ];
    let signs = [
        "aries", "taurus", "gemini", "cancer", "leo", "virgo", "libra", "scorpio", "sagittarius",
        "capricorn", "aquarius", "pisces",
    ];

    let mut fallas = Vec::new();
    let mut check = |cmds: Vec<DrawCommand>, label: &str| {
        for c in cmds {
            if let DrawCommand::Path { d, .. } = c {
                if let Err(e) = BezPath::from_svg(&d) {
                    fallas.push(format!("{label}: {e:?} :: {d}"));
                }
            }
        }
    };
    for p in &planets {
        check(planet_commands(p, 100.0, 100.0, 30.0, color, 2.0), p);
    }
    check(vec![retrograde_marker(100.0, 100.0, 30.0, color)], "retro");
    for s in &signs {
        check(sign_commands(s, 100.0, 100.0, 30.0, color, 2.0), s);
    }
    assert!(
        fallas.is_empty(),
        "{} paths inválidos:\n{}",
        fallas.len(),
        fallas.join("\n")
    );
}
