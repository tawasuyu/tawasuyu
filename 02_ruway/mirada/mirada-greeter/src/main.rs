//! `mirada-greeter` — el greeter del escritorio mirada.
//!
//! Ventana Llimphi de login. El compositor (`mirada-compositor`) la arranca
//! como proceso hijo cuando bootea en modo greeter, la compone a pantalla
//! completa (la reconoce por `app_id = "mirada.greeter"`) y le lee el stdout.
//!
//! Flujo: el usuario teclea usuario + contraseña, el greeter autentica con
//! [`auth_core`], y en éxito **imprime un [`SessionTicket`] a stdout** y
//! termina. El compositor parsea esa línea, hace el traspaso a modo sesión
//! (setuid al usuario + arranque) sin reiniciar el servidor gráfico — la
//! «mutación atómica» del DM.
//!
//! Backend de autenticación (ver [`pick_authenticator`]):
//! - por defecto, PAM contra el servicio `mirada`;
//! - `MIRADA_GREETER_MOCK="usuario:secreto"` usa el mock, para iterar la UI
//!   en cajas sin PAM o con el greeter anidado en otro escritorio.

mod arje_session;
mod aurora;
mod bg;
mod fire;
mod lightning;
mod plasma;
mod rain;
mod sessions;
mod stars;
mod state;
mod waves;

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use auth_core::{
    AuthError, Authenticator, MockAuthenticator, PamAuthenticator, SessionTicket, ShellAction,
    UserInfo, DEFAULT_SERVICE,
};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Position, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_motion::{animate, motion, Tween};
use llimphi_clipboard::SystemClipboard;

/// `app_id` con el que el compositor reconoce y compone el greeter. El mismo
/// en modo login y en modo lock: el compositor lo compone a pantalla completa
/// y le rutea el input en ambos casos (lo distingue por `app_id`, no por modo).
const GREETER_APP_ID: &str = "mirada.greeter";

/// Autenticador compartible entre el hilo de UI y el de fondo.
type DynAuth = Arc<dyn Authenticator + Send + Sync>;

/// En qué papel corre esta instancia del shell de credenciales.
///
/// La misma pantalla (tarjeta, fondos, reloj) sirve para las dos: el modo sólo
/// cambia qué se pide y qué se emite al compositor.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GreeterMode {
    /// Greeter de arranque: pide usuario + contraseña, elige sesión y emite un
    /// [`ShellAction::StartSession`].
    Login,
    /// Lock de la sesión activa: el usuario está fijo (el dueño de la sesión),
    /// no se elige sesión, y al validar emite [`ShellAction::Unlock`].
    Lock,
}

/// Lee el modo de los argumentos: `--lock` ⇒ [`GreeterMode::Lock`], si no
/// [`GreeterMode::Login`]. Se relee en `init` (la fábrica del bucle Elm no
/// recibe args), barato y sin estado global.
fn mode_from_args() -> GreeterMode {
    if std::env::args().any(|a| a == "--lock") {
        GreeterMode::Lock
    } else {
        GreeterMode::Login
    }
}

/// Usuario al que pertenece la sesión bloqueada. El compositor lo pasa por
/// `MIRADA_LOCK_USER`; en pruebas a mano cae a `$USER`. Vacío si no hay ninguno
/// (el lock entonces deja teclear el nombre, como el login).
fn lock_user() -> String {
    std::env::var("MIRADA_LOCK_USER")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_default()
}

fn main() {
    if std::env::var_os("LLIMPHI_TIMING").is_some() {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        eprintln!("[llimphi-timing] greeter:main epoch_ms={ms}");
    }
    rimay_localize::init();
    // Carga el idioma persistido en wawa-config (sobrescribe el default "es-PE").
    let _ = rimay_localize::set_locale(&wawa_config::WawaConfig::load().lang);
    // Captura headless de la UI a PNG (sin bootear el DM): `--shot <png> [W] [H]`.
    // `--lock` lo hace en modo lock (usuario fijo, botón Desbloquear).
    let args: Vec<String> = std::env::args().collect();
    let mode = mode_from_args();
    if let Some(i) = args.iter().position(|a| a == "--shot") {
        let out = args.get(i + 1).cloned().unwrap_or_else(|| "greeter.png".to_string());
        let w: u32 = args.get(i + 2).and_then(|v| v.parse().ok()).unwrap_or(1600);
        let h: u32 = args.get(i + 3).and_then(|v| v.parse().ok()).unwrap_or(900);
        shot_greeter(&out, w, h, mode);
        return;
    }
    llimphi_ui::run::<Greeter>();
}

/// Monitores simulados para `--shot` desde `MIRADA_SHOT_MONITORS`
/// (`"<activo> x,y,w,h x,y,w,h …"`). Vacío ⇒ un solo monitor (tarjeta
/// centrada en toda la ventana).
fn shot_monitors() -> (Vec<MonRect>, usize) {
    std::env::var("MIRADA_SHOT_MONITORS")
        .ok()
        .and_then(|v| parse_layout(&format!("LAYOUT {v}")))
        .unwrap_or((Vec::new(), 0))
}

