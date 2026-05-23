//! `gioser-graph-web` — widget de grafo semántico SVG inline.
//!
//! Fetchea `GET /graph` de la API de gioser, parsea nodos + aristas,
//! y renderiza un grafo SVG interactivo dentro de un contenedor dado.
//! Los nodos son clicleables: al hacer clic en un nodo se navega a la
//! página correspondiente (o se pasa un callback).
//!
//! ## Layout
//!
//! Usa un layout force-directed simple (Fruchterman-Reingold básico)
//! implementado en Rust/WASM. No requiere canvas WebGL ni librerías
//! externas. El SVG se renderiza inline y escala responsivamente.
//!
//! ## Contrato DOM
//!
//! El caller pasa un `<div>` contenedor y un callback `on_navigate(doc_id)`.
//! El widget monta un `<svg>` dentro con viewBox fijo.
//!
//! ## Ejemplo
//!
//! ```ignore
//! let container = document.get_element_by_id("graph-container")
//!     .unwrap().dyn_into::<HtmlElement>().unwrap();
//! let graph = GraphWidget::new(container, api_url);
//! graph.load().await;
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::Promise;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Document, HtmlElement, MouseEvent, Response, SvgCircleElement, SvgLineElement,
    SvgsvgElement, SvgTextElement,
};

/// Helper para obtener el document desde web-sys. Se llama desde los métodos
/// de GraphWidget sin depender de la referencia pasada (aunque la tenemos).
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
const CANVAS_H: f64 = 260.0;
const NODE_RADIUS: f64 = 20.0;

