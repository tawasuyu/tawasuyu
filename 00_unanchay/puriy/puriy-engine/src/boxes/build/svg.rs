//! SVG: recolección de prims/paths, parseo de points/path-d, colores y números SVG, srcset.
//! Extraído de `boxes/build.rs` (regla #1). Sin cambios de lógica.
use super::*;

/// Recolecta primitivas de un `<svg>`: rect/circle/line directos.
/// Soporta atributos `viewBox`, `width`, `height`, `fill`, `stroke`,
/// `stroke-width`. Sin transforms ni groups recursivos.
pub(crate) fn collect_svg(svg_node: &Handle) -> SvgScene {
    let width = dom::attr(svg_node, "width")
        .and_then(|s| s.trim_end_matches("px").trim().parse::<f32>().ok())
        .unwrap_or(300.0);
    let height = dom::attr(svg_node, "height")
        .and_then(|s| s.trim_end_matches("px").trim().parse::<f32>().ok())
        .unwrap_or(150.0);
    let view_box = dom::attr(svg_node, "viewBox").and_then(|s| {
        let nums: Vec<f32> = s
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse::<f32>().ok())
            .collect();
        if nums.len() == 4 {
            Some((nums[0], nums[1], nums[2], nums[3]))
        } else {
            None
        }
    });
    let mut prims: Vec<SvgPrim> = Vec::new();
    collect_svg_prims(svg_node, &mut prims);
    SvgScene { width, height, view_box, prims }
}

pub(crate) fn collect_svg_prims(node: &Handle, out: &mut Vec<SvgPrim>) {
    if let markup5ever_rcdom::NodeData::Element { .. } = &node.data {
        match dom::element_name(node).as_deref() {
            Some("rect") => {
                let x = svg_num(node, "x", 0.0);
                let y = svg_num(node, "y", 0.0);
                let w = svg_num(node, "width", 0.0);
                let h = svg_num(node, "height", 0.0);
                let rx = svg_num(node, "rx", 0.0);
                out.push(SvgPrim::Rect {
                    x, y, w, h, rx,
                    fill: svg_color(node, "fill"),
                    stroke: svg_color(node, "stroke"),
                    stroke_w: svg_num(node, "stroke-width", 1.0),
                });
            }
            Some("circle") => {
                let cx = svg_num(node, "cx", 0.0);
                let cy = svg_num(node, "cy", 0.0);
                let r = svg_num(node, "r", 0.0);
                out.push(SvgPrim::Circle {
                    cx, cy, r,
                    fill: svg_color(node, "fill"),
                    stroke: svg_color(node, "stroke"),
                    stroke_w: svg_num(node, "stroke-width", 1.0),
                });
            }
            Some("line") => {
                let x1 = svg_num(node, "x1", 0.0);
                let y1 = svg_num(node, "y1", 0.0);
                let x2 = svg_num(node, "x2", 0.0);
                let y2 = svg_num(node, "y2", 0.0);
                if let Some(stroke) = svg_color(node, "stroke") {
                    out.push(SvgPrim::Line {
                        x1, y1, x2, y2,
                        stroke,
                        stroke_w: svg_num(node, "stroke-width", 1.0),
                    });
                }
            }
            Some("polygon") | Some("polyline") => {
                let closed = dom::element_name(node).as_deref() == Some("polygon");
                let points = parse_svg_points(&dom::attr(node, "points").unwrap_or_default());
                if !points.is_empty() {
                    out.push(SvgPrim::Polyline {
                        points,
                        closed,
                        fill: svg_color(node, "fill"),
                        stroke: svg_color(node, "stroke"),
                        stroke_w: svg_num(node, "stroke-width", 1.0),
                    });
                }
            }
            Some("path") => {
                if let Some(d) = dom::attr(node, "d") {
                    let cmds = parse_svg_path(&d);
                    if !cmds.is_empty() {
                        out.push(SvgPrim::Path {
                            d: cmds,
                            fill: svg_color(node, "fill"),
                            stroke: svg_color(node, "stroke"),
                            stroke_w: svg_num(node, "stroke-width", 1.0),
                        });
                    }
                }
            }
            // Containers transparentes: recurrir adentro.
            Some("g") | Some("svg") => {}
            // Resto (`text`, `defs`, `mask`, etc.) ignorado.
            _ => return,
        }
    }
    for c in node.children.borrow().iter() {
        collect_svg_prims(c, out);
    }
}

