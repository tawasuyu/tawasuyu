//! Glyphs astrológicos como geometría — paths vectoriales hechos a
//! mano para cada planeta y signo zodiacal.
//!
//! ## Por qué dibujar como path y no como texto unicode
//!
//! El bloque astrológico (`U+2609..U+264F`, signos `U+2648..U+2653`)
//! tiene cobertura **parcial e inconsistente** en las fuentes default
//! del sistema (LiberationSans / AdwaitaSans en Arch/artix sólo traen
//! `♀♂☿♃♄♅♆♇` — faltan `☉☽` y todos los zodiacales). Resultado: el
//! usuario veía solo Venus y Marte, y los signos caían como
//! `.notdef` invisibles. Para no depender de fuentes del sistema —
//! ni embeber una fuente OFL en el binario — dibujamos los glyphs
//! como composiciones de `DrawCommand` (`Circle`, `Line`, `Path`).
//!
//! ## Convención
//!
//! Cada función emite una lista de [`DrawCommand`]s centrados en
//! `(cx, cy)` con un tamaño total ≈ `size` (alto y ancho). Las
//! proporciones se mantienen entre 0.6×size y 1.0×size; el caller
//! elige `size` para que entre en el aro asignado.
//!
//! Los paths usan sintaxis SVG (`M x y`, `L x y`, `A rx ry … x y`,
//! `Z` …). El canvas Llimphi los parsea con
//! `kurbo::BezPath::from_svg`; el SVG exporter los emite directo
//! como atributo `d` de un `<path>`. Ambos backends ven la **misma**
//! geometría.
//!
//! ## Estilo
//!
//! Glyphs estilizados, monoline (sin variación de stroke), lo más
//! reconocibles posible al tamaño usado en el wheel (~20 px). No
//! pretenden ser tipográficamente correctos — sí ser identificables
//! por un astrólogo (un círculo con punto = Sol, una "h" con cruz =
//! Saturno, una "Y" con ω = Aries, etc.).

use crate::draw::{DrawCommand, Rgba};

/// Devuelve los comandos para dibujar el glyph de un planeta. Si el
/// `name` no es uno reconocido, devuelve un círculo pequeño relleno
/// como fallback (mismo comportamiento simbólico que un bullet).
///
/// `cx`, `cy`: centro del glyph en coordenadas absolutas del wheel.
/// `size`: alto/ancho aproximado del símbolo en px.
/// `color`: color del trazo y fill (no se diferencian aquí).
/// `stroke_w`: grosor base del trazo.
pub fn planet_commands(
    name: &str,
    cx: f32,
    cy: f32,
    size: f32,
    color: Rgba,
    stroke_w: f32,
) -> Vec<DrawCommand> {
    let r = size * 0.5;
    match name {
        // ─── Sol: círculo + punto central ──────────────────────────
        "sun" => vec![
            DrawCommand::Circle {
                cx,
                cy,
                r: r * 0.85,
                stroke: Some(color),
                fill: None,
                stroke_w,
            },
            DrawCommand::Circle {
                cx,
                cy,
                r: r * 0.15,
                stroke: None,
                fill: Some(color),
                stroke_w: 0.0,
            },
        ],
        // ─── Luna: crescent abriendo a la derecha ──────────────────
        // Dos curvas Bezier que conforman la media luna: la externa
        // bulga fuerte hacia la izquierda; la interna sólo levemente.
        // El espacio entre ambas es la parte rellena visualmente
        // (cuando hay fill) o la silueta del arco si solo hay trazo.
        "moon" => {
            let top_y = cy - r * 0.90;
            let bot_y = cy + r * 0.90;
            let anchor_x = cx + r * 0.25;
            let d = format!(
                "M {anchor_x} {top_y} Q {} {} {anchor_x} {bot_y} Q {} {} {anchor_x} {top_y} Z",
                cx - r * 1.00,
                cy,
                cx - r * 0.05,
                cy,
            );
            vec![DrawCommand::Path {
                d,
                stroke: Some(color),
                fill: None,
                stroke_w,
            }]
        }
        // ─── Mercurio: cuernos arriba + círculo + cruz abajo ──────
        "mercury" => mercury_commands(cx, cy, r, color, stroke_w),
        // ─── Venus: círculo + cruz abajo ───────────────────────────
        "venus" => circle_with_cross_below(cx, cy, r, color, stroke_w),
        // ─── Marte: círculo + flecha arriba-derecha ────────────────
        "mars" => mars_commands(cx, cy, r, color, stroke_w),
        // ─── Júpiter: gancho con barra horizontal ──────────────────
        "jupiter" => jupiter_commands(cx, cy, r, color, stroke_w),
        // ─── Saturno: cruz arriba + curva en gancho ────────────────
        "saturn" => saturn_commands(cx, cy, r, color, stroke_w),
        // ─── Urano: H con círculo abajo ────────────────────────────
        "uranus" => uranus_commands(cx, cy, r, color, stroke_w),
        // ─── Neptuno: tridente ─────────────────────────────────────
        "neptune" => neptune_commands(cx, cy, r, color, stroke_w),
        // ─── Plutón: círculo dentro de copa + cruz inferior ───────
        "pluto" => pluto_commands(cx, cy, r, color, stroke_w),
        // ─── Nodos ─────────────────────────────────────────────────
        "north_node" => node_commands(cx, cy, r, color, stroke_w, true),
        "south_node" => node_commands(cx, cy, r, color, stroke_w, false),
        // ─── Quirón: K con círculo abajo ───────────────────────────
        "chiron" => chiron_commands(cx, cy, r, color, stroke_w),
        // ─── Lilith: luna creciente con cruz abajo ─────────────────
        "lilith" => lilith_commands(cx, cy, r, color, stroke_w),
        // ─── Fallback: bullet ──────────────────────────────────────
        _ => vec![DrawCommand::Circle {
            cx,
            cy,
            r: r * 0.35,
            stroke: None,
            fill: Some(color),
            stroke_w: 0.0,
        }],
    }
}

