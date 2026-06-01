//! llimphi-raster — Brocha Matemática.
//!
//! Traduce primitivas vectoriales (líneas, curvas de Bézier, texto) a
//! píxeles via Compute Shaders. Backend: `vello`.
//!
//! Punto de entrada: [`Renderer`]. Recibe una [`vello::Scene`] y la pinta
//! sobre un [`llimphi_hal::Frame`].

use llimphi_hal::{Frame, Hal};
pub use vello;
pub use vello::kurbo;
pub use vello::peniko;

pub mod gpu;
pub use gpu::{GpuBatch, GpuPipelines};

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
    ///
    /// **`antialiasing_support`**: pedimos `area` solamente, no `all()`.
    /// `area` es el único método que `render()` usa (`AaConfig::Area`
    /// fijo). Pedir `all()` haría a vello compilar también pipelines
    /// para `msaa8` y `msaa16` que nunca se invocan — en Mali-G57 eso
    /// triplica el cold-start (medido: 3.7s vs ~1.2s). Si alguna app
    /// futura necesita MSAA, agregamos un constructor explícito.
    ///
    /// **`num_init_threads: None`**: vello paraleliza la compilación
    /// de shaders en `None` → todos los CPU cores. Mali-G57 viene en
    /// SoCs octa-core ARM; con 1 thread tardamos 2.0s, con 8 esperamos
    /// ~400-600ms. La compilación de shaders es 100% CPU (Rust →
    /// SPIR-V), el GPU no participa, así que multi-thread escala
    /// casi linealmente hasta saturar el queue del Naga compiler.
    pub fn new(hal: &Hal) -> Result<Self, RasterError> {
        let inner = vello::Renderer::new(
            &hal.device,
            vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport {
                    area: true,
                    msaa8: false,
                    msaa16: false,
                },
                num_init_threads: None,
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
        self.render_to_view(hal, scene, frame.view(), width, height, base_color)
    }

    /// Como [`render`](Self::render) pero contra una vista de textura
    /// explícita (mismo formato/tamaño que la intermedia). Lo usa el
    /// compositor de overlay de `llimphi-ui` para rasterizar la capa de
    /// overlay sobre fondo transparente en su propia textura. Ojo:
    /// `render_to_texture` **limpia** el target con `base_color` y escribe
    /// todos los píxeles — no compone sobre contenido previo.
    pub fn render_to_view(
        &mut self,
        hal: &Hal,
        scene: &vello::Scene,
        view: &llimphi_hal::wgpu::TextureView,
        width: u32,
        height: u32,
        base_color: peniko::Color,
    ) -> Result<(), RasterError> {
        self.inner
            .render_to_texture(
                &hal.device,
                &hal.queue,
                scene,
                view,
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
