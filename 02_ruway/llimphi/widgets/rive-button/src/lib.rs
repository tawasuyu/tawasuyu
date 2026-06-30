//! `llimphi-widget-rive-button` — botón **reactivo** dirigido por una máquina de
//! estados estilo Rive ([`llimphi_anim::StateMachine`]).
//!
//! Es el primer consumidor *real* de la máquina de estados fuera del
//! `llimphi-anim-studio` (que la autora) y de los examples de `llimphi-lottie`.
//! Encapsula la composición canónica del demo `pointer_listeners_demo` como un
//! widget reusable: tres estados —`idle` / `hover` / `press`— cada uno con su
//! clip Lottie, hilados por inputs que el motor deriva del puntero crudo:
//!
//! - **hover** (`bool`): los listeners `Enter`/`Exit` sobre toda la superficie
//!   prenden/apagan el input `hovered`; `idle ⇄ hover` con crossfade.
//! - **press** (`trigger`): el host llama [`RiveButton::press`] desde su
//!   `on_click`; dispara el trigger `pressed`, la máquina entra al estado `press`
//!   (clip **no-loop**) y, al terminar el clip (`ClipDone`), vuelve a `hover` o
//!   `idle` según siga el puntero encima.
//!
//! El widget es **stateful** (como `text-editor`/`nodegraph`): el host lo guarda
//! en su `Model`, lo avanza con [`advance`] desde un tick periódico, le reenvía
//! el puntero con [`pointer`] y el click con [`press`], y lo pinta con [`view`].
//! El hit-testing vive en el motor, no en la app.
//!
//! ```ignore
//! // En el Model del host:
//! boton: RiveButton::builtin(),
//! // update:
//! Msg::Tick      => m.boton.advance(DT),
//! Msg::Pointer(p)=> m.boton.pointer(p),
//! Msg::Click     => { m.boton.press(); /* acción del host */ }
//! // view:
//! m.boton.view(Msg::Pointer, Msg::Click)
//! ```
//!
//! [`advance`]: RiveButton::advance
//! [`pointer`]: RiveButton::pointer
//! [`press`]: RiveButton::press
//! [`view`]: RiveButton::view

#![forbid(unsafe_code)]

use llimphi_anim::{Action, Area, Condition, Instance, PointerTrigger, StateMachine};
use llimphi_lottie::{state_machine_view, LottieAsset};
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::View;

/// id de clip = índice en el `Vec<LottieAsset>` que se pasa a `state_machine_view`.
const IDLE: u32 = 0;
const HOVER: u32 = 1;
const PRESS: u32 = 2;

/// Botón reactivo: la máquina de estados viva + sus tres clips.
pub struct RiveButton {
    inst: Instance,
    clips: Vec<LottieAsset>,
}

impl RiveButton {
    /// Construye el botón a partir de tres clips Lottie propios.
    ///
    /// - `idle` / `hover` / `press`: los clips de cada estado.
    /// - `press_secs`: duración del clip de `press` (segundos) — la condición
    ///   `ClipDone` la usa para devolver el estado a `hover`/`idle`.
    /// - `blend_secs`: crossfade entre estados (`0.0` = salto seco).
    pub fn new(
        idle: LottieAsset,
        hover: LottieAsset,
        press: LottieAsset,
        press_secs: f64,
        blend_secs: f64,
    ) -> Self {
        let mut sm = StateMachine::new();
        let s_idle = sm.add_state("idle", IDLE, 1.0, true);
        let s_hover = sm.add_state("hover", HOVER, 1.0, true);
        let s_press = sm.add_state("press", PRESS, 1.0, false);
        sm.set_clip_duration(PRESS, press_secs);
        sm.set_entry(s_idle);

        // Hover: idle ⇄ hover según el bool `hovered`.
        sm.transition(s_idle, s_hover, vec![Condition::bool("hovered", true)], blend_secs);
        sm.transition(s_hover, s_idle, vec![Condition::bool("hovered", false)], blend_secs);
        // Press: desde cualquier estado, el trigger `pressed` entra a `press`
        // (blend corto para que el pop se sienta inmediato).
        sm.transition_any(s_press, vec![Condition::trigger("pressed")], blend_secs.min(0.06));
        // Salida de press al terminar el clip: vuelve a donde corresponda según
        // el puntero (sin loops: `press` es no-loop, así `ClipDone` aplica).
        sm.transition(s_press, s_hover, vec![Condition::clip_done(), Condition::bool("hovered", true)], blend_secs);
        sm.transition(s_press, s_idle, vec![Condition::clip_done(), Condition::bool("hovered", false)], blend_secs);

        // Listeners: el motor traduce el puntero crudo → inputs (Tier 3).
        sm.listener(Area::All, PointerTrigger::Enter, Action::set_bool("hovered", true));
        sm.listener(Area::All, PointerTrigger::Exit, Action::set_bool("hovered", false));

        Self { inst: sm.instance(), clips: vec![idle, hover, press] }
    }

