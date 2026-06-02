//! Geometría del video en pantalla (V2): aspect ratio / crop / zoom / pan.
//! Núcleo **puro** (regla #2): dado el tamaño del frame fuente, el tamaño del
//! área de dibujo y los ajustes del usuario, calcula qué porción del frame
//! muestrear (`src`) y dónde dibujarla (`dst`). El blit en sí (subir la
//! textura, recortar al viewport) lo hace la UI; acá sólo vive la aritmética,
//! 100% testeable sin GPU.
//!
//! Modela lo de mpv/VLC: modos de encaje (`Fit`/`Fill`/`Stretch`/`Original`),
//! relación de aspecto forzada (4:3, 16:9, 2.35:1…), zoom multiplicativo y
//! paneo. El `dst` puede exceder el viewport (en `Fill`/zoom/pan): la UI lo
//! recorta a su área.

use serde::{Deserialize, Serialize};

/// Cómo encaja el video en el viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FitMode {
    /// Cabe entero preservando aspecto (let/pillarbox). El default.
    Fit,
    /// Cubre todo el viewport preservando aspecto, recortando el sobrante.
    Fill,
    /// Estira ignorando el aspecto (llena exacto).
    Stretch,
    /// 1:1 píxel, centrado (puede exceder o dejar bordes).
    Original,
}

impl Default for FitMode {
    fn default() -> Self {
        FitMode::Fit
    }
}

impl FitMode {
    /// Cicla al siguiente modo (tecla de "aspect ratio" estilo VLC).
    pub fn next(self) -> FitMode {
        match self {
            FitMode::Fit => FitMode::Fill,
            FitMode::Fill => FitMode::Stretch,
            FitMode::Stretch => FitMode::Original,
            FitMode::Original => FitMode::Fit,
        }
    }

    /// Nombre legible para el OSD/menú.
    pub fn label(self) -> &'static str {
        match self {
            FitMode::Fit => "Ajustar",
            FitMode::Fill => "Llenar",
            FitMode::Stretch => "Estirar",
            FitMode::Original => "Original",
        }
    }
}

/// Recorte fraccional de los bordes del frame fuente (`[0, ~0.45]` cada lado).
/// Pensado para podar barras negras o reencuadrar.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Default for Crop {
    fn default() -> Self {
        Crop { left: 0.0, right: 0.0, top: 0.0, bottom: 0.0 }
    }
}

impl Crop {
    pub fn is_identity(&self) -> bool {
        self.left == 0.0 && self.right == 0.0 && self.top == 0.0 && self.bottom == 0.0
    }
}

/// Ajustes de visualización del usuario. Versionado como los demás controles
/// (`ColorControl`/`TransformControl`) para que la UI detecte cambios.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewControl {
    pub fit: FitMode,
    /// Relación de aspecto forzada (ancho/alto); `None` = la nativa del frame
    /// (tras el crop). Útil para corregir PAR raro o forzar 16:9.
    pub aspect: Option<f32>,
    /// Zoom multiplicativo extra sobre el encaje (`1.0` = sin zoom).
    pub zoom: f32,
    /// Paneo en fracción del viewport (`-1.0..1.0`); `0,0` centrado.
    pub pan_x: f32,
    pub pan_y: f32,
    pub crop: Crop,
    version: u64,
}

impl Default for ViewControl {
    fn default() -> Self {
        ViewControl {
            fit: FitMode::default(),
            aspect: None,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            crop: Crop::default(),
            version: 0,
        }
    }
}

impl ViewControl {
    /// Límites de cada parámetro (compartidos por mutadores y `sanitized`).
    pub const ZOOM_MIN: f32 = 0.25;
    pub const ZOOM_MAX: f32 = 8.0;
    pub const PAN_LIMIT: f32 = 1.0;
    pub const CROP_MAX: f32 = 0.45;

    pub fn version(&self) -> u64 {
        self.version
    }

