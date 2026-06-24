//! `llimphi-layer` — corre un [`App`] de Llimphi como una **superficie
//! wlr-layer-shell** (una barra anclada a un borde), no como ventana. Hace que
//! una app se siente al nivel de eww/waybar en cualquier compositor wlroots
//! (mirada, Hyprland, Sway, river…): se ancla a un borde, declara una *exclusive
//! zone* opcional —el compositor le reserva la franja y tesela el resto— y se
//! pinta reusando el pipeline de Llimphi (`mount → compute → paint → render`).
//!
//! Es la plumbing `smithay-client-toolkit` + `wgpu` que `pata` probó en
//! producción, extraída a un runner **genérico** sobre el trait `App`. A
//! diferencia de pata (N superficies + tray/dock/sidebar/…), este corre **una**
//! superficie y nada de dominio: la app provee `view`/`update`/`on_key` y este
//! crate le presta el borde. Útil, p. ej., para que `shuma` se dockee como barra.
//!
//! ```ignore
//! struct MiBarra;
//! impl llimphi_ui::App for MiBarra { /* … */ }
//! llimphi_layer::run::<MiBarra>(llimphi_layer::LayerConfig {
//!     edge: llimphi_layer::Edge::Bottom,
//!     thickness: 40,
//!     keyboard: llimphi_layer::Keyboard::OnDemand,
//!     ..Default::default()
//! })?;
//! ```

use std::error::Error;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Mutex;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT, BTN_RIGHT},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor as LayerAnchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler,
            LayerSurface, LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, Proxy, QueueHandle,
};

use llimphi_ui::llimphi_compositor::{
    hit_test_click, hit_test_hover, hit_test_scroll, measure_text_node, mount, paint, DragFn,
    DragPhase, Mounted,
};
use llimphi_ui::llimphi_hal::{wgpu, Hal, RawSurface, Surface as _};
use llimphi_ui::llimphi_layout::{taffy, ComputedLayout, LayoutTree};
use llimphi_ui::llimphi_raster::{peniko::color::palette, vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers as LMods, NamedKey};

// ── API pública ─────────────────────────────────────────────────────────────

/// El borde al que se ancla la barra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// La esquina a la que se ancla una **caja** de tamaño fijo (no una barra de
/// borde completo). La usa, p. ej., un daemon de notificaciones: una caja en
/// la esquina, no una franja que cruza la pantalla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Márgenes (px) respecto de los bordes a los que la superficie se ancla.
/// El compositor los respeta como separación entre la caja y el borde.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Margins {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

impl Margins {
    /// El mismo margen en los cuatro lados.
    pub fn all(px: i32) -> Self {
        Self { top: px, right: px, bottom: px, left: px }
    }
}

/// La capa de composición (orden en Z respecto de las ventanas).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    /// Detrás de las ventanas (fondo de escritorio).
    Background,
    /// Bajo las ventanas normales.
    Bottom,
    /// Sobre las ventanas normales (lo típico para una barra).
    Top,
    /// Por encima de todo (popups, lock screens).
    Overlay,
}

/// Cuánto teclado acepta la superficie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyboard {
    /// No recibe teclado.
    None,
    /// Lo recibe al ser clickeada/enfocada (lo razonable para una barra que a
    /// veces toma texto, p. ej. un input).
    OnDemand,
    /// Acapara el teclado mientras exista (drawers/popups modales).
    Exclusive,
}

