// Infraestructura de arranque del compositor.
use crate::*;
use std::sync::Arc;
use smithay::reexports::wayland_server::Display;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::dmabuf::DmabufState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::selection::wlr_data_control::DataControlState;
use smithay::wayland::pointer_constraints::PointerConstraintsState;
use smithay::wayland::relative_pointer::RelativePointerManagerState;
use smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState;
use smithay::wayland::foreign_toplevel_list::ForeignToplevelListState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shm::ShmState;
use smithay::input::{Seat, SeatState};
use smithay::backend::egl::EGLDevice;
use smithay::wayland::dmabuf::{DmabufFeedbackBuilder, DmabufHandler};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportDma;
use mirada_brain::{Keymap, Rules, CtlServer};
use mirada_body::BodyState;
use mirada_link::BodyLink;

/// Carga las reglas de ventana del usuario, o ninguna si no hay archivo.
pub(crate) fn load_user_rules() -> Rules {
    match Rules::default_path() {
        Some(p) => Rules::load_or_default(&p),
        None => Rules::default(),
    }
}

/// Carga los permisos de capacidad del usuario (`~/.config/mirada/caps.ron`),
/// o ninguno (todo permitido) si no hay archivo.
pub(crate) fn load_user_caps() -> mirada_brain::Permisos {
    match mirada_brain::permisos::default_path() {
        Some(p) => mirada_brain::permisos::load_or_default(&p),
        None => mirada_brain::Permisos::default(),
    }
}

/// Carga la config general del WM (`~/.config/mirada/config.ron`), o los
/// valores por defecto si no hay archivo.
pub(crate) fn load_user_config() -> mirada_brain::Config {
    match mirada_brain::Config::default_path() {
        Some(p) => mirada_brain::Config::load_or_default(&p),
        None => mirada_brain::Config::default(),
    }
}

/// Arma un Cerebro embebido: un `Desktop` con el keymap del usuario y
/// sus reglas de ventana. Lo usan tanto el modo autónomo como el modo
/// greeter (el DM es siempre autónomo — un Cerebro externo no tiene
/// sentido en la pantalla de login).
pub(crate) fn embedded_brain(keymap_path: &Option<std::path::PathBuf>) -> Brain {
    let keymap = match keymap_path {
        Some(p) => Keymap::load_or_init(p),
        None => Keymap::default(),
    };
    let mut desktop = Desktop::with_keymap(keymap);
    desktop.set_config(load_user_config());
    desktop.set_rules(load_user_rules());
    let _ = desktop.set_caps(load_user_caps());
    Brain::Embedded(desktop)
}

/// Crea y anuncia un `wl_output` (un monitor) en el protocolo Wayland —
/// muchos clientes (`foot` entre ellos) se niegan a arrancar sin uno.
/// Devuelve el [`Output`](smithay::output::Output); hay que mantenerlo
/// vivo mientras el compositor corra.
pub(crate) fn announce_output(
    dh: &DisplayHandle,
    name: &str,
    width: i32,
    height: i32,
    refresh_mhz: i32,
    scale_120: u32,
    transform: Transform,
) -> smithay::output::Output {
    use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
    let output = Output::new(
        name.to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "mirada".into(),
            model: name.to_string(),
        },
    );
    output.create_global::<App>(dh);
    let mode = Mode { size: (width, height).into(), refresh: refresh_mhz };
    let scale = scale_to_smithay(scale_120);
    output.change_current_state(Some(mode), Some(transform), Some(scale), Some((0, 0).into()));
    output.set_preferred(mode);
    output
}

/// Convierte una escala en 120-avos (convención `wp_fractional_scale`:
/// `120` = 100 %) al [`smithay::output::Scale`] correspondiente. Múltiplos
/// exactos de `120` se mapean a `Scale::Integer` (1×, 2×, 3×, …) — el
/// camino rápido del compositor cuando el cliente soporta sólo escalas
/// enteras; el resto cae a `Fractional`.
pub fn scale_to_smithay(scale_120: u32) -> smithay::output::Scale {
    use smithay::output::Scale;
    let s = if scale_120 > 0 { scale_120 } else { 120 };
    if s % 120 == 0 {
        Scale::Integer((s / 120) as i32)
    } else {
        Scale::Fractional(s as f64 / 120.0)
    }
}