// Paleta por camino (misma convención que gioser-web CSS)
const CAMINO_COLORS: &[(&str, &str)] = &[
    ("logos", "#d0dbff"),  // aire
    ("aire", "#d0dbff"),   // aire (alias)
    ("nomos", "#f59056"),  // fuego
    ("fuego", "#f59056"),  // fuego (alias)
    ("kay", "#d49873"),    // tierra
    ("tierra", "#d49873"), // tierra (alias)
    ("uku", "#6cd0f3"),    // agua
    ("agua", "#6cd0f3"),   // agua (alias)
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
    /// Crea un nuevo GraphWidget. `container` es el div donde se monta el SVG.
    /// `api_url` es la URL base de la API de grafo (sin trailing slash).
    /// `on_navigate` se llama cuando el usuario hace clic en un nodo,
    /// pasando el `doc_id` del nodo.
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

    /// Fetchea `/graph` de la API, aplica layout force-directed y renderiza.
    pub async fn load(&mut self) -> Result<(), JsValue> {
        let url = format!("{}/graph?limit=500", self.api_url);
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;

        let resp_value = JsFuture::from(window.fetch_with_str(&url)).await?;
        let resp: web_sys::Response = resp_value.dyn_into()?;
        if !resp.ok() {
            return Err(JsValue::from_str(&format!("HTTP {}", resp.status())));
        }
        let text = JsFuture::from(resp.text()?).await?;
        let body = text.as_string().unwrap_or_default();

        let graph: GraphResponse =
            serde_json::from_str(&body).map_err(|e| JsValue::from_str(&format!("JSON: {e}")))?;

        // Solo nodos de nuestro corpus (que tengan doc_id)
        let nodes: Vec<NodeData> = graph
            .nodes
            .into_iter()
            .map(|n| n.data)
            .filter(|n| n.doc_id.is_some())
            .collect();
        let edges: Vec<EdgeData> = graph.edges.into_iter().map(|e| e.data).collect();

        self.nodes = nodes;
        self.edges = edges;

        self.render();
        Ok(())
    }

    /// Renderiza el SVG con layout force-directed simple.
    fn render(&self) {
        // Limpiar contenedor
        self.container.set_inner_html("");

        if self.nodes.is_empty() {
            return;
        }

        // Force-directed layout: Fruchterman-Reingold simple
        let positions = force_layout(&self.nodes, &self.edges, CANVAS_W, CANVAS_H);

        let ns = "http://www.w3.org/2000/svg";
        let svg: SvgsvgElement = self
            .document
            .create_element_ns(Some(ns), "svg")
            .unwrap()
            .dyn_into()
            .unwrap();
        svg.set_attribute("viewBox", &format!("0 0 {} {}", CANVAS_W, CANVAS_H)).ok();
        svg.set_attribute("width", "100%").ok();
        svg.set_attribute("height", &format!("{}px", CANVAS_H as u32)).ok();
        svg.style()
            .set_property("display", "block")
            .ok();
        svg.style()
            .set_property("margin", "1.5rem auto 0")
            .ok();
        svg.style()
            .set_property("max-width", "100%")
            .ok();

        // Fondo sutil del SVG
        svg.style()
            .set_property("background", "rgba(255,255,255,0.02)")
            .ok();
        svg.style()
            .set_property("border-radius", "12px")
            .ok();
        svg.style()
            .set_property("border", "1px solid rgba(216,168,93,0.15)")
            .ok();

        // Aristas
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
                line.set_attribute("stroke", "rgba(255,255,255,0.12)").ok();
                line.set_attribute("stroke-width", "1.0").ok();
                // Si hay weight, opacidad proporcional
                if let Some(w) = edge.weight {
                    let alpha = ((w - 0.5) * 2.0).clamp(0.1, 0.8);
                    line.set_attribute("stroke-opacity", &format!("{:.2}", alpha)).ok();
                }
                svg.append_child(&line).ok();
            }
        }

        // Nodos
        let on_nav = self.on_navigate.clone();
        for (i, node) in self.nodes.iter().enumerate() {
            let (x, y) = positions.get(i).map(|(_, p)| *p).unwrap_or((100.0, 100.0));
            let color = camino_color(&node.camino).to_string();

            // Círculo
            let circle: SvgCircleElement = self
                .document
                .create_element_ns(Some(ns), "circle")
                .unwrap()
                .dyn_into()
                .unwrap();
            circle.set_attribute("cx", &format!("{:.1}", x)).ok();
            circle.set_attribute("cy", &format!("{:.1}", y)).ok();
            circle.set_attribute("r", &format!("{:.1}", NODE_RADIUS)).ok();
            circle.set_attribute("fill", &color).ok();
            circle.set_attribute("fill-opacity", "0.35").ok();
            circle.set_attribute("stroke", &color).ok();
            circle.set_attribute("stroke-width", "2").ok();
            circle.set_attribute("cursor", "pointer").ok();

            // Glow
            circle.style()
                .set_property("filter", "drop-shadow(0 0 6px rgba(255,255,255,0.1))")
                .ok();
            circle.style()
                .set_property("transition", "all 250ms ease")
                .ok();

            // Hover
            let doc_id = node.doc_id.clone().unwrap_or_default();
            let preview = node.preview.clone().unwrap_or_default();
            let name = node.name.clone();
            let circle_clone = circle.clone();
            let on_nav_clone = on_nav.clone();

            let mouseenter = Closure::<dyn FnMut(MouseEvent)>::new(move |_| {
                circle_clone
                    .set_attribute("fill-opacity", "0.6")
                    .ok();
                circle_clone.style()
                    .set_property("filter", &format!("drop-shadow(0 0 12px {})", color))
                    .ok();
            });
            circle
                .add_event_listener_with_callback("mouseenter", mouseenter.as_ref().unchecked_ref())
                .ok();
            mouseenter.forget();

            let circle_clone2 = circle.clone();
            let mouseleave = Closure::<dyn FnMut(MouseEvent)>::new(move |_| {
                circle_clone2
                    .set_attribute("fill-opacity", "0.35")
                    .ok();
                circle_clone2.style()
                    .set_property("filter", "drop-shadow(0 0 6px rgba(255,255,255,0.1))")
                    .ok();
            });
            circle
                .add_event_listener_with_callback("mouseleave", mouseleave.as_ref().unchecked_ref())
                .ok();
            mouseleave.forget();

            let circle_clone3 = circle.clone();
            let on_nav_clone2 = on_nav.clone();
            let doc_id_clone = doc_id.clone();
            let click = Closure::<dyn FnMut(MouseEvent)>::new(move |_| {
                let mut cb = on_nav_clone2.borrow_mut();
                if let Some(ref mut f) = *cb {
                    f(doc_id_clone.clone());
                }
            });
            circle
                .add_event_listener_with_callback("click", click.as_ref().unchecked_ref())
                .ok();
            click.forget();

            svg.append_child(&circle).ok();

            // Título del nodo (abreviado si muy largo)
            let label = if name.len() > 20 {
                format!("{}…", &name[..18])
            } else {
                name.clone()
            };

            let text: SvgTextElement = self
                .document
                .create_element_ns(Some(ns), "text")
                .unwrap()
                .dyn_into()
                .unwrap();
            text.set_attribute("x", &format!("{:.1}", x)).ok();
            text.set_attribute("y", &format!("{:.1}", y + 36.0)).ok();
            text.set_attribute("text-anchor", "middle").ok();
            text.set_attribute("fill", "rgba(232,234,245,0.6)").ok();
            text.set_attribute("font-size", "9").ok();
            text.set_attribute("font-family", "Inter, sans-serif").ok();
            text.set_text_content(Some(&label));
            svg.append_child(&text).ok();

            // Tooltip sutil (title attribute)
            // El título del elemento svg funciona como tooltip nativo
            let title_el = self
                .document
                .create_element("title")
                .ok();
            if let Some(title_el) = title_el {
                title_el.set_text_content(Some(&format!(
                    "{} — {}",
                    name,
                    node.camino.to_uppercase()
                )));
                svg.append_child(&title_el).ok(); // se lo ponemos al svg, no por nodo
                // Mejor: ponemos title a cada círculo
                circle.set_attribute("title", &format!("{} — {}", name, node.camino.to_uppercase())).ok();
            }
        }

        self.container.append_child(&svg).ok();
    }
}

