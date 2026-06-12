//! Dial uraniano de 90° (Escuela de Hamburgo).
//!
//! Contiene las funciones puras de dibujo (`uranian_dial_cmds`) y el
//! canvas interactivo con arrastre para girar la rueda bajo el puntero.

use cosmos_canvas_llimphi::ViewTransform;
use cosmos_render::{DrawCommand, Palette, Rgba, TextAnchor};
use llimphi_ui::{DragPhase, View};
use llimphi_theme::Theme;

use crate::model::{Model, Msg};
use super::{canvas_column, graphics_bg, graphics_palette, rgba_of, zoom_controls,
            natal_body_lons, topo_body_lons, canon_glyph};

// =====================================================================
// Helpers geométricos
// =====================================================================

/// Mapea una posición del dial (`m90` ∈ [0,90)) al ángulo visual en
/// radianes con 0° arriba y sentido horario.
pub(super) fn dial_ang(m90: f32) -> f32 {
    (m90 / 90.0 * 360.0 - 90.0).to_radians()
}

/// Path SVG de un arco entre dos ángulos (rad) a un radio dado.
fn arc_path(cx: f32, cy: f32, radius: f32, a0: f32, a1: f32) -> String {
    let (x0, y0) = (cx + a0.cos() * radius, cy + a0.sin() * radius);
    let (x1, y1) = (cx + a1.cos() * radius, cy + a1.sin() * radius);
    let large = if (a1 - a0).abs() > std::f32::consts::PI { 1 } else { 0 };
    let sweep = if a1 > a0 { 1 } else { 0 };
    format!("M {x0} {y0} A {radius} {radius} 0 {large} {sweep} {x1} {y1}")
}

// =====================================================================
// Comandos de dibujo (función pura, sin `Model`)
// =====================================================================

