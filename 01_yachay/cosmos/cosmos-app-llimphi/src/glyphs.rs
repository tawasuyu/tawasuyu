//! Glyphs e iconos como **mini-canvas vectorial** — la pieza que mata
//! los tofus de la app.
//!
//! Nada de unicode astrológico (☉☽♈…☌△) ni dingbats (✎✂🗑⚙) como texto:
//! las fuentes default del sistema (LiberationSans/AdwaitaSans) no traen
//! esos bloques y caen como `.notdef`. En su lugar todo se dibuja como
//! geometría (`DrawCommand`) y se pinta con el mismo canvas vello que la
//! rueda (`cosmos_canvas_llimphi::canvas_view`).
//!
//! Tres familias:
//! - **cuerpos** (`body_view`) — planetas/luminarias/nodos vía
//!   `cosmos_render::glyphs::planet_commands`; los puntos del chart
//!   (Asc/MC/…) caen a texto ASCII corto.
//! - **signos** (`sign_view`) y **aspectos** (`aspect_view`) — paths
//!   propios de `cosmos_render::glyphs`.
//! - **iconos de chrome** (`icon_view`) — set vectorial hecho a mano
//!   para la botonera, el rail, las pestañas y el árbol.

use cosmos_canvas_llimphi::canvas_view;
use cosmos_model::ChartKind;
use cosmos_render::glyphs::{aspect_commands, planet_commands, sign_commands};
use cosmos_render::{DrawCommand, Palette, Rgba};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

use crate::format::simbolo_cuerpo;

/// Ids zodiacales en orden — index = longitud / 30.
pub(crate) const SIGN_IDS: [&str; 12] = [
    "aries",
    "taurus",
    "gemini",
    "cancer",
    "leo",
    "virgo",
    "libra",
    "scorpio",
    "sagittarius",
    "capricorn",
    "aquarius",
    "pisces",
];

/// Id del signo (en inglés, para los glyph paths) de una longitud.
pub(crate) fn sign_id(deg: f32) -> &'static str {
    SIGN_IDS[((deg.rem_euclid(360.0) / 30.0) as usize) % 12]
}

/// Cuerpos con glyph vectorial propio en `planet_commands`.
const PLANET_GLYPHS: &[&str] = &[
    "sun",
    "moon",
    "mercury",
    "venus",
    "mars",
    "jupiter",
    "saturn",
    "uranus",
    "neptune",
    "pluto",
    "earth",
    "north_node",
    "south_node",
    "chiron",
    "lilith",
];

/// Normaliza alias de cuerpos al id que entiende `planet_commands`.
fn canon_body(name: &str) -> &str {
    match name {
        "ascending_node" | "mean_node" => "north_node",
        "descending_node" => "south_node",
        other => other,
    }
}

fn rgba(c: Color) -> Rgba {
    let [r, g, b, a] = c.components;
    Rgba { r, g, b, a }
}

/// Grosor de trazo proporcional al tamaño de la celda.
fn sw(px: f32) -> f32 {
    (px * 0.085).clamp(1.1, 3.0)
}

/// Caja cuadrada `px` que pinta `cmds` (centrados en `px/2`) con vello.
fn cell<Msg: Clone + 'static>(cmds: Vec<DrawCommand>, px: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(px),
            height: length(px),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![canvas_view::<Msg>(cmds, px, None)])
}

/// Celda de texto corto (para puntos del chart sin glyph: Asc/MC/…).
fn text_cell<Msg: Clone + 'static>(txt: &str, w: f32, px: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(w),
            height: length(px),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(txt.to_string(), (px * 0.62).clamp(9.0, 12.0), color, Alignment::Center)
}

/// Glyph de un cuerpo. Planetas/nodos → path vectorial; puntos del chart
/// (Asc/MC/…) → texto ASCII corto.
pub(crate) fn body_view<Msg: Clone + 'static>(name: &str, px: f32, color: Color) -> View<Msg> {
    let canon = canon_body(name);
    if PLANET_GLYPHS.contains(&canon) {
        cell(
            planet_commands(canon, px / 2.0, px / 2.0, px * 0.82, rgba(color), sw(px)),
            px,
        )
    } else {
        text_cell(simbolo_cuerpo(name), px * 1.3, px, color)
    }
}