pub(crate) fn svg_num(node: &Handle, name: &str, default: f32) -> f32 {
    dom::attr(node, name)
        .and_then(|s| s.trim_end_matches("px").trim().parse::<f32>().ok())
        .unwrap_or(default)
}

/// Elige una URL del `srcset` HTML. Subset: cada candidato es `url
/// [descriptor]` separados por `,`. Descriptor puede ser `Nx`
/// (densidad) o `Nw` (ancho) o ausente. Estrategia: preferimos la
/// más alta densidad (`Nx`) o el ancho más grande (`Nw`); sin
/// viewport conocido al tiempo de parse, asumimos high-DPI por default.
pub(crate) fn pick_srcset(srcset: &str) -> Option<String> {
    if srcset.trim().is_empty() {
        return None;
    }
    let mut best_score: f32 = -1.0;
    let mut best_url: Option<String> = None;
    for entry in srcset.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (url, desc) = match entry.split_once(char::is_whitespace) {
            Some((u, d)) => (u.trim().to_string(), d.trim().to_string()),
            None => (entry.to_string(), String::new()),
        };
        let score: f32 = if let Some(rest) = desc.strip_suffix('x') {
            rest.parse::<f32>().unwrap_or(1.0) * 1000.0
        } else if let Some(rest) = desc.strip_suffix('w') {
            rest.parse::<f32>().unwrap_or(0.0)
        } else {
            // Sin descriptor — equivalente a 1x.
            1000.0
        };
        if score > best_score {
            best_score = score;
            best_url = Some(url);
        }
    }
    best_url
}

pub(crate) fn parse_svg_points(s: &str) -> Vec<(f32, f32)> {
    let nums: Vec<f32> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse::<f32>().ok())
        .collect();
    nums.chunks_exact(2).map(|c| (c[0], c[1])).collect()
}

