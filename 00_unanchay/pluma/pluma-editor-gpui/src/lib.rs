//! `fana-editor-gpui` — el backend GPUI del editor DAG.
//!
//! Consume un [`RenderPlan`] de `fana-render-plan` y lo vuelca a la
//! pantalla: los bloques de átomo y las marcas del osciloscopio son
//! `div`s posicionados en absoluto (texto y estilo nativos); los
//! conectores de dependencia van por un [`EdgesElement`] que pinta
//! paths debajo de todo.
//!
//! Es el único crate de fana visual que toca `gpui` — el resto de la
//! cadena (`core`, `graph`, `render-plan`) es agnóstico.

#![forbid(unsafe_code)]

use std::panic;

use fana_render_plan::{CoherenceTone, Edge, RenderPlan};
use gpui::{
    div, point, prelude::*, px, App, Bounds, Element, ElementId, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, PathBuilder, Pixels, SharedString, Style, Window,
};
use nahual_theme::Theme;

/// Color semántico de un estado de coherencia. Fijo, no temático: el
/// rojo de "conflicto" y el ámbar de "pendiente" son señales, no estilo.
pub fn tone_color(tone: CoherenceTone) -> Hsla {
    match tone {
        CoherenceTone::Valid => gpui::hsla(145.0 / 360.0, 0.42, 0.55, 1.0),
        CoherenceTone::Pending => gpui::hsla(42.0 / 360.0, 0.82, 0.58, 1.0),
        CoherenceTone::Conflict => gpui::hsla(2.0 / 360.0, 0.70, 0.58, 1.0),
    }
}

/// Etiqueta corta de un tono — para leyendas.
pub fn tone_label(tone: CoherenceTone) -> &'static str {
    match tone {
        CoherenceTone::Valid => "coherente",
        CoherenceTone::Pending => "por evaluar",
        CoherenceTone::Conflict => "en conflicto",
    }
}

/// `Element` que pinta los conectores de dependencia como líneas.
///
/// Llena su contenedor: posiciona cada arista relativa al origen de sus
/// bounds, igual que los `div`s absolutos de los bloques.
pub struct EdgesElement {
    edges: Vec<Edge>,
    color: Hsla,
    width: f32,
}

impl EdgesElement {
    pub fn new(edges: Vec<Edge>, color: Hsla, width: f32) -> Self {
        Self { edges, color, width }
    }
}

impl IntoElement for EdgesElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EdgesElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        style.size.height = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let ox = bounds.origin.x;
        let oy = bounds.origin.y;
        for e in &self.edges {
            let mut pb = PathBuilder::stroke(px(self.width));
            pb.move_to(point(ox + px(e.x1), oy + px(e.y1)));
            // Codo en S: baja recto, cruza, baja recto — legible aunque
            // las columnas estén separadas.
            let mid_y = oy + px((e.y1 + e.y2) * 0.5);
            pb.line_to(point(ox + px(e.x1), mid_y));
            pb.line_to(point(ox + px(e.x2), mid_y));
            pb.line_to(point(ox + px(e.x2), oy + px(e.y2)));
            if let Ok(path) = pb.build() {
                window.paint_path(path, self.color);
            }
        }
    }
}

/// Bloque de un átomo: caja posicionada en absoluto con su preview.
fn block_div(b: &fana_render_plan::AtomBlock, theme: &Theme) -> impl IntoElement {
    div()
        .absolute()
        .left(px(b.x))
        .top(px(b.y))
        .w(px(b.w))
        .h(px(b.h))
        .flex()
        .flex_col()
        .gap(px(3.))
        .p(px(8.))
        .bg(theme.bg_panel)
        .border_2()
        .border_color(tone_color(b.tone))
        .rounded(px(5.))
        .child(
            div()
                .text_size(px(10.))
                .text_color(theme.fg_muted)
                .child(SharedString::from(format!(
                    "{}  ·  profundidad {}  ·  {}",
                    b.branch,
                    b.depth,
                    tone_label(b.tone)
                ))),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(theme.fg_text)
                .child(SharedString::from(b.preview.clone())),
        )
}

/// Marca del osciloscopio de coherencia en el sidepane.
fn mark_div(m: &fana_render_plan::SidepaneMark, cfg: &fana_render_plan::LayoutConfig) -> impl IntoElement {
    let usable = (cfg.sidepane_width - 8.0).max(4.0);
    let w = (m.intensity * usable).max(3.0);
    div()
        .absolute()
        .left(px(cfg.margin))
        .top(px(m.y))
        .w(px(w))
        .h(px(m.h))
        .bg(tone_color(m.tone))
        .rounded(px(3.))
}

/// Compone el plan completo en un árbol GPUI: capa de conectores al
/// fondo, bloques y marcas encima. El resultado mide exactamente el
/// contenido — envolverlo en un contenedor con scroll para documentos
/// largos.
pub fn editor_view(plan: &RenderPlan, theme: &Theme) -> impl IntoElement {
    let cfg = plan.config;
    let content_w = plan
        .blocks
        .iter()
        .map(|b| b.x + b.w)
        .fold(0.0f32, f32::max)
        + cfg.margin;

    let blocks: Vec<_> = plan.blocks.iter().map(|b| block_div(b, theme)).collect();
    let marks: Vec<_> = plan.sidepane.iter().map(|m| mark_div(m, &cfg)).collect();

    div()
        .relative()
        .w(px(content_w.max(cfg.margin * 2.0)))
        .h(px(plan.content_height.max(cfg.margin * 2.0)))
        .child(
            div()
                .absolute()
                .left(px(0.))
                .top(px(0.))
                .size_full()
                .child(EdgesElement::new(plan.edges.clone(), theme.border_strong, 1.6)),
        )
        .children(blocks)
        .children(marks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tones_have_distinct_colors() {
        let v = tone_color(CoherenceTone::Valid);
        let p = tone_color(CoherenceTone::Pending);
        let c = tone_color(CoherenceTone::Conflict);
        assert!(v != p && p != c && v != c);
    }

    #[test]
    fn tone_labels_are_set() {
        assert_eq!(tone_label(CoherenceTone::Conflict), "en conflicto");
    }
}