    /// Botón listo para usar con clips Lottie embebidos (círculo que late en
    /// reposo, gira en hover y da un cuarto de giro al apretarlo). Útil para
    /// demos, tests y como placeholder hasta que la app traiga sus propios clips.
    pub fn builtin() -> Self {
        let idle = LottieAsset::from_str(IDLE_LOTTIE).expect("idle lottie embebido válido");
        let hover = LottieAsset::from_str(HOVER_LOTTIE).expect("hover lottie embebido válido");
        let press = LottieAsset::from_str(PRESS_LOTTIE).expect("press lottie embebido válido");
        // press = 12 frames @ 30 fps = 0.4 s.
        Self::new(idle, hover, press, 0.4, 0.18)
    }

    /// Avanza la animación `dt` segundos. Llamar desde un tick periódico del host
    /// (`Handle::spawn_periodic`) mientras el botón esté visible.
    pub fn advance(&mut self, dt: f64) {
        self.inst.advance(dt);
    }

    /// Reenvía el puntero crudo en coords normalizadas `0..1` sobre el rect del
    /// botón (o `None` si salió). Los listeners del motor derivan `hovered`.
    pub fn pointer(&mut self, pos: Option<(f64, f64)>) {
        self.inst.pointer_move(pos);
    }

    /// Dispara la animación de press (típico: desde el `on_click` del host). El
    /// host hace además su propia acción de click; el widget sólo anima.
    pub fn press(&mut self) {
        self.inst.fire("pressed");
    }

    /// Nombre del estado actual (`"idle"` / `"hover"` / `"press"`; el de origen
    /// mientras hay crossfade).
    pub fn current_state(&self) -> &str {
        self.inst.current_state()
    }

    /// ¿Hay un crossfade entre estados en curso?
    pub fn is_transitioning(&self) -> bool {
        self.inst.is_transitioning()
    }

    /// La vista del botón: pinta el frame de la máquina (con crossfade) y reenvía
    /// puntero + click como mensajes del host.
    ///
    /// - `on_pointer`: mapea la posición normalizada `0..1` (o `None` al salir) a
    ///   un `Msg`; el host lo rutea a [`pointer`](RiveButton::pointer).
    /// - `on_click`: `Msg` emitido al click; el host debe llamar
    ///   [`press`](RiveButton::press) al recibirlo (además de su propia acción).
    pub fn view<Msg, F>(&self, on_pointer: F, on_click: Msg) -> View<Msg>
    where
        Msg: Clone + 'static,
        F: Fn(Option<(f64, f64)>) -> Msg + Clone + Send + Sync + 'static,
    {
        let leave = on_pointer.clone();
        View::new(fill())
            .children(vec![state_machine_view::<Msg>(self.inst.render_frame(), self.clips.clone())])
            .on_pointer_move_at(move |lx, ly, w, h| {
                if w > 0.0 && h > 0.0 {
                    Some(on_pointer(Some((lx as f64 / w as f64, ly as f64 / h as f64))))
                } else {
                    None
                }
            })
            .on_pointer_leave(leave(None))
            .on_click(on_click)
    }
}

/// "Ocupa todo el rect del padre" — el caller envuelve esto en un contenedor con
/// el tamaño que necesite.
fn fill() -> Style {
    Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    }
}

