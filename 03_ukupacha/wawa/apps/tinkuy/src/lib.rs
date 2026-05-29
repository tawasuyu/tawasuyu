//! `tinkuy` — cdylib WASM que re-exporta la ABI plana de `tinkuy-abi`.
//!
//! El kernel de Wawa carga este `.wasm` con `wasmi` y expone los símbolos
//! `tk_sim_*` directamente al userspace. La feature `wasm` está fijada en
//! `Cargo.toml`, así que aquí no hay nada que elegir en runtime: la ABI corre
//! single-thread sin rayon, sin hilos, sin `Instant`.
//!
//! No se hace `#![no_std]` deliberadamente: la ABI usa `Vec`/`Box` y el motor
//! aún arrastra `std`. La pasada `wasm-opt -Os` del pipeline compacta lo no
//! usado; si el footprint llega a ser un problema veremos `no_std + alloc` en
//! una sub-fase posterior.
//!
//! No-mangle se hereda del crate origen porque cada función `tk_*` ya lleva
//! `#[no_mangle] pub unsafe extern "C"`. Un `pub use` basta para que el
//! linker del cdylib emita los exports.

pub use tinkuy_abi::*;
