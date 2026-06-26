//! `llimphi-anim` — máquina de estados de animación, estilo Rive, **clip-agnóstica**.
//!
//! Es el escalón sobre el playback lineal: en vez de "reproducí esta animación",
//! modela "según estos *inputs*, en qué estado estoy y a cuál transiciono, con
//! qué *blend*". Es lo que vuelve una animación *interactiva* (hover, progreso,
//! triggers) en lugar de un loop fijo.
//!
//! ## Por qué clip-agnóstico
//!
//! El núcleo **no sabe de Lottie ni de vello**. Un clip es un [`ClipId`] (u32)
//! con una duración conocida; la máquina secuencia y mezcla clips por id y
//! tiempo, y emite un [`RenderFrame`] que dice *qué* renderizar (clip primario +
//! un clip entrante con su mezcla 0..1 durante una transición). El consumidor
//! mapea `ClipId → asset` y pinta. Hoy ese consumidor es `llimphi-lottie`;
//! mañana puede ser un rig de huesos nativo o un tween de `llimphi-motion` — el
//! mismo motor de estados sirve a todos.
//!
//! ## Modelo (espejo del de Rive)
//!
//! - **Inputs**: `bool`, `number`, `trigger` (los triggers se consumen en cada
//!   `advance`).
//! - **Estado**: un clip con velocidad y loop, más sus transiciones salientes.
//! - **Transición**: destino + condiciones (AND) sobre inputs + duración de
//!   blend. Las transiciones *any-state* se evalúan sin importar el estado
//!   actual (típico para "fire trigger → ir a X desde donde sea").
//! - **Condición**: `bool == v`, `number <cmp> v`, `trigger disparado`, o
//!   `clip terminó` (para "cuando la animación acaba, pasá a la siguiente").
//!
//! ## Bucle
//!
//! El consumidor empuja inputs ([`Instance::set_bool`] etc.), llama
//! [`Instance::advance`] con el `dt` del frame, y pinta [`Instance::render_frame`].
//! Encaja directo en el bucle Elm de Llimphi vía `Handle::spawn_periodic`.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Índice de un estado dentro de la máquina (devuelto por `add_state`).
pub type StateId = usize;

/// Identificador opaco de un clip de animación. Lo interpreta el consumidor
/// (p. ej. índice en un `Vec<LottieAsset>` o clave de un mapa).
pub type ClipId = u32;

/// Operador de comparación para condiciones sobre inputs numéricos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    fn test(self, a: f64, b: f64) -> bool {
        match self {
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
        }
    }
}

/// Condición de guarda de una transición. Una transición dispara cuando **todas**
/// sus condiciones se cumplen (AND).
#[derive(Debug, Clone)]
pub enum Condition {
    /// El input booleano `name` vale `value` (default `false` si no está seteado).
    Bool { name: String, value: bool },
    /// El input numérico `name` comparado con `value` por `op` (default `0.0`).
    Number { name: String, op: Cmp, value: f64 },
    /// El trigger `name` fue disparado en este frame (se consume al `advance`).
    Trigger { name: String },
    /// El clip del estado actual terminó (sólo aplica a estados no-loop).
    ClipDone,
}

impl Condition {
    /// `bool == value`.
    pub fn bool(name: impl Into<String>, value: bool) -> Self {
        Condition::Bool {
            name: name.into(),
            value,
        }
    }
    /// `number <op> value`.
    pub fn number(name: impl Into<String>, op: Cmp, value: f64) -> Self {
        Condition::Number {
            name: name.into(),
            op,
            value,
        }
    }
    /// trigger disparado.
    pub fn trigger(name: impl Into<String>) -> Self {
        Condition::Trigger { name: name.into() }
    }
    /// el clip actual terminó.
    pub fn clip_done() -> Self {
        Condition::ClipDone
    }
}

/// Región sensible al puntero, en coordenadas **normalizadas** `0..1` sobre el
/// rect donde se pinta la animación (origen arriba-izquierda). Resolución- e
/// independiente del tamaño del clip: el consumidor mapea el puntero de pantalla
/// a este espacio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Area {
    /// Toda la superficie de la animación.
    All,
    /// Un rectángulo normalizado.
    Rect { x: f64, y: f64, w: f64, h: f64 },
}

