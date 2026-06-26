//! Demo del Tier 3: **listeners de puntero** en la máquina de estados.
//!
//! El hit-testing vive **en el motor** (`llimphi-anim`), no en la app: la app
//! sólo reenvía el puntero crudo (`pointer_move`), y los `Listener` del estado
//! deciden qué input tocar. Acá, dos listeners sobre toda la animación —
//! `Enter → moving=true`, `Exit → moving=false` — hacen que **pasar el puntero
//! por encima** dispare el crossfade idle→walk, y sacarlo lo revierta. Sin
//! teclado: el puntero es la interacción.
//!
//! Corre con:
//!   `cargo run -p llimphi-lottie --example pointer_listeners_demo --release`

use std::time::Duration;

use llimphi_anim::{Action, Area, Condition, PointerTrigger, StateMachine};
use llimphi_lottie::{state_machine_view, LottieAsset};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, View};

const IDLE: u32 = 0;
const WALK: u32 = 1;

/// idle: círculo azul cuya opacidad late (100→35→100) en 2 s.
const IDLE_LOTTIE: &str = r#"{
  "v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,
  "layers":[{"ty":4,"ip":0,"op":60,"st":0,"sr":1,
    "ks":{"o":{"a":1,"k":[
        {"i":{"x":[0.5],"y":[0.5]},"o":{"x":[0.5],"y":[0.5]},"t":0,"s":[100]},
        {"i":{"x":[0.5],"y":[0.5]},"o":{"x":[0.5],"y":[0.5]},"t":30,"s":[35]},
        {"t":60,"s":[100]}]},
      "r":{"a":0,"k":0},"p":{"a":0,"k":[50,50]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]}},
    "shapes":[{"ty":"gr","it":[
      {"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[72,72]}},
      {"ty":"fl","c":{"a":0,"k":[0.30,0.55,0.95]},"o":{"a":0,"k":100}},
      {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}]}]}]}"#;

/// walk: cuadrado naranja redondeado que gira 360° en 2 s.
const WALK_LOTTIE: &str = r#"{
  "v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,
  "layers":[{"ty":4,"ip":0,"op":60,"st":0,"sr":1,
    "ks":{"o":{"a":0,"k":100},
      "r":{"a":1,"k":[
        {"i":{"x":[0.5],"y":[0.5]},"o":{"x":[0.5],"y":[0.5]},"t":0,"s":[0]},
        {"t":60,"s":[360]}]},
      "p":{"a":0,"k":[50,50]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]}},
    "shapes":[{"ty":"gr","it":[
      {"ty":"rc","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[60,60]},"r":{"a":0,"k":10}},
      {"ty":"fl","c":{"a":0,"k":[0.95,0.55,0.15]},"o":{"a":0,"k":100}},
      {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}]}]}]}"#;

#[derive(Clone)]
enum Msg {
    Tick,
    /// Puntero sobre la animación, en coords normalizadas `0..1` (`None` = salió).
    Pointer(Option<(f64, f64)>),
}

struct Model {
    inst: llimphi_anim::Instance,
    clips: Vec<LottieAsset>,
}

struct Demo;

const TICK: Duration = Duration::from_millis(16);

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · pointer listeners"
    }

    fn initial_size() -> (u32, u32) {
        (420, 500)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let idle = LottieAsset::from_str(IDLE_LOTTIE).expect("idle lottie");
        let walk = LottieAsset::from_str(WALK_LOTTIE).expect("walk lottie");

        let mut sm = StateMachine::new();
        let s_idle = sm.add_state("idle", IDLE, 1.0, true);
        let s_walk = sm.add_state("walk", WALK, 1.0, true);
        sm.set_entry(s_idle);
        sm.transition(s_idle, s_walk, vec![Condition::bool("moving", true)], 0.3);
        sm.transition(s_walk, s_idle, vec![Condition::bool("moving", false)], 0.3);

        // Tier 3: el hover sobre toda la animación maneja el input `moving`.
        sm.listener(Area::All, PointerTrigger::Enter, Action::set_bool("moving", true));
        sm.listener(Area::All, PointerTrigger::Exit, Action::set_bool("moving", false));

        handle.spawn_periodic(TICK, || Msg::Tick);

        Model {
            inst: sm.instance(),
            clips: vec![idle, walk],
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => model.inst.advance(TICK.as_secs_f64()),
            // La app sólo reenvía el puntero crudo; los listeners del motor
            // deciden qué input tocar.
            Msg::Pointer(p) => model.inst.pointer_move(p),
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let stage = View::new(Style {
            size: Size {
                width: length(280.0_f32),
                height: length(280.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .radius(16.0)
        .fill(Color::from_rgba8(26, 30, 40, 255))
        .children(vec![state_machine_view::<Msg>(
            model.inst.render_frame(),
            model.clips.clone(),
        )])
        // Cada movimiento del cursor sobre el stage → posición normalizada.
        .on_pointer_move_at(|lx, ly, w, h| {
            if w > 0.0 && h > 0.0 {
                Some(Msg::Pointer(Some((lx as f64 / w as f64, ly as f64 / h as f64))))
            } else {
                None
            }
        })
        // El cursor salió del stage → fuera de toda área.
        .on_pointer_leave(Msg::Pointer(None));

        let estado = if model.inst.is_transitioning() {
            "· · · crossfade · · ·".to_string()
        } else {
            format!("estado: {}", model.inst.current_state())
        };
        let status = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(26.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(estado, 17.0, Color::from_rgba8(180, 200, 230, 255));

        let hint = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(22.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(
            "pasá el puntero por encima".to_string(),
            14.0,
            Color::from_rgba8(120, 135, 160, 255),
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(16.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            padding: Rect {
                left: length(24.0_f32),
                right: length(24.0_f32),
                top: length(24.0_f32),
                bottom: length(24.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(18, 22, 30, 255))
        .children(vec![stage, status, hint])
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
