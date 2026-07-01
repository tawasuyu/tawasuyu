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
    /// Textura secundaria para la capa de overlay (menús/paleta/modal)
    /// cuando hay contenido `gpu_paint` que la taparía. El overlay se
    /// rasteriza acá con fondo transparente y luego se compone con
    /// alpha SOBRE la intermedia (que ya tiene UI + video). Ver
    /// [`OverlayCompositor`] y el eventloop de `llimphi-ui`.
    overlay_view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl Frame {
    pub fn view(&self) -> &wgpu::TextureView {
        &self.intermediate_view
    }

    /// Vista de la textura de overlay (mismo tamaño y formato que la
    /// intermedia). Sólo se usa en el camino de compositing del overlay.
    pub fn overlay_view(&self) -> &wgpu::TextureView {
        &self.overlay_view
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
            Ok(a) => (primary, a),
            Err(_) => {
                let all = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
                let a = all
                    .request_adapter(&opts)
                    .await
                    .map_err(|_| HalError::NoAdapter)?;
                (all, a)
            }
        };
        // `Limits::default()` cubre los 5 storage buffers/stage que vello
        // necesita. `downlevel_defaults()` solo expone 4 y rompe el raster.
        // Si el adapter no lo aguanta, `using_resolution` recorta lo recortable
        // (texturas/buffers grandes) preservando los conteos mínimos.
        let limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("llimphi-hal-device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
                experimental_features: wgpu::ExperimentalFeatures::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| HalError::RequestDevice(e.to_string()))?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// Construye el `Hal` **y** una [`RawSurface`] a la vez, eligiendo el adaptador
    /// **compatible con esa surface** — el dispositivo que el compositor sabe
    /// presentar. Es el camino correcto para el backend layer-shell de `pata`.
    ///
    /// El problema que resuelve: en sistemas multi-GPU (Optimus), pedir el
    /// adaptador sin pista de surface (`new(None)` con `HighPerformance`) puede
    /// elegir la dGPU mientras el compositor compone en la iGPU → los dmabuf
    /// cruzan dispositivos y `get_capabilities` devuelve 0 formatos (la surface
    /// "no expone formatos"). Pasar `compatible_surface` ata el adaptador al
    /// dispositivo del compositor. Como la surface hace falta ANTES de pedir el
    /// adaptador, y `new` crea la instancia internamente, este constructor une los
    /// dos pasos.
    ///
    /// `make_target` reconstruye el `SurfaceTargetUnsafe` cada vez que se llama
    /// (los `RawHandle` son `Copy`): `create_surface_unsafe` consume el target y
    /// puede que probemos dos instancias (PRIMARY y, si no hay adaptador, todos
    /// los backends — el GL de Mesa/Wayland revienta en teardown, por eso PRIMARY
    /// primero, igual que [`Hal::new`]).
    ///
    /// # Safety
    /// Los handles que produce `make_target` deben apuntar a objetos Wayland/…
    /// vivos durante toda la vida de la `RawSurface` devuelta.
    pub async unsafe fn new_for_raw_surface(
        make_target: impl Fn() -> wgpu::SurfaceTargetUnsafe,
        width: u32,
        height: u32,
    ) -> Result<(Self, RawSurface), HalError> {
        // PRIMARY (Vulkan/Metal/DX12) primero; si no hay adaptador compatible, a
        // todos los backends recreando instancia y surface.
        let primary = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let prim_surface = unsafe { primary.create_surface_unsafe(make_target()) }
            .map_err(|e| HalError::CreateSurface(e.to_string()))?;
        let prim_adapter = primary
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&prim_surface),
            })
            .await;
        let (instance, adapter, wgpu_surface) = match prim_adapter {
            Ok(a) => (primary, a, prim_surface),
            Err(_) => {
                let all = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
                let surface = unsafe { all.create_surface_unsafe(make_target()) }
                    .map_err(|e| HalError::CreateSurface(e.to_string()))?;
                let a = all
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::HighPerformance,
                        force_fallback_adapter: false,
                        compatible_surface: Some(&surface),
                    })
                    .await
                    .map_err(|_| HalError::NoAdapter)?;
                (all, a, surface)
            }
        };
        let limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("llimphi-hal-device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
                experimental_features: wgpu::ExperimentalFeatures::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| HalError::RequestDevice(e.to_string()))?;
        let hal = Self {
            instance,
            adapter,
            device,
            queue,
        };
        // Extraemos los raw handles del target para que la `RawSurface` pueda
        // recrearse ante una pérdida (los `RawHandle` son `Copy`).
        let (raw_display, raw_window) = match make_target() {
            wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle,
                raw_window_handle,
            } => (raw_display_handle, raw_window_handle),
            _ => {
                return Err(HalError::CreateSurface(
                    "new_for_raw_surface requiere SurfaceTargetUnsafe::RawHandle".into(),
                ))
            }
        };
        let surface =
            RawSurface::from_surface(&hal, wgpu_surface, raw_display, raw_window, width, height)?;
        Ok((hal, surface))
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
    /// Textura de la capa de overlay (ver [`Frame::overlay_view`]).
    overlay: wgpu::Texture,
    overlay_view: wgpu::TextureView,
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
        configure_checked(&hal.device, &surface, &config).map_err(HalError::CreateSurface)?;
        let (intermediate, intermediate_view) =
            create_intermediate(&hal.device, config.width, config.height);
        let (overlay, overlay_view) =
            create_intermediate(&hal.device, config.width, config.height);
        let blitter = wgpu::util::TextureBlitter::new(&hal.device, format);
        Ok(Self {
            _window: window,
            surface,
            config,
            device: hal.device.clone(),
            intermediate,
            intermediate_view,
            overlay,
            overlay_view,
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
    overlay: wgpu::Texture,
    overlay_view: wgpu::TextureView,
    blitter: wgpu::util::TextureBlitter,
    /// La instancia + los raw handles del `wl_surface`/`wl_display`, guardados para
    /// **recrear** la `wgpu::Surface` cuando queda irrecuperable (ver
    /// [`RawSurface::recreate`]). Los `RawHandle` son `Copy` y apuntan a objetos
    /// que el caller mantiene vivos toda la vida de la `RawSurface`.
    instance: wgpu::Instance,
    raw_display: raw_window_handle::RawDisplayHandle,
    raw_window: raw_window_handle::RawWindowHandle,
}

impl RawSurface {
    /// Envuelve una `wgpu::Surface` ya creada, con el tamaño físico inicial. Los
    /// `raw_display`/`raw_window` son los handles con que se creó la surface: se
    /// guardan para poder RECREARLA si el compositor la invalida (Iris Xe), ya que
    /// reconfigurar la misma surface no siempre recupera.
    pub fn from_surface(
        hal: &Hal,
        surface: wgpu::Surface<'static>,
        raw_display: raw_window_handle::RawDisplayHandle,
        raw_window: raw_window_handle::RawWindowHandle,
        width: u32,
        height: u32,
    ) -> Result<Self, HalError> {
        let caps = surface.get_capabilities(&hal.adapter);
        let info = hal.adapter.get_info();
        // Si la superficie no expone formatos, el compositor no la soporta por
        // este backend (Vulkan/GL WSI): error claro en vez de un panic por
        // indexar `formats[0]` sobre una lista vacía.
        let format = match caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .or_else(|| caps.formats.first().copied())
        {
            Some(f) => f,
            None => {
                return Err(HalError::CreateSurface(format!(
                    "la superficie no expone formatos (adapter {:?}/{:?}): el compositor no la soporta por {:?} WSI",
                    info.backend, info.device_type, info.backend
                )))
            }
        };
        // Para una layer surface (wlr-layer-shell) la transparencia es
        // crítica: la usamos para popovers/menús que pintan un panel chico y
        // dejan el resto transparente para ver el escritorio. La heurística
        // ingenua `caps.alpha_modes.first()` cae a veces en `Opaque` (el
        // compositor descarta alpha) — el clear TRANSPARENT se compone como
        // negro literal y el menú inicio sale como un cuadrón negro.
        //
        // Preferencia: PreMultiplied > PostMultiplied > Inherit > Auto >
        // Opaque. Los dos primeros componen alpha como esperamos; los dos
        // siguientes dejan que el compositor decida (típicamente respeta el
        // alpha del buffer ARGB); Opaque es el último recurso.
        let alpha_mode = {
            use wgpu::CompositeAlphaMode as Mode;
            let want = [
                Mode::PreMultiplied,
                Mode::PostMultiplied,
                Mode::Inherit,
                Mode::Auto,
            ];
            want.iter()
                .copied()
                .find(|m| caps.alpha_modes.contains(m))
                .or_else(|| caps.alpha_modes.first().copied())
                .unwrap_or(Mode::Auto)
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: choose_present_mode(&caps),
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        configure_checked(&hal.device, &surface, &config).map_err(HalError::CreateSurface)?;
        let (intermediate, intermediate_view) =
            create_intermediate(&hal.device, config.width, config.height);
        let (overlay, overlay_view) =
            create_intermediate(&hal.device, config.width, config.height);
        let blitter = wgpu::util::TextureBlitter::new(&hal.device, format);
        Ok(Self {
            surface,
            config,
            device: hal.device.clone(),
            intermediate,
            intermediate_view,
            overlay,
            overlay_view,
            blitter,
            instance: hal.instance.clone(),
            raw_display,
            raw_window,
        })
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Recrea la `wgpu::Surface` desde el raw handle del `wl_surface` y la
    /// reconfigura. Es la recuperación REAL cuando la surface quedó irrecuperable
    /// —p. ej. `configure` falla con «Surface does not support the adapter's queue
    /// family» tras un reset del compositor en Iris Xe—: reconfigurar la MISMA
    /// surface no basta, hay que crear una nueva. Devuelve el error si falla (el
    /// caller salta el frame y reintenta). Sin esto, la barra quedaba en un bucle
    /// de «surface perdida» hasta que se caía la conexión (Broken pipe).
    fn recreate(&mut self) -> Result<(), String> {
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: self.raw_display,
            raw_window_handle: self.raw_window,
        };
        // SAFETY: los handles apuntan a objetos Wayland que el caller mantiene
        // vivos toda la vida de la `RawSurface` (mismo contrato que `from_surface`).
        let surface = unsafe { self.instance.create_surface_unsafe(target) }
            .map_err(|e| format!("create_surface_unsafe: {e}"))?;
        configure_checked(&self.device, &surface, &self.config)?;
        self.surface = surface;
        Ok(())
    }
}

impl Surface for RawSurface {
    fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    fn resize(&mut self, width: u32, height: u32) {
        let (w, h) = (width.max(1), height.max(1));
        // Sin cambio de tamaño NO reconfiguramos. El backend layer-shell de `pata`
        // llama a `resize` en cada cuadro (no tiene eventos de resize como winit);
        // reconfigurar el swapchain por cuadro lo reconstruye una y otra vez, y en
        // Vulkan WSI eso **destruye el `wl_buffer` recién presentado antes de que el
        // compositor lo componga** — wlroots lo tolera, smithay (mirada) no, y la
        // superficie queda en negro (el compositor ve `buffer=None`).
        if self.config.width == w && self.config.height == h {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        if let Err(e) = configure_checked(&self.device, &self.surface, &self.config) {
            // Igual que en `acquire`: si reconfigurar no recupera, recreamos la
            // surface desde el raw handle antes de rendirnos.
            match self.recreate() {
                Ok(()) => eprintln!("llimphi-hal: surface RECREADA en resize (reconfigure falló: {e})"),
                Err(e2) => {
                    eprintln!("llimphi-hal: configure en resize falló ({e}); recrear también falló ({e2})")
                }
            }
        }
        let (tex, view) = create_intermediate(&self.device, self.config.width, self.config.height);
        self.intermediate = tex;
        self.intermediate_view = view;
        let (otex, oview) =
            create_intermediate(&self.device, self.config.width, self.config.height);
        self.overlay = otex;
        self.overlay_view = oview;
    }

    fn acquire(&mut self) -> Result<Frame, SurfaceError> {
        let texture = match self.surface.get_current_texture() {
            Ok(t) => t,
            // El backend layer-shell no tiene un evento de resize que reconfigure
            // el swapchain; si quedó obsoleto/perdido, lo reconstruimos aquí mismo
            // y reintentamos una vez. Sin esto el panel quedaría en negro para
            // siempre tras el primer `Outdated`.
            Err(e @ (wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost)) => {
                // Reconfigurar CAPTURANDO: si la surface está realmente perdida,
                // `configure` emite un error de validación que, sin captura,
                // mataría el proceso (era el crash de pata). Acá lo devolvemos
                // como `Lost` y el caller salta el frame e intenta de nuevo.
                if let Err(msg) = configure_checked(&self.device, &self.surface, &self.config) {
                    // Reconfigurar no recuperó (surface realmente perdida): RECREAR
                    // desde el raw handle. Si eso también falla, saltamos el frame.
                    match self.recreate() {
                        Ok(()) => eprintln!(
                            "llimphi-hal: surface RECREADA tras {e:?} (reconfigure falló: {msg})"
                        ),
                        Err(msg2) => {
                            eprintln!(
                                "llimphi-hal: reconfigurar tras {e:?} falló ({msg}); recrear también falló ({msg2})"
                            );
                            return Err(SurfaceError::Lost);
                        }
                    }
                }
                self.surface.get_current_texture().map_err(|_| match e {
                    wgpu::SurfaceError::Lost => SurfaceError::Lost,
                    _ => SurfaceError::Outdated,
                })?
            }
            Err(wgpu::SurfaceError::OutOfMemory) => return Err(SurfaceError::OutOfMemory),
            Err(wgpu::SurfaceError::Timeout) => return Err(SurfaceError::Timeout),
            Err(other) => return Err(SurfaceError::Other(format!("{other:?}"))),
        };
        let surface_view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        Ok(Frame {
            surface_texture: texture,
            surface_view,
            intermediate_view: self.intermediate_view.clone(),
            overlay_view: self.overlay_view.clone(),
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

/// Configura el swapchain **capturando** los errores de validación de wgpu, en
/// vez de dejar que el handler por defecto («wgpu errors as fatal») paniquee el
/// proceso entero. Devuelve `Err(mensaje)` si la configuración falló — casi
/// siempre porque la surface se perdió (`SURFACE_LOST`, un hipo transitorio del
/// compositor/GPU): el caller decide (saltar el frame, marcar perdida) en vez de
/// morir. Robustez: una app no debe caerse por un glitch del surface (lo que
/// tumbaba a `pata` con «Surface does not support the adapter's queue family»).
///
/// Los errores de validación de wgpu son CPU-side: el scope los captura durante
/// la llamada, así que `pop_error_scope` resuelve sin bloquear contra la GPU.
fn configure_checked(
    device: &wgpu::Device,
    surface: &wgpu::Surface<'static>,
    config: &wgpu::SurfaceConfiguration,
) -> Result<(), String> {
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    surface.configure(device, config);
    match pollster::block_on(device.pop_error_scope()) {
        Some(e) => Err(e.to_string()),
        None => Ok(()),
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

/// Compositor de la capa de overlay: alpha-blittea una textura source (el
/// overlay rasterizado por vello sobre fondo transparente) SOBRE una textura
/// target (la intermedia, que ya tiene la UI principal + el video pintado por
/// `gpu_paint`). Resuelve el z-order: sin esto, el blit de `gpu_paint` (video)
/// queda encima de la capa vello del overlay y los menús se ven por debajo del
/// video.
///
/// Es un pase de pantalla completa (triángulo) que samplea el source y lo
/// emite con alpha-over. El factor de blend asume alpha **premultiplicado**
/// (lo que produce vello); si en pantalla los menús se ven con halos oscuros o
/// transparencia rara, exportar `LLIMPHI_OVERLAY_BLEND=straight` para usar
/// alpha recto sin recompilar.
pub struct OverlayCompositor {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bind_layout: wgpu::BindGroupLayout,
}

impl OverlayCompositor {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-overlay-composite"),
            source: wgpu::ShaderSource::Wgsl(OVERLAY_COMPOSITE_WGSL.into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-overlay-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-overlay-pl"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });
        // Alpha-over. `src_factor` distingue premultiplicado (One) de recto
        // (SrcAlpha); el resto es siempre OneMinusSrcAlpha.
        let straight = std::env::var("LLIMPHI_OVERLAY_BLEND")
            .map(|v| v.trim().eq_ignore_ascii_case("straight"))
            .unwrap_or(false);
        let color_src = if straight {
            wgpu::BlendFactor::SrcAlpha
        } else {
            wgpu::BlendFactor::One
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-overlay-pipe"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: INTERMEDIATE_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: color_src,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("llimphi-overlay-sampler"),
            ..Default::default()
        });
        OverlayCompositor {
            pipeline,
            sampler,
            bind_layout,
        }
    }

    /// Compone `source` (overlay con fondo transparente) sobre `target` (la
    /// intermedia), preservando el contenido previo del target (LoadOp::Load)
    /// y mezclando con alpha. Graba un render pass en `encoder`.
    pub fn composite(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        source: &wgpu::TextureView,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-overlay-bg"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("llimphi-overlay-composite-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Pase de pantalla completa que samplea la textura de overlay y la emite
/// para alpha-over. Triángulo grande que cubre el viewport; UV mapea clip
/// → texel 1:1 (Y invertida, igual que un blit estándar).
const OVERLAY_COMPOSITE_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let xy = corners[vi];
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src_tex, src_samp, in.uv);
}
"#;

/// Gaussian backdrop blur sobre la intermediate (la textura donde vello pinta
/// la UI). El compositor empuja dos render passes separables (horizontal +
/// vertical) restringidas por scissor al rect del nodo `.backdrop_blur(sigma)`,
/// usando una textura scratch interna del mismo tamaño que la intermediate.
///
/// **Pipeline**: vs = triángulo grande full-screen (clip-space), fs = suma
/// ponderada de N samples a lo largo de `direction`, pesos Gauss `exp(-i²/2σ²)`.
/// El bind group lleva la textura source + sampler bilinear + UBO con
/// `(direction, pixel_size, sigma, radius)`. El scissor recorta el output al
/// rect del nodo; el resto del target queda intacto (LoadOp::Load).
///
/// **Coste**: una pasada por dirección por nodo blur, ~`2*radius+1` taps por
/// pixel del rect. Para `sigma=8` (radius=24), ~49 taps/pixel — barato si el
/// rect es pequeño (chrome), pesado si es full-screen. v1: sin cap dinámico,
/// se asume que el caller no abusa.
///
/// **Limitaciones v1**:
/// - Un scratch full-screen alocado por compositor; resize sigue al `Surface`.
/// - `radius` cap en 32 — sigmas > ~10 se ven menos suaves (clip de cola).
/// - Bordes del rect: clamp-to-edge (sampler) → los pixeles fuera del rect
///   que se muestrean en la cola del Gauss salen como espejo del borde. En
///   un viewport razonable la diferencia es invisible; documentado.
pub struct BlurCompositor {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bind_layout: wgpu::BindGroupLayout,
    scratch: Option<BlurScratch>,
}

struct BlurScratch {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

/// Layout en GPU del UBO del blur. Debe coincidir con el `BlurParams` del WGSL.
/// Padding explícito al final para llegar a múltiplo de 16 bytes (alignment
/// estándar de uniformes en wgpu).
#[repr(C)]
#[derive(Clone, Copy)]
struct BlurUniforms {
    direction: [f32; 2],
    pixel_size: [f32; 2],
    sigma: f32,
    radius: f32,
    _pad: [f32; 2],
}

const BLUR_UBO_SIZE: u64 = std::mem::size_of::<BlurUniforms>() as u64;
const BLUR_MAX_RADIUS: f32 = 32.0;

impl BlurCompositor {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-blur-shader"),
            source: wgpu::ShaderSource::Wgsl(BLUR_WGSL.into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-blur-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-blur-pl"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-blur-pipe"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: INTERMEDIATE_FORMAT,
                    // El blur OVERWRITE el rect; no necesita alpha-over. El
                    // resultado del Gauss es opaco si los pixeles muestreados
                    // lo son (la intermediate tiene UI + background opaco).
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("llimphi-blur-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        BlurCompositor {
            pipeline,
            sampler,
            bind_layout,
            scratch: None,
        }
    }

    /// Aplica un blur Gaussiano sobre `target` en el rect dado (coords pixel
    /// del viewport). Si el rect cae fuera del viewport, no hace nada. Usa
    /// un scratch interno del mismo tamaño que el viewport — se aloca lazy y
    /// se reusa entre frames; se recrea si el viewport cambió.
    ///
    /// `sigma` controla el ancho del kernel. ~`σ=4` da "frosted glass" suave,
    /// `σ=16` un blur fuerte. El radius efectivo se cap a [`BLUR_MAX_RADIUS`].
    pub fn blur(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: (u32, u32),
        rect: (f32, f32, f32, f32),
        sigma: f32,
    ) {
        let (vw, vh) = viewport;
        if vw == 0 || vh == 0 || sigma <= 0.0 {
            return;
        }
        let (rx, ry, rw, rh) = rect;
        // Clamp scissor al viewport (un rect fuera del viewport pifia el
        // RenderPass).
        let x0 = rx.max(0.0) as u32;
        let y0 = ry.max(0.0) as u32;
        let x1 = (rx + rw).min(vw as f32).max(0.0) as u32;
        let y1 = (ry + rh).min(vh as f32).max(0.0) as u32;
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let scissor = (x0, y0, x1 - x0, y1 - y0);

        // Scratch del tamaño del viewport. Si cambió, recrear.
        let need_new = match &self.scratch {
            Some(s) => s.width != vw || s.height != vh,
            None => true,
        };
        if need_new {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("llimphi-blur-scratch"),
                size: wgpu::Extent3d {
                    width: vw,
                    height: vh,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: INTERMEDIATE_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.scratch = Some(BlurScratch {
                _texture: texture,
                view,
                width: vw,
                height: vh,
            });
        }
        let scratch_view = &self.scratch.as_ref().expect("scratch creado arriba").view;

        let radius = (sigma * 3.0).ceil().min(BLUR_MAX_RADIUS);
        let pixel_size = [1.0 / vw as f32, 1.0 / vh as f32];
        let ubo_h_data = BlurUniforms {
            direction: [1.0, 0.0],
            pixel_size,
            sigma,
            radius,
            _pad: [0.0, 0.0],
        };
        let ubo_v_data = BlurUniforms {
            direction: [0.0, 1.0],
            pixel_size,
            sigma,
            radius,
            _pad: [0.0, 0.0],
        };
        // UBOs por llamada (ver nota en `ColorFilterCompositor::apply`): varios
        // blurs en el mismo submit con sigmas distintos no deben aliasar un UBO
        // compartido (ganaría el último). Buffers frescos por llamada (32 bytes).
        let ubo_h = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-blur-ubo-h"),
            size: BLUR_UBO_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ubo_v = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-blur-ubo-v"),
            size: BLUR_UBO_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ubo_h, 0, bytemuck_cast(&ubo_h_data));
        queue.write_buffer(&ubo_v, 0, bytemuck_cast(&ubo_v_data));

        // Pass 1: target → scratch (horizontal).
        let bg_h = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-blur-bg-h"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(target),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: ubo_h.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("llimphi-blur-pass-h"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: scratch_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        // No nos importa qué hay fuera del scissor: el segundo
                        // pase sólo lee dentro del scissor también.
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg_h, &[]);
            pass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: scratch → target (vertical), preservando lo fuera del scissor.
        let bg_v = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-blur-bg-v"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(scratch_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: ubo_v.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("llimphi-blur-pass-v"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg_v, &[]);
            pass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
            pass.draw(0..3, 0..1);
        }
    }
}

/// Aplica una **matriz de color 4×5** (CSS `filter: brightness/contrast/
/// grayscale/sepia/saturate/invert/hue-rotate/opacity`) sobre un rect de la
/// intermediate. Espejo de [`BlurCompositor`] pero con un fragment shader que
/// multiplica cada píxel por la matriz: `out = M·rgba + bias`, clampeado a
/// `[0,1]`. Dos pases (target→scratch aplicando la matriz, scratch→target
/// copia identidad) por la misma razón que el blur: un render pass no puede
/// leer y escribir la misma textura. Fase 7.1233.
pub struct ColorFilterCompositor {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bind_layout: wgpu::BindGroupLayout,
    scratch: Option<BlurScratch>,
}

/// UBO de la matriz de color. 5 `vec4` (filas R/G/B/A + bias) = 80 bytes,
/// múltiplo de 16. Debe coincidir con `ColorParams` del WGSL.
#[repr(C)]
#[derive(Clone, Copy)]
struct ColorUniforms {
    r: [f32; 4],
    g: [f32; 4],
    b: [f32; 4],
    a: [f32; 4],
    bias: [f32; 4],
}

const COLOR_UBO_SIZE: u64 = std::mem::size_of::<ColorUniforms>() as u64;

/// La matriz identidad (copia sin cambios), usada en el segundo pase.
const COLOR_IDENTITY: ColorUniforms = ColorUniforms {
    r: [1.0, 0.0, 0.0, 0.0],
    g: [0.0, 1.0, 0.0, 0.0],
    b: [0.0, 0.0, 1.0, 0.0],
    a: [0.0, 0.0, 0.0, 1.0],
    bias: [0.0, 0.0, 0.0, 0.0],
};

impl ColorFilterCompositor {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-color-filter-shader"),
            source: wgpu::ShaderSource::Wgsl(COLOR_WGSL.into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-color-filter-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-color-filter-pl"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-color-filter-pipe"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: INTERMEDIATE_FORMAT,
                    // OVERWRITE el rect, igual que el blur — el resultado de la
                    // matriz reemplaza el píxel.
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("llimphi-color-filter-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        ColorFilterCompositor {
            pipeline,
            sampler,
            bind_layout,
            scratch: None,
        }
    }

    /// Aplica la matriz de color `matrix` (4×5 row-major: por fila
    /// `[c0, c1, c2, c3, bias]`, salida R/G/B/A) sobre `target` en el rect dado
    /// (coords pixel del viewport). Fuera del viewport no hace nada. Usa un
    /// scratch del tamaño del viewport (lazy, reusado entre frames).
    pub fn apply(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: (u32, u32),
        rect: (f32, f32, f32, f32),
        matrix: [f32; 20],
    ) {
        let (vw, vh) = viewport;
        if vw == 0 || vh == 0 {
            return;
        }
        let (rx, ry, rw, rh) = rect;
        let x0 = rx.max(0.0) as u32;
        let y0 = ry.max(0.0) as u32;
        let x1 = (rx + rw).min(vw as f32).max(0.0) as u32;
        let y1 = (ry + rh).min(vh as f32).max(0.0) as u32;
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let scissor = (x0, y0, x1 - x0, y1 - y0);

        let need_new = match &self.scratch {
            Some(s) => s.width != vw || s.height != vh,
            None => true,
        };
        if need_new {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("llimphi-color-filter-scratch"),
                size: wgpu::Extent3d {
                    width: vw,
                    height: vh,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: INTERMEDIATE_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.scratch = Some(BlurScratch {
                _texture: texture,
                view,
                width: vw,
                height: vh,
            });
        }
        let scratch_view = &self.scratch.as_ref().expect("scratch creado arriba").view;

        // El [f32;20] viene por filas de 5 (`[c0,c1,c2,c3,bias]`); lo partimos
        // en 4 vec4 de coeficientes + un vec4 de bias para el UBO.
        let apply = ColorUniforms {
            r: [matrix[0], matrix[1], matrix[2], matrix[3]],
            g: [matrix[5], matrix[6], matrix[7], matrix[8]],
            b: [matrix[10], matrix[11], matrix[12], matrix[13]],
            a: [matrix[15], matrix[16], matrix[17], matrix[18]],
            bias: [matrix[4], matrix[9], matrix[14], matrix[19]],
        };
        // UBOs **por llamada**: varias `apply` en el mismo encoder/submit
        // comparten cola; `write_buffer` se aplica una vez antes de los command
        // buffers (gana el último valor escrito), así que un UBO compartido haría
        // que todas las pasadas leyeran la última matriz. Buffers frescos por
        // llamada evitan ese alias (80 bytes c/u, despreciable).
        let ubo_apply = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-color-filter-ubo-apply"),
            size: COLOR_UBO_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ubo_copy = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-color-filter-ubo-copy"),
            size: COLOR_UBO_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&ubo_apply, 0, bytemuck_cast(&apply));
        queue.write_buffer(&ubo_copy, 0, bytemuck_cast(&COLOR_IDENTITY));

        // Pass 1: target → scratch (aplica la matriz).
        let bg_apply = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-color-filter-bg-apply"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(target),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: ubo_apply.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("llimphi-color-filter-pass-apply"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: scratch_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg_apply, &[]);
            pass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: scratch → target (copia identidad), preservando lo de afuera.
        let bg_copy = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-color-filter-bg-copy"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(scratch_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: ubo_copy.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("llimphi-color-filter-pass-copy"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bg_copy, &[]);
            pass.set_scissor_rect(scissor.0, scissor.1, scissor.2, scissor.3);
            pass.draw(0..3, 0..1);
        }
    }
}

/// "bytemuck" minimal sin dep: convierte `&T` a `&[u8]`. Sólo para POD repr(C)
/// — usado para escribir los UBOs del blur con `queue.write_buffer`.
fn bytemuck_cast<T: Copy>(v: &T) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            v as *const T as *const u8,
            std::mem::size_of::<T>(),
        )
    }
}

/// Separable Gaussian, una dirección por pase. El vs es el mismo triángulo
/// grande del overlay; el fs samplea `2*radius+1` taps a lo largo de
/// `direction*pixel_size`. Pesos `exp(-i²/2σ²)` normalizados por la suma —
/// independiente del radius por si quedó cortada la cola.
const BLUR_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let xy = corners[vi];
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

struct BlurParams {
    direction: vec2<f32>,
    pixel_size: vec2<f32>,
    sigma: f32,
    radius: f32,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(0) @binding(2) var<uniform> params: BlurParams;

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let dir = params.direction * params.pixel_size;
    let r = i32(params.radius);
    let two_sigma_sq = 2.0 * params.sigma * params.sigma;
    var acc = vec4<f32>(0.0);
    var weight_sum = 0.0;
    for (var i = -r; i <= r; i = i + 1) {
        let fi = f32(i);
        let w = exp(-(fi * fi) / two_sigma_sq);
        acc = acc + textureSample(src_tex, src_samp, in.uv + dir * fi) * w;
        weight_sum = weight_sum + w;
    }
    return acc / weight_sum;
}
"#;

/// Matriz de color 4×5: `out = M·rgba + bias`, clampeado a `[0,1]`. El vs es el
/// mismo triángulo grande; el fs hace 4 `dot` (una fila por canal) más el bias.
const COLOR_WGSL: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    let xy = corners[vi];
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, (1.0 - xy.y) * 0.5);
    return out;
}

struct ColorParams {
    r: vec4<f32>,
    g: vec4<f32>,
    b: vec4<f32>,
    a: vec4<f32>,
    bias: vec4<f32>,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(0) @binding(2) var<uniform> params: ColorParams;

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(src_tex, src_samp, in.uv);
    var o: vec4<f32>;
    o.r = dot(params.r, c) + params.bias.r;
    o.g = dot(params.g, c) + params.bias.g;
    o.b = dot(params.b, c) + params.bias.b;
    o.a = dot(params.a, c) + params.bias.a;
    return clamp(o, vec4<f32>(0.0), vec4<f32>(1.0));
}
"#;

impl Surface for WinitSurface {
    fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        if let Err(e) = configure_checked(&self.device, &self.surface, &self.config) {
            eprintln!("llimphi-hal: configure en resize falló (surface perdida?): {e}");
        }
        let (tex, view) = create_intermediate(&self.device, self.config.width, self.config.height);
        self.intermediate = tex;
        self.intermediate_view = view;
        let (otex, oview) =
            create_intermediate(&self.device, self.config.width, self.config.height);
        self.overlay = otex;
        self.overlay_view = oview;
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
            overlay_view: self.overlay_view.clone(),
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
