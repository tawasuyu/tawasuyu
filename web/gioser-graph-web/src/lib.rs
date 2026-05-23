//! `gioser-graph-web` — grafo semántico SVG inline.
//!
//! Layout grid: 3 columnas, filas según cantidad de nodos.
//! Nodos: rectángulos redondeados 170×44px con texto + subtexto (camino).
//! Aristas: opacidad/brillo según weight (más peso = más blanca y opaca).
//! Respiración CSS suave en el SVG (opacity oscila perpetua).
//! Hover: glow + opacidad.
//! Las aristas conectan por ID (UUID), no por doc_id.

use std::cell::RefCell;
use std::rc::Rc;

use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Document, HtmlElement, MouseEvent, PointerEvent, Response, SvgLineElement, SvgRectElement,
    SvgsvgElement, SvgTextElement, SvgCircleElement,
};

pub(crate) fn document() -> Option<Document> {
    web_sys::window().and_then(|w| w.document())
}

// ─── Tipos de respuesta de `/graph` ──────────────────────────────

#[derive(Deserialize, Debug, Clone)]
struct GraphResponse {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    stats: GraphStats,
}

#[derive(Deserialize, Debug, Clone)]
struct GraphNode {
    data: NodeData,
}

#[derive(Deserialize, Debug, Clone)]
struct NodeData {
    id: String,
    name: String,
    camino: String,
    doc_id: Option<String>,
    #[allow(dead_code)]
    preview: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
struct GraphEdge {
    data: EdgeData,
}

#[derive(Deserialize, Debug, Clone)]
struct EdgeData {
    id: String,
    source: String,
    target: String,
    weight: Option<f64>,
}

#[derive(Deserialize, Debug, Clone)]
struct GraphStats {
    points: u32,
    edges: u32,
    #[allow(dead_code)]
    by_camino: Option<std::collections::HashMap<String, u32>>,
}

// ─── Widget ──────────────────────────────────────────────────────

type NavCallback = Rc<RefCell<Option<Box<dyn FnMut(String)>>>>;

const CANVAS_W: f64 = 800.0;
const CANVAS_H: f64 = 480.0;
const NODE_W: f64 = 170.0;
const NODE_H: f64 = 44.0;
const COLS: usize = 3;

const CAMINO_COLORS: &[(&str, &str)] = &[
    ("logos",    "#d0dbff"), ("aire",   "#d0dbff"),
    ("nomos",    "#f59056"), ("fuego",  "#f59056"),
    ("kay",      "#d49873"), ("tierra", "#d49873"),
    ("uku",      "#6cd0f3"), ("agua",   "#6cd0f3"),
    ("cuerpo",   "#e07a5f"),
    ("sombra",   "#6a6a7a"),
    ("cosmos",   "#d4a843"),
    ("practica", "#2d936c"),
    ("olvido",   "#b0b8c0"),
];

fn camino_color(camino: &str) -> &str {
    for (k, v) in CAMINO_COLORS {
        if *k == camino { return v; }
    }
    "#888888"
}

fn parse_hex(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim_start_matches('#');
    if h.len() == 6 {
        let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(128);
        let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(128);
        let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(128);
        (r, g, b)
    } else {
        (136, 136, 136)
    }
}

fn blend_colors(c1: &str, c2: &str, t: f64) -> String {
    let (r1, g1, b1) = parse_hex(c1);
    let (r2, g2, b2) = parse_hex(c2);
    let r = (r1 as f64 * (1.0 - t) + r2 as f64 * t) as u32;
    let g = (g1 as f64 * (1.0 - t) + g2 as f64 * t) as u32;
    let b = (b1 as f64 * (1.0 - t) + b2 as f64 * t) as u32;
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}

pub struct GraphWidget {
    container: HtmlElement,
    api_url: String,
    nodes: Vec<NodeData>,
    edges: Vec<EdgeData>,
    on_navigate: NavCallback,
    document: Document,
}

impl GraphWidget {
    pub fn new(
        container: HtmlElement,
        api_url: &str,
        on_navigate: Option<Box<dyn FnMut(String)>>,
    ) -> Self {
        let doc = crate::document().unwrap_or_else(|| {
            web_sys::window()
                .and_then(|w| w.document())
                .expect("no document")
        });
        Self {
            container,
            api_url: api_url.to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
            on_navigate: Rc::new(RefCell::new(on_navigate)),
            document: doc,
        }
    }

