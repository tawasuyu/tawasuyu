//! El **documento** del studio: la representación editable del grafo de estados,
//! agnóstica del render. Es la fuente de verdad que la UI manipula y que se
//! **compila** a un [`llimphi_anim::StateMachine`] ejecutable cada vez que cambia,
//! para alimentar el preview en vivo.
//!
//! ## Por qué un modelo propio
//!
//! `llimphi_anim::StateMachine` es un *builder* de una sola dirección: se
//! construye con `add_state`/`transition`, se congela en un `Arc` al hacer
//! `instance()`, y **no expone introspección** (sus campos son privados). Un
//! editor necesita leer, reordenar y reescribir el grafo, así que el studio
//! mantiene este `Doc` editable (con posiciones de canvas, defaults de inputs,
//! etc.) y lo proyecta al runtime con [`Doc::compile`]. La relación es la misma
//! que `Project → render` en voxel-studio: el documento es rico, el runtime es
//! la proyección ejecutable.
//!
//! El `ClipId` que consume el runtime es simplemente el **índice del estado**:
//! como el studio no carga assets reales (Lottie/rig) en la Fase 1, el preview
//! pinta cada estado con un color/movimiento sintético derivado de su índice. El
//! día que se cableen clips reales, sólo cambia el consumidor del `RenderFrame`.

use llimphi_anim::{Cmp, Condition, StateMachine};
use serde::{Deserialize, Serialize};

/// Operador de comparación numérica — espejo serializable de [`llimphi_anim::Cmp`]
/// (que no deriva serde). Se convierte a él al compilar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub const ALL: [CmpOp; 6] = [
        CmpOp::Eq,
        CmpOp::Ne,
        CmpOp::Lt,
        CmpOp::Le,
        CmpOp::Gt,
        CmpOp::Ge,
    ];
    pub fn symbol(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "≠",
            CmpOp::Lt => "<",
            CmpOp::Le => "≤",
            CmpOp::Gt => ">",
            CmpOp::Ge => "≥",
        }
    }
    fn to_anim(self) -> Cmp {
        match self {
            CmpOp::Eq => Cmp::Eq,
            CmpOp::Ne => Cmp::Ne,
            CmpOp::Lt => Cmp::Lt,
            CmpOp::Le => Cmp::Le,
            CmpOp::Gt => Cmp::Gt,
            CmpOp::Ge => Cmp::Ge,
        }
    }
}

/// Tipo de un input de la máquina (espejo del trío Rive bool/number/trigger).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputKind {
    Bool,
    Number,
    Trigger,
}

impl InputKind {
    pub fn label(self) -> &'static str {
        match self {
            InputKind::Bool => "bool",
            InputKind::Number => "número",
            InputKind::Trigger => "trigger",
        }
    }
}

/// Una guarda editable de una transición. Se compila a [`llimphi_anim::Condition`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CondDef {
    /// `bool input == value`.
    Bool { input: String, value: bool },
    /// `number input <op> value`.
    Number { input: String, op: CmpOp, value: f64 },
    /// el trigger `input` fue disparado este frame.
    Trigger { input: String },
    /// el clip del estado de origen terminó (sólo estados no-loop).
    ClipDone,
}

impl CondDef {
    fn to_anim(&self) -> Condition {
        match self {
            CondDef::Bool { input, value } => Condition::bool(input.clone(), *value),
            CondDef::Number { input, op, value } => {
                Condition::number(input.clone(), op.to_anim(), *value)
            }
            CondDef::Trigger { input } => Condition::trigger(input.clone()),
            CondDef::ClipDone => Condition::clip_done(),
        }
    }
}

/// Un estado del grafo: un clip (por índice) con su velocidad y loop, más su
/// posición en el lienzo y la duración nominal del clip (para `ClipDone`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateDef {
    pub name: String,
    /// Velocidad de reproducción del clip (escala el tiempo).
    pub speed: f64,
    /// ¿El clip hace loop? Si no, su `ClipDone` puede disparar transiciones.
    pub looping: bool,
    /// Duración nominal del clip en segundos (para `ClipDone` y el preview).
    pub clip_len: f64,
    /// Posición del nodo en el lienzo (pixels relativos al canvas).
    pub x: f32,
    pub y: f32,
}

impl StateDef {
    pub fn new(name: impl Into<String>, x: f32, y: f32) -> Self {
        StateDef {
            name: name.into(),
            speed: 1.0,
            looping: true,
            clip_len: 2.0,
            x,
            y,
        }
    }
}

/// Una transición editable. `from = None` ⇒ transición *any-state*.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransDef {
    /// Estado de origen (índice en `states`), o `None` para any-state.
    pub from: Option<usize>,
    /// Estado destino (índice en `states`).
    pub to: usize,
    /// Guardas AND. Vacío ⇒ la transición **nunca** dispara (el runtime la ignora).
    pub conditions: Vec<CondDef>,
    /// Duración del crossfade (segundos). `0.0` = salto instantáneo.
    pub duration_secs: f64,
}

/// Un input declarado, con su default. Los defaults siembran los controles en
/// vivo del panel de preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputDef {
    pub name: String,
    pub kind: InputKind,
    pub bool_default: bool,
    pub num_default: f64,
}

impl InputDef {
    pub fn new(name: impl Into<String>, kind: InputKind) -> Self {
        InputDef {
            name: name.into(),
            kind,
            bool_default: false,
            num_default: 0.0,
        }
    }
}

/// El documento entero: estados + transiciones + inputs + estado de entrada.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    pub states: Vec<StateDef>,
    pub transitions: Vec<TransDef>,
    pub inputs: Vec<InputDef>,
    /// Índice del estado de entrada.
    pub entry: usize,
}

