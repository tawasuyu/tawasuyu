// =============================================================================
//  renaser :: kernel/src/memory — la memoria dinamica del kernel (Fase 3)
// -----------------------------------------------------------------------------
//  El heap no es, en renaser, una utilidad pasiva al estilo C. Existe para
//  sostener algo vivo: la cola de futuros y los reactores asincronos sobre los
//  que, fase a fase, se ejecutara el bytecode WASM aislado por software.
// =============================================================================

pub mod allocator;
pub mod cache;
pub mod mmio;

pub use allocator::init;