/// Glyph de un signo zodiacal (por id inglés: `"aries"`…).
pub(crate) fn sign_view<Msg: Clone + 'static>(name: &str, px: f32, color: Color) -> View<Msg> {
    cell(
        sign_commands(name, px / 2.0, px / 2.0, px * 0.82, rgba(color), sw(px)),
        px,
    )
}

/// Glyph de un aspecto, coloreado por la paleta (oscura).
pub(crate) fn aspect_view<Msg: Clone + 'static>(kind: &str, px: f32) -> View<Msg> {
    let c = Palette::dark().aspect(kind);
    cell(
        aspect_commands(kind, px / 2.0, px / 2.0, px * 0.82, c, sw(px)),
        px,
    )
}

// =====================================================================
// Iconos de chrome (botonera, rail, pestañas, controles, árbol)
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Icon {
    Plus,
    Pencil,
    Scissors,
    Clipboard,
    Trash,
    Close,
    Gear,
    Star,
    Refresh,
    ChevronDown,
    ChevronRight,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Grid,
    Window,
    Folder,
    FolderOpen,
    Person,
    Moon,
    Triangle,
    ZoomIn,
    ZoomOut,
    /// Dirección de un aspecto: aplicando (◄) / separando (►).
    Applying,
    Separating,
}

/// Icono de chrome como mini-canvas `px` del color dado.
pub(crate) fn icon_view<Msg: Clone + 'static>(icon: Icon, px: f32, color: Color) -> View<Msg> {
    cell(icon_cmds(icon, px / 2.0, px / 2.0, px, rgba(color)), px)
}

/// Icono del tipo de carta para el árbol.
pub(crate) fn chart_kind_view<Msg: Clone + 'static>(
    kind: ChartKind,
    px: f32,
    color: Color,
) -> View<Msg> {
    cell(chart_kind_cmds(kind, px / 2.0, px / 2.0, px, rgba(color)), px)
}

fn chart_kind_cmds(kind: ChartKind, cx: f32, cy: f32, box_px: f32, c: Rgba) -> Vec<DrawCommand> {
    let s = box_px * 0.82;
    match kind {
        // Retornos: el luminar correspondiente.
        ChartKind::SolarReturn => planet_commands("sun", cx, cy, s, c, sw(box_px)),
        ChartKind::LunarReturn => planet_commands("moon", cx, cy, s, c, sw(box_px)),
        // Natal y derivadas: una rueda chica (anillo + cruz de ejes).
        ChartKind::Natal | ChartKind::Mundane => wheel_icon(cx, cy, box_px, c),
        // Tránsitos / sinastría / compuestas: dos anillos concéntricos.
        ChartKind::Transit | ChartKind::Synastry | ChartKind::Composite | ChartKind::Davison => {
            let r = box_px * 0.40;
            vec![
                ring(cx, cy, r, c, box_px),
                ring(cx, cy, r * 0.55, c, box_px),
            ]
        }
        // Progresiones / arcos / direcciones / perfecciones: anillo + punto.
        _ => {
            let r = box_px * 0.40;
            vec![
                ring(cx, cy, r, c, box_px),
                DrawCommand::Circle {
                    cx,
                    cy,
                    r: box_px * 0.09,
                    stroke: None,
                    fill: Some(c),
                    stroke_w: 0.0,
                },
            ]
        }
    }
}

fn wheel_icon(cx: f32, cy: f32, box_px: f32, c: Rgba) -> Vec<DrawCommand> {
    let r = box_px * 0.40;
    let w = sw(box_px);
    vec![
        ring(cx, cy, r, c, box_px),
        DrawCommand::Line {
            x1: cx - r,
            y1: cy,
            x2: cx + r,
            y2: cy,
            color: c,
            width: w * 0.8,
            dash: None,
        },
        DrawCommand::Line {
            x1: cx,
            y1: cy - r,
            x2: cx,
            y2: cy + r,
            color: c,
            width: w * 0.8,
            dash: None,
        },
    ]
}

