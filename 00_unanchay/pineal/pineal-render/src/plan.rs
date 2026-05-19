//! `RenderPlan` — comandos materializados para backends que no
//! reciben llamadas en vivo (SVG export, snapshot testing).
//!
//! Un painter que escribe contra [`crate::Canvas`] puede ser
//! capturado en un `RenderPlan` usando un `Canvas` adapter que
//! empuja `RenderCmd`s en lugar de dibujar. El exporter consume
//! el plan y emite `<polyline>` / `<rect>` / etc.

use crate::{Color, Point, Rect, StrokeStyle};

#[derive(Debug, Clone)]
pub enum RenderCmd {
    PushClip(Rect),
    PopClip,
    FillRect { rect: Rect, color: Color },
    StrokeRect { rect: Rect, stroke: StrokeStyle },
    StrokeLine { a: Point, b: Point, stroke: StrokeStyle },
    StrokePolyline { coords: Vec<f32>, stroke: StrokeStyle },
    FillTriangleStrip { coords: Vec<f32>, colors: Vec<Color> },
    DrawText { p: Point, text: String, color: Color, size_px: f32 },
}

#[derive(Debug, Clone, Default)]
pub struct RenderPlan {
    pub cmds: Vec<RenderCmd>,
}

impl RenderPlan {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, cmd: RenderCmd) {
        self.cmds.push(cmd);
    }
}