/// Parsea un slug de transformación (ver `mirada_brain::TRANSFORM_SLUGS`)
/// al [`Transform`] de smithay. Slug vacío o desconocido cae a `Normal`
/// (la validación dura se hace al cargar la config en `Config::from_ron`).
pub fn transform_from_slug(slug: &str) -> Transform {
    match slug {
        "90" => Transform::_90,
        "180" => Transform::_180,
        "270" => Transform::_270,
        "flipped" => Transform::Flipped,
        "flipped-90" => Transform::Flipped90,
        "flipped-180" => Transform::Flipped180,
        "flipped-270" => Transform::Flipped270,
        _ => Transform::Normal,
    }
}

/// `true` si el cliente puede bindear `zwp_linux_dmabuf`, según la denylist de
/// permisos por **ejecutable real** del cliente (`SO_PEERCRED → /proc/<pid>/exe`,
/// no el `app_id` falsificable). Sin PID identificable → se permite (no romper
/// apps con GPU legítimas). Mismo patrón que el resto de capacidades gateadas.
pub(crate) fn dmabuf_permitido_para(
    caps: &std::sync::Arc<std::sync::RwLock<mirada_brain::Permisos>>,
    client: &smithay::reexports::wayland_server::Client,
) -> bool {
    let pid = client.get_data::<ClientState>().and_then(|s| s.pid);
    match pid.and_then(exe_de_pid) {
        Some(exe) => leer_tolerante(caps).dmabuf_permitido(&exe),
        None => true,
    }
}

/// Anuncia el global `zwp_linux_dmabuf` con los formatos que el
/// `GlesRenderer` admite. Hay que llamarlo una vez creado el renderer
/// (no antes: los formatos salen de él) — así las apps que pintan por
/// GPU (GPUI, navegadores acelerados) pueden ser clientes del compositor.
///
/// El global se anuncia **filtrado por permisos** (`dmabuf_denylist`): un
/// ejecutable denegado no ve el global y cae al camino `wl_shm` por software,
/// igual que clipboard/virtual-keyboard/foreign-toplevel/screencopy.
pub(crate) fn announce_dmabuf(app: &mut App, dh: &DisplayHandle, renderer: &GlesRenderer) {
    let formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();
    // Nodo de render del adaptador del renderer — necesario para armar el
    // *default feedback* de dmabuf v4.
    let render_node = EGLDevice::device_for_display(renderer.egl_context().display())
        .ok()
        .and_then(|dev| dev.try_get_render_node().ok().flatten());
    let feedback = render_node.and_then(|node| {
        DmabufFeedbackBuilder::new(node.dev_id(), formats.clone())
            .build()
            .ok()
    });
    match feedback {
        // dmabuf **v4 con default feedback**. La WSI Vulkan de Mesa lo EXIGE para
        // determinar el dispositivo y los formatos presentables: con sólo el
        // global v3 (sin feedback) los clientes wgpu/Vulkan ven **0 formatos** y
        // no pueden crear swapchain (era el bug de `pata` por layer-shell, que
        // caía a winit y paniqueaba). EGL/GL y los búferes shm (waybar) andaban
        // con v3; el path Vulkan WSI no. Clientes que se bindean a v3 siguen
        // recibiendo los formatos de la tranche principal.
        Some(feedback) => {
            let caps = app.caps.clone();
            app.dmabuf_state
                .create_global_with_filter_and_default_feedback::<App, _>(
                    dh,
                    &feedback,
                    move |client| dmabuf_permitido_para(&caps, client),
                );
            println!(
                "mirada-compositor · dmabuf v4 (feedback): {} format(s) anunciado(s).",
                formats.len()
            );
        }
        // Sin nodo de render no se puede armar feedback: caemos al global v3.
        None => {
            let n = formats.len();
            let caps = app.caps.clone();
            app.dmabuf_state.create_global_with_filter::<App, _>(
                dh,
                formats,
                move |client| dmabuf_permitido_para(&caps, client),
            );
            eprintln!(
                "mirada-compositor · dmabuf v3 sin feedback ({n} fmt) — sin nodo de render; \
                 los clientes Vulkan podrían ver 0 formatos."
            );
        }
    }
}

/// Vigías de los tres archivos de config recargables en caliente (keymap,
/// config, reglas). Cada uno es `(ruta, vigía)` o `None` si no aplica
/// (Cerebro enlazado, modo greeter o fallo al armar el watcher). Un solo
/// [`poll`](ConfigWatches::poll) atiende los tres — sin duplicar la lógica
/// entre el backend winit y el DRM.
#[derive(Default)]
pub(crate) struct ConfigWatches {
    pub(crate) keymap: Option<(std::path::PathBuf, mirada_brain::FileWatch)>,
    pub(crate) config: Option<(std::path::PathBuf, mirada_brain::FileWatch)>,
    pub(crate) rules: Option<(std::path::PathBuf, mirada_brain::FileWatch)>,
}