/// Comando para el sufijo de retrógrado: un punto pequeño al lado
/// derecho-inferior del glyph (alternativa visual al unicode `℞`).
pub fn retrograde_marker(cx: f32, cy: f32, size: f32, color: Rgba) -> DrawCommand {
    let r = size * 0.5;
    DrawCommand::Circle {
        cx: cx + r * 0.95,
        cy: cy + r * 0.7,
        r: r * 0.14,
        stroke: None,
        fill: Some(color),
        stroke_w: 0.0,
    }
}

/// Devuelve los comandos para dibujar el glyph de un aspecto
/// (`"conjunction"`, `"opposition"`, `"trine"`, …) centrado en
/// `(cx, cy)`. Mismo motivo que [`planet_commands`]/[`sign_commands`]:
/// los unicode ☌☍△□⚹ caen como `.notdef` en las fuentes default. Si el
/// `kind` no es reconocido devuelve un punto relleno (bullet).
pub fn aspect_commands(
    kind: &str,
    cx: f32,
    cy: f32,
    size: f32,
    color: Rgba,
    stroke_w: f32,
) -> Vec<DrawCommand> {
    let r = size * 0.5;
    let stroke = Some(color);
    match kind {
        // ─── Conjunción ☌: círculo con cola hacia arriba-derecha ─────
        "conjunction" => {
            let bcx = cx - r * 0.18;
            let bcy = cy + r * 0.22;
            let br = r * 0.42;
            vec![
                DrawCommand::Circle {
                    cx: bcx,
                    cy: bcy,
                    r: br,
                    stroke,
                    fill: None,
                    stroke_w,
                },
                DrawCommand::Line {
                    x1: bcx + br * 0.55,
                    y1: bcy - br * 0.55,
                    x2: cx + r * 0.75,
                    y2: cy - r * 0.85,
                    color,
                    width: stroke_w,
                    dash: None,
                },
            ]
        }
        // ─── Oposición ☍: dos discos unidos por una recta ────────────
        "opposition" => {
            let dot = r * 0.22;
            vec![
                DrawCommand::Line {
                    x1: cx,
                    y1: cy - r * 0.7,
                    x2: cx,
                    y2: cy + r * 0.7,
                    color,
                    width: stroke_w,
                    dash: None,
                },
                DrawCommand::Circle {
                    cx,
                    cy: cy - r * 0.7,
                    r: dot,
                    stroke: None,
                    fill: Some(color),
                    stroke_w: 0.0,
                },
                DrawCommand::Circle {
                    cx,
                    cy: cy + r * 0.7,
                    r: dot,
                    stroke: None,
                    fill: Some(color),
                    stroke_w: 0.0,
                },
            ]
        }
        // ─── Trígono △: triángulo equilátero apuntando arriba ────────
        "trine" => vec![DrawCommand::Polygon {
            points: vec![
                (cx, cy - r * 0.8),
                (cx + r * 0.72, cy + r * 0.55),
                (cx - r * 0.72, cy + r * 0.55),
            ],
            fill: None,
            stroke,
            stroke_w,
        }],
        // ─── Cuadratura □: cuadrado ──────────────────────────────────
        "square" => {
            let s = r * 0.62;
            vec![DrawCommand::Polygon {
                points: vec![
                    (cx - s, cy - s),
                    (cx + s, cy - s),
                    (cx + s, cy + s),
                    (cx - s, cy + s),
                ],
                fill: None,
                stroke,
                stroke_w,
            }]
        }
        // ─── Sextil ✶: asterisco de 6 puntas (3 rectas por el centro) ─
        "sextile" => {
            let mut out = Vec::with_capacity(3);
            for k in 0..3 {
                let ang = std::f32::consts::PI * (k as f32) / 3.0 + std::f32::consts::FRAC_PI_2;
                let (s, c) = ang.sin_cos();
                out.push(DrawCommand::Line {
                    x1: cx - c * r * 0.8,
                    y1: cy - s * r * 0.8,
                    x2: cx + c * r * 0.8,
                    y2: cy + s * r * 0.8,
                    color,
                    width: stroke_w,
                    dash: None,
                });
            }
            out
        }
        // ─── Quincuncio ⚻: pico con tallo vertical (Y sin bifurcar) ──
        "quincunx" => vec![
            DrawCommand::Line {
                x1: cx,
                y1: cy + r * 0.8,
                x2: cx,
                y2: cy - r * 0.25,
                color,
                width: stroke_w,
                dash: None,
            },
            DrawCommand::Line {
                x1: cx,
                y1: cy - r * 0.25,
                x2: cx - r * 0.6,
                y2: cy - r * 0.8,
                color,
                width: stroke_w,
                dash: None,
            },
            DrawCommand::Line {
                x1: cx,
                y1: cy - r * 0.25,
                x2: cx + r * 0.6,
                y2: cy - r * 0.8,
                color,
                width: stroke_w,
                dash: None,
            },
        ],
        // ─── Semisextil: medio sextil (un chevron ∧) ─────────────────
        "semi_sextile" => vec![
            DrawCommand::Line {
                x1: cx - r * 0.6,
                y1: cy + r * 0.5,
                x2: cx,
                y2: cy - r * 0.5,
                color,
                width: stroke_w,
                dash: None,
            },
            DrawCommand::Line {
                x1: cx,
                y1: cy - r * 0.5,
                x2: cx + r * 0.6,
                y2: cy + r * 0.5,
                color,
                width: stroke_w,
                dash: None,
            },
        ],
        // ─── Semicuadratura ∠: ángulo recto abierto ──────────────────
        "semi_square" => vec![
            DrawCommand::Line {
                x1: cx - r * 0.65,
                y1: cy + r * 0.55,
                x2: cx + r * 0.65,
                y2: cy + r * 0.55,
                color,
                width: stroke_w,
                dash: None,
            },
            DrawCommand::Line {
                x1: cx - r * 0.65,
                y1: cy + r * 0.55,
                x2: cx + r * 0.4,
                y2: cy - r * 0.6,
                color,
                width: stroke_w,
                dash: None,
            },
        ],
        // ─── Sesquicuadratura: ángulo + tilde (semicuadratura×1.5) ───
        "sesquiquadrate" => vec![
            DrawCommand::Line {
                x1: cx - r * 0.65,
                y1: cy + r * 0.55,
                x2: cx + r * 0.65,
                y2: cy + r * 0.55,
                color,
                width: stroke_w,
                dash: None,
            },
            DrawCommand::Line {
                x1: cx - r * 0.65,
                y1: cy + r * 0.55,
                x2: cx + r * 0.4,
                y2: cy - r * 0.6,
                color,
                width: stroke_w,
                dash: None,
            },
            DrawCommand::Line {
                x1: cx + r * 0.15,
                y1: cy - r * 0.7,
                x2: cx + r * 0.7,
                y2: cy - r * 0.2,
                color,
                width: stroke_w,
                dash: None,
            },
        ],
        // Fallback: bullet relleno.
        _ => vec![DrawCommand::Circle {
            cx,
            cy,
            r: r * 0.22,
            stroke: None,
            fill: Some(color),
            stroke_w: 0.0,
        }],
    }
}

