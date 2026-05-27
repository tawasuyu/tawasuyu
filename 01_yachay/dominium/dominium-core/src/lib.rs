//! `dominium-core` — el núcleo lógico del simulador de campo medio.
//!
//! Laboratorio de complejidad emergente: los agentes (Lemmings) no toman
//! decisiones cognitivas — reaccionan mecánicamente a los campos de una
//! grilla plana ejecutando una de 6 acciones atómicas fijas. Civilización,
//! guerra, fe y poder son patrones emergentes, no algoritmos.
//!
//! - [`grid`] — el Sustrato Plano: 5 capas SoA de `f32`.
//! - [`lemmings`] — los Agentes Vectoriales en Structure-of-Arrays.
//! - [`world`] — el `World` + las 6 acciones atómicas (`Action`).
//! - [`params`] — `SimParams`, las constantes que los sliders ajustan.
//! - [`conceptos`] — emisores de campo metaprogramables (datos puros).
//!
//! Cero dependencias gráficas (regla inviolable de la spec): sólo `serde`.
//! La difusión/entropía/cinemática viven en `dominium-physics`; el
//! renderizado isométrico en `dominium-iso` + `dominium-render-plan`.

#![forbid(unsafe_code)]

pub mod conceptos;
pub mod epoch;
pub mod grid;
pub mod lemmings;
pub mod metrics;
pub mod params;
pub mod psi_metrics;
pub mod world;

pub use conceptos::{BehaviorHack, Concepto, Conceptos, LayerMods, Persuasion, Trigger};
pub use epoch::Epoch;
pub use grid::Grid;
pub use lemmings::Lemmings;
pub use metrics::WorldStats;
pub use psi_metrics::{PsiMetrics, POLARIZATION_ALPHA, POLARIZATION_BINS};
pub use params::{
    ActionPolicy, SimParams, TradeTarget, RELIEVE_DEGRADACION, RELIEVE_MATERIA, RELIEVE_ORO,
    RELIEVE_PODER, RELIEVE_PSIQUE,
};
pub use world::{select_action_argmax, Action, World};
