//! `mirada` — la ventana del Cerebro del compositor.
//!
//! Es el "Cerebro" de la arquitectura carmen hecho app GPUI: envuelve
//! [`mirada_brain::Desktop`] (toda la lógica de teselado y foco) y lo
//! pinta. La cadena completa:
//!
//! ```text
//!   mirada-layout ─► mirada-protocol ─► mirada-brain ─► [esta ventana]
//!                                          │
//!                                    mirada-link ─► mirada-compositor (Cuerpo)
//! ```
//!
//! Con un Cuerpo conectado (variable `MIRADA_SOCKET`) sondea sus
//! [`BodyEvent`]s y le devuelve [`BrainCommand`]s por el socket. Sin
//! Cuerpo arranca en **simulación**: las ventanas son sintéticas y el
//! teclado de esta ventana maneja el escritorio — útil para ver el
//! motor de teselado sin hardware.
//!
//! Teclas (simulación):
//!
//! ```text
//!   n            abre una ventana          tab / espacio  cicla layout
//!   w            cierra la enfocada        t g c m        layout directo
//!   j / k        foco siguiente/anterior   1..9           ir a escritorio
//!   Shift+j / k  mueve la enfocada         Ctrl+1..9      enviar a escritorio
//! ```
//!
//! Los pips de escritorio y las ventanas del lienzo son **clicables**, y
//! `mirada-ctl` controla el escritorio desde la terminal — ambos pasan
//! por el mismo `Desktop::apply`.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    div, hsla, prelude::*, px, Context, FocusHandle, IntoElement, KeyDownEvent, MouseButton,
    Render, SharedString, Window,
};
use mirada_brain::{
    BodyEvent, BrainCommand, CtlConn, CtlReply, CtlRequest, CtlServer, Desktop, DesktopAction,
    Keymap, KeymapWatch, LayoutMode, WindowId, WindowPlacement,
};
use mirada_link::BrainLink;
use nahual_launcher::launch_app;
use nahual_theme::Theme;

/// Pantalla virtual del modo simulación — coincide con el lienzo.
const SCREEN_W: i32 = 1280;
const SCREEN_H: i32 = 720;
/// Periodo del sondeo del Cuerpo, en ms (~60 Hz).
const POLL_MS: u64 = 16;

/// Nombres de app ficticios para las ventanas de simulación.
const APPS: &[&str] = &[
    "shuma", "fana", "revista", "cosmobiología", "matilda", "yachay", "barra",
];

/// El Cerebro: el estado del escritorio + lo último colocado + el cable.
struct Mirada {
    desktop: Desktop,
    /// Geometría vigente — lo que se pinta. Es la última `Place` emitida.
    placements: Vec<WindowPlacement>,
    /// Contador de ids para las ventanas sintéticas.
    next_id: WindowId,
    /// Cable al Cuerpo; `None` en simulación.
    link: Option<BrainLink>,
    /// Última acción, para la barra de estado.
    note: SharedString,
    focus: FocusHandle,
    focused_once: bool,
    /// Ruta del keymap del usuario, para recargarlo en caliente.
    keymap_path: Option<PathBuf>,
    /// Vigía del keymap; `None` en simulación o si no hay archivo.
    keymap_watch: Option<KeymapWatch>,
    /// Socket del API de control externo (`mirada-ctl`).
    ctl: Option<CtlServer>,
}

