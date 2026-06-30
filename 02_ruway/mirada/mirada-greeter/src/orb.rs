//! Orbe de estado de la tarjeta de login — un indicador chico dirigido por la
//! **máquina de estados** de `llimphi-anim` (estilo Rive), enganchado a las
//! señales **reales** del greeter (foco/typing, autenticación, error, éxito).
//!
//! Es un consumidor real de `llimphi_anim::StateMachine` en producción: el
//! greeter setea los inputs desde su `Model` en cada `RainTick` y el orbe
//! transiciona `idle → typing → auth → error/ok`. El render es **procedural**
//! (sin clips Lottie): cada estado pinta una figura derivada de su nombre + un
//! reloj propio, así no suma assets a la pantalla de login.

use llimphi_anim::{Condition, Instance, StateMachine};
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Arc, Cap, Circle, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::View;

// ClipId = índice del estado (no se usan clips reales: el render es procedural).
const IDLE: u32 = 0;
const TYPING: u32 = 1;
const AUTH: u32 = 2;
const ERROR: u32 = 3;
const OK: u32 = 4;

/// Duración de los estados no-loop (segundos): la sacudida de error y el pop ok.
const PULSE_SECS: f64 = 0.6;

/// El orbe: la máquina viva + relojes para los efectos procedurales.
pub struct StatusOrb {
    inst: Instance,
    /// Reloj global (segundos) — late/gira en idle/typing/auth.
    phase: f64,
    /// Tiempo en el estado actual — la sacudida de error y el pop ok decaen con él.
    state_clock: f64,
    /// Estado cacheado (índice) para detectar el cambio que resetea `state_clock`.
    cur: u32,
    /// Flanco de subida de `failed` → dispara el estado de error una sola vez.
    last_failed: bool,
}

impl Default for StatusOrb {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusOrb {
    pub fn new() -> Self {
        let mut sm = StateMachine::new();
        let idle = sm.add_state("idle", IDLE, 1.0, true);
        let typing = sm.add_state("typing", TYPING, 1.0, true);
        let auth = sm.add_state("auth", AUTH, 1.0, true);
        let error = sm.add_state("error", ERROR, 1.0, false);
        let ok = sm.add_state("ok", OK, 1.0, false);
        sm.set_clip_duration(ERROR, PULSE_SECS);
        sm.set_clip_duration(OK, PULSE_SECS);
        sm.set_entry(idle);

        let b = |n: &str, v: bool| Condition::bool(n, v);
        // Foco/typing: idle ⇄ typing.
        sm.transition(idle, typing, vec![b("typing", true)], 0.15);
        sm.transition(typing, idle, vec![b("typing", false)], 0.15);
        // Autenticando: desde cualquier estado entra a auth; al terminar vuelve a idle.
        sm.transition_any(auth, vec![b("auth", true)], 0.15);
        sm.transition(auth, idle, vec![b("auth", false)], 0.15);
        // Error: trigger desde cualquier estado; al terminar la sacudida vuelve.
        sm.transition_any(error, vec![Condition::trigger("error")], 0.08);
        sm.transition(error, typing, vec![Condition::clip_done(), b("typing", true)], 0.15);
        sm.transition(error, idle, vec![Condition::clip_done(), b("typing", false)], 0.15);
        // Éxito: pop verde y vuelve a idle (el greeter normalmente cierra antes).
        sm.transition_any(ok, vec![Condition::trigger("ok")], 0.08);
        sm.transition(ok, idle, vec![Condition::clip_done()], 0.2);

        Self { inst: sm.instance(), phase: 0.0, state_clock: 0.0, cur: IDLE, last_failed: false }
    }

    /// Engancha las señales reales del greeter. `typing` = el campo focuseado
    /// tiene texto; `authenticating` = está validando; `failed` = el último
    /// intento falló (su **flanco de subida** dispara la sacudida de error).
    pub fn sync(&mut self, typing: bool, authenticating: bool, failed: bool) {
        self.inst.set_bool("typing", typing);
        self.inst.set_bool("auth", authenticating);
        if failed && !self.last_failed {
            self.inst.fire("error");
        }
        self.last_failed = failed;
    }

    /// Éxito de autenticación (pop verde). Best-effort: el greeter cierra al validar.
    pub fn signal_ok(&mut self) {
        self.inst.fire("ok");
    }

    /// Avanza la máquina + relojes `dt` segundos (desde `Msg::RainTick`).
    pub fn advance(&mut self, dt: f64) {
        self.inst.advance(dt);
        self.phase += dt;
        let cur = clip_of(self.inst.current_state());
        if cur != self.cur {
            self.cur = cur;
            self.state_clock = 0.0;
        } else {
            self.state_clock += dt;
        }
    }