impl Default for Doc {
    fn default() -> Self {
        Doc {
            states: Vec::new(),
            transitions: Vec::new(),
            inputs: Vec::new(),
            entry: 0,
        }
    }
}

impl Doc {
    /// Proyecta el documento a un [`StateMachine`] ejecutable. Índice de estado =
    /// `ClipId`. Transiciones con índices fuera de rango se descartan (defensa
    /// ante un documento inconsistente recién editado).
    pub fn compile(&self) -> StateMachine {
        let mut sm = StateMachine::new();
        let n = self.states.len();
        for (i, s) in self.states.iter().enumerate() {
            sm.add_state(s.name.clone(), i as u32, s.speed, s.looping);
            sm.set_clip_duration(i as u32, s.clip_len);
        }
        if n > 0 {
            sm.set_entry(self.entry.min(n - 1));
        }
        for t in &self.transitions {
            if t.to >= n {
                continue;
            }
            let conds: Vec<Condition> = t.conditions.iter().map(CondDef::to_anim).collect();
            match t.from {
                Some(f) if f < n => sm.transition(f, t.to, conds, t.duration_secs),
                None => sm.transition_any(t.to, conds, t.duration_secs),
                Some(_) => {} // origen fuera de rango → descartar
            }
        }
        sm
    }

    /// Serializa a RON legible (para guardar a disco).
    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(Into::into)
    }

    /// Carga desde RON.
    pub fn from_ron(s: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(s)
    }

    /// Documento de arranque: el clásico `idle ⇄ walk` por un bool `moving`, más
    /// un `jump` any-state por trigger. Da algo vivo que tocar al abrir.
    pub fn starter() -> Self {
        let mut doc = Doc::default();
        doc.states.push(StateDef::new("idle", 60.0, 80.0));
        let mut walk = StateDef::new("walk", 320.0, 80.0);
        walk.speed = 1.0;
        doc.states.push(walk);
        let mut jump = StateDef::new("jump", 190.0, 240.0);
        jump.looping = false;
        jump.clip_len = 0.6;
        doc.states.push(jump);
        doc.entry = 0;

        doc.inputs.push(InputDef::new("moving", InputKind::Bool));
        doc.inputs.push(InputDef::new("jump", InputKind::Trigger));

        doc.transitions.push(TransDef {
            from: Some(0),
            to: 1,
            conditions: vec![CondDef::Bool {
                input: "moving".into(),
                value: true,
            }],
            duration_secs: 0.2,
        });
        doc.transitions.push(TransDef {
            from: Some(1),
            to: 0,
            conditions: vec![CondDef::Bool {
                input: "moving".into(),
                value: false,
            }],
            duration_secs: 0.2,
        });
        // any-state: ⚡jump → jump (instantáneo)
        doc.transitions.push(TransDef {
            from: None,
            to: 2,
            conditions: vec![CondDef::Trigger {
                input: "jump".into(),
            }],
            duration_secs: 0.0,
        });
        // jump termina → idle
        doc.transitions.push(TransDef {
            from: Some(2),
            to: 0,
            conditions: vec![CondDef::ClipDone],
            duration_secs: 0.0,
        });
        doc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compila_y_arranca_en_entry() {
        let doc = Doc::starter();
        let inst = doc.compile().instance();
        assert_eq!(inst.current_state(), "idle");
    }

    #[test]
    fn bool_dispara_idle_a_walk() {
        let mut inst = Doc::starter().compile().instance();
        inst.set_bool("moving", true);
        inst.advance(0.3); // > duración de blend (0.2)
        assert_eq!(inst.current_state(), "walk");
        inst.set_bool("moving", false);
        inst.advance(0.3);
        assert_eq!(inst.current_state(), "idle");
    }

    #[test]
    fn trigger_anystate_salta_a_jump_y_vuelve() {
        let mut inst = Doc::starter().compile().instance();
        inst.fire("jump");
        inst.advance(0.016);
        assert_eq!(inst.current_state(), "jump");
        // El clip de 0.6 s termina (no loop) → vuelve a idle por ClipDone.
        inst.advance(0.7);
        assert_eq!(inst.current_state(), "idle");
    }

    #[test]
    fn transicion_con_indice_fuera_de_rango_se_descarta() {
        let mut doc = Doc::default();
        doc.states.push(StateDef::new("a", 0.0, 0.0));
        // destino inexistente → no debe panickear al compilar
        doc.transitions.push(TransDef {
            from: Some(0),
            to: 99,
            conditions: vec![CondDef::ClipDone],
            duration_secs: 0.0,
        });
        let inst = doc.compile().instance();
        assert_eq!(inst.current_state(), "a");
    }

    #[test]
    fn condicion_numerica_compila() {
        let mut doc = Doc::default();
        doc.states.push(StateDef::new("slow", 0.0, 0.0));
        doc.states.push(StateDef::new("fast", 0.0, 0.0));
        doc.inputs.push(InputDef::new("speed", InputKind::Number));
        doc.transitions.push(TransDef {
            from: Some(0),
            to: 1,
            conditions: vec![CondDef::Number {
                input: "speed".into(),
                op: CmpOp::Gt,
                value: 5.0,
            }],
            duration_secs: 0.0,
        });
        let mut inst = doc.compile().instance();
        inst.set_number("speed", 3.0);
        inst.advance(0.016);
        assert_eq!(inst.current_state(), "slow");
        inst.set_number("speed", 9.0);
        inst.advance(0.016);
        assert_eq!(inst.current_state(), "fast");
    }

    #[test]
    fn round_trip_ron() {
        let doc = Doc::starter();
        let ron = doc.to_ron().expect("serializa");
        let back = Doc::from_ron(&ron).expect("deserializa");
        assert_eq!(doc, back);
    }
}