/// Configuración de la superficie de capa.
///
/// Dos modos de anclaje, según `corner`:
/// - **Barra de borde** (`corner: None`, el default): se ancla a `edge` y el
///   compositor estira el eje paralelo a toda la salida; `thickness` fija el
///   perpendicular. Es el modo waybar/eww.
/// - **Caja de esquina** (`corner: Some(_)`): se ancla a una esquina con
///   `size` explícito (ancho, alto) y `margins`. Ignora `edge`/`thickness` y
///   nunca reserva zona exclusiva (flota). Es el modo de un toast/OSD/popover.
#[derive(Debug, Clone)]
pub struct LayerConfig {
    pub edge: Edge,
    /// Grosor (px) en el eje perpendicular al borde (alto para Top/Bottom, ancho
    /// para Left/Right). El otro eje lo estira el compositor al tamaño de salida.
    /// Solo aplica en modo barra de borde (`corner: None`).
    pub thickness: u32,
    pub layer: LayerKind,
    /// Si `true`, reserva la franja (las ventanas no la tapan); si `false`, flota.
    /// En modo caja de esquina siempre flota, independientemente de este valor.
    pub exclusive: bool,
    pub keyboard: Keyboard,
    /// El namespace que ve el compositor (para reglas por-superficie).
    pub namespace: String,
    /// Si es `Some`, ancla una **caja** a esa esquina en vez de una barra de
    /// borde. Requiere `size`; usa `margins`.
    pub corner: Option<Corner>,
    /// Tamaño explícito `(ancho, alto)` en px para el modo caja de esquina.
    /// Ignorado en modo barra de borde. Si falta en modo esquina, se usa un
    /// default razonable.
    pub size: Option<(u32, u32)>,
    /// Separación (px) respecto de los bordes anclados, en modo caja de esquina.
    pub margins: Margins,
}

impl Default for LayerConfig {
    fn default() -> Self {
        Self {
            edge: Edge::Bottom,
            thickness: 40,
            layer: LayerKind::Top,
            exclusive: true,
            keyboard: Keyboard::OnDemand,
            namespace: "llimphi-layer".to_string(),
            corner: None,
            size: None,
            margins: Margins::default(),
        }
    }
}

/// Levanta el backend layer-shell y corre `A` hasta que la superficie se cierra.
/// Devuelve error si no hay sesión Wayland o el compositor no expone
/// `wlr-layer-shell`.
pub fn run<A: App>(cfg: LayerConfig) -> Result<(), Box<dyn Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init(&conn)?;
    let qh: QueueHandle<Runner<A>> = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;

    // Handle channel-backed: el `update`/efectos de la app usan su propio `Msg`;
    // cada dispatch/spawn/spawn_periodic cae en `rx`, que `draw` drena por frame.
    // El truco (igual que en pata): el `for_test().lift(f)` ejecuta `f` —el
    // `send`— antes del dispatch no-op del handle de test, así el canal recibe.
    let (tx, rx) = channel::<A::Msg>();
    let tx = Mutex::new(tx);
    let handle: Handle<A::Msg> = Handle::<()>::for_test().lift(move |m: A::Msg| {
        let _ = tx.lock().expect("tx layer").send(m);
    });
    let model = A::init(&handle);

    let resolved = resolve_anchor(&cfg);
    let wl_surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        wl_surface,
        cfg.layer.into(),
        Some(cfg.namespace.clone()),
        None,
    );
    layer.set_anchor(resolved.anchor);
    layer.set_size(resolved.size.0, resolved.size.1);
    layer.set_exclusive_zone(resolved.exclusive_zone);
    if let Some(m) = resolved.margins {
        layer.set_margin(m.top, m.right, m.bottom, m.left);
    }
    layer.set_keyboard_interactivity(cfg.keyboard.into());
    layer.commit();
    let size = resolved.size;

    let mut runner = Runner::<A> {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        conn,
        hal: None,
        keyboard: None,
        pointer: None,
        seat: None,
        layer,
        width: size.0.max(1),
        height: size.1.max(1),
        gpu: None,
        cache: None,
        hover_idx: None,
        drag: None,
        mods: Modifiers::default(),
        model: Some(model),
        handle,
        rx,
        exit: false,
    };

    while !runner.exit {
        event_queue.blocking_dispatch(&mut runner)?;
    }
    Ok(())
}

// ── Interno ─────────────────────────────────────────────────────────────────

/// Tamaño por defecto de una caja de esquina cuando `cfg.size` es `None`.
const DEFAULT_CORNER_SIZE: (u32, u32) = (360, 280);

/// El anclaje resuelto para la superficie: anclas sctk, tamaño pedido, zona
/// exclusiva y márgenes opcionales. Unifica los dos modos (barra/caja).
struct Resolved {
    anchor: LayerAnchor,
    size: (u32, u32),
    exclusive_zone: i32,
    margins: Option<Margins>,
}

