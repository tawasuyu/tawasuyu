//! Cámara 2D para el modo `Recorrido` (presentación espacial, tipo Prezi).
//!
//! Matemática agnóstica del lienzo infinito: convierte entre coordenadas de
//! *mundo* (donde viven los marcos) y coordenadas de *pantalla* (px del panel),
//! con zoom + paneo + giro. Es la versión extraída y generalizada del zoom/pan
//! que `tullpu-app-llimphi` resolvió inline (`factor_zoom`/`pan`/zoom-a-cursor):
//! aquí vive como tipo reusable, sin render ni DOM.
//!
//! Convención: `centro` es el punto de mundo que cae en el centro del panel;
//! `zoom` es el factor mundo→px (zoom 2.0 ⇒ 1 unidad de mundo mide 2 px);
//! `rot_rad` gira la vista (la pantalla rota `-rot` respecto al mundo, de modo
//! que un marco con `rot_rad` propio se ve recto cuando la cámara lo iguala).

/// Clamp inferior del zoom — más allá la presentación se pierde en el infinito.
pub const ZOOM_MIN: f64 = 0.02;
/// Clamp superior del zoom — más allá se pixela el contenido.
pub const ZOOM_MAX: f64 = 64.0;
/// Fracción del panel que ocupa un marco al hacer `fit_marco` (deja aire).
pub const FIT_MARGEN: f64 = 0.9;

/// Rectángulo axis-aligned. Sirve tanto para el panel (px de pantalla) como
/// para el `rect` de un marco (coords de mundo).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    /// Centro geométrico.
    pub fn centro(&self) -> (f64, f64) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

/// Función de suavizado para la interpolación entre pasos del recorrido.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ease {
    /// Velocidad constante.
    Lineal,
    /// Arranca y frena suave (`smoothstep`). Es el sabor "vuelo Prezi".
    #[default]
    SuaveInOut,
}

