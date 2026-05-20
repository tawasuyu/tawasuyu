//! `dominium-canvas-gpui` — el único crate de dominium que importa `gpui`.
//!
//! Toda la cadena `dominium-core → physics → iso → render-plan` es
//! agnóstica de backend. Este crate cierra el circuito: un [`Element`]
//! de GPUI que recibe un [`RenderPlan`] ya resuelto y lo vuelca a
//! `paint_quad`, centrando la maqueta en los bounds disponibles.
//!
//! Si mañana el frontend fuera web o TUI, se escribe un
//! `dominium-canvas-web` hermano sin tocar una línea del núcleo.

#![forbid(unsafe_code)]

use std::panic;

use dominium_render_plan::{Color, RenderPlan};
use gpui::{
    fill, hsla, point, px, size, App, Bounds, Element, ElementId, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, Pixels, Style, Window,
};

/// Convierte un color RGBA lineal (`[f32;4]`) a `Hsla`, que es lo que
/// GPUI consume. Misma convención de conversión que el backend de
/// `pineal` — sin gamma.
pub fn rgba_to_hsla(c: Color) -> Hsla {
    let (r, g, b, a) = (c[0], c[1], c[2], c[3]);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let delta = max - min;
    if delta.abs() < 1e-6 {
        return hsla(0.0, 0.0, l, a);
    }
    let s = if l < 0.5 {
        delta / (max + min)
    } else {
        delta / (2.0 - max - min)
    };
    let h = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    hsla(h / 6.0, s, l, a)
}

/// `Element` GPUI que pinta una maqueta isométrica.
///
/// Construir uno nuevo en cada `render()` del host con el `RenderPlan`
/// del frame actual — el Element no guarda estado entre frames.
pub struct DominiumCanvas {
    plan: RenderPlan,
    background: Option<Hsla>,
}

impl DominiumCanvas {
    /// Envuelve un `RenderPlan` listo para pintar.
    pub fn new(plan: RenderPlan) -> Self {
        Self { plan, background: None }
    }

    /// Pinta un fondo sólido antes de los quads.
    pub fn background(mut self, color: Hsla) -> Self {
        self.background = Some(color);
        self
    }
}

impl IntoElement for DominiumCanvas {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for DominiumCanvas {
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
        let ox: f32 = bounds.origin.x.into();
        let oy: f32 = bounds.origin.y.into();
        let bw: f32 = bounds.size.width.into();
        let bh: f32 = bounds.size.height.into();

        if let Some(bg) = self.background {
            window.paint_quad(fill(bounds, bg));
        }

        // Centra la maqueta: el centro de la caja envolvente del plan
        // se alinea con el centro de los bounds del Element.
        let plan_cx = (self.plan.min_x + self.plan.max_x) * 0.5;
        let plan_cy = (self.plan.min_y + self.plan.max_y) * 0.5;
        let off_x = ox + bw * 0.5 - plan_cx;
        let off_y = oy + bh * 0.5 - plan_cy;

        // Los quads ya vienen ordenados de atrás hacia adelante.
        for q in &self.plan.quads {
            let rect = Bounds {
                origin: point(px(q.x + off_x), px(q.y + off_y)),
                size: size(px(q.w), px(q.h)),
            };
            window.paint_quad(fill(rect, rgba_to_hsla(q.color)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_red_maps_to_hue_zero() {
        let h = rgba_to_hsla([1.0, 0.0, 0.0, 1.0]);
        assert!((h.h - 0.0).abs() < 1e-6);
        assert!((h.s - 1.0).abs() < 1e-6);
        assert!((h.l - 0.5).abs() < 1e-6);
    }

    #[test]
    fn grey_has_zero_saturation() {
        let h = rgba_to_hsla([0.4, 0.4, 0.4, 0.8]);
        assert!((h.s - 0.0).abs() < 1e-6);
        assert!((h.a - 0.8).abs() < 1e-6);
    }

    #[test]
    fn alpha_passes_through() {
        let h = rgba_to_hsla([0.0, 0.0, 1.0, 0.25]);
        assert!((h.a - 0.25).abs() < 1e-6);
    }
}
