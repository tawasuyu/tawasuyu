//! Constantes globales de la simulación.
//!
//! Son las que los sliders del Panel de Control alimentan en vivo: cada
//! una sintoniza una de las ecuaciones del núcleo.

use serde::{Deserialize, Serialize};

/// Parámetros que gobiernan las 6 acciones y el ciclo de vida.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimParams {
    /// Velocidad de desplazamiento de `Mover` (celdas por tick).
    pub move_speed: f32,
    /// Energía que consume un paso de `Mover`.
    pub move_cost: f32,
    /// Cantidad extraída de la celda por `Extraer`.
    pub extract_rate: f32,
    /// Degradación añadida al suelo por cada `Extraer`.
    pub degr_per_extract: f32,
    /// Tasa de convergencia de `vector_psi` en `Sincronizar` (0-1).
    pub sync_rate: f32,
    /// Energía transferida por `Intercambiar`.
    pub trade_amount: f32,
    /// Umbral de energía para que `Replicar` dispare.
    pub replicate_threshold: f32,
    /// Fracción de la energía del padre que hereda el hijo en `Replicar`.
    pub child_energy_frac: f32,
    /// Daño de energía que inflige `Degradar`.
    pub fight_damage: f32,
    /// Fracción del daño que el atacante absorbe como energía.
    pub absorb_frac: f32,
    /// Umbral de energía bajo el cual el agente se fuerza a `Pelear`.
    pub desperation_threshold: f32,
    /// Edad máxima; al superarla el agente muere.
    pub max_edad: u32,
    /// Fracción que cada celda difunde hacia sus 4 vecinas por tick (0-1).
    pub diffusion_rate: f32,
    /// Tasa de pérdida natural (entropía) de los campos por tick (0-1).
    pub entropy_rate: f32,
    /// Pesos por capa que definen el **relieve físico** que sienten los
    /// lemmings al moverse (no es lo mismo que el `ZWeights` del render —
    /// el render puede mostrar una vista distinta de la "altura"). El
    /// gradiente del relieve atrae/repele en `act_mover` y cobra
    /// `climb_cost` extra de energía por unidad subida.
    pub relieve: [f32; 5],
    /// Energía consumida por unidad de relieve **subido** en `act_mover`
    /// (los lemmings no pagan extra al bajar). El score de un candidato
    /// se reduce en `climb_cost · max(0, z_dst − z_src)` antes de elegir.
    pub climb_cost: f32,
}

/// Índices semánticos para indexar `SimParams::relieve`. Coinciden con el
/// orden de capas del `Grid`.
pub const RELIEVE_MATERIA: usize = 0;
pub const RELIEVE_PSIQUE: usize = 1;
pub const RELIEVE_PODER: usize = 2;
pub const RELIEVE_ORO: usize = 3;
pub const RELIEVE_DEGRADACION: usize = 4;

impl Default for SimParams {
    fn default() -> Self {
        Self {
            move_speed: 1.0,
            move_cost: 0.10,
            extract_rate: 1.0,
            degr_per_extract: 0.05,
            sync_rate: 0.10,
            trade_amount: 0.50,
            replicate_threshold: 50.0,
            child_energy_frac: 0.30,
            fight_damage: 5.0,
            absorb_frac: 0.50,
            desperation_threshold: 5.0,
            max_edad: 1000,
            diffusion_rate: 0.10,
            entropy_rate: 0.01,
            // Default: el relieve físico sigue a materia, igual que el
            // ZWeights del render por defecto. Las montañas de "biomasa"
            // son las que se sienten al caminar.
            relieve: [1.0, 0.0, 0.0, 0.0, 0.0],
            climb_cost: 0.05,
        }
    }
}