impl Area {
    fn contains(&self, px: f64, py: f64) -> bool {
        match self {
            Area::All => (0.0..=1.0).contains(&px) && (0.0..=1.0).contains(&py),
            Area::Rect { x, y, w, h } => {
                px >= *x && px <= x + w && py >= *y && py <= y + h
            }
        }
    }
}

/// Qué evento de puntero dispara un [`Listener`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerTrigger {
    /// El puntero entró al área.
    Enter,
    /// El puntero salió del área.
    Exit,
    /// Se presionó el botón con el puntero dentro del área.
    Down,
    /// Se soltó el botón con el puntero dentro del área.
    Up,
    /// El puntero se movió dentro del área (cada movimiento).
    Move,
}

/// Qué le hace un [`Listener`] a los inputs cuando dispara.
#[derive(Debug, Clone)]
pub enum Action {
    /// Setea un input booleano.
    SetBool { name: String, value: bool },
    /// Dispara un trigger.
    Fire { name: String },
}

impl Action {
    pub fn set_bool(name: impl Into<String>, value: bool) -> Self {
        Action::SetBool {
            name: name.into(),
            value,
        }
    }
    pub fn fire(name: impl Into<String>) -> Self {
        Action::Fire { name: name.into() }
    }
}

/// Un *listener* estilo Rive: cuando el puntero hace `trigger` sobre `area`,
/// aplica `action` a los inputs. Es el puente puntero → máquina de estados
/// (hover/click → bool/trigger → transición).
#[derive(Debug, Clone)]
pub struct Listener {
    pub area: Area,
    pub trigger: PointerTrigger,
    pub action: Action,
}

/// Una transición saliente: a qué estado, bajo qué condiciones, con cuánto blend.
#[derive(Debug, Clone)]
pub struct Transition {
    pub to: StateId,
    pub conditions: Vec<Condition>,
    /// Duración del crossfade hacia el estado destino, en segundos. `0.0` =
    /// instantáneo (sin mezcla).
    pub duration_secs: f64,
}

/// Un estado de la máquina: un clip con su velocidad y loop, más sus salidas.
#[derive(Debug, Clone)]
struct State {
    name: String,
    clip: ClipId,
    speed: f64,
    looping: bool,
    transitions: Vec<Transition>,
}

/// La máquina de estados *definición* (inmutable, compartible). Construida con
/// `new` + `add_state`/`transition`, congelada en un `Arc` por `instance`.
#[derive(Debug, Clone, Default)]
pub struct StateMachine {
    states: Vec<State>,
    /// Duración conocida de cada clip (segundos). La necesita la condición
    /// `ClipDone` y el wrap de tiempo.
    clip_durations: HashMap<ClipId, f64>,
    /// Transiciones evaluadas desde *cualquier* estado.
    any: Vec<Transition>,
    /// Listeners de puntero (hover/click → inputs).
    listeners: Vec<Listener>,
    entry: StateId,
}

impl StateMachine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Agrega un estado-clip. Devuelve su `StateId`. El primero agregado es el
    /// estado de entrada por defecto (cambialo con [`set_entry`]).
    ///
    /// [`set_entry`]: StateMachine::set_entry
    pub fn add_state(
        &mut self,
        name: impl Into<String>,
        clip: ClipId,
        speed: f64,
        looping: bool,
    ) -> StateId {
        let id = self.states.len();
        self.states.push(State {
            name: name.into(),
            clip,
            speed,
            looping,
            transitions: Vec::new(),
        });
        id
    }

    /// Registra la duración (segundos) de un clip — necesaria para `ClipDone`.
    pub fn set_clip_duration(&mut self, clip: ClipId, secs: f64) {
        self.clip_durations.insert(clip, secs);
    }

    /// Fija el estado de entrada.
    pub fn set_entry(&mut self, entry: StateId) {
        self.entry = entry;
    }

    /// Agrega una transición `from → to` con condiciones y blend.
    pub fn transition(
        &mut self,
        from: StateId,
        to: StateId,
        conditions: Vec<Condition>,
        duration_secs: f64,
    ) {
        self.states[from].transitions.push(Transition {
            to,
            conditions,
            duration_secs,
        });
    }

    /// Agrega una transición *any-state* `* → to` (se evalúa desde cualquier
    /// estado, antes que las salientes del estado actual).
    pub fn transition_any(
        &mut self,
        to: StateId,
        conditions: Vec<Condition>,
        duration_secs: f64,
    ) {
        self.any.push(Transition {
            to,
            conditions,
            duration_secs,
        });
    }

    /// Agrega un listener de puntero: cuando el puntero hace `trigger` sobre
    /// `area`, aplica `action` a los inputs (que luego rigen transiciones).
    pub fn listener(&mut self, area: Area, trigger: PointerTrigger, action: Action) {
        self.listeners.push(Listener {
            area,
            trigger,
            action,
        });
    }

    /// Crea una instancia ejecutable de esta máquina.
    pub fn instance(self) -> Instance {
        let entry = self.entry;
        Instance {
            machine: Arc::new(self),
            inputs: Inputs::default(),
            current: entry,
            state_time: 0.0,
            active: None,
            last_pointer: None,
        }
    }

    fn clip_of(&self, state: StateId) -> ClipId {
        self.states[state].clip
    }
    fn speed_of(&self, state: StateId) -> f64 {
        self.states[state].speed
    }
    fn duration_of_clip(&self, clip: ClipId) -> Option<f64> {
        self.clip_durations.get(&clip).copied()
    }
}

