use super::*;

pub(crate) struct Renderable {
    pub(crate) depth: f32,
    pub(crate) color: Color,
    pub(crate) path: BezPath,
    pub(crate) kind: RenderKind,
}

pub(crate) enum RenderKind {
    /// Fill sólido del `path` con `color`. Walls fallback, floors,
    /// ceilings, sprites fallback.
    Fill,
    /// `scene.draw_image(image, xform)` — `path` y `color` se ignoran.
    /// Sprites texturizados desde el WAD.
    Sprite {
        image: llimphi_ui::llimphi_raster::peniko::Image,
        xform: Affine,
    },
    /// Pared texturizada: fill del `path` con la `image` (Extend::Repeat
    /// activado) usando `brush_xform` como brush_transform — vello
    /// rellena el polígono samplando el image tileado en world coords.
    /// `color` se ignora.
    TexturedWall {
        image: llimphi_ui::llimphi_raster::peniko::Image,
        brush_xform: Affine,
    },
    /// **Fase 3.43** — fill del `path` con un `Gradient` lineal vertical
    /// como brush. `color` se ignora. Usado por el shading/tinte continuo
    /// de paredes texturizadas (reemplaza las bandas discretas de 3.42).
    GradientFill {
        gradient: llimphi_ui::llimphi_raster::peniko::Gradient,
    },
}

// =====================================================================
// Cámara + proyección
// =====================================================================

pub(crate) struct Camera {
    pub(crate) px: f32,
    pub(crate) py: f32,
    pub(crate) view_z: f32,
    pub(crate) cos_pa: f32,
    pub(crate) sin_pa: f32,
}

impl Camera {
    pub(crate) fn new(px: f32, py: f32, view_z: f32, angle: f32) -> Self {
        Self {
            px,
            py,
            view_z,
            cos_pa: angle.cos(),
            sin_pa: angle.sin(),
        }
    }

    /// World (x, y) → camera (X_cam = forward, Y_cam = right).
    pub(crate) fn to_cam_2d(&self, wx: f32, wy: f32) -> (f32, f32) {
        let dx = wx - self.px;
        let dy = wy - self.py;
        let x_cam = dx * self.cos_pa + dy * self.sin_pa;
        let y_cam = dx * self.sin_pa - dy * self.cos_pa;
        (x_cam, y_cam)
    }

    /// Inverso de [`Self::to_cam_2d`]: camera (X, Y) → world (wx, wy).
    /// Útil para recuperar las coords mundo de vértices intermedios
    /// generados por el near-clip 2D (que ya están en cam space).
    pub(crate) fn from_cam_2d(&self, x_cam: f32, y_cam: f32) -> (f32, f32) {
        // Inversa de la rotación: rot⁻¹ = rotᵀ.
        // dx = x_cam·cos + y_cam·sin
        // dy = x_cam·sin - y_cam·cos
        let dx = x_cam * self.cos_pa + y_cam * self.sin_pa;
        let dy = x_cam * self.sin_pa - y_cam * self.cos_pa;
        (self.px + dx, self.py + dy)
    }
}

pub(crate) struct Projection {
    pub(crate) cx: f32,
    pub(crate) cy: f32,
    /// `focal = h / (2·tan(fov_y/2))`. Pixels cuadrados.
    pub(crate) focal: f32,
    /// **Y-shear** del rasterizador para mouse-look cosmético. Suma a
    /// `sy` un offset constante para todos los puntos proyectados
    /// (independiente de la profundidad), lo que equivale a mover la
    /// línea del horizonte arriba/abajo en pantalla. Doom clásico no
    /// hace pitch real (cilindros de hitbox verticales); este offset
    /// preserva esa convención porque sólo afecta el rasterizador.
    ///
    /// `pitch_offset_px = focal · tan(view_pitch)`. Positivo = horizonte
    /// se mueve hacia abajo (mirando hacia arriba).
    pub(crate) pitch_offset_px: f32,
}

impl Projection {
    #[cfg(test)]
    pub(crate) fn new(rect: PaintRect, fov_y_rad: f32) -> Self {
        Self::new_pitched(rect, fov_y_rad, 0.0)
    }

    pub(crate) fn new_pitched(rect: PaintRect, fov_y_rad: f32, view_pitch: f32) -> Self {
        let focal = rect.h * 0.5 / (fov_y_rad * 0.5).tan();
        // Clampeamos el pitch a ±π/3 para evitar tan() explotando y
        // distorsiones absurdas que mostrarían el "horizonte" fuera del
        // viewport. El host también clampea, pero defendemos al renderer.
        let p = view_pitch.clamp(-PITCH_MAX, PITCH_MAX);
        let pitch_offset_px = focal * p.tan();
        Self {
            cx: rect.x + rect.w * 0.5,
            cy: rect.y + rect.h * 0.5,
            focal,
            pitch_offset_px,
        }
    }

    /// `(X_cam, Y_cam, Z_cam)` → coordenada en pantalla.
    /// **Caller garantiza `x_cam > 0`** (post near-clip).
    pub(crate) fn project(&self, x_cam: f32, y_cam: f32, z_cam: f32) -> Point {
        let inv_d = 1.0 / x_cam;
        let sx = self.cx + y_cam * self.focal * inv_d;
        let sy = self.cy + self.pitch_offset_px - z_cam * self.focal * inv_d;
        Point::new(sx as f64, sy as f64)
    }
}

/// Rango sano del pitch (±60°). Más allá el horizonte se sale del
/// viewport y los planos del piso/techo dejan de tener interpretación
/// visual razonable.
pub(crate) const PITCH_MAX: f32 = std::f32::consts::FRAC_PI_3;

// =====================================================================
// Walls + floor/ceiling strips
// =====================================================================