    /// ¿Está en su estado neutro (encaje Fit, sin aspecto/zoom/pan/crop)?
    pub fn is_identity(&self) -> bool {
        self.fit == FitMode::Fit
            && self.aspect.is_none()
            && self.zoom == 1.0
            && self.pan_x == 0.0
            && self.pan_y == 0.0
            && self.crop.is_identity()
    }

    fn touch(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    /// Cicla el modo de encaje (resetea zoom/pan para no acumular sorpresas).
    pub fn cycle_fit(&mut self) {
        self.fit = self.fit.next();
        self.zoom = 1.0;
        self.pan_x = 0.0;
        self.pan_y = 0.0;
        self.touch();
    }

    pub fn set_fit(&mut self, fit: FitMode) {
        self.fit = fit;
        self.touch();
    }

    /// Fija una relación de aspecto forzada (o `None` para la nativa).
    pub fn set_aspect(&mut self, aspect: Option<f32>) {
        self.aspect = aspect.filter(|a| a.is_finite() && *a > 0.0);
        self.touch();
    }

    /// Multiplica el zoom por `factor` (p. ej. `1.1`/`0.9`), con clamp.
    pub fn zoom_by(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.zoom = (self.zoom * factor).clamp(Self::ZOOM_MIN, Self::ZOOM_MAX);
            self.touch();
        }
    }

    /// Desplaza el paneo (en fracción del viewport), con clamp.
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        self.pan_x = (self.pan_x + dx).clamp(-Self::PAN_LIMIT, Self::PAN_LIMIT);
        self.pan_y = (self.pan_y + dy).clamp(-Self::PAN_LIMIT, Self::PAN_LIMIT);
        self.touch();
    }

    pub fn reset(&mut self) {
        let v = self.version;
        *self = ViewControl::default();
        self.version = v;
        self.touch();
    }

    /// Acota todos los parámetros a sus rangos (idempotente; para cargar de RON).
    pub fn sanitized(mut self) -> Self {
        self.zoom = if self.zoom.is_finite() {
            self.zoom.clamp(Self::ZOOM_MIN, Self::ZOOM_MAX)
        } else {
            1.0
        };
        self.pan_x = self.pan_x.clamp(-Self::PAN_LIMIT, Self::PAN_LIMIT);
        self.pan_y = self.pan_y.clamp(-Self::PAN_LIMIT, Self::PAN_LIMIT);
        let c = &mut self.crop;
        for v in [&mut c.left, &mut c.right, &mut c.top, &mut c.bottom] {
            *v = v.clamp(0.0, Self::CROP_MAX);
        }
        self.aspect = self.aspect.filter(|a| a.is_finite() && *a > 0.0);
        self
    }
}

/// Un rectángulo en píxeles (origen arriba-izquierda).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Resultado del cálculo: qué muestrear del frame y dónde pintarlo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// Sub-rectángulo del frame fuente a muestrear (px). Sin crop = frame entero.
    pub src: Rect,
    /// Rectángulo en el viewport donde dibujar `src` (px). Puede exceder el
    /// viewport (Fill/zoom/pan): la UI recorta a su área.
    pub dst: Rect,
}