/// Resuelve el anclaje según el modo de `cfg`: caja de esquina si `corner` es
/// `Some`, barra de borde en caso contrario.
fn resolve_anchor(cfg: &LayerConfig) -> Resolved {
    match cfg.corner {
        Some(corner) => {
            let size = cfg.size.unwrap_or(DEFAULT_CORNER_SIZE);
            let anchor = match corner {
                Corner::TopLeft => LayerAnchor::TOP | LayerAnchor::LEFT,
                Corner::TopRight => LayerAnchor::TOP | LayerAnchor::RIGHT,
                Corner::BottomLeft => LayerAnchor::BOTTOM | LayerAnchor::LEFT,
                Corner::BottomRight => LayerAnchor::BOTTOM | LayerAnchor::RIGHT,
            };
            // Una caja flota siempre: no tiene sentido reservar zona exclusiva.
            Resolved { anchor, size, exclusive_zone: 0, margins: Some(cfg.margins) }
        }
        None => {
            let (anchor, size) = anchor_and_size(cfg.edge, cfg.thickness);
            let exclusive_zone = if cfg.exclusive { cfg.thickness as i32 } else { 0 };
            Resolved { anchor, size, exclusive_zone, margins: None }
        }
    }
}

/// El anclaje sctk + el tamaño `(w, h)` pedido para un borde y grosor. El eje
/// paralelo al borde va en 0 → el compositor lo estira a la salida completa.
fn anchor_and_size(edge: Edge, thickness: u32) -> (LayerAnchor, (u32, u32)) {
    match edge {
        Edge::Top => (
            LayerAnchor::TOP | LayerAnchor::LEFT | LayerAnchor::RIGHT,
            (0, thickness),
        ),
        Edge::Bottom => (
            LayerAnchor::BOTTOM | LayerAnchor::LEFT | LayerAnchor::RIGHT,
            (0, thickness),
        ),
        Edge::Left => (
            LayerAnchor::LEFT | LayerAnchor::TOP | LayerAnchor::BOTTOM,
            (thickness, 0),
        ),
        Edge::Right => (
            LayerAnchor::RIGHT | LayerAnchor::TOP | LayerAnchor::BOTTOM,
            (thickness, 0),
        ),
    }
}

impl From<LayerKind> for Layer {
    fn from(k: LayerKind) -> Self {
        match k {
            LayerKind::Background => Layer::Background,
            LayerKind::Bottom => Layer::Bottom,
            LayerKind::Top => Layer::Top,
            LayerKind::Overlay => Layer::Overlay,
        }
    }
}

impl From<Keyboard> for KeyboardInteractivity {
    fn from(k: Keyboard) -> Self {
        match k {
            Keyboard::None => KeyboardInteractivity::None,
            Keyboard::OnDemand => KeyboardInteractivity::OnDemand,
            Keyboard::Exclusive => KeyboardInteractivity::Exclusive,
        }
    }
}

/// El estado wgpu de la superficie.
struct PanelGpu {
    surface: RawSurface,
    renderer: Renderer,
    typesetter: Typesetter,
    scene: vello::Scene,
    layout: LayoutTree,
}

/// El árbol pintado en el último frame, para hit-test entre frames.
struct RenderCache<Msg> {
    mounted: Mounted<Msg>,
    computed: ComputedLayout,
}

/// El cliente Wayland que corre un `App` como una sola layer surface.
struct Runner<A: App> {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    conn: Connection,
    hal: Option<Hal>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    seat: Option<wl_seat::WlSeat>,
    layer: LayerSurface,
    width: u32,
    height: u32,
    gpu: Option<PanelGpu>,
    cache: Option<RenderCache<A::Msg>>,
    hover_idx: Option<usize>,
    /// Arrastre en curso: handler del nodo + última posición del puntero.
    drag: Option<(DragFn<A::Msg>, (f32, f32))>,
    mods: Modifiers,
    model: Option<A::Model>,
    handle: Handle<A::Msg>,
    rx: Receiver<A::Msg>,
    exit: bool,
}

impl<A: App> Runner<A> {
    /// Aplica un `Msg` a la app (transición pura del modelo) y marca para
    /// repintar invalidando la caché de hit-test.
    fn apply(&mut self, msg: A::Msg) {
        if let Some(model) = self.model.take() {
            self.model = Some(A::update(model, msg, &self.handle));
        }
        self.cache = None;
    }

