//! `DslForce` — fuerza pairwise descrita por un programa de `tinkuy-dsl`.
//!
//! Convención de la magnitud devuelta por el DSL: **F_over_r**. El programa
//! evalúa un escalar `f` y este crate lo aplica como
//!     F⃗_{i ← j} = f · (r_i − r_j)
//! para que la composición vectorial sea idéntica a la del kernel nativo de
//! Lennard-Jones (`f_over_r * dx`). Así una expresión DSL como
//!     `24 * eps * (2 * pow(sigma/r, 12) - pow(sigma/r, 6)) * inv(r2)`
//! es bit-equivalente al LJ nativo módulo el error de aproximación de
//! `libm_powf` (~1e-4 sobre los exponentes 12/6, ver tests).
//!
//! Variables disponibles dentro del DSL durante `apply`:
//!   - `r2`, `r`, `dx`, `dy`, `dz`  — geometría del par (recomputados por par).
//!   - `eps`, `sigma`              — parámetros constantes (de `DslForce`).
//!   - `qi`, `qj`, `mi`, `mj`      — leídos del `World`.
//!
//! El stack del evaluador se aloca **una sola vez** en `DslForce::new`
//! (`stack_depth` ya conocido), así que `apply` no toca el heap en el hot loop.

use alloc::string::String;
use alloc::vec::Vec;

use tinkuy_core::{Grid3D, World};
use tinkuy_dsl::{compile, eval_with_stack, parse, Bytecode, VarBindings};

#[derive(Debug)]
pub enum DslForceError {
    Parse(tinkuy_dsl::ParseError),
    Compile(tinkuy_dsl::CompileError),
}

impl From<tinkuy_dsl::ParseError> for DslForceError {
    fn from(e: tinkuy_dsl::ParseError) -> Self { Self::Parse(e) }
}
impl From<tinkuy_dsl::CompileError> for DslForceError {
    fn from(e: tinkuy_dsl::CompileError) -> Self { Self::Compile(e) }
}

/// Fuerza pairwise descrita por bytecode DSL. Vive en heap (Vec<f32> stack),
/// pero `apply` no aloca: usa el mismo buffer en cada eval.
pub struct DslForce {
    bc: Bytecode,
    /// ε constante visible como variable `eps`.
    pub eps: f32,
    /// σ constante visible como variable `sigma`.
    pub sigma: f32,
    /// r máximo considerado; si r² > cutoff² la partícula j se ignora.
    pub cutoff: f32,
    /// Buffer del stack del evaluador, dimensionado para `bc.stack_depth`.
    stack: Vec<f32>,
    /// Nombre humano para errores; opcional, usado por reportes futuros.
    pub label: String,
}

impl DslForce {
    pub fn from_src(
        src: &str, eps: f32, sigma: f32, cutoff: f32,
    ) -> Result<Self, DslForceError> {
        let ast = parse(src)?;
        let bc = compile(&ast)?;
        Ok(Self::from_bytecode(bc, eps, sigma, cutoff))
    }

    pub fn from_bytecode(bc: Bytecode, eps: f32, sigma: f32, cutoff: f32) -> Self {
        let stack = alloc::vec![0.0f32; bc.stack_depth as usize];
        Self { bc, eps, sigma, cutoff, stack, label: String::new() }
    }

    pub fn with_label(mut self, s: impl Into<String>) -> Self {
        self.label = s.into(); self
    }

    /// Aplica la fuerza a `world` usando la grilla `grid` como neighbor-list.
    /// Acumula `+=` sobre `axs/ays/azs` igual que el kernel LJ nativo: el
    /// caller debe haber llamado `clear_accelerations` si superpone fuerzas.
    pub fn apply(&mut self, world: &mut World, grid: &Grid3D) {
        let n = world.len();
        if n == 0 { return; }
        debug_assert!(
            grid.cell_size >= self.cutoff,
            "cell_size ({}) < cutoff ({}) — vecinos posiblemente perdidos",
            grid.cell_size, self.cutoff
        );
        let cutoff2 = self.cutoff * self.cutoff;
        let eps = self.eps;
        let sigma = self.sigma;

        // Single-thread por ahora. La paralelización con rayon vendrá si los
        // benches (D4) lo justifican. Mantener el evaluador escalar
        // single-thread es lo más fácil de razonar y suficiente para tests +
        // sims pequeñas; el kernel nativo seguirá siendo el fast-path.
        for i in 0..n {
            let xi = world.xs.0[i];
            let yi = world.ys.0[i];
            let zi = world.zs.0[i];
            let qi = world.charges.0[i];
            let mi = world.masses.0[i];
            let cell_i = grid.cell_of[i];

            let mut fx = 0.0f32;
            let mut fy = 0.0f32;
            let mut fz = 0.0f32;
            grid.for_each_neighbor(cell_i, |j| {
                if j == i { return; }
                let dx = xi - world.xs.0[j];
                let dy = yi - world.ys.0[j];
                let dz = zi - world.zs.0[j];
                let r2 = dx * dx + dy * dy + dz * dz;
                if r2 > cutoff2 || r2 < 1.0e-12 { return; }
                // r se calcula explícitamente para que el DSL pueda usarlo
                // como variable (`r`); si el programa no lo referencia, el
                // optimizador en D4 podrá eliminar este sqrt.
                let r = libm_sqrtf(r2);
                let vars = VarBindings {
                    r, r2, eps, sigma,
                    qi, qj: world.charges.0[j],
                    mi, mj: world.masses.0[j],
                    dx, dy, dz,
                };
                let f_over_r = eval_with_stack(&self.bc, &vars, &mut self.stack)
                    .unwrap_or(0.0);
                fx += f_over_r * dx;
                fy += f_over_r * dy;
                fz += f_over_r * dz;
            });

            let inv_m = if mi > 0.0 { 1.0 / mi } else { 0.0 };
            world.axs.0[i] += fx * inv_m;
            world.ays.0[i] += fy * inv_m;
            world.azs.0[i] += fz * inv_m;
        }
    }
}