// =====================================================================
// Implementaciones por planeta
// =====================================================================

fn mercury_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // Cuernos arriba (semicírculo invertido), círculo central, cruz
    // inferior. Layout vertical: -1.0..-0.55 cuernos, -0.55..0.1 circle,
    // 0.1..1.0 cruz.
    let body_r = r * 0.30;
    let body_cy = cy - r * 0.05;
    let horns_top = cy - r * 0.95;
    let horns_open = cy - r * 0.50;
    let cross_top = body_cy + body_r;
    let cross_bot = cy + r * 0.95;
    let cross_hx = r * 0.30;
    let cross_hy = cy + r * 0.55;
    let d_horns = format!(
        "M {} {} A {} {} 0 0 0 {} {}",
        cx - body_r * 1.1,
        horns_open,
        body_r * 1.1,
        body_r * 0.9,
        cx + body_r * 1.1,
        horns_open
    );
    // Top de los cuernos: dos pequeñas líneas verticales conectando el
    // arco con `horns_top` (puntas).
    let d_horn_tips_l = format!(
        "M {} {} L {} {}",
        cx - body_r * 1.1,
        horns_open,
        cx - body_r * 1.1,
        horns_top
    );
    let d_horn_tips_r = format!(
        "M {} {} L {} {}",
        cx + body_r * 1.1,
        horns_open,
        cx + body_r * 1.1,
        horns_top
    );
    vec![
        // Cuernos
        DrawCommand::Path {
            d: d_horns,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Path {
            d: d_horn_tips_l,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Path {
            d: d_horn_tips_r,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        // Cuerpo
        DrawCommand::Circle {
            cx,
            cy: body_cy,
            r: body_r,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        // Cruz
        DrawCommand::Line {
            x1: cx,
            y1: cross_top,
            x2: cx,
            y2: cross_bot,
            color,
            width: sw,
            dash: None,
        },
        DrawCommand::Line {
            x1: cx - cross_hx,
            y1: cross_hy,
            x2: cx + cross_hx,
            y2: cross_hy,
            color,
            width: sw,
            dash: None,
        },
    ]
}

fn circle_with_cross_below(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // Venus: círculo + cruz abajo. Layout: círculo en mitad superior,
    // cruz en mitad inferior.
    let body_r = r * 0.40;
    let body_cy = cy - r * 0.30;
    let cross_top = body_cy + body_r;
    let cross_bot = cy + r * 0.95;
    let cross_h = r * 0.35;
    let cross_hy = (cross_top + cross_bot) * 0.5;
    vec![
        DrawCommand::Circle {
            cx,
            cy: body_cy,
            r: body_r,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Line {
            x1: cx,
            y1: cross_top,
            x2: cx,
            y2: cross_bot,
            color,
            width: sw,
            dash: None,
        },
        DrawCommand::Line {
            x1: cx - cross_h,
            y1: cross_hy,
            x2: cx + cross_h,
            y2: cross_hy,
            color,
            width: sw,
            dash: None,
        },
    ]
}

fn mars_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // Marte: círculo + flecha en dirección 45° (arriba-derecha).
    let body_r = r * 0.40;
    let body_cx = cx - r * 0.15;
    let body_cy = cy + r * 0.15;
    // Punto de salida en la circunferencia a 45° arriba-derecha.
    let exit_x = body_cx + body_r * 0.707;
    let exit_y = body_cy - body_r * 0.707;
    let tip_x = cx + r * 0.85;
    let tip_y = cy - r * 0.85;
    // Cabeza de flecha: dos líneas cortas formando "<".
    let head_len = r * 0.32;
    // Direcciones perpendiculares al eje flecha (que va a 45° arriba-
    // derecha = dir (1,-1)/√2). Perpendicular = (1,1)/√2 y (-1,-1)/√2.
    let head_dx = head_len * 0.5; // proyección de cada barb sobre x
    let head_dy = head_len * 0.5;
    vec![
        DrawCommand::Circle {
            cx: body_cx,
            cy: body_cy,
            r: body_r,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        // Cuerpo de la flecha
        DrawCommand::Line {
            x1: exit_x,
            y1: exit_y,
            x2: tip_x,
            y2: tip_y,
            color,
            width: sw,
            dash: None,
        },
        // Barb hacia abajo-derecha (línea perpendicular al eje)
        DrawCommand::Line {
            x1: tip_x,
            y1: tip_y,
            x2: tip_x - head_dx,
            y2: tip_y,
            color,
            width: sw,
            dash: None,
        },
        // Barb hacia arriba-izquierda (línea perpendicular al eje)
        DrawCommand::Line {
            x1: tip_x,
            y1: tip_y,
            x2: tip_x,
            y2: tip_y + head_dy,
            color,
            width: sw,
            dash: None,
        },
    ]
}

fn jupiter_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // Júpiter: forma de "21" — barra horizontal arriba, curva en gancho
    // desde la mitad de la barra hacia abajo y a la derecha.
    // Reconocible: una curva tipo "4" o un "2½" estilizado.
    let top = cy - r * 0.7;
    let mid = cy;
    let bot = cy + r * 0.7;
    let left = cx - r * 0.6;
    let right = cx + r * 0.5;
    // Path: línea horizontal arriba + curva que cae y termina en gancho.
    let d = format!(
        "M {left} {top} L {} {top} M {} {top} C {} {mid}, {} {mid}, {} {} L {} {} A {} {} 0 0 0 {} {}",
        cx,
        cx,
        cx,
        right,
        right,
        bot - r * 0.25,
        right,
        bot,
        r * 0.3,
        r * 0.3,
        cx,
        bot
    );
    vec![DrawCommand::Path {
        d,
        stroke: Some(color),
        fill: None,
        stroke_w: sw,
    }]
}

fn saturn_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // Saturno: cruz arriba + vertical largo + gancho abajo (anillo
    // simbólico). Forma de "h" cursivo coronado por una cruz.
    let cross_top = cy - r * 0.95;
    let cross_h = r * 0.25;
    let cross_hy = cy - r * 0.55;
    let v_top = cross_hy;
    let v_bot = cy + r * 0.35;
    let hook_end_x = cx + r * 0.55;
    let hook_end_y = cy + r * 0.95;
    vec![
        // Brazo vertical de la cruz
        DrawCommand::Line {
            x1: cx,
            y1: cross_top,
            x2: cx,
            y2: v_top + (cross_hy - cross_top), // hasta el cruce
            color,
            width: sw,
            dash: None,
        },
        // Brazo horizontal de la cruz
        DrawCommand::Line {
            x1: cx - cross_h,
            y1: cross_hy,
            x2: cx + cross_h,
            y2: cross_hy,
            color,
            width: sw,
            dash: None,
        },
        // Vertical largo
        DrawCommand::Line {
            x1: cx,
            y1: cross_hy,
            x2: cx,
            y2: v_bot,
            color,
            width: sw,
            dash: None,
        },
        // Gancho final (curva desde v_bot hacia hook_end)
        DrawCommand::Path {
            d: format!(
                "M {} {} Q {} {}, {} {}",
                cx,
                v_bot,
                cx,
                hook_end_y,
                hook_end_x,
                hook_end_y
            ),
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
    ]
}

fn uranus_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // Urano (forma alquímica): dos paréntesis opuestos atravesados
    // por una barra horizontal (silueta tipo Piscis ¦)¦|⦧) con un
    // círculo colgando debajo. Variante astrológica clásica del
    // glyph "alquímico" (la otra es la "H" de Herschel — descartada
    // por petición del usuario: más reconocible la versión Piscis +
    // círculo).
    let top = cy - r * 0.85;
    let bot_open = cy + r * 0.15;
    let bar_y = cy - r * 0.30;
    let bracket_r = r * 0.45;
    let circ_cy = cy + r * 0.62;
    let circ_r = r * 0.22;
    let left_x = cx - r * 0.55;
    let right_x = cx + r * 0.55;
    // Bracket izquierdo (") apertura a la derecha)
    let d_left = format!(
        "M {} {} A {bracket_r} {bracket_r} 0 0 1 {} {}",
        left_x, top, left_x, bot_open
    );
    // Bracket derecho (apertura a la izquierda)
    let d_right = format!(
        "M {} {} A {bracket_r} {bracket_r} 0 0 0 {} {}",
        right_x, top, right_x, bot_open
    );
    vec![
        DrawCommand::Path {
            d: d_left,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Path {
            d: d_right,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        // Barra horizontal que atraviesa
        DrawCommand::Line {
            x1: left_x - r * 0.05,
            y1: bar_y,
            x2: right_x + r * 0.05,
            y2: bar_y,
            color,
            width: sw,
            dash: None,
        },
        // Conector vertical hacia el círculo inferior
        DrawCommand::Line {
            x1: cx,
            y1: bot_open,
            x2: cx,
            y2: circ_cy - circ_r,
            color,
            width: sw,
            dash: None,
        },
        // Círculo
        DrawCommand::Circle {
            cx,
            cy: circ_cy,
            r: circ_r,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
    ]
}

fn neptune_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // Neptuno: tridente. Tres puntas arriba, mango central abajo, una
    // barra horizontal cerca del fondo.
    let top = cy - r * 0.95;
    let mid = cy - r * 0.10;
    let bot = cy + r * 0.65;
    let cross_y = cy + r * 0.35;
    let arm = r * 0.50;
    let d_trident = format!(
        "M {} {top} L {} {} A {arm} {arm} 0 0 0 {} {} L {} {top}",
        cx - arm,
        cx - arm,
        mid,
        cx + arm,
        mid,
        cx + arm
    );
    vec![
        // Tridente (U invertida con dos puntas hacia arriba)
        DrawCommand::Path {
            d: d_trident,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        // Mango central
        DrawCommand::Line {
            x1: cx,
            y1: top,
            x2: cx,
            y2: bot,
            color,
            width: sw,
            dash: None,
        },
        // Cruz inferior
        DrawCommand::Line {
            x1: cx - r * 0.30,
            y1: cross_y,
            x2: cx + r * 0.30,
            y2: cross_y,
            color,
            width: sw,
            dash: None,
        },
    ]
}

fn pluto_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // Plutón: copa abierta arriba con un círculo dentro + cruz abajo.
    let body_r = r * 0.32;
    let body_cy = cy - r * 0.35;
    let cup_top = cy - r * 0.95;
    let cup_open_y = body_cy - body_r * 0.2;
    let cross_top = body_cy + body_r;
    let cross_bot = cy + r * 0.95;
    let cross_hy = cy + r * 0.55;
    let cross_h = r * 0.30;
    let d_cup = format!(
        "M {} {} L {} {} A {} {} 0 0 0 {} {} L {} {}",
        cx - r * 0.45,
        cup_open_y,
        cx - r * 0.45,
        cup_top,
        r * 0.45,
        r * 0.45,
        cx + r * 0.45,
        cup_top,
        cx + r * 0.45,
        cup_open_y
    );
    vec![
        DrawCommand::Path {
            d: d_cup,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Circle {
            cx,
            cy: body_cy,
            r: body_r,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Line {
            x1: cx,
            y1: cross_top,
            x2: cx,
            y2: cross_bot,
            color,
            width: sw,
            dash: None,
        },
        DrawCommand::Line {
            x1: cx - cross_h,
            y1: cross_hy,
            x2: cx + cross_h,
            y2: cross_hy,
            color,
            width: sw,
            dash: None,
        },
    ]
}

fn node_commands(
    cx: f32,
    cy: f32,
    r: f32,
    color: Rgba,
    sw: f32,
    north: bool,
) -> Vec<DrawCommand> {
    // Nodo lunar: una herradura con un par de circulitos en las
    // puntas. North (☊) = bowl ARRIBA, ears apuntando hacia abajo.
    // South (☋) = espejo vertical — bowl ABAJO, ears apuntando hacia
    // arriba. La diferencia entre ambos es solo orientación.
    //
    // Construimos el path con líneas + un arco semicircular para que
    // el sweep_flag no dependa de la orientación (la simetría la
    // controlamos por la elección de top/bottom).
    let arm = r * 0.50;
    let ear_r = r * 0.18;
    let bowl_h = r * 0.55;
    let leg_h = r * 0.50;
    let (bowl_y_outer, leg_y_inner, sweep) = if north {
        // North: bowl arriba. Endpoints del arco en cy - bowl_h/2,
        // legs bajan a cy + leg_h/2.
        (cy - bowl_h * 0.5, cy + leg_h * 0.5, 1_u8)
    } else {
        // South: bowl abajo. Endpoints arriba, legs hacia arriba.
        (cy + bowl_h * 0.5, cy - leg_h * 0.5, 0_u8)
    };
    let d = format!(
        "M {lx} {leg_y_inner} L {lx} {bowl_y_outer} A {arm} {arm} 0 0 {sweep} {rx} {bowl_y_outer} L {rx} {leg_y_inner}",
        lx = cx - arm,
        rx = cx + arm,
    );
    vec![
        DrawCommand::Path {
            d,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Circle {
            cx: cx - arm,
            cy: leg_y_inner,
            r: ear_r,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Circle {
            cx: cx + arm,
            cy: leg_y_inner,
            r: ear_r,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
    ]
}

fn chiron_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // Quirón: una "K" estilizada con un círculo pequeño abajo (la
    // forma clásica del asteroide).
    let top = cy - r * 0.95;
    let mid = cy - r * 0.10;
    let circ_r = r * 0.27;
    let circ_cy = cy + r * 0.55;
    vec![
        // Vertical izquierda
        DrawCommand::Line {
            x1: cx - r * 0.35,
            y1: top,
            x2: cx - r * 0.35,
            y2: circ_cy - circ_r,
            color,
            width: sw,
            dash: None,
        },
        // Brazo superior diagonal
        DrawCommand::Line {
            x1: cx - r * 0.35,
            y1: mid,
            x2: cx + r * 0.40,
            y2: top,
            color,
            width: sw,
            dash: None,
        },
        // Brazo inferior diagonal (vuelve hacia abajo)
        DrawCommand::Line {
            x1: cx - r * 0.35,
            y1: mid,
            x2: cx + r * 0.20,
            y2: circ_cy - circ_r - r * 0.05,
            color,
            width: sw,
            dash: None,
        },
        // Círculo
        DrawCommand::Circle {
            cx,
            cy: circ_cy,
            r: circ_r,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
    ]
}

fn lilith_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // Lilith (Black Moon): crescent invertida con cruz al pie.
    let cres_r = r * 0.55;
    let cres_cy = cy - r * 0.20;
    let cross_top = cres_cy + cres_r;
    let cross_bot = cy + r * 0.95;
    let cross_hy = cy + r * 0.55;
    let cross_h = r * 0.28;
    // Crescent: cara abierta hacia abajo.
    let inner_r = cres_r * 0.55;
    let d = format!(
        "M {} {} A {cres_r} {cres_r} 0 0 1 {} {} A {} {} 0 0 0 {} {} Z",
        cx - cres_r,
        cres_cy,
        cx + cres_r,
        cres_cy,
        cres_r,
        inner_r,
        cx - cres_r,
        cres_cy
    );
    vec![
        DrawCommand::Path {
            d,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Line {
            x1: cx,
            y1: cross_top,
            x2: cx,
            y2: cross_bot,
            color,
            width: sw,
            dash: None,
        },
        DrawCommand::Line {
            x1: cx - cross_h,
            y1: cross_hy,
            x2: cx + cross_h,
            y2: cross_hy,
            color,
            width: sw,
            dash: None,
        },
    ]
}

// =====================================================================
// Signos zodiacales
// =====================================================================

/// Devuelve los comandos para dibujar el glyph de un signo. Si el
/// nombre no es uno reconocido, devuelve un cuadradito.
pub fn sign_commands(
    name: &str,
    cx: f32,
    cy: f32,
    size: f32,
    color: Rgba,
    stroke_w: f32,
) -> Vec<DrawCommand> {
    let r = size * 0.5;
    match name {
        "aries" => aries_commands(cx, cy, r, color, stroke_w),
        "taurus" => taurus_commands(cx, cy, r, color, stroke_w),
        "gemini" => gemini_commands(cx, cy, r, color, stroke_w),
        "cancer" => cancer_commands(cx, cy, r, color, stroke_w),
        "leo" => leo_commands(cx, cy, r, color, stroke_w),
        "virgo" => virgo_commands(cx, cy, r, color, stroke_w),
        "libra" => libra_commands(cx, cy, r, color, stroke_w),
        "scorpio" => scorpio_commands(cx, cy, r, color, stroke_w),
        "sagittarius" => sagittarius_commands(cx, cy, r, color, stroke_w),
        "capricorn" => capricorn_commands(cx, cy, r, color, stroke_w),
        "aquarius" => aquarius_commands(cx, cy, r, color, stroke_w),
        "pisces" => pisces_commands(cx, cy, r, color, stroke_w),
        _ => vec![DrawCommand::Polygon {
            points: vec![
                (cx - r * 0.3, cy - r * 0.3),
                (cx + r * 0.3, cy - r * 0.3),
                (cx + r * 0.3, cy + r * 0.3),
                (cx - r * 0.3, cy + r * 0.3),
            ],
            fill: None,
            stroke: Some(color),
            stroke_w,
        }],
    }
}

fn aries_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // ♈ Aries: dos cuernos de carnero que parten de un ápice central
    // arriba, bajan diagonal hacia los lados, y se enroscan hacia
    // adentro al final. Estilo Y con curls.
    //
    //         /\          ← ápice
    //        /  \
    //       /    \        ← flancos
    //      (      )       ← curls cerrando hacia adentro
    //       \____/
    let apex_x = cx;
    let apex_y = cy - r * 0.85;
    // Punto donde el flanco se convierte en curl (extremo exterior).
    let curl_outer_y = cy + r * 0.20;
    let curl_outer_dx = r * 0.65;
    // Punto final del curl (hacia adentro, ligeramente arriba del
    // máximo de la curva para dar sensación de enroscar).
    let curl_inner_y = cy + r * 0.15;
    let curl_inner_dx = r * 0.10;
    // Profundidad del curl (cuánto baja antes de subir).
    let curl_bottom_y = cy + r * 0.75;
    // Trazo izquierdo: línea diagonal desde apex hasta el extremo
    // exterior, después una curva Bezier que baja, redondea y vuelve
    // hacia el centro-arriba.
    let left = format!(
        "M {apex_x} {apex_y} L {} {curl_outer_y} C {} {}, {} {}, {} {curl_inner_y}",
        cx - curl_outer_dx,
        cx - curl_outer_dx - r * 0.05,
        curl_bottom_y,
        cx - curl_inner_dx - r * 0.05,
        curl_bottom_y - r * 0.05,
        cx - curl_inner_dx,
    );
    let right = format!(
        "M {apex_x} {apex_y} L {} {curl_outer_y} C {} {}, {} {}, {} {curl_inner_y}",
        cx + curl_outer_dx,
        cx + curl_outer_dx + r * 0.05,
        curl_bottom_y,
        cx + curl_inner_dx + r * 0.05,
        curl_bottom_y - r * 0.05,
        cx + curl_inner_dx,
    );
    vec![
        DrawCommand::Path {
            d: left,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Path {
            d: right,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
    ]
}

fn taurus_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // ♉ Tauro: cara (círculo) abajo y dos cuernos sobre la cara,
    // dibujados como un arco con concavidad hacia abajo (las puntas
    // suben). Equivale al símbolo unicode ♉: ∪ encima de O, donde el
    // ∪ tiene el bowl tocando arriba del círculo y las tips
    // apuntando hacia arriba.
    let body_r = r * 0.38;
    let body_cy = cy + r * 0.30;
    let tip_y = cy - r * 0.85;
    let arm = r * 0.70;
    // Arc bulging DOWN (concavidad hacia abajo): de (cx-arm, tip_y)
    // a (cx+arm, tip_y) pasando por (cx, ~body_cy - body_r). En SVG
    // y-down, sweep_flag=1 va clockwise visualmente, lo que para
    // endpoints en la misma altura significa bulge HACIA ARRIBA. Para
    // bulge DOWN usamos sweep_flag=0.
    let d_horns = format!(
        "M {} {} A {arm} {arm} 0 0 0 {} {}",
        cx - arm,
        tip_y,
        cx + arm,
        tip_y,
    );
    vec![
        DrawCommand::Path {
            d: d_horns,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Circle {
            cx,
            cy: body_cy,
            r: body_r,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
    ]
}

fn gemini_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // ♊ Géminis (gemelos): dos verticales paralelos como las
    // columnas, con techo y piso rectos que las cierran. Forma de
    // "Π" sobre su espejo. El usuario pidió la versión rectángulo
    // limpio (no las barras curvas).
    let top = cy - r * 0.75;
    let bot = cy + r * 0.75;
    let arm = r * 0.40;
    let overhang = r * 0.10;
    vec![
        // Vertical izquierda
        DrawCommand::Line {
            x1: cx - arm,
            y1: top,
            x2: cx - arm,
            y2: bot,
            color,
            width: sw,
            dash: None,
        },
        // Vertical derecha
        DrawCommand::Line {
            x1: cx + arm,
            y1: top,
            x2: cx + arm,
            y2: bot,
            color,
            width: sw,
            dash: None,
        },
        // Techo
        DrawCommand::Line {
            x1: cx - arm - overhang,
            y1: top,
            x2: cx + arm + overhang,
            y2: top,
            color,
            width: sw,
            dash: None,
        },
        // Piso
        DrawCommand::Line {
            x1: cx - arm - overhang,
            y1: bot,
            x2: cx + arm + overhang,
            y2: bot,
            color,
            width: sw,
            dash: None,
        },
    ]
}

fn cancer_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // ♋ Cáncer: dos espirales (69 acostado). Simplificamos: dos
    // círculos pequeños con un mango cada uno, opuestos.
    let circ_r = r * 0.20;
    // Círculo superior derecho con mango hacia la izquierda.
    let c1_cx = cx + r * 0.40;
    let c1_cy = cy - r * 0.35;
    // Círculo inferior izquierdo con mango hacia la derecha.
    let c2_cx = cx - r * 0.40;
    let c2_cy = cy + r * 0.35;
    vec![
        DrawCommand::Circle {
            cx: c1_cx,
            cy: c1_cy,
            r: circ_r,
            stroke: None,
            fill: Some(color),
            stroke_w: 0.0,
        },
        DrawCommand::Circle {
            cx: c2_cx,
            cy: c2_cy,
            r: circ_r,
            stroke: None,
            fill: Some(color),
            stroke_w: 0.0,
        },
        // Mango sup: arco que baja desde c1 hacia abajo-izq.
        DrawCommand::Path {
            d: format!(
                "M {} {} A {} {} 0 0 0 {} {}",
                c1_cx - circ_r,
                c1_cy,
                r * 0.55,
                r * 0.55,
                cx - r * 0.80,
                cy + r * 0.10
            ),
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        // Mango inf: arco que sube desde c2 hacia arriba-der.
        DrawCommand::Path {
            d: format!(
                "M {} {} A {} {} 0 0 0 {} {}",
                c2_cx + circ_r,
                c2_cy,
                r * 0.55,
                r * 0.55,
                cx + r * 0.80,
                cy - r * 0.10
            ),
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
    ]
}

fn leo_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // ♌ Leo: círculo arriba + cola curva tipo S hacia abajo-derecha.
    let head_r = r * 0.30;
    let head_cy = cy - r * 0.40;
    let d_tail = format!(
        "M {} {} C {} {}, {} {}, {} {} C {} {}, {} {}, {} {}",
        cx + head_r * 0.6,
        head_cy + head_r * 0.6,
        cx + r * 0.5,
        cy,
        cx + r * 0.7,
        cy + r * 0.3,
        cx + r * 0.4,
        cy + r * 0.6,
        cx + r * 0.05,
        cy + r * 0.85,
        cx - r * 0.20,
        cy + r * 0.80,
        cx - r * 0.45,
        cy + r * 0.60
    );
    vec![
        DrawCommand::Circle {
            cx,
            cy: head_cy,
            r: head_r,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Path {
            d: d_tail,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
    ]
}

fn virgo_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // ♍ Virgo: "M" con cola que se enrosca a la derecha al final.
    let top = cy - r * 0.60;
    let bot = cy + r * 0.55;
    let step = r * 0.30;
    let d = format!(
        "M {} {} L {} {} L {} {} L {} {} L {} {} L {} {} M {} {} C {} {}, {} {}, {} {}",
        cx - step * 2.2,
        bot,
        cx - step * 2.2,
        top,
        cx - step * 0.5,
        bot,
        cx - step * 0.5,
        top,
        cx + step * 1.2,
        bot,
        cx + step * 1.2,
        top,
        cx + step * 1.2,
        bot,
        cx + step * 1.8,
        bot - r * 0.05,
        cx + step * 1.6,
        bot - r * 0.40,
        cx + step * 0.7,
        bot - r * 0.25
    );
    vec![DrawCommand::Path {
        d,
        stroke: Some(color),
        fill: None,
        stroke_w: sw,
    }]
}

fn libra_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // ♎ Libra: balanza — barra horizontal con un domito más pequeño
    // arriba. El usuario pidió achicar la curva top — antes era casi
    // la mitad del ancho; ahora ~ 1/3.
    let top_y = cy + r * 0.05;
    let bot_y = cy + r * 0.55;
    let bar_h = r * 0.85;
    let arc_r = r * 0.28;
    let d_dome = format!(
        "M {} {top_y} A {arc_r} {arc_r} 0 0 1 {} {top_y}",
        cx - arc_r,
        cx + arc_r,
    );
    vec![
        DrawCommand::Path {
            d: d_dome,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        // Barras horizontales que extienden el domo a los lados
        DrawCommand::Line {
            x1: cx - bar_h * 0.55,
            y1: top_y,
            x2: cx - arc_r,
            y2: top_y,
            color,
            width: sw,
            dash: None,
        },
        DrawCommand::Line {
            x1: cx + arc_r,
            y1: top_y,
            x2: cx + bar_h * 0.55,
            y2: top_y,
            color,
            width: sw,
            dash: None,
        },
        // Línea horizontal inferior (la balanza)
        DrawCommand::Line {
            x1: cx - bar_h * 0.55,
            y1: bot_y,
            x2: cx + bar_h * 0.55,
            y2: bot_y,
            color,
            width: sw,
            dash: None,
        },
    ]
}

fn scorpio_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // ♏ Escorpio: "M" con flecha al final (cola).
    let top = cy - r * 0.60;
    let bot = cy + r * 0.55;
    let step = r * 0.30;
    let arrow_tip_x = cx + step * 2.0;
    let arrow_tip_y = cy - r * 0.20;
    let d = format!(
        "M {} {} L {} {} L {} {} L {} {} L {} {} L {} {} L {} {} M {} {} L {} {} L {} {}",
        cx - step * 2.2,
        bot,
        cx - step * 2.2,
        top,
        cx - step * 0.5,
        bot,
        cx - step * 0.5,
        top,
        cx + step * 1.2,
        bot,
        cx + step * 1.2,
        top,
        cx + step * 1.2,
        bot,
        cx + step * 1.2,
        bot,
        arrow_tip_x,
        bot,
        arrow_tip_x,
        arrow_tip_y
    );
    // Cabeza de flecha
    let d_head = format!(
        "M {} {} L {} {} M {} {} L {} {}",
        arrow_tip_x,
        arrow_tip_y,
        arrow_tip_x - r * 0.18,
        arrow_tip_y + r * 0.18,
        arrow_tip_x,
        arrow_tip_y,
        arrow_tip_x + r * 0.18,
        arrow_tip_y + r * 0.18
    );
    vec![
        DrawCommand::Path {
            d,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Path {
            d: d_head,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
    ]
}

fn sagittarius_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // ♐ Sagitario: flecha diagonal arriba-derecha + barra cruzando.
    let tip_x = cx + r * 0.75;
    let tip_y = cy - r * 0.75;
    let tail_x = cx - r * 0.75;
    let tail_y = cy + r * 0.75;
    // Cuerpo de la flecha
    let d_body = format!("M {tail_x} {tail_y} L {tip_x} {tip_y}");
    // Cabeza
    let d_head = format!(
        "M {tip_x} {tip_y} L {} {} M {tip_x} {tip_y} L {} {}",
        tip_x - r * 0.40,
        tip_y,
        tip_x,
        tip_y + r * 0.40
    );
    // Barra cruzando el cuerpo (típico de sagitario)
    let mid_x = (tail_x + tip_x) * 0.5;
    let mid_y = (tail_y + tip_y) * 0.5;
    let d_cross = format!(
        "M {} {} L {} {}",
        mid_x - r * 0.22,
        mid_y - r * 0.22,
        mid_x + r * 0.22,
        mid_y + r * 0.22
    );
    vec![
        DrawCommand::Path {
            d: d_body,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Path {
            d: d_head,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Path {
            d: d_cross,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
    ]
}

fn capricorn_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // ♑ Capricornio: cabra-pez. Dos trazos diagonales (cabra) que
    // bajan en zig-zag desde una punta superior-izquierda hasta el
    // medio-derecha, y desde ahí un lazo de cola de pez que
    // se enrosca debajo. Distinta de Escorpio (que tiene una M con
    // flecha) — la silueta de Capricornio es angular arriba +
    // curva cerrada abajo.
    let top_y = cy - r * 0.70;
    let mid_y = cy + r * 0.10;
    let loop_y = cy + r * 0.55;
    // Punto inicial (top-left, "punta" del cuerno de la cabra)
    let p1_x = cx - r * 0.65;
    let p1_y = top_y + r * 0.20;
    // Vértice de la N — abajo en el centro
    let p2_x = cx - r * 0.15;
    let p2_y = mid_y;
    // Subida al centro-arriba (el dorso de la cabra)
    let p3_x = cx + r * 0.05;
    let p3_y = top_y;
    // Bajada al inicio del loop (donde empieza la cola)
    let p4_x = cx + r * 0.20;
    let p4_y = mid_y;
    // Trazo angular cabra: p1 → p2 → p3 → p4
    let cabra = format!(
        "M {p1_x} {p1_y} L {p2_x} {p2_y} L {p3_x} {p3_y} L {p4_x} {p4_y}"
    );
    // Cola: lazo que sale de p4 hacia abajo-derecha, dobla y vuelve.
    let cola = format!(
        "M {p4_x} {p4_y} C {} {}, {} {}, {} {} C {} {}, {} {}, {} {} Z",
        // Sale hacia abajo-derecha
        cx + r * 0.65, mid_y,
        cx + r * 0.65, loop_y,
        cx + r * 0.20, loop_y + r * 0.05,
        // Cierra el lazo volviendo hacia arriba-izquierda
        cx - r * 0.05, loop_y,
        cx + r * 0.05, mid_y + r * 0.15,
        p4_x, p4_y,
    );
    vec![
        DrawCommand::Path {
            d: cabra,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Path {
            d: cola,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
    ]
}

fn aquarius_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // ♒ Acuario: dos waves (zigzags suaves) horizontales paralelas.
    let wave = |y: f32| -> String {
        let step = r * 0.30;
        format!(
            "M {} {} L {} {} L {} {} L {} {} L {} {}",
            cx - r * 0.85,
            y,
            cx - step,
            y - r * 0.20,
            cx,
            y,
            cx + step,
            y - r * 0.20,
            cx + r * 0.85,
            y
        )
    };
    vec![
        DrawCommand::Path {
            d: wave(cy - r * 0.25),
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Path {
            d: wave(cy + r * 0.30),
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
    ]
}

fn pisces_commands(cx: f32, cy: f32, r: f32, color: Rgba, sw: f32) -> Vec<DrawCommand> {
    // ♓ Piscis: dos paréntesis opuestos conectados por una barra.
    let arc_r = r * 0.55;
    let d_left = format!(
        "M {} {} A {arc_r} {arc_r} 0 0 1 {} {}",
        cx - r * 0.55,
        cy - r * 0.55,
        cx - r * 0.55,
        cy + r * 0.55
    );
    let d_right = format!(
        "M {} {} A {arc_r} {arc_r} 0 0 0 {} {}",
        cx + r * 0.55,
        cy - r * 0.55,
        cx + r * 0.55,
        cy + r * 0.55
    );
    vec![
        DrawCommand::Path {
            d: d_left,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        DrawCommand::Path {
            d: d_right,
            stroke: Some(color),
            fill: None,
            stroke_w: sw,
        },
        // Barra horizontal central
        DrawCommand::Line {
            x1: cx - r * 0.55,
            y1: cy,
            x2: cx + r * 0.55,
            y2: cy,
            color,
            width: sw,
            dash: None,
        },
    ]
}