/// Construye un modelo de muestra (usuario+contraseña tecleados) y vuelca
/// `Greeter::view` a un PNG, para revisar el layout del login sin loguearse.
fn shot_greeter(out: &str, w: u32, h: u32, mode: GreeterMode) {
    use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
    use llimphi_ui::llimphi_hal::{wgpu, Hal};
    use llimphi_ui::llimphi_layout::{taffy, LayoutTree};
    use llimphi_ui::llimphi_raster::peniko::Color;
    use llimphi_ui::llimphi_raster::{vello, Renderer};
    use llimphi_ui::llimphi_text::Typesetter;

    let saved = state::GreeterState::load();
    let sessions = sessions::discover();
    let mut user = TextInputState::new();
    let user_text = match mode {
        // En lock el usuario está fijo (dueño de la sesión); el shot lo muestra.
        GreeterMode::Lock => {
            let u = lock_user();
            if u.is_empty() { "sergio".to_string() } else { u }
        }
        GreeterMode::Login if saved.last_user.is_empty() => "usuario".to_string(),
        GreeterMode::Login => saved.last_user,
    };
    user.set_text(user_text);
    let mut pass = TextInputState::masked();
    pass.set_text("secreto".to_string());
    // `--shot` puede simular el roster FUS para certificar el selector del lock
    // sin bootear: MIRADA_SHOT_HOSTED="1 0:ana 1:beto" (activo + id:nombre…).
    let (shot_hosted, shot_hosted_active) = std::env::var("MIRADA_SHOT_HOSTED")
        .ok()
        .and_then(|v| parse_sessions(&format!("SESSIONS {v}")))
        .unwrap_or_default();
    let model = Model {
        auth: pick_authenticator(),
        user,
        pass,
        focus: Field::Pass,
        mode,
        status: Status::Idle,
        sessions,
        session_idx: 0,
        clipboard: SystemClipboard::new(),
        // `MIRADA_SHOT_MENU=<idx>` abre ese menú raíz para certificar que el
        // dropdown cae sobre el monitor activo.
        menu_open: std::env::var("MIRADA_SHOT_MENU").ok().and_then(|v| v.parse().ok()),
        edit_menu: None,
        menu_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        edit_active: usize::MAX,
        edit_anim: Tween::idle(1.0),
        // `--shot` puede simular multi-monitor y animación para certificar el
        // viaje de la tarjeta y los fondos sin bootear el DM:
        //   MIRADA_SHOT_MONITORS="1 0,0,1280,720 1280,0,1280,720"  (activo + rects)
        //   MIRADA_GREETER_RAIN=1 MIRADA_GREETER_BG=stars
        rain_enabled: saved.rain_enabled,
        rain_color: saved.rain_color,
        anim: saved.anim,
        rain_t: std::env::var("MIRADA_SHOT_T").ok().and_then(|v| v.parse().ok()).unwrap_or(3.2),
        monitors: shot_monitors().0,
        active_mon: shot_monitors().1,
        prev_mon: shot_monitors().1,
        card_anim: 1.0,
        hosted: shot_hosted,
        hosted_active: shot_hosted_active,
    };
    let view = <Greeter as App>::view(&model);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, view);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    let mut scene = vello::Scene::new();
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    // Si hay overlay (menú abierto), se compone encima en el mismo lienzo —
    // así el shot certifica la posición del dropdown en multi-monitor.
    if let Some(ov) = <Greeter as App>::view_overlay(&model) {
        let mut olayout = LayoutTree::new();
        let omounted = mount(&mut olayout, ov);
        let ocomputed = {
            let tmap = &omounted.text_measures;
            olayout
                .compute_with_measure(omounted.root, (w as f32, h as f32), |nid, known, avail| {
                    match tmap.get(&nid) {
                        Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                        None => taffy::Size::ZERO,
                    }
                })
                .expect("layout overlay")
        };
        paint(&mut scene, &omounted, &ocomputed, &mut ts, None, None);
    }

    let fmt = wgpu::TextureFormat::Rgba8Unorm;
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("greeter-shot"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: fmt,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let tview = target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render_to_view(&hal, &scene, &tview, w, h, Color::from_rgba8(18, 18, 24, 255))
        .expect("render_to_view");

    let unpadded = (w * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * h as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = std::fs::File::create(out).expect("png");
    let mut penc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    penc.set_color(png::ColorType::Rgba);
    penc.set_depth(png::BitDepth::Eight);
    let mut wr = penc.write_header().unwrap();
    wr.write_image_data(&pixels).unwrap();
    eprintln!("mirada-greeter: {out} ({w}x{h})");
}

/// Elige el backend de autenticación según el entorno.
fn pick_authenticator() -> DynAuth {
    // Modo dev: credenciales fijas, sin tocar PAM.
    if let Ok(spec) = std::env::var("MIRADA_GREETER_MOCK") {
        if let Some((user, secret)) = spec.split_once(':') {
            eprintln!("mirada-greeter · backend mock (usuario «{user}»)");
            return Arc::new(MockAuthenticator::new().with_user(user, secret));
        }
        eprintln!("mirada-greeter · MIRADA_GREETER_MOCK mal formado (falta «:»), ignorado");
    }
    // Camino real: PAM. Servicio sobreescribible con `MIRADA_GREETER_PAM`.
    let service =
        std::env::var("MIRADA_GREETER_PAM").unwrap_or_else(|_| DEFAULT_SERVICE.to_string());
    eprintln!("mirada-greeter · backend PAM (servicio «{service}»)");
    Arc::new(PamAuthenticator::new(service))
}

/// Imprime la acción del shell a stdout (la línea que el compositor escanea) y
/// fuerza el flush antes de terminar.
fn emit_action(action: &ShellAction) {
    println!("{}", action.to_line());
    let _ = std::io::stdout().flush();
}

// ---------------------------------------------------------------------
// Modelo + mensajes
// ---------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    User,
    Pass,
}

enum Status {
    Idle,
    Authenticating,
    Failed(String),
}

struct Model {
    auth: DynAuth,
    user: TextInputState,
    pass: TextInputState,
    focus: Field,
    status: Status,
    /// Sesiones de escritorio descubiertas en el sistema (la 0 es mirada).
    sessions: Vec<sessions::Session>,
    /// Índice de la sesión elegida dentro de `sessions`.
    session_idx: usize,
    /// Clipboard del sistema, compartido por el menú de edición.
    clipboard: SystemClipboard,
    /// Menú principal: índice del menú raíz abierto (`None` cerrado).
    menu_open: Option<usize>,
    /// Menú de edición contextual: ancla `(x, y)` en ventana (`None` cerrado).
    edit_menu: Option<(f32, f32)>,
    /// Fila resaltada por teclado en el menú principal (`usize::MAX` = ninguna).
    menu_active: usize,
    /// Animación de aparición/swap del dropdown principal.
    menu_anim: Tween<f32>,
    /// Fila resaltada por teclado en el menú de edición (`usize::MAX` = ninguna).
    edit_active: usize,
    /// Animación de aparición del menú de edición.
    edit_anim: Tween<f32>,
    /// ¿Pintar el fondo animado?
    rain_enabled: bool,
    /// Paleta del fondo.
    rain_color: state::RainColor,
    /// Qué animación de fondo pintar (enchufable, ver [`bg`]).
    anim: state::BgAnim,
    /// Reloj del fondo (segundos), avanzado por `Msg::RainTick`.
    rain_t: f32,
    /// Disposición de monitores (rects locales a la ventana, que cubre la unión
    /// de las salidas). Vacío ⇒ un solo monitor / desconocido: la tarjeta se
    /// centra en toda la ventana, como siempre. Lo empuja el compositor por
    /// stdin en modo DM multi-monitor.
    monitors: Vec<MonRect>,
    /// Índice del monitor con el ratón — adonde viaja la tarjeta de login.
    active_mon: usize,
    /// Monitor del que viene la tarjeta (origen de la animación de viaje).
    prev_mon: usize,
    /// Progreso 0→1 de la animación de viaje de la tarjeta entre monitores.
    /// `1.0` = asentada en `active_mon`.
    card_anim: f32,
    /// Papel de esta instancia: login de arranque o lock de la sesión activa.
    mode: GreeterMode,
    /// FUS: sesiones hosteadas por el compositor `(id, nombre)`, empujadas por
    /// stdin (`SESSIONS …`) sólo al lock. Si hay más de una, el lock pinta un
    /// selector «cambiar a» que emite [`ShellAction::SwitchTo`]. Vacío en login.
    hosted: Vec<(u32, String)>,
    /// Id de la sesión hosteada activa (la que está bloqueada) — no se ofrece
    /// como destino de salto a sí misma.
    hosted_active: u32,
}

/// Rect de un monitor en coordenadas de la ventana del greeter (px):
/// `(x, y, w, h)`.
type MonRect = (f32, f32, f32, f32);

