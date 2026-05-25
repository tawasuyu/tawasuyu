//! Cámara 2D: pan + zoom con zoom anclado a un punto de pantalla.

/// Transformación world↔screen. `screen = (world - pan) * zoom`.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub pan: (f32, f32),
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { pan: (0.0, 0.0), zoom: 1.0 }
    }
}

impl Camera {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn world_to_screen(&self, w: (f32, f32)) -> (f32, f32) {
        ((w.0 - self.pan.0) * self.zoom, (w.1 - self.pan.1) * self.zoom)
    }

    pub fn screen_to_world(&self, s: (f32, f32)) -> (f32, f32) {
        (s.0 / self.zoom + self.pan.0, s.1 / self.zoom + self.pan.1)
    }

    /// Desplaza la cámara `delta` pixels de pantalla.
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        self.pan.0 -= dx / self.zoom;
        self.pan.1 -= dy / self.zoom;
    }

    /// Zoom multiplicando por `factor`, manteniendo fijo el punto de
    /// pantalla `anchor` (el world-point bajo el cursor no se mueve).
    pub fn zoom_at(&mut self, anchor: (f32, f32), factor: f32) {
        let before = self.screen_to_world(anchor);
        self.zoom = (self.zoom * factor).clamp(0.01, 1000.0);
        let after = self.screen_to_world(anchor);
        self.pan.0 += before.0 - after.0;
        self.pan.1 += before.1 - after.1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn world_screen_roundtrip() {
        let mut cam = Camera::new();
        cam.pan = (10.0, 20.0);
        cam.zoom = 2.0;
        let w = (33.0, 44.0);
        assert!(close(cam.screen_to_world(cam.world_to_screen(w)), w));
    }

    #[test]
    fn zoom_at_keeps_anchor_world_point_fixed() {
        let mut cam = Camera::new();
        let anchor = (100.0, 80.0);
        let before = cam.screen_to_world(anchor);
        cam.zoom_at(anchor, 2.5);
        let after = cam.screen_to_world(anchor);
        assert!(close(before, after), "el punto bajo el cursor no se mueve");
    }
}
