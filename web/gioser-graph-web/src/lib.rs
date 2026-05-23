//! `gioser-graph-web` — widget de grafo semántico SVG inline.
//!
//! Fetchea `GET /graph` de la API de gioser, parsea nodos + aristas,
//! y renderiza un grafo SVG interactivo dentro de un contenedor dado.
//!
//! Los nodos son **rectángulos redondeados** horizontales con el texto
//! dentro (no círculos) para mejor legibilidad. Las aristas varían en
//! grosor según la intensidad semántica (k-NN weight).
//!
//! ## Contrato DOM
//!
//! El caller pasa un `<div>` contenedor y un callback `on_navigate(doc_id)`.
//! El widget monta un `<svg>` dentro con viewBox fijo.

use std::cell::RefCell;
use std::rc::Rc;

use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Document, HtmlElement, MouseEvent, Response, SvgLineElement, SvgRectElement,
    SvgsvgElement, SvgTextElement,
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
    chunk: Option<u32>,
    tags: Option<Vec<String>>,
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

const CANVAS_W: f64 = 600.0;
const CANVAS_H: f64 = 270.0;
/// Ancho del rectángulo nodo (horizontal para texto largo).
const NODE_W: f64 = 120.0;
/// Alto del rectángulo nodo.
const NODE_H: f64 = 28.0;

const CAMINO_COLORS: &[(&str, &str)] = &[
    ("logos", "#d0dbff"),
    ("aire", "#d0dbff"),
    ("nomos", "#f59056"),
    ("fuego", "#f59056"),
    ("kay", "#d49873"),
    ("tierra", "#d49873"),
    ("uku", "#6cd0f3"),
    ("agua", "#6cd0f3"),
];

fn camino_color(camino: &str) -> &str {
    for (k, v) in CAMINO_COLORS {
        if *k == camino {
            return v;
        }
    }
    "#888888"
}

pub struct GraphWidget {
    container: HtmlElement,
    api_url: String,
    svg: Option<SvgsvgElement>,
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
            svg: None,
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

        let nodes: Vec<NodeData> = graph
            .nodes
            .into_iter()
            .map(|n| n.data)
            .filter(|n| n.doc_id.is_some())
            .collect();
        let edges: Vec<EdgeData> = graph.edges.into_iter().map(|e| e.data).collect();

        self.nodes = nodes;
        self.edges = edges;