/// Comandos de dibujo del dial de 90° — función pura (sin `Model`) para
/// poder rasterizarla en un test. Replica la lámina clásica del dial:
/// anillo graduado con arcos negros, tres cuñas de modalidad con sus
/// glyphs grandes (cardinal ♈ / fijo ♉ / mutable ♊), eje-puntero rojo y
/// los cuerpos proyectados (mod 90) por fuera con líneas-guía.
pub(crate) fn uranian_dial_cmds(
    render: &cosmos_render::RenderModel,
    size: f32,
    pal: &Palette,
    fg: Rgba,
    grid: Rgba,
    accent: Rgba,
    bg: Rgba,
    rot: f32,
) -> Vec<DrawCommand> {
    use cosmos_render::glyphs::{planet_commands, sign_commands};
    let cx = size / 2.0;
    let cy = size / 2.0;
    let r = size * 0.40;
    let r_in = r * 0.90; // borde interno de la banda graduada
    let grid_soft = Rgba { a: 0.35, ..grid };
    let pt = |radius: f32, ang: f32| (cx + ang.cos() * radius, cy + ang.sin() * radius);
    // Posición del dial girada: el valor `rot` queda bajo el puntero (arriba).
    let da = |m90: f32| dial_ang((m90 - rot).rem_euclid(90.0));
    let tint = |c: Rgba, a: f32| Rgba { a, ..c };

    let mut cmds: Vec<DrawCommand> = Vec::new();
    // Disco + aro exterior.
    cmds.push(DrawCommand::Circle {
        cx,
        cy,
        r,
        stroke: Some(grid),
        fill: Some(bg),
        stroke_w: 1.5,
    });

    // Tres cuñas de modalidad tintadas por su elemento (cardinal=fuego/Aries,
    // fijo=tierra/Tauro, mutable=aire/Géminis), rellenas a baja opacidad.
    for (c0, sign) in [(0.0_f32, "aries"), (30.0, "taurus"), (60.0, "gemini")] {
        let col = pal.sign(sign);
        let mut poly = vec![(cx, cy)];
        for s in 0..=12 {
            let m = c0 + 30.0 * (s as f32 / 12.0);
            poly.push(pt(r_in, da(m)));
        }
        cmds.push(DrawCommand::Polygon {
            points: poly,
            fill: Some(tint(col, 0.10)),
            stroke: None,
            stroke_w: 0.0,
        });
    }
    // Divisores de modalidad (bordes 0/30/60) + glyph grande coloreado en el
    // centro de cada cuña.
    for b in [0.0_f32, 30.0, 60.0] {
        let (ix, iy) = pt(r * 0.12, da(b));
        let (ox, oy) = pt(r_in, da(b));
        cmds.push(DrawCommand::Line {
            x1: ix,
            y1: iy,
            x2: ox,
            y2: oy,
            color: grid_soft,
            width: 0.8,
            dash: None,
        });
    }
    for (center, sign) in [(15.0_f32, "aries"), (45.0, "taurus"), (75.0, "gemini")] {
        let (gx, gy) = pt(r * 0.50, da(center));
        cmds.extend(sign_commands(sign, gx, gy, size * 0.14, tint(pal.sign(sign), 0.85), 2.4));
    }

    // Aro interno de la banda graduada.
    cmds.push(DrawCommand::Circle {
        cx,
        cy,
        r: r_in,
        stroke: Some(grid_soft),
        fill: None,
        stroke_w: 0.8,
    });
    // Arcos negros: 8 segmentos gruesos en la banda, cada 45° visual (giran).
    let rb = (r + r_in) / 2.0;
    for k in 0..8 {
        let c = da(k as f32 * 11.25);
        let half = 4.0_f32.to_radians();
        cmds.push(DrawCommand::Path {
            d: arc_path(cx, cy, rb, c - half, c + half),
            stroke: Some(fg),
            fill: None,
            stroke_w: (r - r_in) * 0.95,
        });
    }
    // Graduación: ticks cada grado (90), medianos cada 5°, mayores cada 15°.
    for d in 0..90 {
        let ang = da(d as f32);
        let (major, medium) = (d % 15 == 0, d % 5 == 0);
        let inner = if major {
            r * 0.84
        } else if medium {
            r * 0.88
        } else {
            r_in
        };
        let (x1, y1) = pt(inner, ang);
        let (x2, y2) = pt(r, ang);
        cmds.push(DrawCommand::Line {
            x1,
            y1,
            x2,
            y2,
            color: if major { fg } else { grid },
            width: if major {
                1.3
            } else if medium {
                0.9
            } else {
                0.5
            },
            dash: None,
        });
        if major {
            let (tx, ty) = pt(r * 0.78, ang);
            cmds.push(DrawCommand::Text {
                x: tx,
                y: ty,
                content: format!("{d}"),
                color: grid,
                size: 9.0,
                anchor: TextAnchor::Middle,
            });
        }
    }

    // Eje-puntero rojo: diámetro vertical FIJO (no gira) con cabezas de
    // flecha arriba y abajo — es el índice; la rueda gira bajo él.
    let (tx, ty) = pt(r, dial_ang(0.0));
    let (bx, by) = pt(r, dial_ang(45.0));
    cmds.push(DrawCommand::Line {
        x1: tx,
        y1: ty,
        x2: bx,
        y2: by,
        color: tint(accent, 0.7),
        width: 1.0,
        dash: Some((4.0, 4.0)),
    });
    for (ax, ay, dir) in [(tx, ty, 1.0_f32), (bx, by, -1.0_f32)] {
        let h = size * 0.022;
        cmds.push(DrawCommand::Polygon {
            points: vec![
                (ax, ay + dir * h * 1.6),
                (ax - h, ay - dir * h * 0.2),
                (ax + h, ay - dir * h * 0.2),
            ],
            fill: Some(accent),
            stroke: None,
            stroke_w: 0.0,
        });
    }

    // Origen.
    cmds.push(DrawCommand::Circle {
        cx,
        cy,
        r: size * 0.006,
        stroke: Some(grid),
        fill: None,
        stroke_w: 1.0,
    });

    // Cuerpos proyectados (longitud mod 90) por fuera del aro, con guía y
    // glyph coloreado por planeta. Los que caen en orbe del puntero (la
    // posición `rot`) se conectan al centro en rojo: la "imagen planetaria".
    const ORB: f32 = 1.5;
    for (sym, deg) in natal_body_lons(render) {
        let m90 = deg.rem_euclid(90.0);
        let ang = da(m90);
        // Distancia circular (mod 90) al valor bajo el puntero.
        let mut dist = (m90 - rot).rem_euclid(90.0);
        if dist > 45.0 {
            dist = 90.0 - dist;
        }
        let on_pointer = dist <= ORB;
        let (lx1, ly1) = pt(r, ang);
        if on_pointer {
            // Línea al centro (radio) en rojo — parte de la imagen planetaria.
            cmds.push(DrawCommand::Line {
                x1: cx,
                y1: cy,
                x2: lx1,
                y2: ly1,
                color: tint(accent, 0.8),
                width: 1.4,
                dash: None,
            });
        }
        let (lx2, ly2) = pt(r * 1.10, ang);
        cmds.push(DrawCommand::Line {
            x1: lx1,
            y1: ly1,
            x2: lx2,
            y2: ly2,
            color: grid,
            width: 0.8,
            dash: None,
        });
        let (gx, gy) = pt(r * 1.17, ang);
        let canon = canon_glyph(&sym);
        let col = if on_pointer { accent } else { pal.planet(&canon) };
        cmds.extend(planet_commands(&canon, gx, gy, size * 0.042, col, 1.7));
    }
    // Posiciones TOPOCÉNTRICAS (si el overlay está activo): un anillo hueco
    // en el aro, en el color del planeta. Donde difiere del geocéntrico (la
    // Luna, sobre todo) el anillo se separa del glyph → se ve el paralaje.
    for (sym, deg) in topo_body_lons(render) {
        let ang = da(deg.rem_euclid(90.0));
        let (mx, my) = pt(r * 0.96, ang);
        cmds.push(DrawCommand::Circle {
            cx: mx,
            cy: my,
            r: size * 0.012,
            stroke: Some(pal.planet(&canon_glyph(&sym))),
            fill: None,
            stroke_w: 1.4,
        });
    }
    cmds
}