impl Ease {
    /// Mapea `t ∈ [0,1]` a la curva elegida (también en `[0,1]`).
    pub fn aplicar(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Lineal => t,
            Ease::SuaveInOut => t * t * (3.0 - 2.0 * t),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camara {
    /// Punto de mundo que cae en el centro del panel.
    pub centro: (f64, f64),
    /// Factor mundo→px. Siempre dentro de `[ZOOM_MIN, ZOOM_MAX]`.
    pub zoom: f64,
    /// Giro de la vista en radianes.
    pub rot_rad: f64,
}

impl Default for Camara {
    fn default() -> Self {
        Self { centro: (0.0, 0.0), zoom: 1.0, rot_rad: 0.0 }
    }
}

impl Camara {
    pub fn new(centro: (f64, f64), zoom: f64, rot_rad: f64) -> Self {
        Self { centro, zoom: zoom.clamp(ZOOM_MIN, ZOOM_MAX), rot_rad }
    }

    /// Mundo → pantalla. `panel` son los px del viewport donde se pinta.
    pub fn world_to_screen(&self, p: (f64, f64), panel: Rect) -> (f64, f64) {
        let (cx, cy) = panel.centro();
        let dx = p.0 - self.centro.0;
        let dy = p.1 - self.centro.1;
        // La pantalla rota -rot respecto al mundo.
        let (s, c) = (-self.rot_rad).sin_cos();
        let rx = dx * c - dy * s;
        let ry = dx * s + dy * c;
        (cx + rx * self.zoom, cy + ry * self.zoom)
    }

    /// Pantalla → mundo. Inversa exacta de [`world_to_screen`](Self::world_to_screen).
    pub fn screen_to_world(&self, p: (f64, f64), panel: Rect) -> (f64, f64) {
        let (cx, cy) = panel.centro();
        let sx = (p.0 - cx) / self.zoom;
        let sy = (p.1 - cy) / self.zoom;
        let (s, c) = self.rot_rad.sin_cos();
        let wx = sx * c - sy * s;
        let wy = sx * s + sy * c;
        (self.centro.0 + wx, self.centro.1 + wy)
    }

    /// Wheel zoom anclado al cursor: el punto de mundo bajo `cursor` queda
    /// fijo en pantalla tras escalar por `mult` (`mult>1` acerca). Réplica del
    /// zoom-a-cursor de tullpu. `panel`/`cursor` en px de pantalla.
    pub fn zoom_a_cursor(&mut self, mult: f64, cursor: (f64, f64), panel: Rect) {
        let ancla = self.screen_to_world(cursor, panel);
        self.zoom = (self.zoom * mult).clamp(ZOOM_MIN, ZOOM_MAX);
        // Recolocar `centro` para que `ancla` vuelva a caer bajo `cursor`.
        let (cx, cy) = panel.centro();
        let sx = (cursor.0 - cx) / self.zoom;
        let sy = (cursor.1 - cy) / self.zoom;
        let (s, c) = self.rot_rad.sin_cos();
        let wx = sx * c - sy * s;
        let wy = sx * s + sy * c;
        self.centro = (ancla.0 - wx, ancla.1 - wy);
    }

    /// Convierte un delta de pantalla (px) en su delta de mundo equivalente,
    /// deshaciendo zoom + giro. Útil para mover un objeto siguiendo al cursor
    /// (`objeto += delta`) o para panear (`centro -= delta`).
    pub fn delta_pantalla_a_mundo(&self, dx: f64, dy: f64) -> (f64, f64) {
        let (s, c) = self.rot_rad.sin_cos();
        ((dx * c - dy * s) / self.zoom, (dx * s + dy * c) / self.zoom)
    }

    /// Paneo: arrastra el contenido `(dx, dy)` px de pantalla. El punto de
    /// mundo bajo el cursor sigue al dedo.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        let (wx, wy) = self.delta_pantalla_a_mundo(dx, dy);
        self.centro = (self.centro.0 - wx, self.centro.1 - wy);
    }

    /// Cámara que centra y encuadra (`contain`) `marco` en `panel`, igualando
    /// su giro para que se vea recto. Deja `FIT_MARGEN` de aire.
    pub fn fit(marco: Rect, marco_rot_rad: f64, panel: Rect) -> Camara {
        let zw = if marco.w > 0.0 { panel.w / marco.w } else { 1.0 };
        let zh = if marco.h > 0.0 { panel.h / marco.h } else { 1.0 };
        let zoom = (zw.min(zh) * FIT_MARGEN).clamp(ZOOM_MIN, ZOOM_MAX);
        Camara { centro: marco.centro(), zoom, rot_rad: marco_rot_rad }
    }

    /// Interpola dos cámaras. El zoom se mezcla en **espacio logarítmico**
    /// (un acercamiento percibido constante — el "vuelo" suave de Prezi);
    /// centro y giro, linealmente. `t` se pasa por `ease` antes de mezclar.
    pub fn interpolar(a: &Camara, b: &Camara, t: f64, ease: Ease) -> Camara {
        let u = ease.aplicar(t);
        let lerp = |x: f64, y: f64| x + (y - x) * u;
        Camara {
            centro: (lerp(a.centro.0, b.centro.0), lerp(a.centro.1, b.centro.1)),
            zoom: (a.zoom.ln() * (1.0 - u) + b.zoom.ln() * u).exp(),
            rot_rad: lerp(a.rot_rad, b.rot_rad),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANEL: Rect = Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };

    fn aprox(a: (f64, f64), b: (f64, f64)) {
        assert!((a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6, "{a:?} != {b:?}");
    }

    #[test]
    fn centro_cae_en_centro_de_panel() {
        let cam = Camara::new((10.0, 20.0), 2.0, 0.0);
        aprox(cam.world_to_screen((10.0, 20.0), PANEL), (400.0, 300.0));
    }

    #[test]
    fn round_trip_world_screen_sin_giro() {
        let cam = Camara::new((5.0, -3.0), 1.7, 0.0);
        let p = (123.0, 456.0);
        aprox(cam.world_to_screen(cam.screen_to_world(p, PANEL), PANEL), p);
    }

    #[test]
    fn round_trip_world_screen_con_giro() {
        let cam = Camara::new((5.0, -3.0), 1.7, 0.6);
        let p = (123.0, 456.0);
        aprox(cam.world_to_screen(cam.screen_to_world(p, PANEL), PANEL), p);
    }

    #[test]
    fn zoom_a_cursor_deja_fijo_el_punto_bajo_el_cursor() {
        let mut cam = Camara::new((0.0, 0.0), 1.0, 0.0);
        let cursor = (650.0, 120.0);
        let mundo_antes = cam.screen_to_world(cursor, PANEL);
        cam.zoom_a_cursor(1.1, cursor, PANEL);
        // El mismo punto de mundo debe seguir bajo el cursor.
        aprox(cam.world_to_screen(mundo_antes, PANEL), cursor);
        assert!((cam.zoom - 1.1).abs() < 1e-9);
    }

    #[test]
    fn zoom_a_cursor_respeta_clamps() {
        let mut cam = Camara::new((0.0, 0.0), ZOOM_MAX, 0.0);
        cam.zoom_a_cursor(10.0, (400.0, 300.0), PANEL);
        assert_eq!(cam.zoom, ZOOM_MAX);
        cam.zoom = ZOOM_MIN;
        cam.zoom_a_cursor(0.001, (400.0, 300.0), PANEL);
        assert_eq!(cam.zoom, ZOOM_MIN);
    }

    #[test]
    fn pan_mueve_el_punto_bajo_el_cursor() {
        let mut cam = Camara::new((0.0, 0.0), 2.0, 0.0);
        let antes = cam.world_to_screen((0.0, 0.0), PANEL);
        cam.pan(30.0, -10.0);
        let despues = cam.world_to_screen((0.0, 0.0), PANEL);
        aprox((despues.0 - antes.0, despues.1 - antes.1), (30.0, -10.0));
    }

    #[test]
    fn fit_centra_y_encuadra() {
        let marco = Rect::new(100.0, 100.0, 400.0, 200.0);
        let cam = Camara::fit(marco, 0.0, PANEL);
        // Centra el marco.
        assert_eq!(cam.centro, (300.0, 200.0));
        // Encaja por el eje más ajustado (800/400=2 vs 600/200=3 → 2) con margen.
        assert!((cam.zoom - 2.0 * FIT_MARGEN).abs() < 1e-9);
        // El centro del marco cae en el centro del panel.
        aprox(cam.world_to_screen((300.0, 200.0), PANEL), (400.0, 300.0));
    }

    #[test]
    fn interpolar_extremos_y_zoom_logaritmico() {
        let a = Camara::new((0.0, 0.0), 1.0, 0.0);
        let b = Camara::new((10.0, 20.0), 4.0, 1.0);
        assert_eq!(Camara::interpolar(&a, &b, 0.0, Ease::Lineal), a);
        assert_eq!(Camara::interpolar(&a, &b, 1.0, Ease::Lineal), b);
        // En t=0.5 lineal el zoom es la media geométrica (2.0), no la aritmética (2.5).
        let m = Camara::interpolar(&a, &b, 0.5, Ease::Lineal);
        assert!((m.zoom - 2.0).abs() < 1e-9);
        assert_eq!(m.centro, (5.0, 10.0));
    }

    #[test]
    fn ease_suave_extremos_y_simetria() {
        assert_eq!(Ease::SuaveInOut.aplicar(0.0), 0.0);
        assert_eq!(Ease::SuaveInOut.aplicar(1.0), 1.0);
        assert!((Ease::SuaveInOut.aplicar(0.5) - 0.5).abs() < 1e-9);
        // Clamp fuera de rango.
        assert_eq!(Ease::SuaveInOut.aplicar(-1.0), 0.0);
        assert_eq!(Ease::SuaveInOut.aplicar(2.0), 1.0);
    }
}
