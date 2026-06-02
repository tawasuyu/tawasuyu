//! Kernel agregador del notebook de pluma.
//!
//! Junta los kernels concretos (wasmi para `wasm`/`wat`, RustPython para
//! `python`/`py`, y el de `media`) detrás del mismo trait
//! [`Kernel`](pluma_notebook_exec::Kernel) y despacha por el string de
//! lenguaje de cada celda. Antes vivía en el frontend
//! `pluma-notebook-llimphi/src/main.rs` (regla #2): el ruteo de kernels es
//! lógica de dominio, no del visor — cualquier frontend lo reusa sin
//! recablear los tres kernels.

use async_trait::async_trait;
use pluma_notebook_exec::{Kernel, KernelError, KernelOutput};
use pluma_notebook_kernel_media::MediaKernel;
use pluma_notebook_kernel_python::PythonKernel;
use pluma_notebook_kernel_wasm::WasmKernel;

/// Dispatcher por `language` — la pieza que junta wasmi + RustPython + media
/// detrás del mismo trait `Kernel`. El visor delega acá y deja que cada
/// celda elija su intérprete con un string.
pub struct MultiKernel {
    wasm: WasmKernel,
    python: PythonKernel,
    media: MediaKernel,
}

impl MultiKernel {
    pub fn new() -> Self {
        Self {
            wasm: WasmKernel::new(),
            python: PythonKernel::new(),
            media: MediaKernel::new(),
        }
    }
}

impl Default for MultiKernel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Kernel for MultiKernel {
    async fn execute(&self, source: &str, language: &str) -> Result<KernelOutput, KernelError> {
        match language {
            "wasm" | "wat" => self.wasm.execute(source, language).await,
            "python" | "py" => self.python.execute(source, language).await,
            "media" => self.media.execute(source, language).await,
            other => Err(KernelError::Runtime(format!(
                "ningún kernel registrado para '{other}' (disponibles: wasm/wat, python/py, media)"
            ))),
        }
    }
}
