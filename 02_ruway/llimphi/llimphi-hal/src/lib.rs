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

/// Superficie gráfica donde llimphi pinta.
///
/// Vello (rasterizador) emite a una textura intermedia con storage binding
/// (la única forma portable: los formatos de swapchain no aceptan writes
/// de compute shader en muchos adapters). En `present` se blittea la
/// intermedia al swapchain real y se hace el flip.
///
/// Implementaciones:
/// - [`WinitSurface`]: ventana Wayland/X11 (dev + producción vía mirada).
/// - `WawaFramebufferSurface` (TODO): framebuffer directo del kernel wawa.
pub trait Surface {
    fn size(&self) -> (u32, u32);
    fn resize(&mut self, width: u32, height: u32);
    /// Adquiere la textura intermedia donde el raster pinta este frame.
    fn acquire(&mut self) -> Result<Frame, SurfaceError>;
    /// Blittea la intermedia al swapchain y la presenta.
    fn present(&mut self, frame: Frame, hal: &Hal);
}

/// Frame en curso. `view()` devuelve la textura intermedia (Rgba8Unorm,
/// STORAGE_BINDING) lista para que vello escriba sobre ella.
pub struct Frame {
    surface_texture: wgpu::SurfaceTexture,
    surface_view: wgpu::TextureView,
    intermediate_view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl Frame {
    pub fn view(&self) -> &wgpu::TextureView {
        &self.intermediate_view
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
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
        let opts = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface,
        };
        // Preferimos backends PRIMARY (Vulkan/Metal/DX12). El backend GL de
        // wgpu sobre Mesa/Wayland tiene un bug de teardown: al soltar la
        // instancia, `eglTerminate` marshalea sobre una conexión Wayland ya
        // muerta (`wl_proxy_marshal`) y revienta con SIGSEGV. Con
        // `Backends::all()` (el default), wgpu puede elegir GL aun habiendo
        // Vulkan, y la app crashea al cerrar/teardown. Forzamos PRIMARY; si la
        // máquina no tiene Vulkan/Metal/DX12 (VM vieja, etc.) caemos a todos
        // los backends —incluido GL— para no dejarla sin gráficos. En el
        // camino de escritorio `compatible_surface` es `None` (la surface se
        // crea después contra esta misma instancia), así que cambiar de
        // instancia aquí es seguro.
        let primary = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let (instance, adapter) = match primary.request_adapter(&opts).await {
            Some(a) => (primary, a),
            None => {
                let all = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
                let a = all.request_adapter(&opts).await.ok_or(HalError::NoAdapter)?;
                (all, a)
            }
        };
        // `Limits::default()` cubre los 5 storage buffers/stage que vello
        // necesita. `downlevel_defaults()` solo expone 4 y rompe el raster.
        // Si el adapter no lo aguanta, `using_resolution` recorta lo recortable
        // (texturas/buffers grandes) preservando los conteos mínimos.
        let limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("llimphi-hal-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: limits,
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

/// Surface basada en `winit::window::Window`. Mantiene una textura
/// intermedia `Rgba8Unorm` con storage binding (donde pinta vello) y
/// un `TextureBlitter` que la copia al swapchain al presentar.
pub struct WinitSurface {
    _window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    intermediate: wgpu::Texture,
    intermediate_view: wgpu::TextureView,
    blitter: wgpu::util::TextureBlitter,
}

const INTERMEDIATE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl WinitSurface {
    /// Constructor "feliz": crea la `wgpu::Surface` internamente.
    /// Conveniente en desktop donde la secuencia normal es
    /// `Hal::new(None)` → `WinitSurface::new(hal, window)`. **En Android
    /// usar [`WinitSurface::from_surface`]** — allí la surface debe
    /// existir antes del `request_adapter(compatible_surface=Some(...))`,
    /// y crearla dos veces sobre la misma `ANativeWindow` falla con
    /// `ERROR_NATIVE_WINDOW_IN_USE_KHR`.
    pub fn new(hal: &Hal, window: Arc<Window>) -> Result<Self, HalError> {
        let surface = hal
            .instance
            .create_surface(window.clone())
            .map_err(|e| HalError::CreateSurface(e.to_string()))?;
        Self::from_surface(hal, window, surface)
    }

    /// Constructor reutilizable: arma el `WinitSurface` envolviendo una
    /// `wgpu::Surface` ya creada por el caller. Necesario en Android
    /// porque el orden allí es:
    ///
    /// 1. `instance.create_surface(window)`
    /// 2. `instance.request_adapter(compatible_surface=Some(&surface))`
    /// 3. `adapter.request_device(...)`
    /// 4. `WinitSurface::from_surface(hal, window, surface)`
    ///
    /// — no se puede dropear la surface entre 2 y 4 ni recrearla, porque
    /// Android reserva la `ANativeWindow` por VkSurface y rechaza un
    /// segundo `vkCreateAndroidSurfaceKHR` sobre la misma ventana.
    pub fn from_surface(
        hal: &Hal,
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
    ) -> Result<Self, HalError> {
        let size = window.inner_size();
        let caps = surface.get_capabilities(&hal.adapter);
        // Preferimos Bgra8Unorm o Rgba8Unorm (no sRGB) para que el blit
        // desde la intermedia lineal preserve los valores tal cual.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            // El swapchain solo necesita render-attachment: vello no escribe
            // directo, escribe a la intermedia y luego se blittea.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: choose_present_mode(&caps),
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&hal.device, &config);
        let (intermediate, intermediate_view) =
            create_intermediate(&hal.device, config.width, config.height);
        let blitter = wgpu::util::TextureBlitter::new(&hal.device, format);
        Ok(Self {
            _window: window,
            surface,
            config,
            device: hal.device.clone(),
            intermediate,
            intermediate_view,
            blitter,
        })
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }
}

/// Surface sobre una `wgpu::Surface` creada desde **handles raw** (sin
/// `winit::Window`): la usa el backend `wlr-layer-shell` de `pata` para pintar
/// en una *layer surface* de Wayland (barras/paneles al nivel de eww/waybar).
/// Misma mecánica que [`WinitSurface`] —intermedia `Rgba8Unorm` + blit al
/// swapchain— pero el tamaño se pasa explícito porque no hay ventana que
/// consultar. La `wgpu::Surface` la crea el caller (típicamente con
/// `instance.create_surface_unsafe` desde los punteros `wl_display`/`wl_surface`).
pub struct RawSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    intermediate: wgpu::Texture,
    intermediate_view: wgpu::TextureView,
    blitter: wgpu::util::TextureBlitter,
}

impl RawSurface {
    /// Envuelve una `wgpu::Surface` ya creada, con el tamaño físico inicial.
    pub fn from_surface(
        hal: &Hal,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Self, HalError> {
        let caps = surface.get_capabilities(&hal.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: choose_present_mode(&caps),
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&hal.device, &config);
        let (intermediate, intermediate_view) =
            create_intermediate(&hal.device, config.width, config.height);
        let blitter = wgpu::util::TextureBlitter::new(&hal.device, format);
        Ok(Self {
            surface,
            config,
            device: hal.device.clone(),
            intermediate,
            intermediate_view,
            blitter,
        })
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }
}

impl Surface for RawSurface {
    fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
        let (tex, view) = create_intermediate(&self.device, self.config.width, self.config.height);
        self.intermediate = tex;
        self.intermediate_view = view;
    }

