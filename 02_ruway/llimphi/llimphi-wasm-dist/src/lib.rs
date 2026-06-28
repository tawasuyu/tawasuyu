//! llimphi-wasm-dist — distribución por hash + puente al runner Tier 3.
//!
//! Es la cara "con baterías" de [`llimphi_wasm_core`]: re-exporta toda la cadena
//! resolver→verificar y le suma el runner, de modo que una [`VerifiedApp`] se
//! pueda **correr** con `verified.load()` (extensión [`VerifiedAppExt`]). Lo
//! pesado (wgpu/vello/winit) entra por acá; quien sólo distribuye/verifica
//! depende de `llimphi-wasm-core` y se ahorra el stack gráfico.

pub use llimphi_wasm_core::*;
pub use llimphi_wasm_runner::{EventId, EventPayload, RunnerMsg, WasmGuest};

/// Carga una [`VerifiedApp`] en el runner Tier 3 con sus permisos efectivos
/// (que gatean qué host imports se enlazan). El método vive acá —y no en
/// `llimphi-wasm-core`— porque es lo único de la cadena que toca el runner.
pub trait VerifiedAppExt {
    fn load(&self) -> Result<WasmGuest, DistError>;
}

impl VerifiedAppExt for VerifiedApp {
    fn load(&self) -> Result<WasmGuest, DistError> {
        WasmGuest::load(&self.wasm, self.permisos).map_err(DistError::Carga)
    }
}
