//! `tinkuy-forces` — kernels de fuerza física.
//!
//! Patrón común a todos los kernels:
//!   - Paralelo por **partícula** (rayon). Cada worker escribe solo a sus
//!     propias `world.axs/ays/azs[i]`, sin contención.
//!   - **Sin Newton-3 optimization**: cada par (i, j) se computa dos veces (i
//!     ve a j; j ve a i). Coste: 2× FLOPs. Beneficio: cero atómicos, escalado
//!     lineal hasta saturar memory bandwidth.
//!   - Neighbor-list por **27 celdas** vía `Grid3D::for_each_neighbor`.
//!     Precondición: `grid.cell_size >= cutoff` para que ninguna interacción
//!     quede fuera del barrido.
//!
//! Los kernels asumen que `world.axs/ays/azs` ya está a cero (o que el caller
//! aceptará la sobreescritura). En la práctica los invoca
//! `velocity_verlet_step` justo después del drift, sin requerir clear previo.

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod lennard_jones;
pub mod coulomb;

pub use lennard_jones::{clear_accelerations, lennard_jones, LjParams};
pub use coulomb::{coulomb, CoulombParams};