/// Newton-Raphson sobre 1/√x — mismo método que `tinkuy_dsl::bytecode`. Lo
/// duplicamos aquí porque la función equivalente de `tinkuy-dsl` es privada;
/// dependerlo de `std::f32::sqrt` ataría este crate a `std`, y queremos que
/// `tinkuy-forces` siga compilando bajo `cpu`/`wasm` indistintamente.
#[inline] fn libm_sqrtf(x: f32) -> f32 {
    if x <= 0.0 { return if x == 0.0 { 0.0 } else { f32::NAN }; }
    let mut g = f32::from_bits(0x5f37_5a86u32.wrapping_sub(x.to_bits() >> 1));
    g = g * (1.5 - 0.5 * x * g * g);
    g = g * (1.5 - 0.5 * x * g * g);
    g = g * (1.5 - 0.5 * x * g * g);
    x * g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lennard_jones::{clear_accelerations, lennard_jones, LjParams};

    fn rebuild_grid(world: &World, cell_size: f32, dims: [u32; 3]) -> Grid3D {
        let mut g = Grid3D::new([-50.0; 3], cell_size, dims, world.len());
        g.rebuild(world);
        g
    }

    /// Dos `World` idénticos: uno con LJ nativo, otro con LJ DSL. Las
    /// aceleraciones tras un step de fuerzas deben coincidir dentro de la
    /// tolerancia del `pow` aproximado.
    #[test]
    fn dsl_lj_matches_native_lj_for_dense_cube() {
        // 4×4×4 partículas espaciadas a 1.1σ.
        let make = || {
            let mut w = World::with_capacity(64);
            for k in 0..4 { for j in 0..4 { for i in 0..4 {
                w.spawn(
                    [i as f32 * 1.1, j as f32 * 1.1, k as f32 * 1.1],
                    [0.; 3], 1.0, 0.0,
                );
            }}}
            w
        };

        // Nativo.
        let mut wn = make();
        let gn = rebuild_grid(&wn, 3.0, [40, 40, 40]);
        clear_accelerations(&mut wn);
        lennard_jones(&mut wn, &gn, &LjParams { epsilon: 1.0, sigma: 1.0, cutoff: 2.5 });

        // DSL — misma fórmula que el kernel nativo.
        let src = "24 * eps * (2 * pow(sigma / r, 12) - pow(sigma / r, 6)) * inv(r2)";
        let mut dsl = DslForce::from_src(src, 1.0, 1.0, 2.5).unwrap();
        let mut wd = make();
        let gd = rebuild_grid(&wd, 3.0, [40, 40, 40]);
        clear_accelerations(&mut wd);
        dsl.apply(&mut wd, &gd);

        // Comparación: max |Δa| sobre todas las partículas y componentes.
        let mut max_err = 0.0f32;
        for i in 0..wn.len() {
            let d = [
                (wn.axs.0[i] - wd.axs.0[i]).abs(),
                (wn.ays.0[i] - wd.ays.0[i]).abs(),
                (wn.azs.0[i] - wd.azs.0[i]).abs(),
            ];
            max_err = max_err.max(d[0]).max(d[1]).max(d[2]);
        }
        // pow aproximado (Taylor sobre exp/ln) introduce ~1e-3 en exponente 12.
        // El producto por |a| ~ O(100) lleva a tolerancia neta ~1e-1.
        assert!(max_err < 0.5, "max |Δa| DSL ↔ nativo = {max_err}");
    }

    #[test]
    fn coulomb_dsl_replicates_native_for_pair_of_charges() {
        // Par de cargas opuestas a r=2.0 ε⁻¹·k_e=1.
        let mut wn = World::with_capacity(2);
        wn.spawn([0., 0., 0.], [0.; 3], 1.0,  1.0);
        wn.spawn([2., 0., 0.], [0.; 3], 1.0, -1.0);
        let gn = rebuild_grid(&wn, 3.0, [40, 40, 40]);
        clear_accelerations(&mut wn);
        crate::coulomb::coulomb(&mut wn, &gn, &crate::coulomb::CoulombParams { ke: 1.0, cutoff: 2.5 });

        // DSL: F_over_r para Coulomb es qi·qj/r³. En unidades reducidas ke=1.
        // `inv(r2) · inv(r)` ≡ `1/r³`.
        let mut dsl = DslForce::from_src("qi * qj * inv(r2) * inv(r)", 0.0, 0.0, 2.5).unwrap();
        let mut wd = World::with_capacity(2);
        wd.spawn([0., 0., 0.], [0.; 3], 1.0,  1.0);
        wd.spawn([2., 0., 0.], [0.; 3], 1.0, -1.0);
        let gd = rebuild_grid(&wd, 3.0, [40, 40, 40]);
        clear_accelerations(&mut wd);
        dsl.apply(&mut wd, &gd);

        for i in 0..2 {
            let d = (wn.axs.0[i] - wd.axs.0[i]).abs();
            assert!(d < 1e-3, "Coulomb DSL ↔ nativo i={i}: Δa = {d}");
        }
    }

    #[test]
    fn parse_error_propagates() {
        let r = DslForce::from_src("foo + 1", 1.0, 1.0, 2.5);
        assert!(matches!(r, Err(DslForceError::Parse(_))));
    }
}
