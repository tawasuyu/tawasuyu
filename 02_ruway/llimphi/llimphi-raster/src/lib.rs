//! llimphi-raster — Brocha Matemática.
//!
//! Traduce primitivas vectoriales (líneas, curvas de Bézier, texto) a
//! píxeles via Compute Shaders. Backend: `vello`.
//!
//! Punto de entrada: [`Renderer`]. Recibe una [`vello::Scene`] y la pinta
//! sobre un [`llimphi_hal::Frame`].

use std::num::NonZeroUsize;

use llimphi_hal::{Frame, Hal};
pub use vello;
pub use vello::kurbo;
pub use vello::peniko;

/// Errores del rasterizador.
#[derive(Debug)]
pub enum RasterError {
    Init(String),
    Render(String),
}

impl std::fmt::Display for RasterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Init(s) => write!(f, "vello init: {s}"),
            Self::Render(s) => write!(f, "vello render: {s}"),
        }
    }
}

impl std::error::Error for RasterError {}

/// Rasterizador vectorial. Una instancia por surface (porque vello cachea
/// resources contra un `surface_format` específico).
pub struct Renderer {
    inner: vello::Renderer,
}

impl Renderer {
    /// Inicializa el rasterizador. Vello acepta cualquier textura compatible
    /// (Rgba8Unorm / Bgra8Unorm) en `render`, así que no se fija un formato
    /// en construcción.
    pub fn new(hal: &Hal) -> Result<Self, RasterError> {
        let inner = vello::Renderer::new(
            &hal.device,
            vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport::all(),
                num_init_threads: NonZeroUsize::new(1),
                pipeline_cache: None,
            },
        )
        .map_err(|e| RasterError::Init(e.to_string()))?;
        Ok(Self { inner })
    }

    /// Renderiza `scene` sobre `frame` limpiando con `base_color`. AA fija
    /// en area-sampling (precisión Δ < 10⁻⁹ rad del SDD).
    pub fn render(
        &mut self,
        hal: &Hal,
        scene: &vello::Scene,
        frame: &Frame,
        base_color: peniko::Color,
    ) -> Result<(), RasterError> {
        let (width, height) = frame.size();
        self.inner
            .render_to_texture(
                &hal.device,
                &hal.queue,
                scene,
                frame.view(),
                &vello::RenderParams {
                    base_color,
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .map_err(|e| RasterError::Render(e.to_string()))
    }
}
