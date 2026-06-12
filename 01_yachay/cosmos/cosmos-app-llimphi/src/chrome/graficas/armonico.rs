//! Rueda armónica (Cochrane / Addey) y flor de aspectos.
//!
//! Contiene las funciones puras de dibujo (`harmonic_flower_cmds`) y el
//! canvas interactivo de la rueda armónica con zoom/paneo.

use cosmos_render::{DrawCommand, Palette, Rgba};
use llimphi_ui::{DragPhase, View};
use llimphi_theme::Theme;

use crate::model::{Model, Msg};
use super::{
    canvas_column, custom_canvas, graphics_bg, graphics_palette, rgba_of, zoom_controls,
    natal_body_lons, topo_body_lons, canon_glyph, sign_color_theme,
};

// =====================================================================
// Flor de aspectos (función pura)
// =====================================================================

/// Flor armónica central: la **trama de aspectos** que el motor recomputa
/// sobre las posiciones armónicas (capa `Aspects`, módulo `natal`). Cada
/// aspecto se dibuja como un pétalo-lente que conecta los dos cuerpos
/// pasando cerca del centro; al cruzarse forman la roseta. El color es el
/// del tipo de aspecto y la opacidad/grosor crecen con la exactitud (orbe).
/// Función pura (sin `Model`) para poder rasterizarla en un test.
pub(crate) fn harmonic_flower_cmds(
    render: &cosmos_render::RenderModel,
    cx: f32,
    cy: f32,
    rp: f32,
    pal: &Palette,
    show_minor: bool,
    harmonic: u32,
) -> Vec<DrawCommand> {
    use cosmos_render::{Geometry, LayerKind};
    let mut cmds: Vec<DrawCommand> = Vec::new();

    let is_minor = |k: &str| {
        !matches!(
            k,
            "conjunction" | "sextile" | "square" | "trine" | "opposition"
        )
    };
    // Un lóbulo convexo del centro hacia el planeta en `deg`. `filled`:
    // geocéntrico (relleno); topocéntrico va sólo de contorno para que las
    // diferencias salten (donde topo se separa de geo, queda el contorno
    // suelto sin relleno debajo).
    let lobe = |deg: f32, col: Rgba, intensity: f32, filled: bool| -> DrawCommand {
        let th = (deg - 90.0).to_radians();
        let (ux, uy) = (th.cos(), th.sin());
        let (px, py) = (-uy, ux);
        let (tx, ty) = (cx + ux * rp, cy + uy * rp);
        let br = rp * 0.66;
        let w = rp * (0.09 + 0.13 * intensity);
        let (s1x, s1y) = (cx + ux * br + px * w, cy + uy * br + py * w);
        let (s2x, s2y) = (cx + ux * br - px * w, cy + uy * br - py * w);
        let d = format!("M {cx} {cy} Q {s1x} {s1y} {tx} {ty} Q {s2x} {s2y} {cx} {cy} Z");
        if filled {
            let a = 0.16 + 0.30 * intensity;
            DrawCommand::Path {
                d,
                fill: Some(Rgba { a, ..col }),
                stroke: Some(Rgba { a: (a + 0.20).min(0.7), ..col }),
                stroke_w: 0.7,
            }
        } else {
            DrawCommand::Path {
                d,
                fill: None,
                stroke: Some(Rgba { a: (0.45 + 0.35 * intensity).min(0.9), ..col }),
                stroke_w: 1.3,
            }
        }
    };

    // (1) GEOCÉNTRICO: la trama de aspectos que el motor ya recomputó en
    //     armónica (capa Aspects/natal) — lóbulos rellenos.
    for layer in &render.layers {
        if !matches!(layer.kind, LayerKind::Aspects) || layer.module_id != "natal" {
            continue;
        }
        let Geometry::Lines(segs) = &layer.geometry else {
            continue;
        };
        for seg in segs {
            if is_minor(&seg.kind) && !show_minor {
                continue;
            }
            let col = pal.aspect(&seg.kind);
            let intensity = ((1.0 - seg.orb_deg.abs() / 8.0).clamp(0.15, 1.0)) * seg.opacity;
            cmds.push(lobe(seg.from_deg, col, intensity, true));
            cmds.push(lobe(seg.to_deg, col, intensity, true));
        }
    }

    // (2) TOPOCÉNTRICO: longitudes topo × H y su propia trama de aspectos
    //     (mismo algoritmo del motor) — lóbulos de contorno. Resalta el
    //     paralaje: donde topo difiere de geo, el contorno se despega.
    let hf = harmonic.max(1) as f32;
    let topo: Vec<(String, f32)> = topo_body_lons(render)
        .into_iter()
        .map(|(s, d)| (s, (d * hf).rem_euclid(360.0)))
        .collect();
    if !topo.is_empty() {
        for seg in cosmos_render::harmonic::harmonic_aspect_lines(&topo) {
            if is_minor(&seg.kind) && !show_minor {
                continue;
            }
            let col = pal.aspect(&seg.kind);
            let intensity = (1.0 - seg.orb_deg.abs() / 8.0).clamp(0.15, 1.0);
            cmds.push(lobe(seg.from_deg, col, intensity, false));
            cmds.push(lobe(seg.to_deg, col, intensity, false));
        }
    }

    // Centro luminoso.
    cmds.push(DrawCommand::Circle {
        cx,
        cy,
        r: rp * 0.07,
        stroke: None,
        fill: Some(Rgba { r: 1.0, g: 1.0, b: 1.0, a: 0.9 }),
        stroke_w: 0.0,
    });
    cmds
}