        // Pequeño delay para evitar "Layout was forced before fully loaded"
        let _ = js_sys::Promise::resolve(&JsValue::NULL);
        let mut_self = &*self as *const GraphWidget;
        // Render síncrono, el delay no es necesario pero mantenemos la deferencia.
        self.render();
        Ok(())
    }

    fn render(&self) {
        self.container.set_inner_html("");

        if self.nodes.is_empty() {
            return;
        }

        let positions = force_layout(&self.nodes, &self.edges, CANVAS_W, CANVAS_H);

        let ns = "http://www.w3.org/2000/svg";
        let svg: SvgsvgElement = self
            .document
            .create_element_ns(Some(ns), "svg")
            .unwrap()
            .dyn_into()
            .unwrap();
        svg.set_attribute("viewBox", &format!("0 0 {} {}", CANVAS_W as u32, CANVAS_H as u32))
            .ok();
        svg.set_attribute("width", "100%").ok();
        svg.set_attribute("height", &format!("{}px", CANVAS_H as u32)).ok();
        svg.style().set_property("display", "block").ok();
        svg.style().set_property("margin", "1.5rem auto 0").ok();
        svg.style().set_property("max-width", "100%").ok();
        svg.style()
            .set_property("background", "rgba(255,255,255,0.02)")
            .ok();
        svg.style().set_property("border-radius", "12px").ok();
        svg.style()
            .set_property("border", "1px solid rgba(216,168,93,0.15)")
            .ok();

        // ── Aristas con grosor proporcional al weight ──
        for edge in &self.edges {
            let src_pos = positions.iter().find(|(id, _)| *id == edge.source);
            let tgt_pos = positions.iter().find(|(id, _)| *id == edge.target);
            if let (Some((_, (x1, y1))), Some((_, (x2, y2)))) = (src_pos, tgt_pos) {
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

                // Grosor según peso: 0.5→1, 1.0→4 (clamped)
                let sw = edge
                    .weight
                    .map(|w| 0.5 + (w - 0.5) * 6.0)
                    .unwrap_or(1.0);
                line.set_attribute("stroke", "rgba(255,255,255,0.20)").ok();
                line.set_attribute("stroke-width", &format!("{:.1}", sw.clamp(0.5, 5.0)))
                    .ok();

                svg.append_child(&line).ok();
            }
        }

        // ── Nodos como rectángulos con texto dentro ──
        let on_nav = self.on_navigate.clone();
        let ns_local = ns; // copy for closure captures
        for (i, node) in self.nodes.iter().enumerate() {
            let (cx, cy) = positions.get(i).map(|(_, p)| *p).unwrap_or((100.0, 100.0));
            let color = camino_color(&node.camino).to_string();
            let label = if node.name.len() > 18 {
                format!("{}…", &node.name[..16])
            } else {
                node.name.clone()
            };
            let camino_up = node.camino.to_uppercase();

            // Grupo contenedor (para hover + click)
            let g: web_sys::SvgElement = self
                .document
                .create_element_ns(Some(ns_local), "g")
                .unwrap()
                .dyn_into()
                .unwrap();
            g.style().set_property("cursor", "pointer").ok();
            g.set_attribute("title", &format!("{} — {}", node.name, camino_up)).ok();

            // Rectángulo redondeado
            let rect: SvgRectElement = self
                .document
                .create_element_ns(Some(ns_local), "rect")
                .unwrap()
                .dyn_into()
                .unwrap();
            let rx = cx - NODE_W / 2.0;
            let ry = cy - NODE_H / 2.0;
            rect.set_attribute("x", &format!("{:.1}", rx)).ok();
            rect.set_attribute("y", &format!("{:.1}", ry)).ok();
            rect.set_attribute("width", &format!("{:.1}", NODE_W)).ok();
            rect.set_attribute("height", &format!("{:.1}", NODE_H)).ok();
            rect.set_attribute("rx", "6").ok();
            rect.set_attribute("ry", "6").ok();
            rect.set_attribute("fill", &color).ok();
            rect.set_attribute("fill-opacity", "0.25").ok();
            rect.set_attribute("stroke", &color).ok();
            rect.set_attribute("stroke-width", "1.5").ok();
            rect.style().set_property("transition", "all 200ms ease").ok();
            rect.style()
                .set_property("filter", "drop-shadow(0 0 4px rgba(255,255,255,0.06))")
                .ok();

            // Texto dentro del rectángulo
            let text: SvgTextElement = self
                .document
                .create_element_ns(Some(ns_local), "text")
                .unwrap()
                .dyn_into()
                .unwrap();
            text.set_attribute("x", &format!("{:.1}", cx)).ok();
            text.set_attribute("y", &format!("{:.1}", cy + 5.0)).ok();
            text.set_attribute("text-anchor", "middle").ok();
            text.set_attribute("dominant-baseline", "middle").ok();
            text.set_attribute("fill", "rgba(232,234,245,0.85)").ok();
            text.set_attribute("font-size", "12").ok();
            text.set_attribute("font-family", "Inter, system-ui, sans-serif").ok();
            text.set_attribute("font-weight", "500").ok();
            text.set_text_content(Some(&label));

            // Subtexto (camino) más pequeño debajo
            let sub: SvgTextElement = self
                .document
                .create_element_ns(Some(ns_local), "text")
                .unwrap()
                .dyn_into()
                .unwrap();
            sub.set_attribute("x", &format!("{:.1}", cx)).ok();
            sub.set_attribute("y", &format!("{:.1}", cy + 19.0)).ok();
            sub.set_attribute("text-anchor", "middle").ok();
            sub.set_attribute("dominant-baseline", "middle").ok();
            sub.set_attribute("fill", "rgba(232,234,245,0.40)").ok();
            sub.set_attribute("font-size", "8").ok();
            sub.set_attribute("font-family", "Inter, system-ui, sans-serif").ok();
            sub.set_attribute("letter-spacing", "0.3em").ok();
            sub.set_text_content(Some(&camino_up));

            g.append_child(&rect).ok();
            g.append_child(&text).ok();
            g.append_child(&sub).ok();

            // Hover: opacidad más alta
            let rect_clone = rect.clone();
            let color_c = color.clone();
            let enter = Closure::<dyn FnMut(MouseEvent)>::new(move |_| {
                rect_clone.set_attribute("fill-opacity", "0.50").ok();
                rect_clone
                    .style()
                    .set_property(
                        "filter",
                        &format!("drop-shadow(0 0 10px {})", color_c),
                    )
                    .ok();
            });
            g.add_event_listener_with_callback("mouseenter", enter.as_ref().unchecked_ref())
                .ok();
            enter.forget();

            let rect_clone2 = rect.clone();
            let leave = Closure::<dyn FnMut(MouseEvent)>::new(move |_| {
                rect_clone2
                    .set_attribute("fill-opacity", "0.25")
                    .ok();
                rect_clone2
                    .style()
                    .set_property("filter", "drop-shadow(0 0 4px rgba(255,255,255,0.06))")
                    .ok();
            });
            g.add_event_listener_with_callback("mouseleave", leave.as_ref().unchecked_ref())
                .ok();
            leave.forget();

            // Click
            let doc_id = node.doc_id.clone().unwrap_or_default();
            let on_nav2 = on_nav.clone();
            let click = Closure::<dyn FnMut(MouseEvent)>::new(move |_| {
                let mut cb = on_nav2.borrow_mut();
                if let Some(ref mut f) = *cb {
                    f(doc_id.clone());
                }
            });
            g.add_event_listener_with_callback("click", click.as_ref().unchecked_ref())
                .ok();
            click.forget();

            svg.append_child(&g).ok();
        }

        self.container.append_child(&svg).ok();
    }
}

