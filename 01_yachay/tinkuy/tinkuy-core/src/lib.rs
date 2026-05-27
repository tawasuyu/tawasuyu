//! `tinkuy-core` — motor de partículas data-oriented.
//!
//! Invariantes:
//! - SoA por eje, arrays alineados a 64 B (línea de caché x86_64/aarch64).
//! - Cero `Arc`/`Rc`/`Box` en el hot path.
//! - Cero atómicos en el hot loop: la transferencia entre celdas usa outboxes
//!   worker-local + merge determinista al final del substep.
//! - Snapshots content-addressed (BLAKE3) para integración con Wawa/Akasha:
//!   el CID de un estado vale como identidad inmutable que AoE puede servir
//!   y que el render puede cachear.
//!
//! El backend de cómputo se selecciona por feature: `cpu` (rayon, default) o
//! `wasm` (single-thread, para AOT cranelift dentro de Wawa). En el futuro,
//! una feature `gpu` ofrecerá los mismos kernels en WGSL compute pipelines.

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod ecs;
pub mod grid;
pub mod integrator;
pub mod snapshot;

pub use ecs::{Aligned64, EntityHandle, World};
pub use grid::{CellId, Grid3D, Outbox, Transfer};
pub use integrator::{velocity_verlet_step, IntegratorParams};
pub use snapshot::Snapshot;