/// idle: círculo azul cuya opacidad late (100→55→100) en 2 s.
const IDLE_LOTTIE: &str = r#"{
  "v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,
  "layers":[{"ty":4,"ip":0,"op":60,"st":0,"sr":1,
    "ks":{"o":{"a":1,"k":[
        {"i":{"x":[0.5],"y":[0.5]},"o":{"x":[0.5],"y":[0.5]},"t":0,"s":[100]},
        {"i":{"x":[0.5],"y":[0.5]},"o":{"x":[0.5],"y":[0.5]},"t":30,"s":[55]},
        {"t":60,"s":[100]}]},
      "r":{"a":0,"k":0},"p":{"a":0,"k":[50,50]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]}},
    "shapes":[{"ty":"gr","it":[
      {"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[64,64]}},
      {"ty":"fl","c":{"a":0,"k":[0.30,0.55,0.95]},"o":{"a":0,"k":100}},
      {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}]}]}]}"#;

/// hover: círculo verde que gira 360° en 2 s (loop).
const HOVER_LOTTIE: &str = r#"{
  "v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,
  "layers":[{"ty":4,"ip":0,"op":60,"st":0,"sr":1,
    "ks":{"o":{"a":0,"k":100},
      "r":{"a":1,"k":[
        {"i":{"x":[0.5],"y":[0.5]},"o":{"x":[0.5],"y":[0.5]},"t":0,"s":[0]},
        {"t":60,"s":[360]}]},
      "p":{"a":0,"k":[50,50]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]}},
    "shapes":[{"ty":"gr","it":[
      {"ty":"rc","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[60,60]},"r":{"a":0,"k":12}},
      {"ty":"fl","c":{"a":0,"k":[0.30,0.80,0.45]},"o":{"a":0,"k":100}},
      {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}]}]}]}"#;

/// press: cuadrado naranja que da un cuarto de giro rápido (0→90° en 0,4 s), no-loop.
const PRESS_LOTTIE: &str = r#"{
  "v":"5.5.2","fr":30,"ip":0,"op":12,"w":100,"h":100,
  "layers":[{"ty":4,"ip":0,"op":12,"st":0,"sr":1,
    "ks":{"o":{"a":0,"k":100},
      "r":{"a":1,"k":[
        {"i":{"x":[0.5],"y":[0.5]},"o":{"x":[0.5],"y":[0.5]},"t":0,"s":[0]},
        {"t":12,"s":[90]}]},
      "p":{"a":0,"k":[50,50]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]}},
    "shapes":[{"ty":"gr","it":[
      {"ty":"rc","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[66,66]},"r":{"a":0,"k":14}},
      {"ty":"fl","c":{"a":0,"k":[0.95,0.55,0.15]},"o":{"a":0,"k":100}},
      {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}]}]}]}"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// El ciclo completo idle → hover → press → hover → idle dirigido sólo por
    /// puntero y click, certificando la máquina sin GPU.
    #[test]
    fn ciclo_hover_press_certificado() {
        let mut rb = RiveButton::builtin();
        assert_eq!(rb.current_state(), "idle");

        // Puntero al centro → Enter prende `hovered` → idle→hover (tras el blend).
        rb.pointer(Some((0.5, 0.5)));
        rb.advance(0.5);
        assert_eq!(rb.current_state(), "hover", "el hover no entró con el puntero encima");

        // Click → trigger `pressed` → estado press (clip no-loop).
        rb.press();
        rb.advance(0.2);
        assert_eq!(rb.current_state(), "press", "el click no disparó el press");

        // El clip de press (0,4 s) termina → ClipDone → vuelve a hover (sigue encima).
        rb.advance(0.6);
        assert_eq!(rb.current_state(), "hover", "press no volvió a hover al terminar el clip");

        // Puntero fuera → Exit apaga `hovered` → hover→idle.
        rb.pointer(None);
        rb.advance(0.5);
        assert_eq!(rb.current_state(), "idle", "el botón no volvió a idle al salir el puntero");
    }

    /// Un press estando fuera (sin hover) vuelve a `idle`, no a `hover`.
    #[test]
    fn press_sin_hover_vuelve_a_idle() {
        let mut rb = RiveButton::builtin();
        rb.press();
        rb.advance(0.2);
        assert_eq!(rb.current_state(), "press");
        rb.advance(0.6);
        assert_eq!(rb.current_state(), "idle", "sin puntero encima, press debe volver a idle");
    }

    /// La vista se construye sin panic con un Msg real (humo del cableado de hooks).
    #[test]
    fn view_se_construye() {
        #[derive(Clone)]
        enum Msg {
            Pointer(Option<(f64, f64)>),
            Click,
        }
        let rb = RiveButton::builtin();
        let _v = rb.view(Msg::Pointer, Msg::Click);
    }
}