    /// Traduce un evento de teclado sctk al `KeyEvent` de Llimphi.
    fn keysym_to_keyevent(
        &self,
        event: &smithay_client_toolkit::seat::keyboard::KeyEvent,
    ) -> Option<KeyEvent> {
        use smithay_client_toolkit::seat::keyboard::Keysym as K;
        let named = match event.keysym {
            K::Return | K::KP_Enter => Some(NamedKey::Enter),
            K::BackSpace => Some(NamedKey::Backspace),
            K::Tab | K::ISO_Left_Tab => Some(NamedKey::Tab),
            K::Escape => Some(NamedKey::Escape),
            K::Up => Some(NamedKey::ArrowUp),
            K::Down => Some(NamedKey::ArrowDown),
            K::Right => Some(NamedKey::ArrowRight),
            K::Left => Some(NamedKey::ArrowLeft),
            K::Home => Some(NamedKey::Home),
            K::End => Some(NamedKey::End),
            K::Page_Up => Some(NamedKey::PageUp),
            K::Page_Down => Some(NamedKey::PageDown),
            K::Delete => Some(NamedKey::Delete),
            K::Insert => Some(NamedKey::Insert),
            K::F1 => Some(NamedKey::F1),
            K::F2 => Some(NamedKey::F2),
            K::F3 => Some(NamedKey::F3),
            K::F4 => Some(NamedKey::F4),
            K::F5 => Some(NamedKey::F5),
            K::F6 => Some(NamedKey::F6),
            K::F7 => Some(NamedKey::F7),
            K::F8 => Some(NamedKey::F8),
            K::F9 => Some(NamedKey::F9),
            K::F10 => Some(NamedKey::F10),
            K::F11 => Some(NamedKey::F11),
            K::F12 => Some(NamedKey::F12),
            _ => None,
        };
        let modifiers = LMods {
            shift: self.mods.shift,
            ctrl: self.mods.ctrl,
            alt: self.mods.alt,
            meta: self.mods.logo,
        };
        let (key, text) = if let Some(n) = named {
            (Key::Named(n), None)
        } else {
            let txt = match event.utf8.as_deref() {
                Some(s) if !s.is_empty() && !s.chars().all(char::is_control) => s.to_string(),
                _ => event.keysym.key_char()?.to_string(),
            };
            (Key::Character(txt.as_str().into()), Some(txt))
        };
        Some(KeyEvent {
            key,
            state: KeyState::Pressed,
            text,
            modifiers,
            repeat: false,
        })
    }