    fn acquire(&mut self) -> Result<Frame, SurfaceError> {
        let texture = self.surface.get_current_texture().map_err(|e| match e {
            wgpu::SurfaceError::Lost => SurfaceError::Lost,
            wgpu::SurfaceError::Outdated => SurfaceError::Outdated,
            wgpu::SurfaceError::OutOfMemory => SurfaceError::OutOfMemory,
            wgpu::SurfaceError::Timeout => SurfaceError::Timeout,
            other => SurfaceError::Other(format!("{other:?}")),
        })?;
        let surface_view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        Ok(Frame {
            surface_texture: texture,
            surface_view,
            intermediate_view: self.intermediate_view.clone(),
            width: self.config.width,
            height: self.config.height,
        })
    }

    fn present(&mut self, frame: Frame, hal: &Hal) {
        let mut encoder = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("llimphi-blit-raw"),
        });
        self.blitter.copy(
            &hal.device,
            &mut encoder,
            &frame.intermediate_view,
            &frame.surface_view,
        );
        hal.queue.submit(std::iter::once(encoder.finish()));
        frame.surface_texture.present();
    }
}

/// Elige el modo de presentación del swapchain.
///
/// Default: **Mailbox** si el driver lo expone, sino **Fifo**. La razón es
/// el cuelgue observado en las apps Llimphi (investigación 2026-05-30): con
/// `Fifo`/`AutoVsync`, `surface.get_current_texture()` **bloquea** esperando
/// el frame-callback del compositor Wayland — si el compositor no suelta un
/// buffer, el hilo del UI queda dormido (CPU baja, deadlock aparente).
/// `Mailbox` no bloquea (triple-buffer, descarta frames viejos), así que el
/// loop nunca se queda esperando al compositor. `Fifo` está garantizado por
/// spec como fallback.
///
/// Override por entorno para A/B sin recompilar (útil en la laptop con
/// display real): `LLIMPHI_PRESENT_MODE = fifo | mailbox | immediate |
/// fifo_relaxed`. Si el modo pedido no está soportado, se ignora y se aplica
/// el default.
fn choose_present_mode(caps: &wgpu::SurfaceCapabilities) -> wgpu::PresentMode {
    use wgpu::PresentMode::{Fifo, FifoRelaxed, Immediate, Mailbox};
    if let Ok(v) = std::env::var("LLIMPHI_PRESENT_MODE") {
        let want = match v.trim().to_ascii_lowercase().as_str() {
            "fifo" | "vsync" => Some(Fifo),
            "fifo_relaxed" | "fiforelaxed" => Some(FifoRelaxed),
            "mailbox" => Some(Mailbox),
            "immediate" | "novsync" => Some(Immediate),
            _ => None,
        };
        if let Some(m) = want {
            if caps.present_modes.contains(&m) {
                return m;
            }
        }
    }
    if caps.present_modes.contains(&Mailbox) {
        Mailbox
    } else {
        Fifo
    }
}