// ─── Force-directed layout (Fruchterman-Reingold) ────────────────
//
// Implementación inline para no depender de petgraph. Layout 2D
// con repulsión de Coulomb, atracción de resorte en aristas.

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
    let k = (area / (n as f64)).sqrt();

    // Inicializar posiciones en círculo
    let cx = w / 2.0;
    let cy = h / 2.0;
    let radius = (w.min(h) * 0.35).max(50.0);
    let mut positions: Vec<(f64, f64)> = nodes
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let angle = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
            (cx + radius * angle.cos(), cy + radius * angle.sin())
        })
        .collect();

    // Índice de nodo por id para lookup rápido de aristas
    let id_to_idx: std::collections::HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.doc_id.as_deref().unwrap_or(""), i))
        .filter(|(id, _)| !id.is_empty())
        .collect();

    // Construir adjacency: edge_ids
    let mut adj: Vec<Vec<usize>> = vec![vec![]; n];
    for e in edges {
        if let (Some(&si), Some(&ti)) = (id_to_idx.get(e.source.as_str()), id_to_idx.get(e.target.as_str())) {
            if !adj[si].contains(&ti) {
                adj[si].push(ti);
            }
            if !adj[ti].contains(&si) {
                adj[ti].push(si);
            }
        }
    }

    // Iteraciones
    let iterations = 60;
    let temp_init = w.max(h) / 8.0;

    let mut disp: Vec<(f64, f64)> = vec![(0.0, 0.0); n];

    for iter in 0..iterations {
        let temp = temp_init * (1.0 - (iter as f64) / (iterations as f64));

        // Reset displacements
        for d in disp.iter_mut() {
            *d = (0.0, 0.0);
        }

        // Repulsión: Coulomb entre todo par
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = positions[i].0 - positions[j].0;
                let dy = positions[i].1 - positions[j].1;
                let dist = (dx * dx + dy * dy).sqrt().max(1.0);
                let force = k * k / dist;
                let fx = force * dx / dist;
                let fy = force * dy / dist;
                disp[i].0 += fx;
                disp[i].1 += fy;
                disp[j].0 -= fx;
                disp[j].1 -= fy;
            }
        }

        // Atracción: Hooke en aristas
        for i in 0..n {
            for &j in &adj[i] {
                let dx = positions[j].0 - positions[i].0;
                let dy = positions[j].1 - positions[i].1;
                let dist = (dx * dx + dy * dy).sqrt().max(1.0);
                let force = dist * dist / k;
                let fx = force * dx / dist;
                let fy = force * dy / dist;
                disp[i].0 += fx;
                disp[i].1 += fy;
                disp[j].0 -= fx;
                disp[j].1 -= fy;
            }
        }

        // Aplicar desplazamientos con temperatura
        let margin = NODE_RADIUS + 8.0;
        for i in 0..n {
            let d = (disp[i].0 * disp[i].0 + disp[i].1 * disp[i].1)
                .sqrt()
                .max(0.001);
            let step = disp[i].0.min(temp).max(-temp);
            let step_y = disp[i].1.min(temp).max(-temp);
            let new_x = (positions[i].0 + (step / d) * temp).clamp(margin, w - margin);
            let new_y = (positions[i].1 + (step_y / d) * temp).clamp(margin, h - margin);
            positions[i] = (new_x, new_y);
        }
    }

    nodes
        .iter()
        .zip(positions.into_iter())
        .map(|(n, pos)| (n.doc_id.clone().unwrap_or_default(), pos))
        .collect()
}
