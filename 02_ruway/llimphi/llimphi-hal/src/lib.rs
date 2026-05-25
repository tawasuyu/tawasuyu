//! llimphi-hal — Puente al Silicio.
//!
//! Aísla el motor del sistema operativo. Pinta en ventana Wayland/X11
//! (vía `mirada` en producción, vía `winit` en dev) o framebuffer directo
//! del kernel `wawa` (TODO). Trait `Surface` abstracto + struct `Hal`
//! que posee Instance/Adapter/Device/Queue de wgpu.

use std::sync::Arc;

pub use raw_window_handle;
pub use wgpu;
pub use winit;

use winit::window::Window;

/// Errores al adquirir un frame de la superficie.
#[derive(Debug)]
pub enum SurfaceError {
    Lost,
    Outdated,
    OutOfMemory,
    Timeout,
    Other(String),
}

impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lost => write!(f, "surface lost"),
            Self::Outdated => write!(f, "surface outdated"),
            Self::OutOfMemory => write!(f, "surface out of memory"),
            Self::Timeout => write!(f, "surface timeout"),
            Self::Other(s) => write!(f, "surface error: {s}"),
        }
    }
}

impl std::error::Error for SurfaceError {}

/// Errores al construir Hal o crear una Surface.
#[derive(Debug)]
pub enum HalError {
    NoAdapter,
    RequestDevice(String),
    CreateSurface(String),
}

impl std::fmt::Display for HalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "no GPU adapter available"),
            Self::RequestDevice(s) => write!(f, "request_device failed: {s}"),
            Self::CreateSurface(s) => write!(f, "create_surface failed: {s}"),
        }
    }
}

impl std::error::Error for HalError {}

/// Superficie gráfica donde llimphi pinta. Una sola cosa: ofrecer un
/// `wgpu::TextureView` por frame y presentarlo.
///
/// Implementaciones:
/// - [`WinitSurface`]: ventana Wayland/X11 (dev + producción vía mirada).
/// - `WawaFramebufferSurface` (TODO): framebuffer directo del kernel wawa.
pub trait Surface {
    fn size(&self) -> (u32, u32);
    fn resize(&mut self, width: u32, height: u32);
    /// Adquiere el próximo frame. Llamar [`Frame::present`] para mostrar.
    fn acquire(&mut self) -> Result<Frame, SurfaceError>;
}

/// Frame adquirido. Si se dropea sin `present()`, el frame se descarta.
pub struct Frame {
    texture: wgpu::SurfaceTexture,
    view: wgpu::TextureView,
}

impl Frame {
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn size(&self) -> (u32, u32) {
        let t = &self.texture.texture;
        (t.width(), t.height())
    }

    pub fn present(self) {
        self.texture.present();
    }
}

/// Estado wgpu compartido. Una instancia por proceso. `Device` y `Queue`
/// son `Arc` internamente, así que clonar es barato.
pub struct Hal {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Hal {
    /// Construye Hal pidiendo un adapter compatible con una surface dada
    /// (recomendado: pasar `Some(&surface)` para garantizar que el adapter
    /// elegido sabe presentar a esa surface).
    pub async fn new(
        compatible_surface: Option<&wgpu::Surface<'static>>,
    ) -> Result<Self, HalError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface,
            })
            .await
            .ok_or(HalError::NoAdapter)?;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("llimphi-hal-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .map_err(|e| HalError::RequestDevice(e.to_string()))?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }
}

/// Surface basada en `winit::window::Window`. Owna el `Arc<Window>` para
/// extender su vida al `wgpu::Surface<'static>`.
pub struct WinitSurface {
    _window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
}

impl WinitSurface {
    pub fn new(hal: &Hal, window: Arc<Window>) -> Result<Self, HalError> {
        let surface = hal
            .instance
            .create_surface(window.clone())
            .map_err(|e| HalError::CreateSurface(e.to_string()))?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&hal.adapter);
        // vello acepta Rgba8Unorm o Bgra8Unorm (sin sRGB porque el blit hace su propia gamma).
        // Si el adapter no ofrece ninguno, caemos al primero disponible.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            // STORAGE_BINDING permite que el rasterizador vectorial (vello) escriba
            // directo al swapchain vía compute shader.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::STORAGE_BINDING,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&hal.device, &config);
        Ok(Self {
            _window: window,
            surface,
            config,
            device: hal.device.clone(),
        })
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }
}

impl Surface for WinitSurface {
    fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    fn acquire(&mut self) -> Result<Frame, SurfaceError> {
        let texture = self.surface.get_current_texture().map_err(|e| match e {
            wgpu::SurfaceError::Lost => SurfaceError::Lost,
            wgpu::SurfaceError::Outdated => SurfaceError::Outdated,
            wgpu::SurfaceError::OutOfMemory => SurfaceError::OutOfMemory,
            wgpu::SurfaceError::Timeout => SurfaceError::Timeout,
            other => SurfaceError::Other(format!("{other:?}")),
        })?;
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        Ok(Frame { texture, view })
    }
}