/// Calcula la disposición del frame `src_w×src_h` en un viewport `vw×vh` según
/// `ctl`. Función pura. Devuelve dimensiones `0×0` benignas si algún tamaño no
/// es positivo (la UI no pinta nada).
pub fn compute_layout(src_w: f32, src_h: f32, vw: f32, vh: f32, ctl: &ViewControl) -> Layout {
    let zero = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
    if !(src_w > 0.0 && src_h > 0.0 && vw > 0.0 && vh > 0.0) {
        return Layout { src: zero, dst: zero };
    }

    // 1) Crop → rectángulo fuente efectivo.
    let c = ctl.crop;
    let (cl, cr, ct, cb) = (
        c.left.clamp(0.0, ViewControl::CROP_MAX),
        c.right.clamp(0.0, ViewControl::CROP_MAX),
        c.top.clamp(0.0, ViewControl::CROP_MAX),
        c.bottom.clamp(0.0, ViewControl::CROP_MAX),
    );
    let src = Rect {
        x: src_w * cl,
        y: src_h * ct,
        w: src_w * (1.0 - cl - cr),
        h: src_h * (1.0 - ct - cb),
    };

    // 2) Aspecto efectivo a respetar (override o el del recorte).
    let aspect = ctl
        .aspect
        .filter(|a| a.is_finite() && *a > 0.0)
        .unwrap_or(src.w / src.h);

    // 3) Tamaño base según el modo de encaje.
    let (mut w, mut h) = match ctl.fit {
        FitMode::Stretch => (vw, vh),
        FitMode::Original => (src.w, src.h),
        FitMode::Fit => {
            // Cabe dentro: limita por el eje más restrictivo.
            let by_w = (vw, vw / aspect);
            if by_w.1 <= vh {
                by_w
            } else {
                (vh * aspect, vh)
            }
        }
        FitMode::Fill => {
            // Cubre todo: extiende por el eje menos restrictivo.
            let by_w = (vw, vw / aspect);
            if by_w.1 >= vh {
                by_w
            } else {
                (vh * aspect, vh)
            }
        }
    };

    // 4) Zoom multiplicativo.
    let zoom = ctl.zoom.clamp(ViewControl::ZOOM_MIN, ViewControl::ZOOM_MAX);
    w *= zoom;
    h *= zoom;

    // 5) Centrado + paneo (fracción del viewport).
    let x = (vw - w) / 2.0 + ctl.pan_x.clamp(-1.0, 1.0) * vw;
    let y = (vh - h) / 2.0 + ctl.pan_y.clamp(-1.0, 1.0) * vh;

    Layout { src, dst: Rect { x, y, w, h } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 0.01, "{a} ≉ {b}");
    }

    #[test]
    fn fit_16_9_en_viewport_cuadrado() {
        // 1920×1080 (16:9) en 1000×1000 → ancho lleno, alto 562.5, centrado.
        let l = compute_layout(1920.0, 1080.0, 1000.0, 1000.0, &ViewControl::default());
        approx(l.dst.w, 1000.0);
        approx(l.dst.h, 562.5);
        approx(l.dst.x, 0.0);
        approx(l.dst.y, (1000.0 - 562.5) / 2.0);
        // src = frame entero.
        assert_eq!(l.src, Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 });
    }

    #[test]
    fn fill_recorta_y_excede() {
        let mut ctl = ViewControl::default();
        ctl.set_fit(FitMode::Fill);
        // 16:9 en 1000×1000 Fill → alto lleno, ancho 1777.78, x negativo.
        let l = compute_layout(1920.0, 1080.0, 1000.0, 1000.0, &ctl);
        approx(l.dst.h, 1000.0);
        approx(l.dst.w, 1777.78);
        approx(l.dst.x, (1000.0 - 1777.78) / 2.0);
    }

    #[test]
    fn stretch_llena_exacto() {
        let mut ctl = ViewControl::default();
        ctl.set_fit(FitMode::Stretch);
        let l = compute_layout(640.0, 480.0, 1000.0, 700.0, &ctl);
        assert_eq!(l.dst, Rect { x: 0.0, y: 0.0, w: 1000.0, h: 700.0 });
    }

    #[test]
    fn original_centra_a_escala_1() {
        let mut ctl = ViewControl::default();
        ctl.set_fit(FitMode::Original);
        let l = compute_layout(1920.0, 1080.0, 1000.0, 1000.0, &ctl);
        approx(l.dst.w, 1920.0);
        approx(l.dst.h, 1080.0);
        approx(l.dst.x, (1000.0 - 1920.0) / 2.0);
        approx(l.dst.y, (1000.0 - 1080.0) / 2.0);
    }

    #[test]
    fn aspecto_forzado() {
        // Fuente 16:9 pero forzamos 4:3 en viewport ancho.
        let mut ctl = ViewControl::default();
        ctl.set_aspect(Some(4.0 / 3.0));
        let l = compute_layout(1920.0, 1080.0, 1200.0, 900.0, &ctl);
        // 4:3 en 1200×900 (también 4:3) → llena exacto.
        approx(l.dst.w, 1200.0);
        approx(l.dst.h, 900.0);
    }

    #[test]
    fn zoom_escala_y_recentra() {
        let mut ctl = ViewControl::default();
        ctl.zoom_by(2.0);
        let l = compute_layout(1000.0, 1000.0, 1000.0, 1000.0, &ctl);
        approx(l.dst.w, 2000.0);
        approx(l.dst.h, 2000.0);
        approx(l.dst.x, -500.0); // sigue centrado
    }

    #[test]
    fn pan_desplaza() {
        let mut ctl = ViewControl::default();
        ctl.set_fit(FitMode::Stretch);
        ctl.pan_by(0.25, -0.1);
        let l = compute_layout(100.0, 100.0, 1000.0, 1000.0, &ctl);
        approx(l.dst.x, 250.0);
        approx(l.dst.y, -100.0);
    }

    #[test]
    fn crop_reduce_src_y_cambia_aspecto() {
        // Recorta 25% a cada lado horizontal → src 50% de ancho.
        let mut ctl = ViewControl::default();
        ctl.crop = Crop { left: 0.25, right: 0.25, top: 0.0, bottom: 0.0 };
        let l = compute_layout(800.0, 600.0, 1000.0, 1000.0, &ctl);
        assert_eq!(l.src, Rect { x: 200.0, y: 0.0, w: 400.0, h: 600.0 });
        // Aspecto efectivo tras crop = 400/600 = 2:3; en 1000×1000 Fit:
        // alto = 1000 limita → w = 1000*2/3 ≈ 666.67.
        approx(l.dst.h, 1000.0);
        approx(l.dst.w, 666.67);
    }

    #[test]
    fn mutadores_clamp_y_version() {
        let mut ctl = ViewControl::default();
        let v0 = ctl.version();
        ctl.zoom_by(100.0); // clamp a ZOOM_MAX
        approx(ctl.zoom, ViewControl::ZOOM_MAX);
        ctl.pan_by(5.0, -5.0); // clamp a ±PAN_LIMIT
        approx(ctl.pan_x, 1.0);
        approx(ctl.pan_y, -1.0);
        assert!(ctl.version() > v0);
        assert!(!ctl.is_identity());
        ctl.reset();
        assert!(ctl.is_identity());
    }

    #[test]
    fn cycle_fit_resetea_zoom_pan() {
        let mut ctl = ViewControl::default();
        ctl.zoom_by(2.0);
        ctl.pan_by(0.3, 0.3);
        ctl.cycle_fit();
        assert_eq!(ctl.fit, FitMode::Fill);
        approx(ctl.zoom, 1.0);
        approx(ctl.pan_x, 0.0);
    }

    #[test]
    fn sanitized_acota_basura() {
        let mut ctl = ViewControl::default();
        ctl.zoom = f32::INFINITY;
        ctl.pan_x = 9.0;
        ctl.crop = Crop { left: 0.9, right: 0.0, top: -1.0, bottom: 0.0 };
        ctl.aspect = Some(-3.0);
        let s = ctl.sanitized();
        approx(s.zoom, 1.0);
        approx(s.pan_x, 1.0);
        approx(s.crop.left, ViewControl::CROP_MAX);
        approx(s.crop.top, 0.0);
        assert_eq!(s.aspect, None);
    }

    #[test]
    fn tamano_invalido_no_rompe() {
        let l = compute_layout(0.0, 1080.0, 1000.0, 1000.0, &ViewControl::default());
        assert_eq!(l.dst.w, 0.0);
        let l = compute_layout(1920.0, 1080.0, 0.0, 1000.0, &ViewControl::default());
        assert_eq!(l.dst.h, 0.0);
    }
}
