//! Binary entry point del launcher Llimphi.

use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};

use mirada_launcher_llimphi::config::{Config, FloatingCard};
use mirada_launcher_llimphi::panel;
use mirada_launcher_llimphi::widget::{Msg, Widget};
use mirada_launcher_llimphi::widgets;
use mirada_launcher_llimphi::widgets::clock::TzMode;
use mirada_launcher_llimphi::widgets::quake::{ask_ia_blocking, QuakeInput, SubmitKind};

struct Model {
    theme: Theme,
    cfg: Config,
    left: Vec<Box<dyn Widget>>,
    center: Vec<Box<dyn Widget>>,
    right: Vec<Box<dyn Widget>>,
    /// Tarjetas flotantes (esquemas + widgets vivos en paralelo).
    floating: Vec<(FloatingCard, Vec<Box<dyn Widget>>)>,
}

impl Model {
    fn each_widget_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Widget>> {
        self.left
            .iter_mut()
            .chain(self.center.iter_mut())
            .chain(self.right.iter_mut())
            .chain(self.floating.iter_mut().flat_map(|(_, ws)| ws.iter_mut()))
    }

    fn each_widget(&self) -> impl Iterator<Item = &Box<dyn Widget>> {
        self.left
            .iter()
            .chain(self.center.iter())
            .chain(self.right.iter())
            .chain(self.floating.iter().flat_map(|(_, ws)| ws.iter()))
    }

    fn route_to_quake(&mut self, msg: &Msg) {
        for w in self.each_widget_mut() {
            if let Some(q) = w.as_any_mut().downcast_mut::<QuakeInput>() {
                q.apply(msg);
            }
        }
    }

    /// `true` si algún `QuakeInput` está abierto. Cuando lo está, Esc
    /// lo cierra; cuando no, Esc cierra la app.
    fn quake_open(&self) -> bool {
        self.each_widget().any(|w| {
            w.as_any()
                .downcast_ref::<QuakeInput>()
                .map(|q| q.open)
                .unwrap_or(false)
        })
    }
}

struct LauncherApp;

impl App for LauncherApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str { "mirada-launcher" }

    fn app_id() -> Option<&'static str> { Some("mirada.launcher") }

    fn initial_size() -> (u32, u32) { (1280, 720) }

    fn init(handle: &Handle<Msg>) -> Model {
        let cfg = Config::load_or_default();
        let ctx = widgets::BuildCtx {
            tz: TzMode::from_config(&cfg.general.timezone),
        };
        let left = cfg.panel.left.iter().map(|s| widgets::build(s, &ctx)).collect();
        let center = cfg.panel.center.iter().map(|s| widgets::build(s, &ctx)).collect();
        let right = cfg.panel.right.iter().map(|s| widgets::build(s, &ctx)).collect();
        let floating = cfg
            .panel
            .floating
            .iter()
            .map(|card| {
                let ws = card.widgets.iter().map(|s| widgets::build(s, &ctx)).collect();
                (card.clone(), ws)
            })
            .collect();

        handle.spawn_periodic(Duration::from_secs(1), || Msg::Tick);

        Model { theme: Theme::dark(), cfg, left, center, right, floating }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match &msg {
            Msg::Tick => {
                for w in model.each_widget_mut() {
                    w.tick();
                }
            }
            Msg::Quit => handle.quit(),
            Msg::QuakeSubmit => {
                // Tomamos posesión del buffer del primer quake abierto.
                let mut taken: Option<String> = None;
                for w in model.each_widget_mut() {
                    if let Some(q) = w.as_any_mut().downcast_mut::<QuakeInput>() {
                        if q.open && !q.buffer.is_empty() {
                            taken = Some(std::mem::take(&mut q.buffer));
                            break;
                        }
                    }
                }
                if let Some(buffer) = taken {
                    match QuakeInput::classify(&buffer) {
                        SubmitKind::Empty => {}
                        SubmitKind::Shell(cmd) => {
                            let exec = cmd.to_string();
                            let status = std::process::Command::new("sh")
                                .arg("-c")
                                .arg(&exec)
                                .spawn()
                                .map(|_| ());
                            for w in model.each_widget_mut() {
                                if let Some(q) = w.as_any_mut().downcast_mut::<QuakeInput>() {
                                    if q.open {
                                        q.set_shell_result(&exec, status);
                                        break;
                                    }
                                }
                            }
                        }
                        SubmitKind::Ia(_) => {
                            for w in model.each_widget_mut() {
                                if let Some(q) = w.as_any_mut().downcast_mut::<QuakeInput>() {
                                    if q.open {
                                        q.mark_pending();
                                        break;
                                    }
                                }
                            }
                            let prompt = buffer;
                            handle.spawn(move || Msg::QuakeIaResult(ask_ia_blocking(&prompt)));
                        }
                    }
                }
            }
            Msg::QuakeToggle
            | Msg::QuakeChar(_)
            | Msg::QuakeBackspace
            | Msg::QuakeIaResult(_) => {
                model.route_to_quake(&msg);
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        panel::build(
            &model.cfg.panel,
            &model.theme,
            &model.left,
            &model.center,
            &model.right,
            &model.floating,
        )
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        panel::overlay_view(&model.theme, model.each_widget())
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // 1) hotkeys declarados por los widgets (quake_input.hotkey, etc.)
        for w in model.each_widget() {
            if let Some(msg) = w.try_key(event) {
                return Some(msg);
            }
        }
        // 2) routing implícito del input quake mientras esté abierto.
        // Como no chequeamos estado aquí, dejamos que `route_to_quake`
        // lo filtre: si no está abierto, el Msg llega y el quake lo
        // ignora.
        // 2) si el quake está abierto, las teclas estándar van a él.
        //    Si no, Esc cierra la app (Backspace/Enter quedan inertes).
        let quake_open = model.quake_open();
        match &event.key {
            Key::Named(NamedKey::Escape) if quake_open => Some(Msg::QuakeToggle),
            Key::Named(NamedKey::Escape) => Some(Msg::Quit),
            Key::Named(NamedKey::Backspace) if quake_open => Some(Msg::QuakeBackspace),
            Key::Named(NamedKey::Enter) if quake_open => Some(Msg::QuakeSubmit),
            Key::Character(s) if quake_open => s.chars().next().map(Msg::QuakeChar),
            _ => None,
        }
    }
}

fn main() {
    llimphi_ui::run::<LauncherApp>();
}