    /// El nombre del estado actual — para que el greeter rotule/teste sin exponer índices.
    pub fn estado(&self) -> &str {
        self.inst.current_state()
    }

    /// El orbe como un bloque de alto fijo (centrado) que pinta según el estado.
    /// `accent` tiñe los estados normales; `error_color` la sacudida de error.
    pub fn view<Msg: 'static>(&self, accent: Color, error_color: Color) -> View<Msg> {
        let (cur, phase, state_clock) = (self.cur, self.phase, self.state_clock);
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
            ..Default::default()
        })
        .paint_with(move |scene, _ts, rect| {
            let cx = rect.x as f64 + rect.w as f64 * 0.5;
            let cy = rect.y as f64 + rect.h as f64 * 0.5;
            let base = (rect.w.min(rect.h) as f64) * 0.5 * 0.7;
            if base <= 0.0 {
                return;
            }
            let fill = |scene: &mut llimphi_ui::llimphi_raster::vello::Scene, c: Color, r: f64, dx: f64| {
                scene.fill(Fill::NonZero, Affine::IDENTITY, c, None, &Circle::new((cx + dx, cy), r));
            };
            match cur {
                TYPING => {
                    // Núcleo + anillo que late rápido al teclear.
                    fill(scene, accent, base * 0.72, 0.0);
                    let ring = base * (1.05 + 0.4 * ((phase * 5.0).sin() * 0.5 + 0.5));
                    scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, accent, None, &Circle::new((cx, cy), ring));
                }
                AUTH => {
                    // Spinner: arco de 270° que gira (mismo patrón que el widget spinner).
                    let theta0 = phase * std::f64::consts::TAU;
                    let arc = Arc::new((cx, cy), (base, base), theta0, std::f64::consts::PI * 1.5, 0.0);
                    let stroke = Stroke::new((base * 0.24).max(2.0)).with_caps(Cap::Round);
                    scene.stroke(&stroke, Affine::IDENTITY, accent, None, &arc);
                }
                ERROR => {
                    // Sacudida horizontal que decae sobre la duración del estado.
                    let decay = (1.0 - state_clock / PULSE_SECS).clamp(0.0, 1.0);
                    let dx = (state_clock * 42.0).sin() * base * 0.55 * decay;
                    fill(scene, error_color, base, dx);
                }
                OK => {
                    // Pop verde: núcleo + anillo que se expande y se desvanece.
                    let green = Color::from_rgba8(70, 200, 120, 255);
                    fill(scene, green, base, 0.0);
                    let t = (state_clock / PULSE_SECS).clamp(0.0, 1.0);
                    let ring = base * (1.0 + 1.4 * t);
                    let a = (200.0 * (1.0 - t)) as u8;
                    scene.stroke(&Stroke::new(3.0), Affine::IDENTITY, Color::from_rgba8(70, 200, 120, a), None, &Circle::new((cx, cy), ring));
                }
                _ => {
                    // idle: latido suave.
                    let r = base * (1.0 + 0.07 * (phase * 2.0).sin());
                    fill(scene, accent, r, 0.0);
                }
            }
        })
    }
}

fn clip_of(name: &str) -> u32 {
    match name {
        "typing" => TYPING,
        "auth" => AUTH,
        "error" => ERROR,
        "ok" => OK,
        _ => IDLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_y_auth_y_error_transicionan() {
        let mut orb = StatusOrb::new();
        assert_eq!(orb.estado(), "idle");
        // teclear → typing
        orb.sync(true, false, false);
        orb.advance(0.3);
        assert_eq!(orb.estado(), "typing");
        // validar → auth (cualquier estado)
        orb.sync(true, true, false);
        orb.advance(0.3);
        assert_eq!(orb.estado(), "auth");
        // fallo → error (flanco), y la sacudida termina volviendo a typing (sigue con texto)
        orb.sync(true, false, true);
        orb.advance(0.2);
        assert_eq!(orb.estado(), "error");
        orb.advance(PULSE_SECS + 0.1);
        assert_eq!(orb.estado(), "typing");
    }

    #[test]
    fn error_solo_dispara_en_el_flanco() {
        let mut orb = StatusOrb::new();
        // failed sostenido en true no debe re-disparar error cada frame.
        orb.sync(false, false, true);
        orb.advance(0.2);
        assert_eq!(orb.estado(), "error");
        orb.advance(PULSE_SECS + 0.1); // vuelve a idle (sin texto)
        assert_eq!(orb.estado(), "idle");
        orb.sync(false, false, true); // sigue failed, pero NO es flanco
        orb.advance(0.2);
        assert_eq!(orb.estado(), "idle", "failed sostenido no debe re-disparar error");
    }
}