/// Estado de los inputs de una instancia. Booleanos y números persisten; los
/// triggers se limpian al final de cada `advance`.
#[derive(Debug, Clone, Default)]
struct Inputs {
    bools: HashMap<String, bool>,
    numbers: HashMap<String, f64>,
    triggers: HashSet<String>,
}

/// Una transición en curso: hacia dónde, su tiempo propio y cuánto blend lleva.
#[derive(Debug, Clone)]
struct Active {
    to: StateId,
    to_time: f64,
    elapsed: f64,
    duration: f64,
}

/// Instancia ejecutable: inputs vivos + estado actual + transición en curso.
/// `Clone` barato más allá de los inputs (la definición va por `Arc`).
#[derive(Debug, Clone)]
pub struct Instance {
    machine: Arc<StateMachine>,
    inputs: Inputs,
    current: StateId,
    state_time: f64,
    active: Option<Active>,
    /// Última posición del puntero en coords normalizadas `0..1` (`None` =
    /// fuera de la animación). La usan los listeners de `Down`/`Up` y la
    /// detección de `Enter`/`Exit`.
    last_pointer: Option<(f64, f64)>,
}

/// Una muestra de clip a renderizar: qué clip y en qué instante (segundos).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipSample {
    pub clip: ClipId,
    pub time_secs: f64,
}

/// Lo que el consumidor debe pintar este frame: el clip primario y, si hay una
/// transición en curso, el clip entrante con su mezcla (`0.0`=nada del entrante,
/// `1.0`=todo el entrante).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderFrame {
    pub primary: ClipSample,
    pub blend: Option<(ClipSample, f32)>,
}

impl Instance {
    /// Setea un input booleano.
    pub fn set_bool(&mut self, name: impl Into<String>, value: bool) {
        self.inputs.bools.insert(name.into(), value);
    }
    /// Setea un input numérico.
    pub fn set_number(&mut self, name: impl Into<String>, value: f64) {
        self.inputs.numbers.insert(name.into(), value);
    }
    /// Dispara un trigger (se consume en el próximo `advance`).
    pub fn fire(&mut self, name: impl Into<String>) {
        self.inputs.triggers.insert(name.into());
    }

    /// Nombre del estado actual (de origen mientras hay una transición en curso).
    pub fn current_state(&self) -> &str {
        &self.machine.states[self.current].name
    }
    /// ¿Hay una transición (crossfade) en curso?
    pub fn is_transitioning(&self) -> bool {
        self.active.is_some()
    }

    /// ¿El clip del estado actual terminó? (`false` si el estado es loop o si no
    /// se registró su duración.)
    fn clip_done(&self) -> bool {
        let st = &self.machine.states[self.current];
        if st.looping {
            return false;
        }
        match self.machine.duration_of_clip(st.clip) {
            Some(dur) => self.state_time >= dur,
            None => false,
        }
    }

