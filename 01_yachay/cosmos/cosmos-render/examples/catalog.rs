//! Genera un SVG con todos los glyphs astrológicos y su nombre,
//! para que el usuario identifique cuáles necesitan ajuste.

use cosmos_render::draw::{draw_commands_to_svg, DrawCommand, Rgba, TextAnchor};
use cosmos_render::glyphs::{planet_commands, sign_commands};

fn main() {
    let planets: &[(&str, &str)] = &[
        ("sun", "Sol"),
        ("moon", "Luna"),
        ("mercury", "Mercurio"),
        ("venus", "Venus"),
        ("mars", "Marte"),
        ("jupiter", "Júpiter"),
        ("saturn", "Saturno"),
        ("uranus", "Urano"),
        ("neptune", "Neptuno"),
        ("pluto", "Plutón"),
        ("north_node", "Nodo Norte"),
        ("south_node", "Nodo Sur"),
        ("chiron", "Quirón"),
        ("lilith", "Lilith"),
    ];
    let signs: &[(&str, &str)] = &[
        ("aries", "Aries"),
        ("taurus", "Tauro"),
        ("gemini", "Géminis"),
        ("cancer", "Cáncer"),
        ("leo", "Leo"),
        ("virgo", "Virgo"),
        ("libra", "Libra"),
        ("scorpio", "Escorpio"),
        ("sagittarius", "Sagitario"),
        ("capricorn", "Capricornio"),
        ("aquarius", "Acuario"),
        ("pisces", "Piscis"),
    ];

    let cell_w = 130.0_f32;
    let cell_h = 140.0_f32;
    let glyph_size = 80.0_f32;
    let cols = 7_u32;
    let total = planets.len() + signs.len() + 2; // headers
    let rows = (total as f32 / cols as f32).ceil() as u32 + 2;
    let width = cell_w * cols as f32;
    let height = cell_h * rows as f32 + 50.0;

    let fg = Rgba::opaque(0.92, 0.92, 0.92);
    let muted = Rgba::opaque(0.65, 0.65, 0.75);
    let header = Rgba::opaque(0.95, 0.85, 0.40);

    let mut cmds: Vec<DrawCommand> = Vec::new();
    // Fondo
    cmds.push(DrawCommand::Polygon {
        points: vec![
            (0.0, 0.0),
            (width, 0.0),
            (width, height),
            (0.0, height),
        ],
        fill: Some(Rgba::opaque(0.03, 0.04, 0.06)),
        stroke: None,
        stroke_w: 0.0,
    });

    let mut y_off = 30.0_f32;

    // Header planetas
    cmds.push(DrawCommand::Text {
        x: 20.0,
        y: y_off,
        content: "Planetas".into(),
        color: header,
        size: 22.0,
        anchor: TextAnchor::Start,
    });
    y_off += 22.0;

    let mut col = 0_u32;
    let mut row_top = y_off;
    for (sym, name) in planets {
        let cx = (col as f32 + 0.5) * cell_w;
        let cy = row_top + cell_h * 0.45;
        cmds.extend(planet_commands(sym, cx, cy, glyph_size, fg, 2.8));
        cmds.push(DrawCommand::Text {
            x: cx,
            y: row_top + cell_h - 25.0,
            content: (*name).into(),
            color: fg,
            size: 14.0,
            anchor: TextAnchor::Middle,
        });
        cmds.push(DrawCommand::Text {
            x: cx,
            y: row_top + cell_h - 10.0,
            content: format!("({sym})"),
            color: muted,
            size: 10.0,
            anchor: TextAnchor::Middle,
        });
        col += 1;
        if col >= cols {
            col = 0;
            row_top += cell_h;
        }
    }
    if col != 0 {
        row_top += cell_h;
    }
    row_top += 20.0;
    y_off = row_top;

    // Header signos
    cmds.push(DrawCommand::Text {
        x: 20.0,
        y: y_off,
        content: "Signos".into(),
        color: header,
        size: 22.0,
        anchor: TextAnchor::Start,
    });
    y_off += 22.0;
    col = 0;
    row_top = y_off;
    for (sym, name) in signs {
        let cx = (col as f32 + 0.5) * cell_w;
        let cy = row_top + cell_h * 0.45;
        cmds.extend(sign_commands(sym, cx, cy, glyph_size, fg, 2.8));
        cmds.push(DrawCommand::Text {
            x: cx,
            y: row_top + cell_h - 25.0,
            content: (*name).into(),
            color: fg,
            size: 14.0,
            anchor: TextAnchor::Middle,
        });
        cmds.push(DrawCommand::Text {
            x: cx,
            y: row_top + cell_h - 10.0,
            content: format!("({sym})"),
            color: muted,
            size: 10.0,
            anchor: TextAnchor::Middle,
        });
        col += 1;
        if col >= cols {
            col = 0;
            row_top += cell_h;
        }
    }

    // Truco: draw_commands_to_svg usa un solo `size` cuadrado — generamos
    // a mano con dimensiones explícitas.
    let inner = draw_commands_to_svg(&cmds, width.max(height));
    let inner = inner.replace(
        &format!("viewBox=\"0 0 {0} {0}\"", width.max(height) as i32),
        &format!("viewBox=\"0 0 {} {}\"", width as i32, height as i32),
    );
    let inner = inner.replace(
        &format!("width=\"{0}\" height=\"{0}\"", width.max(height) as i32),
        &format!("width=\"{}\" height=\"{}\"", width as i32, height as i32),
    );

    std::fs::write("/tmp/cosmos_catalog.svg", inner).unwrap();
    println!("→ /tmp/cosmos_catalog.svg  ({} cmds)", cmds.len());
}
