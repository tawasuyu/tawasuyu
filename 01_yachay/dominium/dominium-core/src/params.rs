//! Constantes globales de la simulación.
//!
//! Son las que los sliders del Panel de Control alimentan en vivo: cada
//! una sintoniza una de las ecuaciones del núcleo.

use serde::{Deserialize, Serialize};

/// A quién dona un Lemming cuando ejecuta `act_intercambiar`. Permite
/// elegir entre la semántica original (vecino físico) y la redistribución
/// solidaria (el más necesitado del mundo) — esta última es la que cierra
/// el ciclo termodinámico y produce un punto fijo `N* > 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeTarget {
    /// Dona al vecino físico más cercano. Comportamiento histórico de
    /// `act_intercambiar`. Conserva la semántica geográfica pero no
    /// redistribuye eficientemente — la energía oscila localmente y los
    /// Replicadores aislados se agotan.
    Nearest,
    /// Dona al lemming con menor energía global (O(n) determinista).
    /// "Solidaridad universal": los Traders ricos alimentan a los
    /// Replicadores pobres, sostiene la natalidad.
    Poorest,
}

impl Default for TradeTarget {
    fn default() -> Self {
        TradeTarget::Poorest
    }
}

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
    /// Umbral de energía por encima del cual el agente se fuerza a
    /// `Replicar` — el atractor simétrico de la desesperación. Cierra el
    /// ciclo termodinámico: sin esta transición, los Replicadores
    /// genéticos se agotan en pocas generaciones y `dN/dt < 0`
    /// estructural. `0.0` deshabilita la transición (motor pre-2026-05-26).
    #[serde(default)]
    pub abundance_threshold: f32,
    /// A quién dona un Lemming cuando ejecuta `act_intercambiar`.
    /// Default `Poorest` — la redistribución solidaria es la que cierra
    /// el ciclo termodinámico del sistema. Ver [`TradeTarget`].
    #[serde(default)]
    pub trade_target: TradeTarget,
    /// Edad máxima; al superarla el agente muere.
    pub max_edad: u32,
    /// Costo metabólico basal: energía drenada cada tick a TODOS los
    /// lemmings por el simple hecho de estar vivos, independiente de la
    /// acción. Es el freno termodinámico que estabiliza la población —
    /// sin él, los Extractores acumulan E sin techo y la natalidad
    /// (vía abundance side-effect) se descontrola. Con él, dE/dt → 0
    /// cuando N llega a la capacidad de carga del territorio.
    /// `0.0` deshabilita (motor pre-2026-05-26).
    #[serde(default)]
    pub metabolic_cost: f32,
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
    /// Período del ciclo estacional, en ticks. Una estación completa
    /// (verano→invierno→verano) toma `season_period` ticks. `0` deshabilita
    /// el ciclo y el motor se comporta como antes (campos sin modulación).
    #[serde(default)]
    pub season_period: u32,
    /// Amplitud del ciclo estacional, ∈ [0, 1]. Modula multiplicativamente
    /// `diffusion_rate` y `entropy_rate` por un factor
    /// `1 + amp · sin(2π · t / period)`. Con `0.0` no hay ciclo (equivalente
    /// a `season_period = 0`). Es el "clima" del mundo: en verano (factor
    /// alto) los campos difunden y decaen más rápido; en invierno se
    /// congelan. Cero semántica de calendario — son sólo dos floats que
    /// pasan por la libm.
    #[serde(default)]
    pub season_amplitude: f32,
    /// Fracción del *espacio libre* que la naturaleza repuebla con materia
    /// por tick (regrowth logístico). En cada celda:
    /// `materia += regrowth_rate · max(0, carrying_capacity − materia)`.
    /// Vive *dentro* de la fase de difusión — no agrega una fase nueva al
    /// §1.5. Es el cierre termodinámico del motor: sin esta fuente la
    /// entropía vence siempre y la población se extingue.
    #[serde(default)]
    pub regrowth_rate: f32,
    /// Asíntota del regrowth: hacia este valor empuja la materia por
    /// celda. Inyecciones por Conceptos o por muerte de lemmings pueden
    /// superarlo; el regrowth nunca lo hace.
    #[serde(default)]
    pub carrying_capacity: f32,
}

/// Índices semánticos para indexar `SimParams::relieve`. Coinciden con el
/// orden de capas del `Grid`.
pub const RELIEVE_MATERIA: usize = 0;
pub const RELIEVE_PSIQUE: usize = 1;
pub const RELIEVE_PODER: usize = 2;
pub const RELIEVE_ORO: usize = 3;
pub const RELIEVE_DEGRADACION: usize = 4;