// =====================================================================
// Canvas interactivo
// =====================================================================

/// Rueda armónica (Cochrane / Addey): cada longitud natal se multiplica
/// por el armónico activo (mod 360) y se grafica en un zodíaco de 12
/// signos, con una **flor armónica** (roseta de pétalos por cuerpo) en el
/// centro. H1 = la carta natal.
pub(super) fn harmonic_wheel_canvas(
    model: &Model,
    render: &cosmos_render::RenderModel,
    size: f32,
    theme: &Theme,
    fill: bool,
) -> View<Msg> {
    use cosmos_render::glyphs::{planet_commands, sign_commands};
    let cx = size / 2.0;
    let cy = size / 2.0;
    let r = size * 0.42;
    let grid = rgba_of(theme.fg_muted);
    let grid_soft = Rgba { a: 0.4, ..grid };
    let fg = rgba_of(theme.fg_text);

    let mut cmds: Vec<DrawCommand> = Vec::new();
    cmds.push(DrawCommand::Circle {
        cx,
        cy,
        r,
        stroke: Some(grid),
        fill: Some(rgba_of(theme.bg_panel)),
        stroke_w: 1.5,
    });
    cmds.push(DrawCommand::Circle {
        cx,
        cy,
        r: r * 0.80,
        stroke: Some(grid_soft),
        fill: None,
        stroke_w: 0.8,
    });
    // Flor armónica central: la trama de aspectos recomputada por el motor
    // sobre las posiciones armónicas (los pétalos cruzan por el centro).
    cmds.extend(harmonic_flower_cmds(
        render,
        cx,
        cy,
        r * 0.64,
        &graphics_palette(model),
        model.cfg.minor_aspects,
        model.harmonic,
    ));
    // 12 sectores zodiacales + glyph de cada signo en el anillo exterior.
    let sign_ids = crate::glyphs::SIGN_IDS;
    for i in 0..12 {
        let ang = (i as f32 * 30.0 - 90.0).to_radians();
        cmds.push(DrawCommand::Line {
            x1: cx + ang.cos() * r * 0.80,
            y1: cy + ang.sin() * r * 0.80,
            x2: cx + ang.cos() * r,
            y2: cy + ang.sin() * r,
            color: grid_soft,
            width: 0.7,
            dash: None,
        });
        let mid = ((i as f32 + 0.5) * 30.0 - 90.0).to_radians();
        let sx = cx + mid.cos() * r * 0.90;
        let sy = cy + mid.sin() * r * 0.90;
        let scol = rgba_of(sign_color_theme(i, model));
        cmds.extend(sign_commands(sign_ids[i], sx, sy, size * 0.035, scol, 1.4));
    }
    // Cuerpos: el render YA viene con el armónico aplicado por el motor
    // (`apply_harmonic`), así que se usan sus longitudes tal cual — sin
    // volver a multiplicar por H (eso duplicaba el armónico).
    for (sym, deg) in natal_body_lons(render) {
        let hl = deg.rem_euclid(360.0);
        let ang = (hl - 90.0).to_radians();
        let gx = cx + ang.cos() * r * 0.66;
        let gy = cy + ang.sin() * r * 0.66;
        cmds.push(DrawCommand::Circle {
            cx: cx + ang.cos() * r * 0.80,
            cy: cy + ang.sin() * r * 0.80,
            r: 2.0,
            stroke: None,
            fill: Some(grid),
            stroke_w: 0.0,
        });
        cmds.extend(planet_commands(&canon_glyph(&sym), gx, gy, size * 0.045, fg, 1.7));
    }
    cmds.push(DrawCommand::Text {
        x: size * 0.07,
        y: size * 0.07,
        content: format!("H{}", model.harmonic),
        color: grid,
        size: 15.0,
        anchor: cosmos_render::TextAnchor::Start,
    });

    custom_canvas(model, cmds, size, theme, fill)
}