// =====================================================================
// Canvas interactivo
// =====================================================================

/// Dial uraniano de 90° (Escuela de Hamburgo). Los cuerpos se proyectan
/// a su longitud módulo 90° sobre un disco graduado y coloreado; se
/// **arrastra para girar** el dial bajo el puntero rojo, y los cuerpos en
/// orbe con el puntero se conectan al centro (imagen planetaria). 0° arriba.
pub(super) fn uranian_dial_canvas(
    model: &Model,
    render: &cosmos_render::RenderModel,
    size: f32,
    theme: &Theme,
    fill: bool,
) -> View<Msg> {
    let pal = graphics_palette(model);
    let cmds = uranian_dial_cmds(
        render,
        size,
        &pal,
        rgba_of(theme.fg_text),
        rgba_of(theme.fg_muted),
        rgba_of(theme.fg_destructive),
        rgba_of(theme.bg_panel),
        model.dial_rot,
    );
    let t = ViewTransform {
        zoom: model.wheel_zoom,
        pan: model.wheel_pan,
    };
    let canvas = cosmos_canvas_llimphi::canvas_view_ex::<Msg>(cmds, size, Some(graphics_bg(model)), t)
        .draggable_at(|phase, dx, _dy, _lx, _ly| match phase {
            DragPhase::Move => Some(Msg::DialRotate(dx)),
            DragPhase::End => None,
        });
    canvas_column(Some(zoom_controls(model, theme)), canvas, size, fill)
}