// ─── Force-directed layout (Fruchterman-Reingold) ────────────────

fn force_layout(
    nodes: &[NodeData],
    edges: &[EdgeData],
    w: f64,
    h: f64,
) -> Vec<(String, (f64, f64))> {
    let n = nodes.len();
    if n == 0 {
        return vec![];
    }

    let area = w * h;
    let k = (area / (n as f64)).sqrt() * 1.6; // más separación

    let cx = w / 2.0;
    let cy = h / 2.0;
    let radius = (w.min(h) * 0.30).max(60.0);
    let mut positions: Vec<(f64, f64)> = nodes
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64)
                - std::f64::consts::PI / 2.0;
            (cx + radius * angle.cos(), cy + radius * angle.sin())
        })
        .collect();

    let id_to_idx: std::collections::HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.doc_id.as_deref().unwrap_or(""), i))
        .filter(|(id, _)| !id.is_empty())
        .collect();

    let mut adj: Vec<Vec<usize>> = vec![vec![]; n];
    for e in edges {
        if let (Some(&si), Some(&ti)) =
            (id_to_idx.get(e.source.as_str()), id_to_idx.get(e.target.as_str()))
        {
            if !adj[si].contains(&ti) {
                adj[si].push(ti);
            }
            if !adj[ti].contains(&si) {
                adj[ti].push(si);
            }
        }
    }

    let iterations = 80;
    let temp_init = w.max(h) / 5.0;
    let mut disp: Vec<(f64, f64)> = vec![(0.0, 0.0); n];
    let half_w = NODE_W / 2.0 + 6.0;
    let half_h = NODE_H / 2.0 + 4.0;

    for iter in 0..iterations {
        let temp = temp_init * (1.0 - (iter as f64) / (iterations as f64));

        for d in disp.iter_mut() {
            *d = (0.0, 0.0);
        }

        // Repulsión
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = positions[i].0 - positions[j].0;
                let dy = positions[i].1 - positions[j].1;
                let dist = (dx * dx + dy * dy).sqrt().max(1.0);
                let force = k * k / dist;
                disp[i].0 += force * dx / dist;
                disp[i].1 += force * dy / dist;
                disp[j].0 -= force * dx / dist;
                disp[j].1 -= force * dy / dist;
            }
        }

        // Atracción en aristas
        for i in 0..n {
            for &j in &adj[i] {
                let dx = positions[j].0 - positions[i].0;
                let dy = positions[j].1 - positions[i].1;
                let dist = (dx * dx + dy * dy).sqrt().max(1.0);
                let force = dist * dist / k;
                disp[i].0 += force * dx / dist;
                disp[i].1 += force * dy / dist;
                disp[j].0 -= force * dx / dist;
                disp[j].1 -= force * dy / dist;
            }
        }

        // Aplicar
        for i in 0..n {
            let d = (disp[i].0 * disp[i].0 + disp[i].1 * disp[i].1)
                .sqrt()
                .max(0.001);
            let step_x = (disp[i].0 / d * temp).clamp(-temp, temp);
            let step_y = (disp[i].1 / d * temp).clamp(-temp, temp);
            let new_x = (positions[i].0 + step_x).clamp(half_w, w - half_w);
            let new_y = (positions[i].1 + step_y).clamp(half_h, h - half_h);
            positions[i] = (new_x, new_y);
        }
    }

    nodes
        .iter()
        .zip(positions.into_iter())
        .map(|(n, pos)| (n.doc_id.clone().unwrap_or_default(), pos))
        .collect()
}