/// Parser de `d=` minimal: soporta M/m, L/l, H/h, V/v, C/c, Q/q, Z/z.
/// No soporta A (arcs), T, S (smooth bezier).
pub(crate) fn parse_svg_path(d: &str) -> Vec<PathCmd> {
    // Tokenize: cada comando es una letra, cada arg es un f32 (separados
    // por whitespace o coma; el signo `-` puede arrancar un nuevo número
    // sin separador).
    let bytes = d.as_bytes();
    let mut i = 0;
    let n = bytes.len();
    let mut out: Vec<PathCmd> = Vec::new();
    let mut cx = 0.0_f32; // cursor x absoluto
    let mut cy = 0.0_f32;
    let mut start_x = 0.0_f32;
    let mut start_y = 0.0_f32;
    let mut current_cmd: u8 = 0;
    while i < n {
        let c = bytes[i];
        if c.is_ascii_whitespace() || c == b',' {
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() {
            current_cmd = c;
            i += 1;
            // Z/z no toma args — ejecutalo acá directamente, sino el
            // loop nunca llega al match (no hay número que dispare).
            if c == b'Z' || c == b'z' {
                out.push(PathCmd::ClosePath);
                cx = start_x;
                cy = start_y;
            }
            continue;
        }
        // c es dígito o `-`/`+`/`.`: leer un número.
        let read_num = |from: usize| -> Option<(f32, usize)> {
            let mut j = from;
            if j < n && (bytes[j] == b'-' || bytes[j] == b'+') {
                j += 1;
            }
            while j < n && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            if j < n && (bytes[j] == b'e' || bytes[j] == b'E') {
                j += 1;
                if j < n && (bytes[j] == b'-' || bytes[j] == b'+') {
                    j += 1;
                }
                while j < n && bytes[j].is_ascii_digit() {
                    j += 1;
                }
            }
            std::str::from_utf8(&bytes[from..j])
                .ok()
                .and_then(|s| s.parse::<f32>().ok())
                .map(|v| (v, j))
        };
        let read_args = |from: usize, count: usize| -> Option<(Vec<f32>, usize)> {
            let mut nums = Vec::with_capacity(count);
            let mut k = from;
            while nums.len() < count {
                while k < n && (bytes[k].is_ascii_whitespace() || bytes[k] == b',') {
                    k += 1;
                }
                let (v, after) = read_num(k)?;
                nums.push(v);
                k = after;
            }
            Some((nums, k))
        };
        let rel = current_cmd.is_ascii_lowercase();
        match current_cmd.to_ascii_uppercase() {
            b'M' => {
                let (args, after) = match read_args(i, 2) {
                    Some(v) => v,
                    None => break,
                };
                let (mut x, mut y) = (args[0], args[1]);
                if rel { x += cx; y += cy; }
                out.push(PathCmd::MoveTo(x, y));
                cx = x; cy = y;
                start_x = x; start_y = y;
                i = after;
                // M con args extra implícitamente lineTo.
                current_cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let (args, after) = match read_args(i, 2) {
                    Some(v) => v,
                    None => break,
                };
                let (mut x, mut y) = (args[0], args[1]);
                if rel { x += cx; y += cy; }
                out.push(PathCmd::LineTo(x, y));
                cx = x; cy = y;
                i = after;
            }
            b'H' => {
                let (args, after) = match read_args(i, 1) {
                    Some(v) => v,
                    None => break,
                };
                let mut x = args[0];
                if rel { x += cx; }
                out.push(PathCmd::LineTo(x, cy));
                cx = x;
                i = after;
            }
            b'V' => {
                let (args, after) = match read_args(i, 1) {
                    Some(v) => v,
                    None => break,
                };
                let mut y = args[0];
                if rel { y += cy; }
                out.push(PathCmd::LineTo(cx, y));
                cy = y;
                i = after;
            }
            b'C' => {
                let (args, after) = match read_args(i, 6) {
                    Some(v) => v,
                    None => break,
                };
                let (mut x1, mut y1, mut x2, mut y2, mut x, mut y) =
                    (args[0], args[1], args[2], args[3], args[4], args[5]);
                if rel {
                    x1 += cx; y1 += cy;
                    x2 += cx; y2 += cy;
                    x += cx; y += cy;
                }
                out.push(PathCmd::CubicTo(x1, y1, x2, y2, x, y));
                cx = x; cy = y;
                i = after;
            }
            b'Q' => {
                let (args, after) = match read_args(i, 4) {
                    Some(v) => v,
                    None => break,
                };
                let (mut x1, mut y1, mut x, mut y) = (args[0], args[1], args[2], args[3]);
                if rel {
                    x1 += cx; y1 += cy;
                    x += cx; y += cy;
                }
                out.push(PathCmd::QuadTo(x1, y1, x, y));
                cx = x; cy = y;
                i = after;
            }
            b'Z' => {
                out.push(PathCmd::ClosePath);
                cx = start_x;
                cy = start_y;
            }
            _ => {
                // Comando no soportado (`A`, `T`, `S`) — saltea un número
                // para evitar loops infinitos.
                if let Some((_, after)) = read_num(i) {
                    i = after;
                } else {
                    break;
                }
            }
        }
    }
    out
}

pub(crate) fn svg_color(node: &Handle, name: &str) -> Option<Color> {
    let v = dom::attr(node, name)?;
    let v = v.trim();
    if v.eq_ignore_ascii_case("none") {
        return None;
    }
    crate::style::parse_color_named_or_hex(v)
}

