//! Demo interactiva del Tier 1+2: máquina de estados de animación (`llimphi-anim`)
//! manejando dos clips Lottie con **crossfade**, manejada por un input vivo.
//!
//! Dos estados: `idle` (círculo azul que late) ⇄ `walk` (cuadrado naranja que
//! gira), con un blend de 0.35 s entre ellos. El input booleano `moving` dispara
//! las transiciones en ambos sentidos. **Espacio** o el botón lo togglean —
//! ves el crossfade disolver una animación en la otra.
//!
//! Esto es el cableado canónico inputs↔app (Tier 2): el `Model` tiene una
//! `Instance`, un `spawn_periodic` la avanza por frame (`advance(dt)`), los
//! eventos setean inputs (`set_bool`), y `view` pinta `render_frame()` vía
//! `state_machine_view`. El núcleo de estados no sabe de Lottie ni de vello.
//!
//! Corre con:
//!   `cargo run -p llimphi-lottie --example state_machine_demo --release`

use std::time::Duration;

use llimphi_anim::{Condition, StateMachine};
use llimphi_lottie::{state_machine_view, LottieAsset};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};

// ClipId 0 = idle, 1 = walk.
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
    /// Tick de animación: avanza la máquina por `dt`.
    Tick,
    /// Togglea el input `moving` (Espacio o el botón).
    ToggleMoving,
}

struct Model {
    inst: llimphi_anim::Instance,
    clips: Vec<LottieAsset>,
    moving: bool,
}

struct Demo;

/// Período del tick → también el `dt` que le pasamos a `advance` (fijo, alcanza
/// para una demo; una app real mediría el dt real).
const TICK: Duration = Duration::from_millis(16);

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · state machine + crossfade"
    }

    fn initial_size() -> (u32, u32) {
        (420, 520)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let idle = LottieAsset::from_str(IDLE_LOTTIE).expect("idle lottie");
        let walk = LottieAsset::from_str(WALK_LOTTIE).expect("walk lottie");

        let mut sm = StateMachine::new();
        let s_idle = sm.add_state("idle", IDLE, 1.0, true);
        let s_walk = sm.add_state("walk", WALK, 1.0, true);
        sm.set_entry(s_idle);
        // El mismo input `moving` rige ambos sentidos, con blend de 0.35 s.
        sm.transition(s_idle, s_walk, vec![Condition::bool("moving", true)], 0.35);
        sm.transition(s_walk, s_idle, vec![Condition::bool("moving", false)], 0.35);

        // Corre el reloj de animación (~60 fps).
        handle.spawn_periodic(TICK, || Msg::Tick);

        Model {
            inst: sm.instance(),
            clips: vec![idle, walk],
            moving: false,
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => model.inst.advance(TICK.as_secs_f64()),
            Msg::ToggleMoving => {
                model.moving = !model.moving;
                model.inst.set_bool("moving", model.moving);
            }
        }
        model
    }

    fn on_key(_: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state == KeyState::Pressed && e.key == Key::Named(NamedKey::Space) {
            Some(Msg::ToggleMoving)
        } else {
            None
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        // Caja cuadrada que aloja la animación (state_machine_view es absolute-fill).
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
        .children(vec![state_machine_view::<Msg>(
            model.inst.render_frame(),
            model.clips.clone(),
        )]);

        let estado = if model.inst.is_transitioning() {
            "· · · crossfade · · ·".to_string()
        } else {
            format!("estado: {}", model.inst.current_state())
        };
        let status = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(estado, 18.0, Color::from_rgba8(180, 200, 230, 255));

        let label = if model.moving {
            "moving = true   (Espacio para parar)"
        } else {
            "moving = false   (Espacio para mover)"
        };
        let boton = View::new(Style {
            size: Size {
                width: length(280.0_f32),
                height: length(52.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(if model.moving {
            Color::from_rgba8(200, 120, 50, 255)
        } else {
            Color::from_rgba8(50, 110, 190, 255)
        })
        .radius(12.0)
        .text(label.to_string(), 16.0, Color::from_rgba8(245, 245, 250, 255))
        .on_click(Msg::ToggleMoving);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(20.0_f32),
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
        .children(vec![stage, status, boton])
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