    fn cond_met(&self, c: &Condition, clip_done: bool) -> bool {
        match c {
            Condition::Bool { name, value } => {
                self.inputs.bools.get(name).copied().unwrap_or(false) == *value
            }
            Condition::Number { name, op, value } => {
                op.test(self.inputs.numbers.get(name).copied().unwrap_or(0.0), *value)
            }
            Condition::Trigger { name } => self.inputs.triggers.contains(name),
            Condition::ClipDone => clip_done,
        }
    }

    fn transition_ready(&self, t: &Transition, clip_done: bool) -> bool {
        // Sin condiciones = nunca dispara sola (evita loops degenerados);
        // las transiciones útiles siempre tienen al menos una guarda.
        !t.conditions.is_empty() && t.conditions.iter().all(|c| self.cond_met(c, clip_done))
    }

    /// Busca la primera transición disparable: any-state primero, luego las
    /// salientes del estado actual. No retorna self-transiciones (to == current).
    fn pick_transition(&self, clip_done: bool) -> Option<Transition> {
        self.machine
            .any
            .iter()
            .chain(self.machine.states[self.current].transitions.iter())
            .find(|t| t.to != self.current && self.transition_ready(t, clip_done))
            .cloned()
    }

    fn begin(&mut self, t: Transition) {
        if t.duration_secs <= 0.0 {
            // Instantáneo: saltamos sin blend.
            self.current = t.to;
            self.state_time = 0.0;
            self.active = None;
        } else {
            self.active = Some(Active {
                to: t.to,
                to_time: 0.0,
                elapsed: 0.0,
                duration: t.duration_secs,
            });
        }
    }

    /// Avanza la transición en curso (si la hay) por `dt` y la confirma cuando
    /// el blend completa. Aislado para poder aplicarlo tanto a una transición
    /// que ya venía como a una recién arrancada en este mismo frame.
    fn step_active(&mut self, dt: f64) {
        let Some(at) = self.active.as_ref() else {
            return;
        };
        let to = at.to;
        let speed = self.machine.speed_of(to);
        let at = self.active.as_mut().unwrap();
        at.to_time += dt * speed;
        at.elapsed += dt;
        if at.elapsed >= at.duration {
            let to_time = at.to_time;
            self.current = to;
            self.state_time = to_time;
            self.active = None;
        }
    }

    fn apply_action(&mut self, action: &Action) {
        match action {
            Action::SetBool { name, value } => {
                self.inputs.bools.insert(name.clone(), *value);
            }
            Action::Fire { name } => {
                self.inputs.triggers.insert(name.clone());
            }
        }
    }

    /// Reporta la posición del puntero en coords **normalizadas** `0..1` sobre
    /// el rect de la animación, o `None` si el puntero salió. Dispara los
    /// listeners `Enter`/`Exit`/`Move` correspondientes y deja la posición
    /// guardada para los de `Down`/`Up`.
    pub fn pointer_move(&mut self, pos: Option<(f64, f64)>) {
        let machine = self.machine.clone();
        let was = self.last_pointer;
        for l in &machine.listeners {
            let was_in = was.map_or(false, |(x, y)| l.area.contains(x, y));
            let now_in = pos.map_or(false, |(x, y)| l.area.contains(x, y));
            let fire = match l.trigger {
                PointerTrigger::Enter => !was_in && now_in,
                PointerTrigger::Exit => was_in && !now_in,
                PointerTrigger::Move => now_in,
                _ => false,
            };
            if fire {
                self.apply_action(&l.action);
            }
        }
        self.last_pointer = pos;
    }

    /// Botón presionado: dispara los listeners `Down` cuya área contiene la
    /// última posición conocida del puntero.
    pub fn pointer_down(&mut self) {
        self.pointer_button(PointerTrigger::Down);
    }

    /// Botón soltado: dispara los listeners `Up`.
    pub fn pointer_up(&mut self) {
        self.pointer_button(PointerTrigger::Up);
    }

    fn pointer_button(&mut self, which: PointerTrigger) {
        let Some((px, py)) = self.last_pointer else {
            return;
        };
        let machine = self.machine.clone();
        for l in &machine.listeners {
            if l.trigger == which && l.area.contains(px, py) {
                self.apply_action(&l.action);
            }
        }
    }