impl ConfigWatches {
    /// Recarga lo que haya cambiado en disco. Llamar una vez por iteración
    /// del bucle de eventos de cada backend. Devuelve `true` si la **config**
    /// general (`config.ron`) cambió — el backend DRM lo usa para refrescar sus
    /// cachés derivadas de config (menú, wallpaper, fuente).
    pub(crate) fn poll(&self, app: &mut App) -> bool {
        if let Some((p, w)) = &self.keymap {
            if w.changed() {
                app.reload_keymap_from(p);
            }
        }
        let mut config_changed = false;
        if let Some((p, w)) = &self.config {
            if w.changed() {
                app.reload_config_from(p);
                config_changed = true;
            }
        }
        if let Some((p, w)) = &self.rules {
            if w.changed() {
                app.reload_rules_from(p);
            }
        }
        config_changed
    }
}

/// Lo que comparten los dos backends gráficos: el `Display` de Wayland,
/// el `App` ya armado y la maquinaria de recarga en caliente y control.
pub(crate) struct Setup {
    pub(crate) display: Display<App>,
    pub(crate) app: App,
    pub(crate) watches: ConfigWatches,
    pub(crate) ctl: Option<CtlServer>,
}

/// Arma el estado del compositor — todo lo independiente del backend
/// gráfico (Wayland, Cerebro, teclado, keymap, control). Cada backend
/// (winit o DRM) registra luego su propia salida y monta su bucle.
pub(crate) fn build_app(greeter: bool) -> Result<Setup, Box<dyn std::error::Error>> {
    let display: Display<App> = Display::new()?;
    let dh = display.handle();

    let mut seat_state = SeatState::new();
    let seat = seat_state.new_wl_seat(&dh, "mirada");

    // Anuncia el gestor de decoración: las ventanas van sin marco (ver
    // `XdgDecorationHandler`). El `XdgDecorationState` sólo serviría para
    // retirar el global más tarde, cosa que nunca hacemos.
    let _ = XdgDecorationState::new::<App>(&dh);

    // El keymap del usuario (`~/.config/mirada/keymap.ron`). Sólo lo usa
    // el Cerebro embebido; con un Cerebro enlazado, el keymap es asunto suyo.
    let keymap_path = Keymap::default_path();

    // Elige el Cerebro. El modo greeter (DM) fuerza Cerebro embebido;
    // si no, enlazado cuando `MIRADA_SOCKET` está puesto, autónomo si no.
    let brain = if greeter {
        println!("mirada-compositor · modo greeter (DM) — Cerebro embebido.");
        embedded_brain(&keymap_path)
    } else {
        match std::env::var("MIRADA_SOCKET") {
            Ok(path) => {
                println!("mirada-compositor · esperando al Cerebro en {path} …");
                let link = BodyLink::listen(&path)?;
                println!("mirada-compositor · Cerebro conectado.");
                Brain::Linked(link)
            }
            Err(_) => {
                println!("mirada-compositor · modo autónomo (Cerebro embebido).");
                embedded_brain(&keymap_path)
            }
        }
    };

    // Los permisos de capacidad: un Arc compartido entre el `App` (que lo
    // reemplaza cuando el Cerebro recarga la política) y el filtro del global
    // `zwlr_data_control`, que decide qué clientes lo ven. Arranca permitiendo a
    // todos; `apply_commands` lo siembra con la política del usuario en cuanto
    // el arranque emite `SetCapabilities`.
    let caps: Arc<std::sync::RwLock<mirada_brain::Permisos>> = Arc::default();
    let caps_filter = caps.clone();
    let caps_vk_filter = caps.clone();
    let caps_ftl_filter = caps.clone();
    let caps_ftm_filter = caps.clone();
    let caps_sc_filter = caps.clone();

    // TODO idle-notify (ext_idle_notify_v1) + DPMS por inactividad.
    // No se cablea acá porque `IdleNotifierState<App>` exige un
    // `calloop::LoopHandle<'static, App>` en su constructor (lo usa para sus
    // timers internos de idled/resumed), pero el bucle de eventos del backend
    // DRM despacha `DrmState` (`EventLoop<DrmState>`), no `App`, y el backend
    // `winit` no usa calloop en absoluto. Integrarlo requiere unificar el tipo
    // de estado del bucle (que el `EventLoop` despache el mismo tipo que
    // implementa los handlers Wayland) — un refactor del bucle, no un parche
    // local. El punto de apagado físico de la pantalla (DPMS real sobre DRM)
    // iría en el handler del timer de inactividad, sobre `DrmState::outputs`.
    let mut app = App {
        compositor_state: CompositorState::new::<App>(&dh),
        xdg_shell_state: XdgShellState::new::<App>(&dh),
        popups: smithay::desktop::PopupManager::default(),
        layer_shell_state: WlrLayerShellState::new::<App>(&dh),
        output_manager_state: OutputManagerState::new_with_xdg_output::<App>(&dh),
        output: None,
        outputs: Vec::new(),
        output_ids: Vec::new(),
        shm_state: ShmState::new::<App>(&dh, Vec::new()),
        dmabuf_state: DmabufState::new(),
        seat_state,
        data_device_state: DataDeviceState::new::<App>(&dh),
        // Selección primaria (middle-click paste): protocolo abierto a todos los
        // clientes, igual que `wl_data_device` (no es una capacidad sensible).
        primary_selection_state: PrimarySelectionState::new::<App>(&dh),
        // Restricciones de puntero y movimiento relativo: globales abiertos a
        // todos (los necesitan juegos/apps 3D; el lock sólo se activa cuando la
        // superficie restringida tiene el foco del puntero).
        _pointer_constraints_state: PointerConstraintsState::new::<App>(&dh),
        _relative_pointer_state: RelativePointerManagerState::new::<App>(&dh),
        led_state: smithay::input::keyboard::LedState::default(),
        // `zwlr_data_control` (snoop de portapapeles): el filtro consulta los
        // permisos por el **ejecutable real** del cliente (de su PID por
        // `SO_PEERCRED`). Sin PID identificable → se permite (no romper).
        data_control_state: DataControlState::new::<App, _>(&dh, None, move |client| {
            let pid = client.get_data::<ClientState>().and_then(|s| s.pid);
            match pid.and_then(exe_de_pid) {
                Some(exe) => leer_tolerante(&caps_filter).clipboard_permitido(&exe),
                None => true,
            }
        }),
        // `zwp_virtual_keyboard` (inyección de pulsaciones): mismo filtro por
        // **ejecutable real** del cliente. Sin PID identificable → se permite
        // (no romper teclados en pantalla / automatización legítima).
        _virtual_keyboard_state: VirtualKeyboardManagerState::new::<App, _>(&dh, move |client| {
            let pid = client.get_data::<ClientState>().and_then(|s| s.pid);
            match pid.and_then(exe_de_pid) {
                Some(exe) => leer_tolerante(&caps_vk_filter).virtual_input_permitido(&exe),
                None => true,
            }
        }),
        // `ext_foreign_toplevel_list` (censo de ventanas): mismo filtro por
        // **ejecutable real** del cliente. Sin PID identificable → se permite
        // (no romper taskbars/docks legítimos).
        foreign_toplevel_state: ForeignToplevelListState::new_with_filter::<App>(
            &dh,
            move |client| {
                let pid = client.get_data::<ClientState>().and_then(|s| s.pid);
                match pid.and_then(exe_de_pid) {
                    Some(exe) => leer_tolerante(&caps_ftl_filter).window_list_permitido(&exe),
                    None => true,
                }
            },
        ),
        dh: dh.clone(),
        // `zwlr_foreign_toplevel_management` (taskbar: listar/activar/cerrar
        // ventanas): mismo filtro por **ejecutable real** que el censo ext.
        foreign_toplevel_manager: crate::foreign_toplevel::ForeignToplevelManagerState::new(
            &dh,
            move |client| {
                let pid = client.get_data::<ClientState>().and_then(|s| s.pid);
                match pid.and_then(exe_de_pid) {
                    Some(exe) => leer_tolerante(&caps_ftm_filter).window_list_permitido(&exe),
                    None => true,
                }
            },
        ),
        // `zwlr_screencopy` (captura de pantalla — la más sensible): mismo
        // filtro por **ejecutable real** del cliente. Sin PID identificable →
        // se permite (no romper herramientas de screenshot legítimas).
        _screencopy_state: screencopy::ScreencopyState::new(&dh, move |client| {
            let pid = client.get_data::<ClientState>().and_then(|s| s.pid);
            match pid.and_then(exe_de_pid) {
                Some(exe) => leer_tolerante(&caps_sc_filter).screencopy_permitido(&exe),
                None => true,
            }
        }),
        pending_screencopy: Vec::new(),
        seat,
        keyboard: None,
        pending_kb_focus: None,
        pointer: None,
        pointer_loc: (0.0, 0.0),
        cursor_status: CursorImageStatus::default_named(),
        drag: None,
        dnd_paths: None,
        output_size: (0, 0),
        // Con autohide, el dock arranca oculto (se revela al tocar el borde).
        shell_hidden: shell_dock().autohide,
        reserved: (0, 0, 0, 0),
        windows: Vec::new(),
        body: BodyState::new(),
        brain,
        mode: if greeter { BodyMode::Greeter } else { BodyMode::Session },
        session_user: None,
        session_env: Vec::new(),
        grabs: Vec::new(),
        debug_keys: std::env::var_os("MIRADA_DEBUG_KEYS").is_some(),
        switcher: None,
        switcher_step: None,
        switcher_cancel: false,
        overview_open: false,
        overview_closing: false,
        overview_via_wintab: false,
        overview_selected: 0,
        linked_ws: None,
        decorations: mirada_brain::Decorations::default(),
        ssd_surfaces: std::collections::HashSet::new(),
        caps,
        pending_keybind: None,
        pending_vt: None,
        pending_session: None,
        next_id: 1,
        running: true,
        greeter_stdin: None,
        greeter_active_output: usize::MAX,
    };

    // Distribución de teclado de la config del usuario (vacío = la del
    // sistema). `ucfg` vive hasta el final de la función; XkbConfig sólo la
    // toma prestada para compilar el keymap dentro de `add_keyboard`.
    let ucfg = load_user_config();
    let xkb = smithay::input::keyboard::XkbConfig {
        layout: &ucfg.xkb_layout,
        variant: &ucfg.xkb_variant,
        ..Default::default()
    };
    let keyboard = app.seat.add_keyboard(xkb, 200, 25)?;
    app.keyboard = Some(keyboard);
    app.pointer = Some(app.seat.add_pointer());

    // En modo embebido, el propio Desktop dicta los atajos a
    // interceptar — salvo en modo greeter: en la pantalla de login
    // todas las teclas van al greeter (que el usuario no pueda lanzar
    // nada ni cerrar el compositor). Los atajos se registran luego, en
    // el traspaso a la sesión (`complete_greeter_handoff`).
    if !greeter {
        if let Brain::Embedded(desktop) = &app.brain {
            let cmds = vec![desktop.grab_keys(), desktop.decorations(), desktop.capabilities()];
            app.apply_commands(cmds);
        }
    }

    // Vigilancia de los archivos de config (keymap, config, reglas) para
    // recargarlos en caliente — sólo con el Cerebro embebido y fuera del
    // modo greeter (donde no hay nada registrado que recargar). Cada vigía
    // empareja la ruta con su `FileWatch`; un fallo al armarlo deja `None`.
    let watches = if matches!(app.brain, Brain::Embedded(_)) && !greeter {
        let watch_pair = |p: &Option<std::path::PathBuf>| {
            p.as_ref()
                .and_then(|p| mirada_brain::FileWatch::new(p).ok().map(|w| (p.clone(), w)))
        };
        let w = ConfigWatches {
            keymap: watch_pair(&keymap_path),
            config: watch_pair(&mirada_brain::Config::default_path()),
            rules: watch_pair(&Rules::default_path()),
        };
        let n = [&w.keymap, &w.config, &w.rules].iter().filter(|x| x.is_some()).count();
        if n > 0 {
            println!("mirada-compositor · vigilando {n} archivo(s) de config (recarga en caliente).");
        }
        w
    } else {
        ConfigWatches::default()
    };

    // API de control (mirada-ctl) — sólo con el Cerebro embebido; si es
    // externo, el socket de control lo abre él.
    let ctl = match &app.brain {
        Brain::Embedded(_) => {
            let path = mirada_brain::ctl::default_socket_path();
            match CtlServer::bind(&path) {
                Ok(s) => {
                    println!("mirada-compositor · API de control en {}", path.display());
                    Some(s)
                }
                Err(e) => {
                    eprintln!("mirada-compositor · sin API de control: {e}");
                    None
                }
            }
        }
        Brain::Linked(_) => None,
    };

    Ok(Setup { display, app, watches, ctl })
}