    pub async fn load(&mut self) -> Result<(), JsValue> {
        let url = format!("{}/graph?limit=500", self.api_url);
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
        let resp_value = JsFuture::from(window.fetch_with_str(&url)).await?;
        let resp: Response = resp_value.dyn_into()?;
        if !resp.ok() {
            return Err(JsValue::from_str(&format!("HTTP {}", resp.status())));
        }
        let text = JsFuture::from(resp.text()?).await?;
        let body = text.as_string().unwrap_or_default();

        let graph: GraphResponse =
            serde_json::from_str(&body).map_err(|e| JsValue::from_str(&format!("JSON: {e}")))?;

        let nodes: Vec<NodeData> = graph.nodes.into_iter().map(|n| n.data).collect();
        let edges: Vec<EdgeData> = graph.edges.into_iter().map(|e| e.data).collect();

        self.nodes = nodes;
        self.edges = edges;
        self.render();
        Ok(())
    }

    fn render(&self) {
        self.container.set_inner_html("");

        if self.nodes.is_empty() {
            return;
        }

        let positions = grid_layout(&self.nodes, CANVAS_W, CANVAS_H);

        let ns = "http://www.w3.org/2000/svg";
        let svg: SvgsvgElement = self
            .document
            .create_element_ns(Some(ns), "svg")
            .unwrap()
            .dyn_into()
            .unwrap();
        svg.set_attribute("viewBox", &format!("0 0 {} {}", CANVAS_W as u32, CANVAS_H as u32)).ok();
        svg.set_attribute("width", "100%").ok();
        svg.set_attribute("preserveAspectRatio", "xMidYMid meet").ok();
        svg.style().set_property("display", "block").ok();
        svg.style().set_property("margin", "1.5rem auto 0").ok();
        svg.style().set_property("max-width", "100%").ok();
        svg.style().set_property("height", "auto").ok();
        svg.style().set_property("background", "rgba(255,255,255,0.02)").ok();
        svg.style().set_property("border-radius", "12px").ok();
        svg.style().set_property("border", "1px solid rgba(216,168,93,0.15)").ok();

        // Estilo inline en SVG: respiración y transiciones
        let style_el = self.document.create_element_ns(Some(ns), "style").unwrap();
        style_el.set_text_content(Some(
            "@keyframes graph-breathe {\
              0%, 100% { opacity: 1; }\
              50% { opacity: 0.92; }\
            }\
            .gb-svg { animation: graph-breathe 5s ease-in-out infinite; }\
            .gb-node { transition: filter 250ms ease, opacity 200ms ease; }\
            .gb-node:hover {\
              filter: drop-shadow(0 0 14px rgba(255,255,255,0.2));\
            }\
            .gb-edge-group { pointer-events: none; }\
            .gb-line { transition: opacity 400ms ease; }",
        ));
        svg.append_child(&style_el).ok();

        // Grupo para aristas (sin animación de respiración, están detrás)
        let edges_group: web_sys::SvgElement = self
            .document
            .create_element_ns(Some(ns), "g")
            .unwrap()
            .dyn_into()
            .unwrap();
        edges_group.set_attribute("class", "gb-edge-group").ok();

        // Grupo para nodos (con respiración, encima)
        let nodes_group: web_sys::SvgElement = self
            .document
            .create_element_ns(Some(ns), "g")
            .unwrap()
            .dyn_into()
            .unwrap();
        nodes_group.set_attribute("class", "gb-svg").ok();

        // Mapas: UUID → posición y color
        let pos_map: std::collections::HashMap<&str, (f64, f64)> = positions
            .iter()
            .map(|(id, p)| (id.as_str(), *p))
            .collect();
        let color_map: std::collections::HashMap<&str, &str> = self.nodes
            .iter()
            .map(|n| (n.id.as_str(), camino_color(&n.camino)))
            .collect();

        // ── Aristas: mezcla de colores de los nodos que conecta ──
        let mut drawn = std::collections::HashSet::new();
        for edge in &self.edges {
            let key = if edge.source < edge.target {
                (edge.source.as_str(), edge.target.as_str())
            } else {
                (edge.target.as_str(), edge.source.as_str())
            };
            if !drawn.insert(key) { continue; }

            let Some((x1, y1)) = pos_map.get(edge.source.as_str()) else { continue; };
            let Some((x2, y2)) = pos_map.get(edge.target.as_str()) else { continue; };

            let c1 = color_map.get(edge.source.as_str()).copied().unwrap_or("#888");
            let c2 = color_map.get(edge.target.as_str()).copied().unwrap_or("#888");

            let w = edge.weight.unwrap_or(0.7);

            // Min-max scaling: expandir el rango real (0.83-0.93) a 0.0-1.0
            // Con un mínimo global de 0.80 para capturar todo
            let norm = ((w - 0.80) / 0.15).clamp(0.0, 1.0);
            // Curva cuadrática para exagerar diferencias: norm²
            let expanded = norm * norm;

            // Grosor sutil: 0.5px a 3.5px (no grueso, diferenciado)
            let thick = 0.8 + expanded * 3.0;

            // Color mezclado
            let blended = blend_colors(c1, c2, 0.5);
            let (br, bg, bb) = parse_hex(&blended);
            // Brillo: sin expander, normal 0-1, sútil
            let bright_factor = norm * 0.5;
            let r = (br as f64 + (255.0 - br as f64) * bright_factor).min(255.0) as u32;
            let g = (bg as f64 + (255.0 - bg as f64) * bright_factor).min(255.0) as u32;
            let b = (bb as f64 + (255.0 - bb as f64) * bright_factor).min(255.0) as u32;

            // Opacidad baja y sutil: 15% a 40%
            let alpha = 0.12 + expanded * 0.30;

            let line: SvgLineElement = self
                .document
                .create_element_ns(Some(ns), "line")
                .unwrap()
                .dyn_into()
                .unwrap();
            line.set_attribute("x1", &format!("{:.1}", x1)).ok();
            line.set_attribute("y1", &format!("{:.1}", y1)).ok();
            line.set_attribute("x2", &format!("{:.1}", x2)).ok();
            line.set_attribute("y2", &format!("{:.1}", y2)).ok();
            line.set_attribute("stroke", &format!("#{:02x}{:02x}{:02x}", r, g, b)).ok();
            line.set_attribute("stroke-width", &format!("{:.1}", thick)).ok();
            line.set_attribute("stroke-opacity", &format!("{:.2}", alpha)).ok();
            line.set_attribute("class", "gb-line").ok();
            edges_group.append_child(&line).ok();
        }

        // ── Nodos ──
        let on_nav = self.on_navigate.clone();
        for (i, node) in self.nodes.iter().enumerate() {
            let Some((cx, cy)) = positions.get(i).map(|(_, p)| *p) else { continue; };
            let color = camino_color(&node.camino).to_string();
            let label = if node.name.chars().count() > 20 {
                let cutoff: String = node.name.chars().take(18).collect();
                format!("{}…", cutoff)
            } else {
                node.name.clone()
            };
            let camino_up = node.camino.to_uppercase();

            let g: web_sys::SvgElement = self
                .document
                .create_element_ns(Some(ns), "g")
                .unwrap()
                .dyn_into()
                .unwrap();
            g.style().set_property("cursor", "pointer").ok();
            g.set_attribute("class", "gb-node").ok();
            g.set_attribute("title", &format!("{} — {}", node.name, camino_up)).ok();

            let rx = cx - NODE_W / 2.0;
            let ry = cy - NODE_H / 2.0;

            let glow: SvgCircleElement = self
                .document
                .create_element_ns(Some(ns), "circle")
                .unwrap()
                .dyn_into()
                .unwrap();
            glow.set_attribute("cx", &format!("{:.1}", cx)).ok();
            glow.set_attribute("cy", &format!("{:.1}", cy)).ok();
            glow.set_attribute("r", "32").ok();
            glow.set_attribute("fill", &color).ok();
            glow.set_attribute("fill-opacity", "0.05").ok();
            g.append_child(&glow).ok();

            let rect: SvgRectElement = self
                .document
                .create_element_ns(Some(ns), "rect")
                .unwrap()
                .dyn_into()
                .unwrap();
            rect.set_attribute("x", &format!("{:.1}", rx)).ok();
            rect.set_attribute("y", &format!("{:.1}", ry)).ok();
            rect.set_attribute("width", &format!("{:.1}", NODE_W)).ok();
            rect.set_attribute("height", &format!("{:.1}", NODE_H)).ok();
            rect.set_attribute("rx", "8").ok();
            rect.set_attribute("ry", "8").ok();
            rect.set_attribute("fill", &color).ok();
            rect.set_attribute("fill-opacity", "0.28").ok();
            rect.set_attribute("stroke", &color).ok();
            rect.set_attribute("stroke-width", "1.8").ok();
            rect.set_attribute("stroke-opacity", "0.7").ok();
            rect.style().set_property("transition", "all 200ms ease").ok();

            let text: SvgTextElement = self
                .document
                .create_element_ns(Some(ns), "text")
                .unwrap()
                .dyn_into()
                .unwrap();
            text.set_attribute("x", &format!("{:.1}", cx)).ok();
            text.set_attribute("y", &format!("{:.1}", cy - 2.0)).ok();
            text.set_attribute("text-anchor", "middle").ok();
            text.set_attribute("dominant-baseline", "middle").ok();
            text.set_attribute("fill", "rgba(232,234,245,0.88)").ok();
            text.set_attribute("font-size", "13").ok();
            text.set_attribute("font-family", "Inter, system-ui, sans-serif").ok();
            text.set_attribute("font-weight", "500").ok();
            text.set_text_content(Some(&label));

            let sub: SvgTextElement = self
                .document
                .create_element_ns(Some(ns), "text")
                .unwrap()
                .dyn_into()
                .unwrap();
            sub.set_attribute("x", &format!("{:.1}", cx)).ok();
            sub.set_attribute("y", &format!("{:.1}", cy + 15.0)).ok();
            sub.set_attribute("text-anchor", "middle").ok();
            sub.set_attribute("dominant-baseline", "middle").ok();
            sub.set_attribute("fill", "rgba(232,234,245,0.45)").ok();
            sub.set_attribute("font-size", "9").ok();
            sub.set_attribute("font-family", "Inter, system-ui, sans-serif").ok();
            sub.set_attribute("letter-spacing", "0.25em").ok();
            sub.set_text_content(Some(&camino_up));

            g.append_child(&rect).ok();
            g.append_child(&text).ok();
            g.append_child(&sub).ok();

            // Hover + drag
            let rect_h = rect.clone();
            let col_h = color.clone();
            let glow_h = glow.clone();
            let g_hover = g.clone();
            let g_clone_for_drag = g.clone();

            let enter = Closure::<dyn FnMut(MouseEvent)>::new(move |_| {
                rect_h.set_attribute("fill-opacity", "0.55").ok();
                rect_h.set_attribute("stroke-opacity", "1").ok();
                rect_h.style()
                    .set_property("filter", &format!("drop-shadow(0 0 12px {})", col_h))
                    .ok();
                glow_h.set_attribute("fill-opacity", "0.20").ok();
            });
            g_hover.add_event_listener_with_callback("mouseenter", enter.as_ref().unchecked_ref()).ok();
            enter.forget();

            let rect_l = rect.clone();
            let glow_l = glow.clone();
            let leave = Closure::<dyn FnMut(MouseEvent)>::new(move |_| {
                rect_l.set_attribute("fill-opacity", "0.28").ok();
                rect_l.set_attribute("stroke-opacity", "0.7").ok();
                rect_l.style().set_property("filter", "none").ok();
                glow_l.set_attribute("fill-opacity", "0.05").ok();
            });
            g.add_event_listener_with_callback("mouseleave", leave.as_ref().unchecked_ref()).ok();
            leave.forget();

            // Click → navegar
            let nav_target = node.camino.clone();
            let on_nav2 = on_nav.clone();
            let click = Closure::<dyn FnMut(MouseEvent)>::new(move |_| {
                let mut cb = on_nav2.borrow_mut();
                if let Some(ref mut f) = *cb { f(nav_target.clone()); }
            });
            g.add_event_listener_with_callback("click", click.as_ref().unchecked_ref()).ok();
            click.forget();

            // Drag: pointerdown → move → up (reacomodar al soltar)
            let g_drag = g.clone();
            let g_move = g.clone();
            let g_up = g.clone();
            let drag_start = Rc::new(RefCell::new(None::<(f64, f64, f64, f64)>));
            let drag_start2 = drag_start.clone();
            let drag_start3 = drag_start.clone();

            let pdown = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
                e.prevent_default();
                let g_rect = g_drag.get_bounding_client_rect();
                *drag_start.borrow_mut() = Some((
                    e.client_x() as f64,
                    e.client_y() as f64,
                    g_rect.left() + g_rect.width() / 2.0,
                    g_rect.top() + g_rect.height() / 2.0,
                ));
                g_drag.set_pointer_capture(e.pointer_id()).ok();
            });
            g.add_event_listener_with_callback("pointerdown", pdown.as_ref().unchecked_ref()).ok();
            pdown.forget();

