//! `dominium-iso` — proyección pseudo-3D isométrica.
//!
//! GPUI no maneja matrices de proyección 3D ni mallas: la ilusión de
//! relieve se calcula en CPU antes de emitir quads 2D. Matriz iso fija:
//!
//! ```text
//!   x_pantalla = (x - y) · cos(30°)
//!   y_pantalla = (x + y) · sin(30°) − Z
//! ```
//!
//! La altura `Z` no existe en el motor lógico — se extrae de los campos
//! de la grilla como una combinación lineal config'able de las 5 capas
//! ([`ZWeights`]). Los `cos`/`sin` van por `libm` para que la proyección
//! sea bit-exacta en cualquier plataforma.

#![forbid(unsafe_code)]

use dominium_core::Grid;
use serde::{Deserialize, Serialize};

/// Pesos del Z compuesto — uno por capa de la grilla. El panel expone
/// estos 5 sliders; el relieve es `Σ wᵢ · capaᵢ`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ZWeights {
    pub materia: f32,
    pub psique: f32,
    pub poder: f32,
    pub oro: f32,
    pub degradacion: f32,
}

impl Default for ZWeights {
    /// Por defecto el relieve sigue la `materia`.
    fn default() -> Self {
        Self { materia: 1.0, psique: 0.0, poder: 0.0, oro: 0.0, degradacion: 0.0 }
    }
}

impl ZWeights {
    /// Z compuesto de la celda `idx`: combinación lineal de las 5 capas.
    pub fn z_of(&self, grid: &Grid, idx: usize) -> f32 {
        self.materia * grid.materia[idx]
            + self.psique * grid.psique[idx]
            + self.poder * grid.poder[idx]
            + self.oro * grid.oro[idx]
            + self.degradacion * grid.degradacion[idx]
    }
}

/// Proyector isométrico. `cos`/`sin` de 30° precomputados vía `libm`.
#[derive(Debug, Clone, Copy)]
pub struct IsoProjector {
    cos30: f32,
    sin30: f32,
    /// Escala de pantalla (pixels por unidad de mundo).
    pub scale: f32,
    /// Cuánto eleva el `Z` en pixels de pantalla.
    pub z_factor: f32,
}

impl IsoProjector {
    /// Crea un proyector. `scale` = pixels por celda; `z_factor` = cuánto
    /// levanta una unidad de Z.
    pub fn new(scale: f32, z_factor: f32) -> Self {
        // 30° en radianes. libm da el mismo bit en x86 y ARM.
        let rad = core::f32::consts::FRAC_PI_6;
        Self {
            cos30: libm::cosf(rad),
            sin30: libm::sinf(rad),
            scale,
            z_factor,
        }
    }

    /// Proyecta una coordenada de mundo `(x, y)` con altura `z` a
    /// coordenadas de pantalla.
    pub fn project(&self, x: f32, y: f32, z: f32) -> (f32, f32) {
        let sx = (x - y) * self.cos30 * self.scale;
        let sy = ((x + y) * self.sin30 - z * self.z_factor) * self.scale;
        (sx, sy)
    }

    /// Proyecta la sombra de un punto sobre el suelo (Lambert plano): la
    /// sombra cae en `z = 0` desplazada según la dirección de la luz, con
    /// largo proporcional a la altura del punto.
    pub fn shadow(&self, x: f32, y: f32, z: f32, light_dir: (f32, f32)) -> (f32, f32) {
        let foot_x = x + light_dir.0 * z;
        let foot_y = y + light_dir.1 * z;
        self.project(foot_x, foot_y, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn origin_projects_to_origin() {
        let iso = IsoProjector::new(1.0, 1.0);
        let (x, y) = iso.project(0.0, 0.0, 0.0);
        assert!(approx(x, 0.0) && approx(y, 0.0));
    }

    #[test]
    fn diamond_axis_collapses_x() {
        // En iso, (a, a) cae sobre x_pantalla = 0 (la diagonal del rombo).
        let iso = IsoProjector::new(1.0, 1.0);
        let (sx, _) = iso.project(5.0, 5.0, 0.0);
        assert!(approx(sx, 0.0));
    }

    #[test]
    fn z_raises_the_point_upward() {
        let iso = IsoProjector::new(1.0, 10.0);
        let (_, y0) = iso.project(3.0, 3.0, 0.0);
        let (_, y1) = iso.project(3.0, 3.0, 2.0);
        // Más Z → menor y de pantalla (sube).
        assert!(y1 < y0);
    }

    #[test]
    fn composite_z_is_a_linear_combination() {
        let mut g = Grid::new(4, 4);
        let idx = g.idx(1, 1);
        g.materia[idx] = 10.0;
        g.poder[idx] = 4.0;
        let w = ZWeights { materia: 0.5, psique: 0.0, poder: 2.0, oro: 0.0, degradacion: 0.0 };
        // 0.5*10 + 2*4 = 13
        assert!(approx(w.z_of(&g, idx), 13.0));
    }

    #[test]
    fn projector_is_deterministic() {
        let a = IsoProjector::new(2.0, 3.0);
        let b = IsoProjector::new(2.0, 3.0);
        assert_eq!(a.project(7.0, 11.0, 1.5), b.project(7.0, 11.0, 1.5));
    }

    #[test]
    fn shadow_of_ground_point_equals_its_projection() {
        let iso = IsoProjector::new(1.0, 5.0);
        // z = 0 → la sombra coincide con el punto.
        let p = iso.project(4.0, 2.0, 0.0);
        let s = iso.shadow(4.0, 2.0, 0.0, (1.0, 0.5));
        assert!(approx(p.0, s.0) && approx(p.1, s.1));
    }
}
