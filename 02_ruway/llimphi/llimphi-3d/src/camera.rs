//! Cámara 3D — produce la matriz `view_proj` que el shader aplica a cada
//! vértice. Convención de mano derecha y profundidad `0..1` (la de wgpu/
//! Vulkan/Metal/DX12, **no** la `-1..1` de OpenGL).

use glam::{Mat4, Vec3};

/// Cámara en perspectiva. `eye` mira a `target` con `up` como vertical.
#[derive(Debug, Clone, Copy)]
pub struct Camera3d {
    /// Posición del ojo en mundo.
    pub eye: Vec3,
    /// Punto al que mira.
    pub target: Vec3,
    /// Vector "arriba" (normalmente `Vec3::Y`).
    pub up: Vec3,
    /// Campo de visión vertical, en radianes.
    pub fovy_rad: f32,
    /// Plano cercano (`> 0`).
    pub znear: f32,
    /// Plano lejano.
    pub zfar: f32,
}

impl Default for Camera3d {
    fn default() -> Self {
        Self {
            eye: Vec3::new(2.5, 2.0, 3.5),
            target: Vec3::ZERO,
            up: Vec3::Y,
            fovy_rad: 60_f32.to_radians(),
            znear: 0.1,
            zfar: 100.0,
        }
    }
}

impl Camera3d {
    /// Cámara orbitando `target` a `dist`, con `yaw`/`pitch` en radianes.
    /// `yaw` gira alrededor del eje Y; `pitch` sube/baja (clamp suave para no
    /// cruzar los polos y degenerar el `up`).
    pub fn orbit(target: Vec3, yaw: f32, pitch: f32, dist: f32) -> Self {
        let lim = std::f32::consts::FRAC_PI_2 - 0.01;
        let pitch = pitch.clamp(-lim, lim);
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let offset = Vec3::new(cp * sy, sp, cp * cy) * dist;
        Self {
            eye: target + offset,
            target,
            ..Self::default()
        }
    }

    /// Cámara **libre / primera persona**: parada en `eye`, mirando según
    /// `yaw` (giro alrededor de Y) y `pitch` (cabeceo, clamped para no cruzar el
    /// cenit). Complementa a [`orbit`](Self::orbit): `orbit` mira un punto desde
    /// afuera (vista de paisaje), `fly` te pone *adentro* del mundo (vuelo / FPS).
    /// `yaw=0` mira hacia `+Z`.
    pub fn fly(eye: Vec3, yaw: f32, pitch: f32) -> Self {
        let lim = std::f32::consts::FRAC_PI_2 - 0.01;
        let pitch = pitch.clamp(-lim, lim);
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let dir = Vec3::new(cp * sy, sp, cp * cy);
        Self {
            eye,
            target: eye + dir,
            ..Self::default()
        }
    }

    /// Matriz `proj * view` lista para `mvp * vec4(pos, 1.0)` en el shader.
    /// `aspect` = ancho/alto del viewport en pixels.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::look_at_rh(self.eye, self.target, self.up);
        // `perspective_rh` (no `_gl`): profundidad 0..1, la que espera wgpu.
        let proj = Mat4::perspective_rh(self.fovy_rad, aspect.max(1e-4), self.znear, self.zfar);
        proj * view
    }
}