#[derive(Clone)]
enum Msg {
    Focus(Field),
    /// Tecla a aplicar al campo focado (`TextInputState::apply_key`).
    EditKey(KeyEvent),
    Submit,
    AuthDone(Result<UserInfo, AuthError>),
    /// Avanza la sesión elegida (con wrap) — clic en el selector de la
    /// tarjeta.
    CycleSession(i32),
    /// FUS «cambiar usuario» desde el lock (F2): emite [`ShellAction::NewSession`]
    /// y cierra — el compositor abre un login nuevo para hostear otra sesión
    /// junto a la bloqueada.
    SwitchUser,
    /// FUS: roster de sesiones hosteadas `(id, nombre)` + id de la activa,
    /// empujado por el compositor (`SESSIONS …`). Alimenta el selector del lock.
    SetHosted(Vec<(u32, String)>, u32),
    /// FUS: saltar a la sesión hosteada `id` (clic en el selector del lock).
    /// Emite [`ShellAction::SwitchTo`] y cierra.
    SwitchTo(u32),
    /// Fija la sesión elegida por índice — elección desde el menú.
    PickSession(usize),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` = cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Right-click sobre la ventana → abre el menú de edición en `(x, y)`
    /// operando sobre el campo focuseado.
    EditMenuOpen(f32, f32),
    /// Acción elegida en el menú de edición.
    EditMenuAction(EditAction),
    /// Navegación ↑/↓ por la fila activa del menú principal.
    MenuNav(i32),
    /// Enter sobre la fila activa del menú principal.
    MenuActivate,
    /// Tick de animación de aparición/swap (re-render).
    MenuTick,
    /// Navegación ↑/↓ por la fila activa del menú de edición.
    EditNav(i32),
    /// Enter sobre la fila activa del menú de edición.
    EditActivate,
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Tick del fondo — avanza el reloj del fondo y la animación de viaje de
    /// la tarjeta, y repinta.
    RainTick,
    /// El compositor (modo DM) informa la disposición de monitores y cuál
    /// tiene el ratón: `(rects locales a la ventana, índice activo)`.
    SetLayout(Vec<MonRect>, usize),
    /// Elegir la animación de fondo (desde el menú «Fondo»).
    SetAnim(state::BgAnim),
}

// ---------------------------------------------------------------------
// Bucle Elm
// ---------------------------------------------------------------------

struct Greeter;

impl App for Greeter {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "mirada · greeter"
    }

    fn app_id() -> Option<&'static str> {
        Some(GREETER_APP_ID)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let saved = state::GreeterState::load();
        let sessions = sessions::discover();
        let mode = mode_from_args();

        // Prerellena el usuario y arranca el foco en la contraseña:
        //  · lock  → usuario fijo (dueño de la sesión), siempre foco en pass.
        //  · login → último usuario recordado; foco en pass si lo hay.
        let mut user = TextInputState::new();
        let focus = match mode {
            GreeterMode::Lock => {
                user.set_text(lock_user());
                Field::Pass
            }
            GreeterMode::Login => {
                if !saved.last_user.is_empty() {
                    user.set_text(saved.last_user.clone());
                    Field::Pass
                } else {
                    Field::User
                }
            }
        };

        // Restaura el último escritorio elegido buscándolo por nombre (los
        // índices no son estables entre arranques: las sesiones del sistema
        // pueden aparecer/desaparecer).
        let session_idx = sessions
            .iter()
            .position(|s| s.name == saved.last_session)
            .unwrap_or(0);

        // Reloj de animación (~30 fps): mueve el fondo y el viaje de la tarjeta
        // entre monitores. Siempre encendido — barato para una pantalla de
        // login, y el viaje de la tarjeta lo necesita aunque el fondo esté
        // apagado.
        handle.spawn_periodic(Duration::from_millis(33), || Msg::RainTick);

        // Hilo lector del stdin: el compositor (modo DM) empuja por acá la
        // disposición de monitores y cuál tiene el ratón. Cada línea `LAYOUT …`
        // reentra al bucle Elm como `Msg::SetLayout`. En modo dev (sin
        // compositor) el stdin queda mudo y el hilo simplemente espera.
        {
            let h = handle.clone();
            std::thread::spawn(move || {
                use std::io::BufRead;
                let stdin = std::io::stdin();
                for line in stdin.lock().lines().map_while(Result::ok) {
                    if let Some((mons, active)) = parse_layout(&line) {
                        h.dispatch(Msg::SetLayout(mons, active));
                    } else if let Some((hosted, active)) = parse_sessions(&line) {
                        h.dispatch(Msg::SetHosted(hosted, active));
                    }
                }
            });
        }

        Model {
            auth: pick_authenticator(),
            user,
            pass: TextInputState::masked(),
            focus,
            status: Status::Idle,
            sessions,
            session_idx,
            clipboard: SystemClipboard::new(),
            menu_open: None,
            edit_menu: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
            rain_enabled: saved.rain_enabled,
            rain_color: saved.rain_color,
            anim: saved.anim,
            rain_t: 0.0,
            monitors: Vec::new(),
            active_mon: 0,
            prev_mon: 0,
            card_anim: 1.0,
            mode,
            hosted: Vec::new(),
            hosted_active: 0,
        }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Mientras esperamos a PAM, no aceptamos input.
        if matches!(model.status, Status::Authenticating) {
            return None;
        }
        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
        // cierra.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
            match &e.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => {
                    return Some(Msg::MenuOpen(Some((mi + n - 1) % n)));
                }
                Key::Named(NamedKey::ArrowRight) => {
                    return Some(Msg::MenuOpen(Some((mi + 1) % n)));
                }
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => return Some(Msg::MenuActivate),
                _ => return None,
            }
        }
        // Menú de edición abierto: ↑/↓ navegan, Enter ejecuta, Esc cierra.
        if model.edit_menu.is_some() {
            match &e.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::EditNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::EditNav(-1)),
                Key::Named(NamedKey::Enter) => return Some(Msg::EditActivate),
                _ => return None,
            }
        }
        // En lock no hay campo usuario ni selector de sesión: Tab y ↑/↓ no
        // tienen a dónde ir, todo el teclado va a la contraseña.
        let lock = model.mode == GreeterMode::Lock;
        match &e.key {
            Key::Named(NamedKey::Tab) if !lock => Some(Msg::Focus(toggle(model.focus))),
            // ↑/↓ cambian de escritorio sin tocar el ratón (los campos de una
            // línea no usan las flechas verticales, así que quedan libres).
            Key::Named(NamedKey::ArrowUp) if !lock => Some(Msg::CycleSession(-1)),
            Key::Named(NamedKey::ArrowDown) if !lock => Some(Msg::CycleSession(1)),
            // FUS: en el lock, F2 = «cambiar usuario» (hostear otra sesión).
            Key::Named(NamedKey::F2) if lock => Some(Msg::SwitchUser),
            Key::Named(NamedKey::Enter) => {
                if model.focus == Field::User {
                    Some(Msg::Focus(Field::Pass))
                } else {
                    Some(Msg::Submit)
                }
            }
            _ => {
                // Todo lo demás se delega al widget — `apply_key` decide
                // si la consume (printable, Backspace) o no.
                Some(Msg::EditKey(e.clone()))
            }
        }
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Focus(f) => m.focus = f,
            Msg::EditKey(ev) => {
                let dst = match m.focus {
                    Field::User => &mut m.user,
                    Field::Pass => &mut m.pass,
                };
                if dst.apply_key(&ev) {
                    // Tipear limpia el error previo — el usuario está
                    // corrigiendo.
                    if matches!(m.status, Status::Failed(_)) {
                        m.status = Status::Idle;
                    }
                }
            }
            Msg::Submit => {
                if matches!(m.status, Status::Authenticating) {
                    return m;
                }
                let user = m.user.text().trim().to_string();
                if user.is_empty() {
                    m.status = Status::Failed(rimay_localize::t("greeter-error-empty-user"));
                    m.focus = Field::User;
                    return m;
                }
                let secret = m.pass.text().to_string();
                let auth = Arc::clone(&m.auth);
                m.status = Status::Authenticating;
                handle.spawn(move || Msg::AuthDone(auth.authenticate(&user, &secret)));
            }
            Msg::AuthDone(Ok(user)) => {
                // Modo lock: la contraseña validó al dueño de la sesión ⇒ basta
                // con desbloquear. No hay tiquet ni sesión nueva que arrancar.
                if m.mode == GreeterMode::Lock {
                    emit_action(&ShellAction::Unlock);
                    handle.quit();
                    return m;
                }
                // El comando de la sesión elegida viaja en el tiquet. Vacío
                // (sesión nativa mirada) ⇒ el compositor usa su autostart.
                let chosen = m.sessions.get(m.session_idx);
                let exec = chosen.map(|s| s.exec.clone()).unwrap_or_default();
                let foreign = chosen.map(|s| s.foreign).unwrap_or(false);
                // Reconciliar los backends de sistema con la sesión elegida:
                // levantar el perfil que necesite (p. ej. los shims systemd que
                // GNOME consulta) y bajar los que quedaron de una sesión previa
                // (teardown gnome→mirada). Best-effort: sin bus o perfil, sigue.
                arje_session::reconcile(chosen.and_then(arje_session::profile_for));
                // Recuerda usuario + escritorio (y la config del fondo) para
                // el próximo login.
                state::GreeterState {
                    last_user: m.user.text().trim().to_string(),
                    last_session: chosen.map(|s| s.name.clone()).unwrap_or_default(),
                    rain_enabled: m.rain_enabled,
                    rain_color: m.rain_color,
                    anim: m.anim,
                }
                .save();
                let ticket = SessionTicket::new(user);
                let ticket = if exec.is_empty() {
                    ticket
                } else {
                    ticket.with_session(exec).foreign(foreign)
                };
                emit_action(&ShellAction::StartSession(ticket));
                handle.quit();
            }
            Msg::CycleSession(dir) => {
                let n = m.sessions.len().max(1) as i32;
                let cur = m.session_idx as i32;
                m.session_idx = (((cur + dir) % n + n) % n) as usize;
            }
            Msg::SwitchUser => {
                // Sólo tiene sentido desde el lock; el login de arranque ya es
                // el camino de «otra sesión». Pide al compositor abrir un login.
                if m.mode == GreeterMode::Lock {
                    emit_action(&ShellAction::NewSession);
                    handle.quit();
                }
            }
            Msg::SetHosted(hosted, active) => {
                m.hosted = hosted;
                m.hosted_active = active;
            }
            Msg::SwitchTo(id) => {
                // Saltar directo a otra sesión hosteada (no se ofrece la activa).
                if m.mode == GreeterMode::Lock && id != m.hosted_active {
                    emit_action(&ShellAction::SwitchTo(id));
                    handle.quit();
                }
            }
            Msg::PickSession(i) => {
                if i < m.sessions.len() {
                    m.session_idx = i;
                }
                m.menu_open = None;
            }
            Msg::AuthDone(Err(e)) => {
                m.status = Status::Failed(e.to_string());
                m.pass.clear();
                m.focus = Field::Pass;
            }
            Msg::MenuOpen(idx) => {
                m.menu_open = idx;
                m.edit_menu = None;
                m.menu_active = usize::MAX;
                if idx.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    if let Some(cmd) = menubar_command_at(&menu, mi, m.menu_active) {
                        return handle_menu_command(m, cmd, handle);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::EditNav(dir) => {
                let (input, masked) = focused_input(&m);
                let flags = EditFlags::from_editor(input.editor(), masked);
                m.edit_active = editmenu::edit_menu_step(flags, m.edit_active, dir);
            }
            Msg::EditActivate => {
                let (input, masked) = focused_input(&m);
                let flags = EditFlags::from_editor(input.editor(), masked);
                if let Some(a) = editmenu::edit_menu_action_at(flags, m.edit_active) {
                    return apply_edit_menu_action(m, a);
                }
            }
            Msg::CloseMenus => {
                m.menu_open = None;
                m.edit_menu = None;
                m.menu_active = usize::MAX;
                m.edit_active = usize::MAX;
            }
            Msg::MenuCommand(cmd) => return handle_menu_command(m, cmd, handle),
            Msg::EditMenuOpen(x, y) => {
                // Mientras autenticamos no abrimos el menú de edición.
                if !matches!(m.status, Status::Authenticating) {
                    m.edit_menu = Some((x, y));
                    m.edit_active = usize::MAX;
                    m.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::EditMenuAction(action) => return apply_edit_menu_action(m, action),
            Msg::RainTick => {
                // Avanza el reloj del fondo. Se envuelve para no perder
                // precisión `f32` en sesiones largas del greeter.
                m.rain_t = (m.rain_t + 0.033) % 100_000.0;
                // Avanza el viaje de la tarjeta entre monitores (~280 ms).
                if m.card_anim < 1.0 {
                    m.card_anim = (m.card_anim + 0.033 / 0.28).min(1.0);
                }
            }
            Msg::SetLayout(mons, active) => {
                let active = if mons.is_empty() {
                    0
                } else {
                    active.min(mons.len() - 1)
                };
                // Si cambió el monitor activo, arranca el viaje desde el actual.
                if active != m.active_mon {
                    m.prev_mon = m.active_mon;
                    m.card_anim = 0.0;
                }
                m.monitors = mons;
                m.active_mon = active;
            }
            Msg::SetAnim(a) => {
                m.anim = a;
                m.rain_enabled = true;
                // Paleta por defecto para los fondos que piden un tono propio
                // (el fuego en verde parece pasto; el plasma luce en cian).
                match a {
                    state::BgAnim::Fire => m.rain_color = state::RainColor::Amber,
                    state::BgAnim::Plasma => m.rain_color = state::RainColor::Cyan,
                    state::BgAnim::Aurora => m.rain_color = state::RainColor::Green,
                    state::BgAnim::Lightning => m.rain_color = state::RainColor::Cyan,
                    _ => {}
                }
                persist(&m);
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = Theme::dark();
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
        let input_palette = TextInputPalette::from_theme(&theme);
        let lock = model.mode == GreeterMode::Lock;

        // Barrita de acento sobre el título — el toque de color del DM.
        let accent_bar = View::new(Style {
            size: Size {
                width: length(46.0_f32),
                height: length(4.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.accent)
        .radius(2.0);

        let title = row(30.0, "mirada", 23.0, theme.fg_text);
        // Subtítulo: en lock anuncia el bloqueo; en login, el lema del DM.
        let subtitle_key = if lock { "mirada-lock-subtitle" } else { "greeter-subtitle" };
        let subtitle = row(16.0, &rimay_localize::t(subtitle_key), 12.0, theme.fg_muted);

        // Sección del usuario. En login es un campo editable; en lock el usuario
        // está fijo (dueño de la sesión), así que se muestra como rótulo.
        let user_section: Vec<View<Msg>> = if lock {
            let cap = row(14.0, &rimay_localize::t("mirada-lock-label-user"), 10.0, theme.fg_muted);
            let name = View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
                align_items: Some(AlignItems::Center),
                padding: Rect { left: length(12.0_f32), right: length(12.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
                ..Default::default()
            })
            .fill(theme.bg_input)
            .radius(7.0)
            .text_aligned(model.user.text().to_string(), 13.0, theme.fg_text, Alignment::Start);
            vec![cap, name]
        } else {
            let user_cap = row(14.0, &rimay_localize::t("greeter-label-user"), 10.0, theme.fg_muted);
            let user_box = text_input_view(
                &model.user,
                &rimay_localize::t("greeter-placeholder-user"),
                model.focus == Field::User,
                &input_palette,
                Msg::Focus(Field::User),
            );
            vec![user_cap, user_box]
        };

        let pass_cap = row(
            14.0,
            &rimay_localize::t("greeter-label-password"),
            10.0,
            theme.fg_muted,
        );
        let pass_box = text_input_view(
            &model.pass,
            "·······",
            model.focus == Field::Pass,
            &input_palette,
            Msg::Focus(Field::Pass),
        );

        let (status_msg, status_color) = match &model.status {
            Status::Idle => (String::new(), theme.fg_muted),
            Status::Authenticating => (
                rimay_localize::t("greeter-status-authenticating"),
                theme.fg_muted,
            ),
            Status::Failed(m) => (m.clone(), theme.fg_destructive),
        };
        let status_line = row(16.0, &status_msg, 11.0, status_color);

        // Selector de sesión: una pastilla «‹ nombre · tipo ›». Siempre hay
        // al menos «mirada» y «mirada · pata», así que las flechas sirven.
        let sess = model.sessions.get(model.session_idx);
        let sess_name = sess.map(|s| s.name.clone()).unwrap_or_else(|| "mirada".into());
        let sess_kind = sess.map(|s| s.kind.tag()).unwrap_or("wayland");
        let sess_cap = row(14.0, &rimay_localize::t("mirada-greeter-label-desktop"), 10.0, theme.fg_muted);
        let arrow = |glyph: &str, msg: Msg| {
            View::new(Style {
                size: Size {
                    width: length(30.0_f32),
                    height: length(28.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .fill(theme.bg_button)
            .radius(7.0)
            .text_aligned(glyph.to_string(), 14.0, theme.fg_text, Alignment::Center)
            .on_click(msg)
        };
        let sess_center = View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: Dimension::auto(),
                height: length(28.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.bg_input)
        .radius(7.0)
        .text_aligned(
            format!("{sess_name} · {sess_kind}"),
            11.0,
            theme.fg_text,
            Alignment::Center,
        );
        let session_selector = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            gap: Size {
                width: length(6.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![
            arrow("‹", Msg::CycleSession(-1)),
            sess_center,
            arrow("›", Msg::CycleSession(1)),
        ]);

        // Botón de entrar: la acción primaria, en color de acento. Mientras
        // autentica se atenúa y cambia de rótulo.
        let busy = matches!(model.status, Status::Authenticating);
        let (btn_label, btn_fill) = match (busy, lock) {
            (true, true) => (rimay_localize::t("mirada-lock-btn-busy"), theme.bg_button),
            (true, false) => (rimay_localize::t("mirada-greeter-btn-submitting"), theme.bg_button),
            (false, true) => (rimay_localize::t("mirada-lock-btn"), theme.accent),
            (false, false) => (rimay_localize::t("mirada-greeter-btn-submit"), theme.accent),
        };
        let enter_btn = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(38.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(btn_fill)
        .radius(9.0)
        .text_aligned(
            btn_label.to_string(),
            13.0,
            Color::from_rgba8(245, 246, 250, 255),
            Alignment::Center,
        )
        .on_click(Msg::Submit);

        let card = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: length(360.0_f32),
                height: Dimension::auto(),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(11.0_f32),
            },
            padding: Rect {
                left: length(32.0_f32),
                right: length(32.0_f32),
                top: length(30.0_f32),
                bottom: length(26.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(14.0)
        .children({
            let mut items = vec![accent_bar, title, subtitle, spacer(6.0)];
            items.extend(user_section);
            items.push(pass_cap);
            items.push(pass_box);
            items.push(status_line);
            // El selector de sesión sólo en login: en lock ya estás en una.
            if !lock {
                items.push(spacer(2.0));
                items.push(sess_cap);
                items.push(session_selector);
            }
            items.push(spacer(6.0));
            items.push(enter_btn);
            // FUS: selector «cambiar a» — sólo en lock y si hay OTRA sesión
            // hosteada además de ésta. Una fila clicable por destino.
            let otras: Vec<&(u32, String)> = if lock {
                model.hosted.iter().filter(|(id, _)| *id != model.hosted_active).collect()
            } else {
                Vec::new()
            };
            if !otras.is_empty() {
                items.push(spacer(4.0));
                items.push(row(13.0, &rimay_localize::t("mirada-lock-switch-cap"), 9.0, theme.fg_muted));
                for (id, name) in otras {
                    items.push(switch_row(*id, name, &theme));
                }
            }
            items.push(spacer(2.0));
            // Pistas: en lock desbloquear + cambiar usuario; en login navegación + consola.
            if lock {
                items.push(row(13.0, &rimay_localize::t("mirada-lock-hint"), 9.0, theme.fg_muted));
                items.push(row(13.0, &rimay_localize::t("mirada-lock-hint-switch"), 9.0, theme.fg_muted));
            } else {
                items.push(row(13.0, &rimay_localize::t("mirada-greeter-hint-nav"), 9.0, theme.fg_muted));
                items.push(row(13.0, &rimay_localize::t("mirada-greeter-hint-console"), 9.0, theme.fg_muted));
            }
            items
        });

        // Reloj grande sobre la tarjeta — hora local de pared. Avanza con el
        // tick del fondo (`Msg::RainTick`, ~30 fps, siempre encendido), así que
        // no necesita estado propio: se lee del sistema en cada repintado.
        let stack = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: Dimension::auto(), height: Dimension::auto() },
            gap: Size { width: length(0.0_f32), height: length(20.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![clock_view(&theme), card]);

        // Zona central que aloja el reloj + la tarjeta, centrados. Transparente:
        // el fondo animado (pintado en la raíz) se ve por detrás, también en el
        // monitor activo.
        let body = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .children(vec![stack]);

        // Panel de contenido (barra de menú + tarjeta). En multi-monitor (modo
        // DM) se posiciona —absoluto— sobre el monitor con el ratón y viaja
        // hacia él con una animación; el fondo sigue en todos los monitores.
        // Sin info de monitores (un solo monitor / modo dev) ocupa toda la
        // ventana, como siempre.
        let content_style = match content_rect(model) {
            Some((x, y, w, h)) => Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(x),
                    top: length(y),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size {
                    width: length(w),
                    height: length(h),
                },
                flex_direction: FlexDirection::Column,
                ..Default::default()
            },
            None => Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(0.0_f32),
                    top: length(0.0_f32),
                    right: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                flex_direction: FlexDirection::Column,
                ..Default::default()
            },
        };
        let content = View::new(content_style).children(vec![menubar, body]);

        // Raíz: cubre toda la ventana (la unión de las salidas en multi-monitor)
        // y pinta el fondo animado por detrás de todo. El right-click se
        // engancha acá (origen 0,0 ⇒ las coords locales ya son de ventana).
        let mut root = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app);
        if model.rain_enabled {
            let t = model.rain_t;
            let anim = model.anim;
            let bright = rain_bright(model.rain_color, &theme);
            root = root.paint_with(move |scene, ts, rect| {
                bg::paint(anim, scene, ts, rect, t, bright);
            });
        }
        root.on_right_click_at(|x, y, _w, _h| Some(Msg::EditMenuOpen(x, y)))
            .children(vec![content])
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        let theme = Theme::dark();
        let (w, h) = Self::initial_size();
        let viewport = (w as f32, h as f32);
        // El menú de edición tiene prioridad si está abierto.
        if let Some((x, y)) = model.edit_menu {
            let (input, masked) = focused_input(model);
            let flags = EditFlags::from_editor(input.editor(), masked);
            let mut spec = editmenu::edit_context_menu(
                (x, y),
                viewport,
                &theme,
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            );
            spec.active = model.edit_active;
            return Some(context_menu_view_ex(
                spec,
                ContextMenuExtras { appear: model.edit_anim.value(), ..Default::default() },
            ));
        }
        // Si no, el dropdown del menú principal. La barra vive en el panel de
        // contenido —desplazado al monitor activo—, pero el overlay se posiciona
        // en coords del menubar con origen (0,0); hay que correrlo el mismo
        // offset para que caiga bajo la barra y no en el monitor primario.
        let menu = app_menu(model);
        let overlay = menubar_overlay_animated(
            &menubar_spec(&menu, model, &theme),
            model.menu_active,
            model.menu_anim.value(),
        )?;
        Some(offset_to_active_monitor(model, overlay))
    }
}

/// Desplaza `view` (el dropdown del menú) al rect del monitor activo, para que
/// caiga bajo la barra y no en el monitor primario. El offset va en un **hijo**
/// absoluto, no en la raíz: taffy ignora el `inset` del nodo raíz (siempre lo
/// pone en 0,0), así que la raíz es de ventana completa y el contenedor
/// desplazado cuelga de ella. Sin info de monitores devuelve la vista tal cual.
fn offset_to_active_monitor(model: &Model, view: View<Msg>) -> View<Msg> {
    match content_rect(model) {
        Some((x, y, w, h)) => {
            let shifted = View::new(Style {
                position: Position::Absolute,
                inset: Rect { left: length(x), top: length(y), right: auto(), bottom: auto() },
                size: Size { width: length(w), height: length(h) },
                ..Default::default()
            })
            .children(vec![view]);
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .children(vec![shifted])
        }
        None => view,
    }
}

/// El campo de texto focuseado + si está enmascarado.
fn focused_input(model: &Model) -> (&TextInputState, bool) {
    match model.focus {
        Field::User => (&model.user, model.user.is_masked()),
        Field::Pass => (&model.pass, model.pass.is_masked()),
    }
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(
    menu: &'a app_bus::AppMenu,
    model: &Model,
    theme: &'a Theme,
) -> MenuBarSpec<'a, Msg> {
    let (w, h) = Greeter::initial_size();
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: (w as f32, h as f32),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// Construye el menú principal del greeter reflejando el estado real del
/// campo focuseado (Cortar/Copiar grises sin selección o si enmascarado).
fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};
    let t = rimay_localize::t;
    let (input, masked) = focused_input(model);
    let editor = input.editor();
    let has_sel = editor.has_selection();
    let can_undo = editor.can_undo();
    let can_redo = editor.can_redo();
    let has_text = !editor.is_empty();
    let busy = matches!(model.status, Status::Authenticating);

    let mut undo = MenuItem::new(t("undo"), "edit.undo").shortcut("Ctrl+Z");
    if !can_undo { undo = undo.disabled(); }
    let mut redo = MenuItem::new(t("redo"), "edit.redo").shortcut("Ctrl+Y");
    if !can_redo { redo = redo.disabled(); }
    let mut cut = MenuItem::new(t("cut"), "edit.cut").shortcut("Ctrl+X").separated();
    let mut copy = MenuItem::new(t("copy"), "edit.copy").shortcut("Ctrl+C");
    // Enmascarado o sin selección ⇒ no se puede cortar/copiar.
    if !has_sel || masked { cut = cut.disabled(); copy = copy.disabled(); }
    let paste = MenuItem::new(t("paste"), "edit.paste").shortcut("Ctrl+V");
    let mut sel_all = MenuItem::new(t("select-all"), "edit.selectall").shortcut("Ctrl+A").separated();
    if !has_text { sel_all = sel_all.disabled(); }

    let mut iniciar = MenuItem::new(t("mirada-greeter-session-submit"), "session.submit").shortcut("Enter");
    if busy { iniciar = iniciar.disabled(); }

    // Menú "Sesión": acciones de login + la lista de sesiones descubiertas.
    // La elegida lleva «●»; el resto «  ».
    let mut sesion = Menu::new(t("mirada-greeter-menu-session"))
        .item(iniciar)
        .item(MenuItem::new(t("mirada-greeter-session-goto-user"), "session.user"))
        .item(MenuItem::new(t("mirada-greeter-session-goto-pass"), "session.pass"));
    for (i, s) in model.sessions.iter().enumerate() {
        let mark = if i == model.session_idx { "● " } else { "   " };
        let label = format!("{mark}{} · {}", s.name, s.kind.tag());
        let mut item = MenuItem::new(label, format!("session.pick.{i}"));
        if i == 0 {
            item = item.separated();
        }
        sesion = sesion.item(item);
    }

    // Menú de idioma: autónimos sin traducir. El item activo lleva ✔.
    // El comando `lang.<code>` lo resuelve `handle_menu_command`.
    let cur = rimay_localize::current_locale();
    let lang_item = |label: &str, code: &str| {
        let mut it = MenuItem::new(label, format!("lang.{code}"));
        if cur == code {
            it = it.icon("\u{2714}");
        }
        it
    };

    // Menú "Fondo": elige la animación enchufable. La activa lleva ✔.
    let bg_item = |label: &str, a: state::BgAnim| {
        let mut it = MenuItem::new(label, format!("bg.{}", a.tag()));
        if model.rain_enabled && model.anim == a {
            it = it.icon("\u{2714}");
        }
        it
    };
    let bg_menu = Menu::new(t("mirada-greeter-menu-bg"))
        .item(bg_item(&t("mirada-greeter-bg-matrix"), state::BgAnim::Matrix))
        .item(bg_item(&t("mirada-greeter-bg-stars"), state::BgAnim::Stars))
        .item(bg_item(&t("mirada-greeter-bg-waves"), state::BgAnim::Waves))
        .item(bg_item(&t("mirada-greeter-bg-fire"), state::BgAnim::Fire))
        .item(bg_item(&t("mirada-greeter-bg-plasma"), state::BgAnim::Plasma))
        .item(bg_item(&t("mirada-greeter-bg-aurora"), state::BgAnim::Aurora))
        .item(bg_item(&t("mirada-greeter-bg-lightning"), state::BgAnim::Lightning))
        .item(MenuItem::new(t("mirada-greeter-bg-off"), "bg.off").separated());

    AppMenu::new()
        .menu(sesion)
        .menu(
            Menu::new(t("edit"))
                .item(undo)
                .item(redo)
                .item(cut)
                .item(copy)
                .item(paste)
                .item(sel_all),
        )
        .menu(bg_menu)
        .menu(
            Menu::new(t("language"))
                .item(lang_item("Español", "es-PE"))
                .item(lang_item("English", "en-US"))
                .item(lang_item("Runasimi", "qu-PE")),
        )
}

/// Traduce el `command` del menú principal al `Msg` real y lo despacha.
fn handle_menu_command(mut model: Model, command: String, handle: &Handle<Msg>) -> Model {
    model.menu_open = None;
    // Cambio de idioma desde el menú "Idioma": aplica el locale en caliente
    // y lo persiste en la capa de usuario de wawa-config.
    if let Some(code) = command.strip_prefix("lang.") {
        let _ = rimay_localize::set_locale(code);
        let mut cfg = wawa_config::WawaConfig::load();
        cfg.lang = code.to_string();
        let _ = cfg.save();
        return model;
    }
    // Fondo animado: «bg.<tag>» elige la animación, «bg.off» lo apaga.
    if let Some(tag) = command.strip_prefix("bg.") {
        if tag == "off" {
            model.rain_enabled = false;
            persist(&model);
            return model;
        }
        let anim = match tag {
            "matrix" => state::BgAnim::Matrix,
            "stars" => state::BgAnim::Stars,
            "waves" => state::BgAnim::Waves,
            "fire" => state::BgAnim::Fire,
            "plasma" => state::BgAnim::Plasma,
            "aurora" => state::BgAnim::Aurora,
            "lightning" => state::BgAnim::Lightning,
            _ => return model,
        };
        return Greeter::update(model, Msg::SetAnim(anim), handle);
    }
    // Elección de sesión: «session.pick.<idx>».
    if let Some(rest) = command.strip_prefix("session.pick.") {
        if let Ok(i) = rest.parse::<usize>() {
            return Greeter::update(model, Msg::PickSession(i), handle);
        }
        return model;
    }
    let target = match command.as_str() {
        "session.submit" => Some(Msg::Submit),
        "session.user" => Some(Msg::Focus(Field::User)),
        "session.pass" => Some(Msg::Focus(Field::Pass)),
        "edit.undo" => Some(Msg::EditMenuAction(EditAction::Undo)),
        "edit.redo" => Some(Msg::EditMenuAction(EditAction::Redo)),
        "edit.cut" => Some(Msg::EditMenuAction(EditAction::Cut)),
        "edit.copy" => Some(Msg::EditMenuAction(EditAction::Copy)),
        "edit.paste" => Some(Msg::EditMenuAction(EditAction::Paste)),
        "edit.selectall" => Some(Msg::EditMenuAction(EditAction::SelectAll)),
        _ => None,
    };
    match target {
        Some(Msg::Submit) => Greeter::update(model, Msg::Submit, handle),
        Some(msg) => Greeter::update(model, msg, handle),
        None => model,
    }
}

/// Aplica una acción del menú de edición al campo focuseado. Limpia el
/// error previo si el contenido cambió (el usuario está corrigiendo).
fn apply_edit_menu_action(mut model: Model, action: EditAction) -> Model {
    model.edit_menu = None;
    let r = {
        let mut clip = std::mem::replace(&mut model.clipboard, SystemClipboard::new());
        let editor = match model.focus {
            Field::User => model.user.editor_mut(),
            Field::Pass => model.pass.editor_mut(),
        };
        let r = editmenu::apply(editor, action, &mut clip);
        model.clipboard = clip;
        r
    };
    if r.changed() && matches!(model.status, Status::Failed(_)) {
        model.status = Status::Idle;
    }
    model
}

/// Persiste la config del fondo (animación + paleta) sin esperar al login —
/// para que la elección del menú sobreviva un reinicio del greeter. Conserva
/// el último usuario/sesión recordados (no los pisa con el estado en curso).
fn persist(m: &Model) {
    let mut st = state::GreeterState::load();
    st.rain_enabled = m.rain_enabled;
    st.rain_color = m.rain_color;
    st.anim = m.anim;
    st.save();
}

/// Rect (px, locales a la ventana) donde colocar el panel de contenido: el
/// monitor activo, interpolado desde el anterior por la animación de viaje.
/// `None` si no hay info de monitores (un solo monitor / modo dev) ⇒ el panel
/// ocupa toda la ventana.
fn content_rect(m: &Model) -> Option<MonRect> {
    if m.monitors.is_empty() {
        return None;
    }
    let to = *m.monitors.get(m.active_mon)?;
    let from = m.monitors.get(m.prev_mon).copied().unwrap_or(to);
    // Ease-out cúbico: arranca rápido y desacelera al asentarse.
    let t = m.card_anim.clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - t).powi(3);
    let lerp = |a: f32, b: f32| a + (b - a) * e;
    Some((
        lerp(from.0, to.0),
        lerp(from.1, to.1),
        lerp(from.2, to.2),
        lerp(from.3, to.3),
    ))
}

/// Parsea una línea `LAYOUT <activo> x,y,w,h x,y,w,h …` empujada por el
/// compositor. Devuelve `(rects de monitor, índice activo)`, o `None` si la
/// línea no es un `LAYOUT` bien formado.
fn parse_layout(line: &str) -> Option<(Vec<MonRect>, usize)> {
    let rest = line.trim().strip_prefix("LAYOUT ")?;
    let mut it = rest.split_whitespace();
    let active: usize = it.next()?.parse().ok()?;
    let mut mons = Vec::new();
    for tok in it {
        let mut nums = tok.split(',');
        let x: f32 = nums.next()?.parse().ok()?;
        let y: f32 = nums.next()?.parse().ok()?;
        let w: f32 = nums.next()?.parse().ok()?;
        let h: f32 = nums.next()?.parse().ok()?;
        mons.push((x, y, w, h));
    }
    Some((mons, active))
}

/// Parsea una línea `SESSIONS <id_activo> <id>:<nombre> …` empujada por el
/// compositor al lock (FUS). Devuelve `(roster, id_activo)`. `None` si la línea
/// no es un roster bien formado.
fn parse_sessions(line: &str) -> Option<(Vec<(u32, String)>, u32)> {
    let rest = line.trim().strip_prefix("SESSIONS ")?;
    let mut it = rest.split_whitespace();
    let active: u32 = it.next()?.parse().ok()?;
    let mut hosted = Vec::new();
    for tok in it {
        let (id, name) = tok.split_once(':')?;
        hosted.push((id.parse().ok()?, name.to_string()));
    }
    Some((hosted, active))
}

/// Resuelve el color base (RGB brillante) del fondo de lluvia. `Accent` toma
/// el acento del tema; el resto son paletas fijas.
fn rain_bright(color: state::RainColor, theme: &Theme) -> (u8, u8, u8) {
    match color {
        state::RainColor::Green => (120, 255, 140),
        state::RainColor::Red => (255, 90, 80),
        state::RainColor::Amber => (255, 200, 90),
        state::RainColor::Cyan => (110, 235, 255),
        state::RainColor::Accent => {
            let c = theme.accent.to_rgba8();
            (c.r, c.g, c.b)
        }
    }
}

fn toggle(f: Field) -> Field {
    match f {
        Field::User => Field::Pass,
        Field::Pass => Field::User,
    }
}

// ---------------------------------------------------------------------
// Helpers de vista
// ---------------------------------------------------------------------

/// Un hueco vertical de `h` px — separa grupos dentro de la tarjeta.
fn spacer(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        ..Default::default()
    })
}

/// Fila de ancho completo con un texto a la izquierda.
fn row(height: f32, text: &str, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(height),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), size, color, Alignment::Start)
}

/// FUS: fila clicable del selector «cambiar a» — salta a la sesión `id` del
/// roster. Sutil (relleno de panel-botón), con el nombre y un chevron.
fn switch_row(id: u32, name: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_button)
    .radius(7.0)
    .text_aligned(format!("→  {name}"), 12.5, theme.fg_text, Alignment::Center)
    .on_click(Msg::SwitchTo(id))
}

/// Hora (`HH:MM`) y fecha localizada del reloj, leídas de la hora local del
/// sistema (con conciencia de zona horaria, vía `chrono::Local`). Quechua y
/// cualquier locale no inglés caen al español (el default del repo).
fn now_strings() -> (String, String) {
    use chrono::{Datelike, Timelike};
    const DIAS_ES: [&str; 7] =
        ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"];
    const DIAS_EN: [&str; 7] =
        ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    const MESES_ES: [&str; 12] = [
        "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre",
        "octubre", "noviembre", "diciembre",
    ];
    const MESES_EN: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September",
        "October", "November", "December",
    ];

    let now = chrono::Local::now();
    let time = format!("{:02}:{:02}", now.hour(), now.minute());
    let en = rimay_localize::current_locale().starts_with("en");
    let dow = now.weekday().num_days_from_monday() as usize;
    let mon = (now.month() as usize).saturating_sub(1).min(11);
    let (dia, mes) = if en {
        (DIAS_EN[dow], MESES_EN[mon])
    } else {
        (DIAS_ES[dow], MESES_ES[mon])
    };
    let mut date = if en {
        format!("{dia}, {} {mes} {}", now.day(), now.year())
    } else {
        format!("{dia}, {} de {mes} de {}", now.day(), now.year())
    };
    // Mayúscula inicial (en español el día va en minúscula, pero como rótulo
    // suelto luce mejor capitalizado; en inglés ya viene así).
    if let Some(first) = date.get(0..1) {
        date = format!("{}{}", first.to_uppercase(), &date[1..]);
    }
    (time, date)
}

/// Reloj grande (hora + fecha) centrado, del ancho de la tarjeta. Sin estado
/// propio: se relee la hora del sistema en cada repintado (el tick del fondo
/// repinta ~30 fps).
fn clock_view(theme: &Theme) -> View<Msg> {
    let (time, date) = now_strings();
    let time_v = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(74.0_f32) },
        ..Default::default()
    })
    .text_aligned(time, 64.0, theme.fg_text, Alignment::Center);
    let date_v = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        ..Default::default()
    })
    .text_aligned(date, 14.0, theme.fg_muted, Alignment::Center);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(360.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![time_v, date_v])
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_layout_dos_monitores() {
        let (mons, active) = parse_layout("LAYOUT 1 0,0,1920,1080 1920,0,2560,1440").unwrap();
        assert_eq!(active, 1);
        assert_eq!(mons, vec![(0.0, 0.0, 1920.0, 1080.0), (1920.0, 0.0, 2560.0, 1440.0)]);
    }

    #[test]
    fn parse_layout_rechaza_basura() {
        assert!(parse_layout("hola mundo").is_none());
        assert!(parse_layout("LAYOUT").is_none());
        assert!(parse_layout("LAYOUT x 0,0,1,1").is_none());
    }

    #[test]
    fn parse_sessions_roster() {
        let (hosted, active) = parse_sessions("SESSIONS 1 0:ana 1:beto").unwrap();
        assert_eq!(active, 1);
        assert_eq!(hosted, vec![(0, "ana".to_string()), (1, "beto".to_string())]);
        // Sin sesiones más allá del activo (caso N=1): lista vacía, válido.
        let (hosted, active) = parse_sessions("SESSIONS 3").unwrap();
        assert_eq!(active, 3);
        assert!(hosted.is_empty());
    }

    #[test]
    fn parse_sessions_rechaza_basura() {
        assert!(parse_sessions("LAYOUT 0 0,0,1,1").is_none());
        assert!(parse_sessions("SESSIONS").is_none());
        assert!(parse_sessions("SESSIONS x 0:ana").is_none());
        assert!(parse_sessions("SESSIONS 0 nocolon").is_none());
    }

    /// Construye un modelo mínimo para ejercitar `content_rect` (los campos no
    /// usados van por defecto vía un modelo de `--shot`).
    fn model_con_monitores(mons: Vec<MonRect>, active: usize, prev: usize, t: f32) -> Model {
        let saved = state::GreeterState::default();
        Model {
            auth: pick_authenticator(),
            user: TextInputState::new(),
            pass: TextInputState::masked(),
            focus: Field::User,
            status: Status::Idle,
            sessions: Vec::new(),
            session_idx: 0,
            clipboard: SystemClipboard::new(),
            menu_open: None,
            edit_menu: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
            rain_enabled: true,
            rain_color: saved.rain_color,
            anim: saved.anim,
            rain_t: 0.0,
            monitors: mons,
            active_mon: active,
            prev_mon: prev,
            card_anim: t,
            mode: GreeterMode::Login,
            hosted: Vec::new(),
            hosted_active: 0,
        }
    }

    #[test]
    fn content_rect_sin_monitores_es_none() {
        let m = model_con_monitores(Vec::new(), 0, 0, 1.0);
        assert!(content_rect(&m).is_none());
    }

    #[test]
    fn content_rect_asentado_en_monitor_activo() {
        let mons = vec![(0.0, 0.0, 1920.0, 1080.0), (1920.0, 0.0, 2560.0, 1440.0)];
        // card_anim = 1.0 ⇒ asentada exactamente en el activo (monitor 1).
        let m = model_con_monitores(mons, 1, 0, 1.0);
        assert_eq!(content_rect(&m).unwrap(), (1920.0, 0.0, 2560.0, 1440.0));
    }

    #[test]
    fn content_rect_viaja_entre_monitores() {
        let mons = vec![(0.0, 0.0, 1000.0, 1000.0), (1000.0, 0.0, 1000.0, 1000.0)];
        // A mitad del viaje, el panel está entre ambos (x estrictamente dentro).
        let m = model_con_monitores(mons, 1, 0, 0.5);
        let (x, _, _, _) = content_rect(&m).unwrap();
        assert!(x > 0.0 && x < 1000.0, "x={x} debería estar entre 0 y 1000");
    }
}