    /// Crea el estado wgpu de la superficie (idempotente).
    fn ensure_gpu(&mut self) {
        if self.gpu.is_some() {
            return;
        }
        let display_ptr = self.conn.backend().display_ptr() as *mut c_void;
        let surface_ptr = self.layer.wl_surface().id().as_ptr() as *mut c_void;
        let (w, h) = (self.width, self.height);
        let display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            NonNull::new(display_ptr).expect("wl_display ptr"),
        ));
        let window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
            NonNull::new(surface_ptr).expect("wl_surface ptr"),
        ));
        // SAFETY: los handles apuntan a objetos Wayland que `self` mantiene vivos.
        let make_target = || wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: display_handle,
            raw_window_handle: window_handle,
        };

        let surface = if self.hal.is_none() {
            match pollster::block_on(unsafe { Hal::new_for_raw_surface(make_target, w, h) }) {
                Ok((hal, surface)) => {
                    self.hal = Some(hal);
                    surface
                }
                Err(e) => {
                    eprintln!("llimphi-layer · sin gpu: {e}");
                    return;
                }
            }
        } else {
            let hal = self.hal.as_ref().expect("hal");
            let wgpu_surface = match unsafe { hal.instance.create_surface_unsafe(make_target()) } {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("llimphi-layer · sin gpu: {e}");
                    return;
                }
            };
            match RawSurface::from_surface(hal, wgpu_surface, w, h) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("llimphi-layer · sin gpu: {e}");
                    return;
                }
            }
        };
        let hal = self.hal.as_ref().expect("hal");
        let renderer = Renderer::new(hal).expect("renderer");
        self.gpu = Some(PanelGpu {
            surface,
            renderer,
            typesetter: Typesetter::new(),
            scene: vello::Scene::new(),
            layout: LayoutTree::new(),
        });
    }

    /// Mantiene vivo el latido: pide el siguiente frame-callback. El loop de
    /// frames se auto-sostiene así (cada `frame` re-pide el próximo), lo que
    /// permite drenar el canal de la app por frame sin un timer aparte.
    fn latido(&self, qh: &QueueHandle<Self>) {
        let surface = self.layer.wl_surface();
        surface.frame(qh, surface.clone());
        surface.commit();
    }

    /// Avanza un frame: drena el canal, construye la vista, layout, pinta y
    /// presenta. Cachea el árbol para el hit-test del próximo evento.
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        // Drenar los Msg que la app empujó al canal (ticks/async/follow-ups).
        while let Ok(m) = self.rx.try_recv() {
            self.apply(m);
        }
        self.ensure_gpu();

        let (w, h) = (self.width, self.height);
        let view = A::view(self.model.as_ref().expect("model"));
        let hover_idx = self.hover_idx;
        let hal = match self.hal.as_ref() {
            Some(h) => h,
            None => {
                self.latido(qh);
                return;
            }
        };
        let gpu = match self.gpu.as_mut() {
            Some(g) => g,
            None => {
                self.latido(qh);
                return;
            }
        };
        gpu.surface.resize(w, h);
        let frame = match gpu.surface.acquire() {
            Ok(f) => f,
            Err(_) => {
                self.latido(qh);
                return;
            }
        };
        gpu.layout.clear();
        let mounted = mount(&mut gpu.layout, view);
        let computed = {
            let ts = &mut gpu.typesetter;
            let tmap = &mounted.text_measures;
            gpu.layout
                .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                    match tmap.get(&nid) {
                        Some(tm) => measure_text_node(ts, tm, known, avail),
                        None => taffy::Size::ZERO,
                    }
                })
                .expect("layout")
        };
        gpu.scene.reset();
        paint(&mut gpu.scene, &mounted, &computed, &mut gpu.typesetter, hover_idx, None);
        if let Err(e) = gpu.renderer.render(hal, &gpu.scene, &frame, palette::css::TRANSPARENT) {
            eprintln!("llimphi-layer · render: {e}");
        }
        gpu.surface.present(frame, hal);

        self.cache = Some(RenderCache { mounted, computed });
        self.latido(qh);
    }
}

// ── Handlers sctk ───────────────────────────────────────────────────────────

impl<A: App> CompositorHandler for Runner<A> {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
        self.draw(qh);
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl<A: App> LayerShellHandler for Runner<A> {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (cw, ch) = configure.new_size;
        const MAX_DIM: u32 = 16384;
        if (1..=MAX_DIM).contains(&cw) {
            self.width = cw;
        }
        if (1..=MAX_DIM).contains(&ch) {
            self.height = ch;
        }
        self.draw(qh);
    }
}

impl<A: App> OutputHandler for Runner<A> {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl<A: App> SeatHandler for Runner<A> {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.seat.is_none() {
            self.seat = Some(seat);
        }
    }
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard if self.keyboard.is_none() => {
                if let Ok(kbd) = self.seat_state.get_keyboard(qh, &seat, None) {
                    self.keyboard = Some(kbd);
                }
            }
            Capability::Pointer if self.pointer.is_none() => {
                if let Ok(ptr) = self.seat_state.get_pointer(qh, &seat) {
                    self.pointer = Some(ptr);
                }
            }
            _ => {}
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Keyboard => {
                if let Some(k) = self.keyboard.take() {
                    k.release();
                }
            }
            Capability::Pointer => {
                if let Some(p) = self.pointer.take() {
                    p.release();
                }
            }
            _ => {}
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl<A: App> KeyboardHandler for Runner<A> {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: smithay_client_toolkit::seat::keyboard::KeyEvent,
    ) {
        if let Some(ke) = self.keysym_to_keyevent(&event) {
            let msg = A::on_key(self.model.as_ref().expect("model"), &ke);
            if let Some(msg) = msg {
                self.apply(msg);
            }
        }
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: smithay_client_toolkit::seat::keyboard::KeyEvent,
    ) {
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: u32,
    ) {
        self.mods = modifiers;
    }
}