            let pmove = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
                if let Some((start_cx, start_cy, _, _)) = *drag_start2.borrow() {
                    let dx = e.client_x() as f64 - start_cx;
                    let dy = e.client_y() as f64 - start_cy;
                    g_move.set_attribute(
                        "transform",
                        &format!("translate({:.1},{:.1})", dx, dy),
                    ).ok();
                }
            });
            g.add_event_listener_with_callback("pointermove", pmove.as_ref().unchecked_ref()).ok();
            pmove.forget();

            let pup = Closure::<dyn FnMut(PointerEvent)>::new(move |_e: PointerEvent| {
                *drag_start3.borrow_mut() = None;
                g_up.set_attribute("transform", "translate(0,0)").ok();
            });
            g.add_event_listener_with_callback("pointerup", pup.as_ref().unchecked_ref()).ok();
            pup.forget();

            g.style().set_property("transition", "transform 0.35s cubic-bezier(0.34, 1.56, 0.64, 1)").ok();

            nodes_group.append_child(&g).ok();
        }

        svg.append_child(&edges_group).ok();
        svg.append_child(&nodes_group).ok();
        self.container.append_child(&svg).ok();
    }
}

// ─── Layout grid: 3 columnas ─────────────────────────────────────

fn grid_layout(nodes: &[NodeData], w: f64, h: f64) -> Vec<(String, (f64, f64))> {
    let n = nodes.len();
    if n == 0 { return vec![]; }

    let rows = (n + COLS - 1) / COLS;
    let actual_rows = rows.max(3);
    let margin_x = NODE_W / 2.0 + 20.0;
    let margin_y = NODE_H / 2.0 + 20.0;
    let usable_w = w - margin_x * 2.0;
    let usable_h = h - margin_y * 2.0;
    let col_gap = usable_w / (COLS as f64);
    let row_gap = usable_h / (actual_rows as f64);

    let mut out = Vec::with_capacity(n);
    for (i, node) in nodes.iter().enumerate() {
        let col = i % COLS;
        let row = i / COLS;
        let offset_x = if row == rows - 1 && n % COLS != 0 {
            let remaining = n - row * COLS;
            (usable_w - remaining as f64 * col_gap) / 2.0
        } else { 0.0 };
        let x = margin_x + offset_x + col as f64 * col_gap + col_gap / 2.0;
        let y = margin_y + row as f64 * row_gap + row_gap / 2.0;
        out.push((node.id.clone(), (x, y)));
    }
    out
}