impl SimParams {
    /// Factor multiplicativo del ciclo estacional para el tick `t`. Vale
    /// `1.0 + season_amplitude · sin(2π · t / season_period)` cuando hay
    /// ciclo activo, y `1.0` cuando `season_period == 0` o
    /// `season_amplitude == 0.0`. Resultado siempre clamped a `[0, 2]` para
    /// que la modulación no invierta el signo de las tasas.
    ///
    /// **Determinismo bit-exacto**: usamos `libm::sinf` para evitar
    /// divergencias entre `f32::sin` de x86 vs ARM. El argumento se calcula
    /// en `f64` y se castea al final, así fases consecutivas no acumulan
    /// drift por wrap-around de grandes `t`.
    pub fn season_factor(&self, t: u64) -> f32 {
        if self.season_period == 0 || self.season_amplitude == 0.0 {
            return 1.0;
        }
        let period = self.season_period as f64;
        // Fase en [0, 2π) — modular antes de pasar a f32 para no perder
        // precisión cuando t es grande.
        let phase = ((t as f64).rem_euclid(period)) / period;
        let arg = (phase * std::f64::consts::TAU) as f32;
        let s = libm::sinf(arg);
        (1.0 + self.season_amplitude * s).clamp(0.0, 2.0)
    }
}

impl Default for SimParams {
    fn default() -> Self {
        Self {
            move_speed: 1.0,
            move_cost: 0.06,
            // Extracción generosa: la principal fuente de energía del sistema.
            extract_rate: 2.5,
            degr_per_extract: 0.02,
            sync_rate: 0.10,
            // Intercambio AGRESIVO: el mecanismo de redistribución que evita
            // que el Gini suba a 1 y los Replicadores se agoten.
            // Sin redistribución, la energía se concentra en Extractores y
            // los Replicadores (que no extraen) se quedan sin combustible.
            trade_amount: 1.5,
            // Threshold de reproducción más alto: filtra para que sólo
            // agentes con energía sustancial puedan tener hijos. Combinado
            // con `abundance_threshold` alto (ver abajo), el sistema
            // converge a un N* finito en lugar de crecer monotónicamente.
            replicate_threshold: 25.0,
            child_energy_frac: 0.50,
            fight_damage: 4.0,
            absorb_frac: 0.55,
            desperation_threshold: 4.0,
            // Atractor de abundancia: cualquier agente con E > 60 se vuelve
            // Replicador. Calibrado para que pase con frecuencia moderada
            // dado el flujo neto de energía típico (~0.5/tick por Extractor).
            // Threshold de abundancia alto: sólo agentes con MUCHA energía
            // (mucha más que la del equilibrio E* ≈ 27) replican como
            // bonus. Esto frena el crecimiento poblacional y mantiene
            // N* en el rango ~500-2000 en una grilla 80×80 con regrowth
            // moderado.
            abundance_threshold: 80.0,
            trade_target: TradeTarget::Poorest,
            // Vida larga + sin cliff: la cohorte inicial llega a max_edad
            // al mismo tiempo y la mortalidad sincronizada extingue al
            // sistema. Con max_edad alto, las cohortes se desincronizan
            // por la natalidad estocástica vía Replicar y la mortalidad
            // queda repartida.
            max_edad: 6000,
            // Costo metabólico basal: 0.05 E/tick. Calibrado para que el
            // punto fijo N* quede en ~500-1500 (manejable para perf O(N²)
            // de nearest/poorest), no en decenas de miles.
            metabolic_cost: 0.05,
            diffusion_rate: 0.10,
            // Entropía a la mitad: la pérdida por tick era demasiado agresiva
            // para el ciclo materia→energía→muerte→materia.
            entropy_rate: 0.005,
            // Default: el relieve físico sigue a materia, igual que el
            // ZWeights del render por defecto. Las montañas de "biomasa"
            // son las que se sienten al caminar.
            relieve: [1.0, 0.0, 0.0, 0.0, 0.0],
            climb_cost: 0.05,
            // Sin estaciones por default — el motor sigue siendo el de antes
            // a menos que el usuario las prenda explícitamente.
            season_period: 0,
            season_amplitude: 0.0,
            // Regrowth lento + capacidad chica: la materia es escasa.
            // Esto cierra la capacidad del territorio en N* manejable.
            // Si subís estos, N* explota (validate empíricamente).
            regrowth_rate: 0.015,
            carrying_capacity: 18.0,
        }
    }
}