impl<A: App> PointerHandler for Runner<A> {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for e in events {
            match e.kind {
                PointerEventKind::Motion { .. } => {
                    let (px, py) = (e.position.0 as f32, e.position.1 as f32);
                    // Drag en curso: el delta va al handler del nodo.
                    if let Some((handler, last)) = self.drag.as_ref().map(|(h, l)| (h.clone(), *l)) {
                        if let Some(d) = self.drag.as_mut() {
                            d.1 = (px, py);
                        }
                        if let Some(msg) = (handler)(DragPhase::Move, px - last.0, py - last.1) {
                            self.apply(msg);
                        }
                        continue;
                    }
                    let nuevo = self
                        .cache
                        .as_ref()
                        .and_then(|c| hit_test_hover(&c.mounted, &c.computed, px, py));
                    if self.hover_idx != nuevo {
                        self.hover_idx = nuevo;
                    }
                    continue;
                }
                PointerEventKind::Leave { .. } => {
                    self.hover_idx = None;
                    continue;
                }
                _ => {}
            }
            if let PointerEventKind::Axis { vertical, .. } = e.kind {
                let dy = if vertical.discrete != 0 {
                    vertical.discrete as f32
                } else {
                    vertical.absolute as f32 / 20.0
                };
                if dy != 0.0 {
                    let (px, py) = (e.position.0 as f32, e.position.1 as f32);
                    let msg = self.cache.as_ref().and_then(|c| {
                        hit_test_scroll(&c.mounted, &c.computed, px, py)
                            .and_then(|i| c.mounted.nodes.get(i))
                            .and_then(|n| n.on_scroll.as_ref().and_then(|h| h(0.0, dy)))
                    });
                    if let Some(msg) = msg {
                        self.apply(msg);
                    }
                }
                continue;
            }
            if let PointerEventKind::Release { button, .. } = e.kind {
                if button == BTN_LEFT {
                    if let Some((handler, _)) = self.drag.take() {
                        if let Some(msg) = (handler)(DragPhase::End, 0.0, 0.0) {
                            self.apply(msg);
                        }
                    }
                }
                continue;
            }
            if let PointerEventKind::Press { button, .. } = e.kind {
                if button != BTN_LEFT && button != BTN_RIGHT {
                    continue;
                }
                let (px, py) = (e.position.0 as f32, e.position.1 as f32);
                let derecho = button == BTN_RIGHT;
                // Nodo arrastrable bajo el press izquierdo: arranca un drag.
                if !derecho {
                    let handler = self.cache.as_ref().and_then(|c| {
                        let i = hit_test_click(&c.mounted, &c.computed, px, py)?;
                        c.mounted.nodes.get(i)?.drag.clone()
                    });
                    if let Some(handler) = handler {
                        self.drag = Some((handler, (px, py)));
                        continue;
                    }
                }
                let msg = self.cache.as_ref().and_then(|c| {
                    let i = hit_test_click(&c.mounted, &c.computed, px, py)?;
                    let n = c.mounted.nodes.get(i)?;
                    if derecho {
                        if let Some(m) = n.on_right_click.clone() {
                            return Some(m);
                        }
                        let at = n.on_right_click_at.as_ref()?;
                        let r = c.computed.get(n.id)?;
                        at(px - r.x, py - r.y, r.w, r.h)
                    } else {
                        if let Some(m) = n.on_click.clone() {
                            return Some(m);
                        }
                        let at = n.on_click_at.as_ref()?;
                        let r = c.computed.get(n.id)?;
                        at(px - r.x, py - r.y, r.w, r.h)
                    }
                });
                if let Some(msg) = msg {
                    self.apply(msg);
                }
            }
        }
    }
}

impl<A: App> ProvidesRegistryState for Runner<A> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(@<A: App> Runner<A>);
delegate_output!(@<A: App> Runner<A>);
delegate_layer!(@<A: App> Runner<A>);
delegate_seat!(@<A: App> Runner<A>);
delegate_keyboard!(@<A: App> Runner<A>);
delegate_pointer!(@<A: App> Runner<A>);
delegate_registry!(@<A: App> Runner<A>);