fn create_intermediate(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("llimphi-intermediate"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: INTERMEDIATE_FORMAT,
        // STORAGE_BINDING: vello escribe via compute shader.
        // TEXTURE_BINDING: el blitter la lee como sampler source.
        // RENDER_ATTACHMENT: render passes con clear-only (sin vello)
        //   también escriben acá — desktop drivers lo tolerían sin este
        //   flag, Adreno con validación estricta rechaza el frame.
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

impl Surface for WinitSurface {
    fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
        let (tex, view) = create_intermediate(&self.device, self.config.width, self.config.height);
        self.intermediate = tex;
        self.intermediate_view = view;
    }

    fn acquire(&mut self) -> Result<Frame, SurfaceError> {
        let texture = self.surface.get_current_texture().map_err(|e| match e {
            wgpu::SurfaceError::Lost => SurfaceError::Lost,
            wgpu::SurfaceError::Outdated => SurfaceError::Outdated,
            wgpu::SurfaceError::OutOfMemory => SurfaceError::OutOfMemory,
            wgpu::SurfaceError::Timeout => SurfaceError::Timeout,
            other => SurfaceError::Other(format!("{other:?}")),
        })?;
        let surface_view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        // `TextureView` envuelve un Arc — clonar es atomic-incref, no
        // recrea la vista. La intermedia sólo cambia en `resize`.
        Ok(Frame {
            surface_texture: texture,
            surface_view,
            intermediate_view: self.intermediate_view.clone(),
            width: self.config.width,
            height: self.config.height,
        })
    }

    fn present(&mut self, frame: Frame, hal: &Hal) {
        let mut encoder = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("llimphi-blit"),
        });
        self.blitter.copy(
            &hal.device,
            &mut encoder,
            &frame.intermediate_view,
            &frame.surface_view,
        );
        hal.queue.submit(std::iter::once(encoder.finish()));
        frame.surface_texture.present();
    }
}