fn ring(cx: f32, cy: f32, r: f32, c: Rgba, box_px: f32) -> DrawCommand {
    DrawCommand::Circle {
        cx,
        cy,
        r,
        stroke: Some(c),
        fill: None,
        stroke_w: sw(box_px),
    }
}

/// Geometría de cada icono, centrada en `(cx, cy)` dentro de una caja de
/// lado `box_px`. Coordenadas absolutas dentro de `[0, box_px]`.
fn icon_cmds(icon: Icon, cx: f32, cy: f32, box_px: f32, c: Rgba) -> Vec<DrawCommand> {
    let r = box_px * 0.5;
    let w = sw(box_px);
    let line = |x1: f32, y1: f32, x2: f32, y2: f32| DrawCommand::Line {
        x1,
        y1,
        x2,
        y2,
        color: c,
        width: w,
        dash: None,
    };
    match icon {
        Icon::Plus => vec![
            line(cx - r * 0.6, cy, cx + r * 0.6, cy),
            line(cx, cy - r * 0.6, cx, cy + r * 0.6),
        ],
        Icon::Close => vec![
            line(cx - r * 0.55, cy - r * 0.55, cx + r * 0.55, cy + r * 0.55),
            line(cx + r * 0.55, cy - r * 0.55, cx - r * 0.55, cy + r * 0.55),
        ],
        Icon::ChevronDown => vec![
            line(cx - r * 0.5, cy - r * 0.25, cx, cy + r * 0.3),
            line(cx, cy + r * 0.3, cx + r * 0.5, cy - r * 0.25),
        ],
        Icon::ChevronRight => vec![
            line(cx - r * 0.25, cy - r * 0.5, cx + r * 0.3, cy),
            line(cx + r * 0.3, cy, cx - r * 0.25, cy + r * 0.5),
        ],
        Icon::ArrowLeft => vec![
            line(cx + r * 0.6, cy, cx - r * 0.55, cy),
            line(cx - r * 0.55, cy, cx - r * 0.05, cy - r * 0.45),
            line(cx - r * 0.55, cy, cx - r * 0.05, cy + r * 0.45),
        ],
        Icon::ArrowRight | Icon::Separating => vec![
            line(cx - r * 0.6, cy, cx + r * 0.55, cy),
            line(cx + r * 0.55, cy, cx + r * 0.05, cy - r * 0.45),
            line(cx + r * 0.55, cy, cx + r * 0.05, cy + r * 0.45),
        ],
        Icon::ArrowUp => vec![
            line(cx, cy + r * 0.6, cx, cy - r * 0.55),
            line(cx, cy - r * 0.55, cx - r * 0.45, cy - r * 0.05),
            line(cx, cy - r * 0.55, cx + r * 0.45, cy - r * 0.05),
        ],
        Icon::ArrowDown => vec![
            line(cx, cy - r * 0.6, cx, cy + r * 0.55),
            line(cx, cy + r * 0.55, cx - r * 0.45, cy + r * 0.05),
            line(cx, cy + r * 0.55, cx + r * 0.45, cy + r * 0.05),
        ],
        // Aplicando: triángulo izquierdo relleno.
        Icon::Applying => vec![DrawCommand::Polygon {
            points: vec![
                (cx - r * 0.5, cy),
                (cx + r * 0.4, cy - r * 0.5),
                (cx + r * 0.4, cy + r * 0.5),
            ],
            fill: Some(c),
            stroke: None,
            stroke_w: 0.0,
        }],
        Icon::Triangle => vec![DrawCommand::Polygon {
            points: vec![
                (cx, cy - r * 0.6),
                (cx + r * 0.6, cy + r * 0.5),
                (cx - r * 0.6, cy + r * 0.5),
            ],
            fill: None,
            stroke: Some(c),
            stroke_w: w,
        }],
        Icon::Pencil => vec![
            // Cuerpo diagonal del lápiz + punta.
            line(cx - r * 0.45, cy + r * 0.5, cx + r * 0.35, cy - r * 0.4),
            line(cx + r * 0.35, cy - r * 0.4, cx + r * 0.5, cy - r * 0.55),
            line(cx - r * 0.45, cy + r * 0.5, cx - r * 0.6, cy + r * 0.62),
        ],
        Icon::Scissors => {
            let h = box_px * 0.07;
            vec![
                DrawCommand::Circle {
                    cx: cx - r * 0.35,
                    cy: cy + r * 0.45,
                    r: h,
                    stroke: Some(c),
                    fill: None,
                    stroke_w: w * 0.8,
                },
                DrawCommand::Circle {
                    cx: cx + r * 0.35,
                    cy: cy + r * 0.45,
                    r: h,
                    stroke: Some(c),
                    fill: None,
                    stroke_w: w * 0.8,
                },
                line(cx - r * 0.28, cy + r * 0.38, cx + r * 0.55, cy - r * 0.55),
                line(cx + r * 0.28, cy + r * 0.38, cx - r * 0.55, cy - r * 0.55),
            ]
        }
        Icon::Clipboard => {
            let bw = r * 0.55;
            let top = cy - r * 0.55;
            let bot = cy + r * 0.6;
            vec![
                DrawCommand::Polygon {
                    points: vec![
                        (cx - bw, top),
                        (cx + bw, top),
                        (cx + bw, bot),
                        (cx - bw, bot),
                    ],
                    fill: None,
                    stroke: Some(c),
                    stroke_w: w,
                },
                // Pestaña superior.
                DrawCommand::Polygon {
                    points: vec![
                        (cx - r * 0.22, top - r * 0.18),
                        (cx + r * 0.22, top - r * 0.18),
                        (cx + r * 0.22, top + r * 0.1),
                        (cx - r * 0.22, top + r * 0.1),
                    ],
                    fill: Some(c),
                    stroke: None,
                    stroke_w: 0.0,
                },
            ]
        }
        Icon::Trash => {
            let bw = r * 0.45;
            let top = cy - r * 0.35;
            let bot = cy + r * 0.6;
            vec![
                // Cuerpo (trapecio).
                DrawCommand::Polygon {
                    points: vec![
                        (cx - bw, top),
                        (cx + bw, top),
                        (cx + bw * 0.78, bot),
                        (cx - bw * 0.78, bot),
                    ],
                    fill: None,
                    stroke: Some(c),
                    stroke_w: w,
                },
                // Tapa.
                line(cx - r * 0.62, top, cx + r * 0.62, top),
                // Asa.
                line(cx - r * 0.2, top, cx - r * 0.12, cy - r * 0.6),
                line(cx + r * 0.2, top, cx + r * 0.12, cy - r * 0.6),
                line(cx - r * 0.12, cy - r * 0.6, cx + r * 0.12, cy - r * 0.6),
            ]
        }
        Icon::Gear => {
            let mut out = vec![
                ring(cx, cy, r * 0.42, c, box_px),
                DrawCommand::Circle {
                    cx,
                    cy,
                    r: r * 0.16,
                    stroke: None,
                    fill: Some(c),
                    stroke_w: 0.0,
                },
            ];
            for k in 0..8 {
                let a = std::f32::consts::PI * (k as f32) / 4.0;
                let (s, co) = a.sin_cos();
                out.push(line(
                    cx + co * r * 0.42,
                    cy + s * r * 0.42,
                    cx + co * r * 0.7,
                    cy + s * r * 0.7,
                ));
            }
            out
        }
        Icon::Star => {
            let mut pts = Vec::with_capacity(10);
            for k in 0..10 {
                let a = std::f32::consts::PI * (k as f32) / 5.0 - std::f32::consts::FRAC_PI_2;
                let rad = if k % 2 == 0 { r * 0.72 } else { r * 0.3 };
                pts.push((cx + a.cos() * rad, cy + a.sin() * rad));
            }
            vec![DrawCommand::Polygon {
                points: pts,
                fill: None,
                stroke: Some(c),
                stroke_w: w,
            }]
        }
        Icon::Refresh => {
            // Arco ~270° + cabeza de flecha.
            let rr = r * 0.5;
            let d = format!(
                "M {} {} A {rr} {rr} 0 1 1 {} {}",
                cx,
                cy - rr,
                cx + rr,
                cy,
            );
            vec![
                DrawCommand::Path {
                    d,
                    stroke: Some(c),
                    fill: None,
                    stroke_w: w,
                },
                line(cx + rr, cy, cx + rr * 0.45, cy - rr * 0.55),
                line(cx + rr, cy, cx + rr * 1.05, cy - rr * 0.55),
            ]
        }
        Icon::Grid => {
            let s = r * 0.6;
            vec![
                DrawCommand::Polygon {
                    points: vec![
                        (cx - s, cy - s),
                        (cx + s, cy - s),
                        (cx + s, cy + s),
                        (cx - s, cy + s),
                    ],
                    fill: None,
                    stroke: Some(c),
                    stroke_w: w,
                },
                line(cx, cy - s, cx, cy + s),
                line(cx - s, cy, cx + s, cy),
            ]
        }
        Icon::Window => {
            let s = r * 0.6;
            vec![
                DrawCommand::Polygon {
                    points: vec![
                        (cx - s, cy - s),
                        (cx + s, cy - s),
                        (cx + s, cy + s),
                        (cx - s, cy + s),
                    ],
                    fill: None,
                    stroke: Some(c),
                    stroke_w: w,
                },
                line(cx - s, cy - s * 0.45, cx + s, cy - s * 0.45),
            ]
        }
        Icon::Folder | Icon::FolderOpen => {
            let left = cx - r * 0.62;
            let right = cx + r * 0.62;
            let top = cy - r * 0.32;
            let bot = cy + r * 0.45;
            let mut out = vec![DrawCommand::Polygon {
                points: vec![
                    (left, top),
                    (cx - r * 0.1, top),
                    (cx + r * 0.02, top - r * 0.18),
                    (right, top - r * 0.18),
                    (right, bot),
                    (left, bot),
                ],
                fill: None,
                stroke: Some(c),
                stroke_w: w,
            }];
            if icon == Icon::FolderOpen {
                out.push(line(left, cy, right, cy));
            }
            out
        }
        Icon::Person => vec![
            DrawCommand::Circle {
                cx,
                cy: cy - r * 0.32,
                r: r * 0.26,
                stroke: Some(c),
                fill: None,
                stroke_w: w,
            },
            DrawCommand::Path {
                d: format!(
                    "M {} {} A {} {} 0 0 1 {} {}",
                    cx - r * 0.45,
                    cy + r * 0.55,
                    r * 0.45,
                    r * 0.45,
                    cx + r * 0.45,
                    cy + r * 0.55,
                ),
                stroke: Some(c),
                fill: None,
                stroke_w: w,
            },
        ],
        Icon::Moon => planet_commands("moon", cx, cy, box_px * 0.82, c, w),
        Icon::ZoomIn | Icon::ZoomOut => {
            let lens = r * 0.38;
            let lcx = cx - r * 0.12;
            let lcy = cy - r * 0.12;
            let mut out = vec![
                DrawCommand::Circle {
                    cx: lcx,
                    cy: lcy,
                    r: lens,
                    stroke: Some(c),
                    fill: None,
                    stroke_w: w,
                },
                line(lcx + lens * 0.7, lcy + lens * 0.7, cx + r * 0.6, cy + r * 0.6),
                line(lcx - lens * 0.5, lcy, lcx + lens * 0.5, lcy),
            ];
            if icon == Icon::ZoomIn {
                out.push(line(lcx, lcy - lens * 0.5, lcx, lcy + lens * 0.5));
            }
            out
        }
    }
}
