//! Binary entry point del launcher Llimphi.

use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};

use mirada_launcher_llimphi::config::Config;
use mirada_launcher_llimphi::panel;
use mirada_launcher_llimphi::widget::{Msg, Widget};
use mirada_launcher_llimphi::widgets;
use mirada_launcher_llimphi::widgets::quake::QuakeInput;

struct Model {
    theme: Theme,
    cfg: Config,
    left: Vec<Box<dyn Widget>>,
    center: Vec<Box<dyn Widget>>,
    right: Vec<Box<dyn Widget>>,
}

impl Model {
    /// Recorre los tres slots dando acceso mutable a cada widget — uso
    /// fundamental: `tick` periódico + rutear `Msg`s al quake.
    fn each_widget_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Widget>> {
        self.left
            .iter_mut()
            .chain(self.center.iter_mut())
            .chain(self.right.iter_mut())
    }

    fn route_to_quake(&mut self, msg: &Msg) {
        for w in self.each_widget_mut() {
            if let Some(q) = w.as_any_mut().downcast_mut::<QuakeInput>() {
                q.apply(msg);
            }
        }
    }
}

struct LauncherApp;

impl App for LauncherApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str { "mirada-launcher" }

    fn app_id() -> Option<&'static str> { Some("mirada.launcher") }

    fn initial_size() -> (u32, u32) { (1280, 36) }

    fn init(handle: &Handle<Msg>) -> Model {
        let cfg = Config::load_or_default();
        let left = cfg.panel.left.iter().map(widgets::build).collect();
        let center = cfg.panel.center.iter().map(widgets::build).collect();
        let right = cfg.panel.right.iter().map(widgets::build).collect();

        handle.spawn_periodic(Duration::from_secs(1), || Msg::Tick);

        Model { theme: Theme::dark(), cfg, left, center, right }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match &msg {
            Msg::Tick => {
                for w in model.each_widget_mut() {
                    w.tick();
                }
            }
            Msg::Quit => handle.quit(),
            Msg::QuakeToggle
            | Msg::QuakeChar(_)
            | Msg::QuakeBackspace
            | Msg::QuakeSubmit => {
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
        )
    }

    fn on_key(_model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        match &event.key {
            Key::Named(NamedKey::F12) => Some(Msg::QuakeToggle),
            Key::Named(NamedKey::Escape) => Some(Msg::Quit),
            Key::Named(NamedKey::Backspace) => Some(Msg::QuakeBackspace),
            Key::Named(NamedKey::Enter) => Some(Msg::QuakeSubmit),
            Key::Character(s) => s.chars().next().map(Msg::QuakeChar),
            _ => None,
        }
    }
}

fn main() {
    llimphi_ui::run::<LauncherApp>();
}