    /// Avanza la máquina `dt` segundos: corre el tiempo del estado, avanza/
    /// confirma la transición en curso, evalúa una nueva transición (y le aplica
    /// el `dt` de este frame si arranca) y consume los triggers.
    pub fn advance(&mut self, dt: f64) {
        let dt = dt.max(0.0);
        self.state_time += dt * self.machine.speed_of(self.current);

        // Transición que ya venía de frames anteriores.
        self.step_active(dt);

        // Nueva transición sólo si no estamos mezclando (sin interrupción de
        // blend en el MVP). Si arranca, le aplicamos el dt de este frame para
        // que el blend progrese (y se complete si el dt es grande).
        if self.active.is_none() {
            let done = self.clip_done();
            if let Some(t) = self.pick_transition(done) {
                self.begin(t);
                self.step_active(dt);
            }
        }

        self.inputs.triggers.clear();
    }

    /// Qué pintar este frame. Durante una transición, `primary` es el estado de
    /// origen y `blend` el entrante con su mezcla 0..1.
    pub fn render_frame(&self) -> RenderFrame {
        let primary = ClipSample {
            clip: self.machine.clip_of(self.current),
            time_secs: self.state_time,
        };
        let blend = self.active.as_ref().map(|at| {
            let mix = (at.elapsed / at.duration).clamp(0.0, 1.0) as f32;
            (
                ClipSample {
                    clip: self.machine.clip_of(at.to),
                    time_secs: at.to_time,
                },
                mix,
            )
        });
        RenderFrame { primary, blend }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: ClipId = 0;
    const WALK: ClipId = 1;
    const JUMP: ClipId = 2;

    /// idle ⇄ walk por un bool "moving", con blend de 0.2 s.
    fn idle_walk() -> StateMachine {
        let mut sm = StateMachine::new();
        let idle = sm.add_state("idle", IDLE, 1.0, true);
        let walk = sm.add_state("walk", WALK, 1.0, true);
        sm.set_entry(idle);
        sm.transition(idle, walk, vec![Condition::bool("moving", true)], 0.2);
        sm.transition(walk, idle, vec![Condition::bool("moving", false)], 0.2);
        sm
    }

    #[test]
    fn arranca_en_entry() {
        let inst = idle_walk().instance();
        assert_eq!(inst.current_state(), "idle");
        assert_eq!(inst.render_frame().primary.clip, IDLE);
        assert!(inst.render_frame().blend.is_none());
    }

    #[test]
    fn bool_dispara_transicion_con_blend() {
        let mut inst = idle_walk().instance();
        inst.set_bool("moving", true);
        inst.advance(0.1); // arranca la transición (dur 0.2)
        assert!(inst.is_transitioning(), "debería estar mezclando");
        let rf = inst.render_frame();
        assert_eq!(rf.primary.clip, IDLE); // origen
        let (incoming, mix) = rf.blend.expect("hay blend");
        assert_eq!(incoming.clip, WALK);
        assert!((mix - 0.5).abs() < 1e-6, "0.1/0.2 = 0.5, fue {mix}");
        // Completar el blend.
        inst.advance(0.15);
        assert!(!inst.is_transitioning());
        assert_eq!(inst.current_state(), "walk");
        assert!(inst.render_frame().blend.is_none());
    }

    #[test]
    fn vuelve_cuando_el_bool_se_apaga() {
        let mut inst = idle_walk().instance();
        inst.set_bool("moving", true);
        inst.advance(0.3); // idle→walk completo
        assert_eq!(inst.current_state(), "walk");
        inst.set_bool("moving", false);
        inst.advance(0.3); // walk→idle completo
        assert_eq!(inst.current_state(), "idle");
    }

    #[test]
    fn trigger_es_de_un_solo_frame() {
        let mut sm = idle_walk();
        let jump = sm.add_state("jump", JUMP, 1.0, false);
        sm.set_clip_duration(JUMP, 0.5);
        // any-state: fire "jump" → jump (instantáneo)
        sm.transition_any(jump, vec![Condition::trigger("jump")], 0.0);
        // jump termina → vuelve a idle
        sm.transition(jump, 0, vec![Condition::clip_done()], 0.0);
        let mut inst = sm.instance();

        inst.fire("jump");
        inst.advance(0.016);
        assert_eq!(inst.current_state(), "jump");
        // El trigger se consumió: avanzar de nuevo no re-dispara nada raro.
        inst.advance(0.016);
        assert_eq!(inst.current_state(), "jump");
        // Cuando el clip de 0.5 s termina, vuelve a idle.
        inst.advance(0.5);
        assert_eq!(inst.current_state(), "idle");
    }

    #[test]
    fn condicion_numerica() {
        let mut sm = StateMachine::new();
        let slow = sm.add_state("slow", IDLE, 1.0, true);
        let fast = sm.add_state("fast", WALK, 1.0, true);
        sm.set_entry(slow);
        sm.transition(
            slow,
            fast,
            vec![Condition::number("speed", Cmp::Gt, 5.0)],
            0.0,
        );
        let mut inst = sm.instance();
        inst.set_number("speed", 3.0);
        inst.advance(0.016);
        assert_eq!(inst.current_state(), "slow");
        inst.set_number("speed", 9.0);
        inst.advance(0.016);
        assert_eq!(inst.current_state(), "fast");
    }

    #[test]
    fn transicion_instantanea_sin_blend() {
        let mut sm = StateMachine::new();
        let a = sm.add_state("a", IDLE, 1.0, true);
        let b = sm.add_state("b", WALK, 1.0, true);
        sm.set_entry(a);
        sm.transition(a, b, vec![Condition::bool("go", true)], 0.0);
        let mut inst = sm.instance();
        inst.set_bool("go", true);
        inst.advance(0.016);
        assert!(!inst.is_transitioning());
        assert_eq!(inst.current_state(), "b");
    }

    #[test]
    fn hover_enter_exit_maneja_un_bool() {
        // idle ⇄ walk por "moving"; hover sobre toda la animación setea moving.
        let mut sm = idle_walk();
        sm.listener(Area::All, PointerTrigger::Enter, Action::set_bool("moving", true));
        sm.listener(Area::All, PointerTrigger::Exit, Action::set_bool("moving", false));
        let mut inst = sm.instance();

        // Puntero entra al centro → moving=true → transiciona a walk.
        inst.pointer_move(Some((0.5, 0.5)));
        inst.advance(0.3);
        assert_eq!(inst.current_state(), "walk");

        // Puntero sale → moving=false → vuelve a idle.
        inst.pointer_move(None);
        inst.advance(0.3);
        assert_eq!(inst.current_state(), "idle");
    }

    #[test]
    fn click_dispara_trigger_en_su_area() {
        let mut sm = idle_walk();
        let jump = sm.add_state("jump", JUMP, 1.0, false);
        sm.set_clip_duration(JUMP, 0.5);
        sm.transition_any(jump, vec![Condition::trigger("tap")], 0.0);
        sm.transition(jump, 0, vec![Condition::clip_done()], 0.0);
        // Sólo la mitad derecha responde al click.
        sm.listener(
            Area::Rect { x: 0.5, y: 0.0, w: 0.5, h: 1.0 },
            PointerTrigger::Down,
            Action::fire("tap"),
        );
        let mut inst = sm.instance();

        // Click en la mitad IZQUIERDA: fuera del área, no dispara.
        inst.pointer_move(Some((0.2, 0.5)));
        inst.pointer_down();
        inst.advance(0.016);
        assert_eq!(inst.current_state(), "idle");

        // Click en la mitad DERECHA: dispara "tap" → jump.
        inst.pointer_move(Some((0.8, 0.5)));
        inst.pointer_down();
        inst.advance(0.016);
        assert_eq!(inst.current_state(), "jump");
    }

    #[test]
    fn move_fuera_del_area_no_dispara() {
        let mut sm = idle_walk();
        sm.listener(
            Area::Rect { x: 0.0, y: 0.0, w: 0.4, h: 0.4 },
            PointerTrigger::Enter,
            Action::set_bool("moving", true),
        );
        let mut inst = sm.instance();
        // Entra pero a una zona fuera del rect del listener.
        inst.pointer_move(Some((0.9, 0.9)));
        inst.advance(0.3);
        assert_eq!(inst.current_state(), "idle", "fuera del área no debe disparar");
    }

    #[test]
    fn la_velocidad_escala_el_tiempo() {
        let mut sm = StateMachine::new();
        let s = sm.add_state("s", IDLE, 2.0, true);
        sm.set_entry(s);
        let mut inst = sm.instance();
        inst.advance(1.0);
        // speed 2.0 → 1 s real = 2 s de clip.
        assert!((inst.render_frame().primary.time_secs - 2.0).abs() < 1e-9);
    }
}