impl Mirada {
    fn new(cx: &mut Context<Self>) -> Self {
        // Keymap del usuario (~/.config/mirada/keymap.ron): define los
        // atajos que el Cuerpo intercepta y nos devuelve como `Keybind`.
        let keymap_path = Keymap::default_path();
        let keymap = match &keymap_path {
            Some(p) => Keymap::load_or_init(p),
            None => Keymap::default(),
        };
        let link = connect_body();
        // Vigilar el keymap sólo tiene sentido con un Cuerpo conectado;
        // en simulación, mirada usa las teclas de su propia ventana.
        let keymap_watch = if link.is_some() {
            keymap_path.as_deref().and_then(|p| Keymap::watch(p).ok())
        } else {
            None
        };
        // API de control: mirada siempre posee el Desktop, así que
        // siempre abre el socket de `mirada-ctl`.
        let ctl = match CtlServer::bind(&mirada_brain::ctl::default_socket_path()) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("mirada · sin API de control: {e}");
                None
            }
        };

        let mut app = Self {
            desktop: Desktop::with_keymap(keymap),
            placements: Vec::new(),
            next_id: 1,
            link,
            note: SharedString::from("listo"),
            focus: cx.focus_handle(),
            focused_once: false,
            keymap_path,
            keymap_watch,
            ctl,
        };
        if let Some(link) = app.link.as_mut() {
            // Registra los atajos globales en el Cuerpo.
            let _ = link.send(&app.desktop.grab_keys());
            app.note = SharedString::from("Cuerpo conectado");
        } else {
            // Simulación: una pantalla virtual y tres ventanas de muestra.
            app.feed(BodyEvent::OutputAdded { id: 0, width: SCREEN_W, height: SCREEN_H });
            for _ in 0..3 {
                app.open_window();
            }
            app.note = SharedString::from("simulación — sin Cuerpo");
        }
        // El sondeo corre siempre: drena el Cuerpo (si lo hay), vigila el
        // keymap y atiende `mirada-ctl`.
        app.start_poll(cx);
        app
    }

    /// Bucle de fondo: drena los eventos del Cuerpo y los procesa.
    fn start_poll(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(POLL_MS))
                .await;
            let alive = this.update(cx, |app, cx| {
                let events: Vec<BodyEvent> = match app.link.as_ref() {
                    Some(link) => link.drain(),
                    None => Vec::new(),
                };
                let had_events = !events.is_empty();
                let keymap_changed = app.keymap_watch.as_ref().is_some_and(|w| w.changed());
                if keymap_changed {
                    app.reload_keymap();
                }
                let ctl_served = app.poll_ctl();
                for ev in events {
                    app.feed(ev);
                }
                if had_events || keymap_changed || ctl_served {
                    cx.notify();
                }
            });
            if alive.is_err() {
                break; // ventana cerrada
            }
        })
        .detach();
    }

    /// Abre una ventana sintética (sólo tiene sentido en simulación).
    fn open_window(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        let app = APPS[(id as usize) % APPS.len()];
        self.feed(BodyEvent::WindowOpened {
            id,
            app_id: format!("org.brahman.{app}"),
            title: format!("{app} · ventana {id}"),
        });
        self.note = SharedString::from(format!("abierta ventana {id}"));
    }

    /// Inyecta un evento del Cuerpo en el `Desktop` y despacha la salida.
    fn feed(&mut self, event: BodyEvent) {
        let cmds = self.desktop.on_event(event);
        self.dispatch(cmds);
    }

    /// Aplica una acción de escritorio (desde una tecla de esta ventana).
    fn act(&mut self, action: DesktopAction) {
        let cmds = self.desktop.apply(action);
        self.dispatch(cmds);
    }

    /// Recarga el keymap del disco y re-registra los atajos en el Cuerpo.
    fn reload_keymap(&mut self) {
        let Some(path) = self.keymap_path.clone() else {
            return;
        };
        match Keymap::load(&path) {
            Ok(km) => {
                let cmd = self.desktop.set_keymap(km);
                self.dispatch(vec![cmd]);
                self.note = SharedString::from("keymap recargado");
            }
            Err(e) => self.note = SharedString::from(format!("keymap inválido: {e}")),
        }
    }

    /// Atiende las peticiones pendientes del API de control. Devuelve
    /// `true` si sirvió alguna (para repintar).
    fn poll_ctl(&mut self) -> bool {
        let conns: Vec<CtlConn> = match &self.ctl {
            Some(ctl) => std::iter::from_fn(|| ctl.poll()).collect(),
            None => return false,
        };
        let mut served = false;
        for mut conn in conns {
            let reply = match conn.read_request() {
                Ok(Some(req)) => {
                    served = true;
                    self.serve_ctl(req)
                }
                Ok(None) => continue,
                Err(e) => CtlReply::Error(format!("{e}")),
            };
            let _ = conn.reply(&reply);
        }
        served
    }

    /// Resuelve una petición de control: la acción pasa por el mismo
    /// `apply` que el teclado; la consulta lee el `Desktop`.
    fn serve_ctl(&mut self, req: CtlRequest) -> CtlReply {
        match req {
            CtlRequest::Do(action) => {
                self.act(action);
                CtlReply::Ok
            }
            CtlRequest::ListWindows => CtlReply::Windows(self.desktop.window_lines()),
        }
    }

    /// Reparte los comandos del Cerebro: actualiza lo pintado y, o bien
    /// los manda al Cuerpo, o bien —en simulación— cierra las ventanas
    /// por su cuenta (no hay nadie que devuelva el `WindowClosed`).
    fn dispatch(&mut self, cmds: Vec<BrainCommand>) {
        for cmd in &cmds {
            if let BrainCommand::Place(p) = cmd {
                self.placements = p.clone();
            }
        }
        match self.link.as_mut() {
            Some(link) => {
                for cmd in &cmds {
                    let _ = link.send(cmd);
                }
            }
            None => {
                for cmd in cmds {
                    match cmd {
                        BrainCommand::Close(id) | BrainCommand::Kill(id) => {
                            self.feed(BodyEvent::WindowClosed { id });
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Traduce una tecla de la ventana a una acción de escritorio.
    fn handle_key(&mut self, event: &KeyDownEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let ctrl = ks.modifiers.control;
        let shift = ks.modifiers.shift;
        let connected = self.link.is_some();

        match ks.key.as_str() {
            "n" if !connected => self.open_window(),
            "w" => self.act(DesktopAction::CloseFocused),
            "j" if shift => self.act(DesktopAction::MoveForward),
            "k" if shift => self.act(DesktopAction::MoveBackward),
            "j" => self.act(DesktopAction::FocusNext),
            "k" => self.act(DesktopAction::FocusPrev),
            "tab" | "space" => self.act(DesktopAction::CycleLayout),
            "t" => self.act(DesktopAction::SetLayout(LayoutMode::MasterStack)),
            "m" => self.act(DesktopAction::SetLayout(LayoutMode::Monocle)),
            "g" => self.act(DesktopAction::SetLayout(LayoutMode::Grid)),
            "c" => self.act(DesktopAction::SetLayout(LayoutMode::Columns)),
            d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && d != "0" => {
                let n = (d.as_bytes()[0] - b'1') as usize;
                if ctrl {
                    self.act(DesktopAction::SendToWorkspace(n));
                } else {
                    self.act(DesktopAction::SwitchWorkspace(n));
                }
            }
            _ => return,
        }
        cx.notify();
    }
}

/// Conecta con el Cuerpo si `MIRADA_SOCKET` apunta a un socket vivo.
fn connect_body() -> Option<BrainLink> {
    let path = std::env::var("MIRADA_SOCKET").ok()?;
    BrainLink::connect(&path).ok()
}

/// Nombre legible de un modo de teselado.
fn mode_name(m: LayoutMode) -> &'static str {
    match m {
        LayoutMode::MasterStack => "maestro + pila",
        LayoutMode::Monocle => "monóculo",
        LayoutMode::Grid => "rejilla",
        LayoutMode::Columns => "columnas",
    }
}

impl Render for Mirada {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // El lienzo necesita el foco del teclado desde el primer frame.
        if !self.focused_once {
            window.focus(&self.focus);
            self.focused_once = true;
        }

        let theme = Theme::global(cx).clone();
        let win_bg = hsla(220.0 / 360.0, 0.16, 0.13, 1.0);
        let bar_bg = hsla(220.0 / 360.0, 0.20, 0.09, 1.0);
        let canvas_bg = hsla(220.0 / 360.0, 0.24, 0.05, 1.0);
        // Texto legible sobre un fondo de acento.
        let on_accent = hsla(220.0 / 360.0, 0.24, 0.06, 1.0);

        let active = self.desktop.active_index();
        let mode = self.desktop.active_workspace().params().mode;
        let loads = self.desktop.workspace_loads();
        let focused = self.desktop.focused_window();

        // --- Barra superior: identidad + escritorios + modo ----------
        let pips = loads.iter().enumerate().map(|(i, &load)| {
            let is_active = i == active;
            let fg = if is_active {
                on_accent
            } else if load > 0 {
                theme.fg_text
            } else {
                theme.fg_disabled
            };
            div()
                .w(px(24.))
                .h(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .when(is_active, |d| d.bg(theme.accent))
                .when(!is_active && load > 0, |d| d.bg(theme.bg_row_hover))
                .text_color(fg)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |app, _, _, cx| {
                        app.act(DesktopAction::SwitchWorkspace(i));
                        cx.notify();
                    }),
                )
                .child(SharedString::from(format!("{}", i + 1)))
        });

        let focus_label = match focused.and_then(|id| self.desktop.window_info(id)) {
            Some(info) => info.title.clone(),
            None => "—".to_string(),
        };

        let bar = div()
            .h(px(44.))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .bg(bar_bg)
            .text_color(theme.fg_text)
            .child(div().text_color(theme.accent).child("mirada"))
            .child(div().text_color(theme.fg_disabled).child("·"))
            .child(div().flex().flex_row().gap(px(4.)).children(pips))
            .child(div().text_color(theme.fg_disabled).child("·"))
            .child(
                div()
                    .text_color(theme.fg_muted)
                    .child(SharedString::from(format!("layout: {}", mode_name(mode)))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_color(theme.fg_muted)
                    .child(SharedString::from(format!("foco: {focus_label}"))),
            );

        // --- Lienzo: el escritorio teselado --------------------------
        let mut canvas = div()
            .relative()
            .w(px(SCREEN_W as f32))
            .h(px(SCREEN_H as f32))
            .bg(canvas_bg)
            .overflow_hidden();

        let visible = self.placements.iter().filter(|p| p.visible).count();
        if visible == 0 {
            canvas = canvas.child(
                div()
                    .absolute()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.fg_disabled)
                    .child("escritorio vacío — pulsa  n  para abrir una ventana"),
            );
        }
        for p in self.placements.iter().filter(|p| p.visible) {
            let info = self.desktop.window_info(p.id);
            let title = info
                .map(|i| i.title.clone())
                .unwrap_or_else(|| format!("ventana {}", p.id));
            let app_id = info.map(|i| i.app_id.clone()).unwrap_or_default();
            let border = if p.focused { theme.accent } else { theme.border };
            let tb_bg = if p.focused { theme.accent } else { theme.bg_row_hover };
            let tb_fg = if p.focused { on_accent } else { theme.fg_muted };
            let pid = p.id;

            canvas = canvas.child(
                div()
                    .absolute()
                    .left(px(p.rect.x as f32))
                    .top(px(p.rect.y as f32))
                    .w(px(p.rect.w as f32))
                    .h(px(p.rect.h as f32))
                    .border_2()
                    .border_color(border)
                    .bg(win_bg)
                    .rounded(px(5.))
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |app, _, _, cx| {
                            app.act(DesktopAction::FocusWindow(pid));
                            cx.notify();
                        }),
                    )
                    .flex()
                    .flex_col()
                    .child(
                        // Barra de título de la ventana.
                        div()
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .px(px(8.))
                            .bg(tb_bg)
                            .text_color(tb_fg)
                            .child(SharedString::from(title)),
                    )
                    .child(
                        // Interior: en el compositor real lo compone el
                        // Cuerpo (zero-copy); aquí es un marcador.
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px(4.))
                            .text_color(theme.fg_disabled)
                            .child(SharedString::from(app_id))
                            .child("· superficie del Cuerpo ·"),
                    ),
            );
        }

        // --- Composición ---------------------------------------------
        div()
            .track_focus(&self.focus)
            .key_context("Mirada")
            .on_key_down(cx.listener(Self::handle_key))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg_app)
            .text_color(theme.fg_text)
            .child(bar)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.bg_app)
                    .child(canvas),
            )
            .child(
                // Pie: el estado.
                div()
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .px(px(14.))
                    .bg(bar_bg)
                    .text_color(theme.fg_disabled)
                    .child(self.note.clone()),
            )
    }
}

fn main() {
    launch_app("brahman · mirada", (SCREEN_W as f32, (SCREEN_H + 70) as f32), Mirada::new);
}
