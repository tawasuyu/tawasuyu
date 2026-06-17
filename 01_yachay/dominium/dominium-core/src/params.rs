//! Constantes globales de la simulación.
//!
//! Son las que los sliders del Panel de Control alimentan en vivo: cada
//! una sintoniza una de las ecuaciones del núcleo.

use serde::{Deserialize, Serialize};

/// Política de elección de la `accion` base de los Lemmings.
///
/// El motor histórico fija la acción una sola vez (en `seed` / al replicarse
/// se hereda del padre) y nunca la recalcula salvo por transiciones de
/// supervivencia (desesperación → pelear) o captura por Conceptos
/// (`apply_hacks`). Eso convierte al `vector_psi` en una variable casi
/// decorativa: la psicología del agente no decide qué hace, sólo cómo se
/// mueve.
///
/// `PsiArgmax` cierra el bucle: cada `policy_reeval_period` ticks, los
/// agentes libres (sin `hack_lock`) recalculan su byte de acción tomando el
/// `argmax` de `action_weights · vector_psi`. Determinista bit-exacto: sin
/// RNG, sin softmax — comparación lineal de 6 escalares con tie-break por
/// menor índice. Es el complemento mínimo que vuelve endógena la
/// heterogeneidad poblacional sin romper §1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionPolicy {
    /// Comportamiento histórico: la acción se asigna al spawn (o se hereda
    /// del padre en `Replicar`) y sólo cambia por transiciones de
    /// supervivencia o hacks. La psicología no decide qué hace el agente.
    Fixed,
    /// La acción se reelige cada `policy_reeval_period` ticks como
    /// `argmax(action_weights · vector_psi)`. Determinista, sin RNG.
    PsiArgmax,
}

impl Default for ActionPolicy {
    fn default() -> Self {
        ActionPolicy::Fixed
    }
}

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
    /// Intensidad con la que el `vector_psi` del agente modula los efectos
    /// de sus 5 acciones físicas (Mover, Extraer, Intercambiar, Replicar,
    /// Degradar). Con `0.0` los efectos son idénticos al motor histórico
    /// — bit-exacto. Con `> 0`, el psi entra en cada cantidad afín:
    ///
    /// - `Mover`: `move_cost ← move_cost · (1 + mod · 0.5 · psi[MIEDO])`
    ///   — el miedoso se cansa más al moverse.
    /// - `Extraer`: `extract_rate ← extract_rate · (1 + mod · psi[CORRUPTIBILIDAD])`
    ///   — el corrupto saca más del suelo y deja más cicatriz.
    /// - `Intercambiar`: `trade_amount ← trade_amount · max(0, 1 + mod ·
    ///   (psi[ORDEN] − psi[CORRUPTIBILIDAD]))` — el ordenado comparte, el
    ///   corrupto retiene.
    /// - `Replicar`: `replicate_threshold ← replicate_threshold · max(0.1,
    ///   1 − mod · 0.3 · psi[ORDEN])` — el ordenado replica antes.
    /// - `Degradar`: `fight_damage ← fight_damage · max(0, 1 + mod ·
    ///   (psi[CORRUPTIBILIDAD] − psi[MIEDO]))` — el miedoso pega menos, el
    ///   corrupto más.
    ///
    /// Rango sugerido `[0, 1]`. Valores > 1 amplifican la heterogeneidad
    /// pero pueden producir efectos no-monotónicos cuando un psi extremo
    /// hace flip al signo del factor (los clamps a 0/0.1 lo previenen).
    #[serde(default)]
    pub psi_effect_modulation: f32,
    /// Política de elección de la `accion` base. Ver [`ActionPolicy`].
    /// Default `Fixed` → comportamiento histórico bit-exacto.
    #[serde(default)]
    pub action_policy: ActionPolicy,
    /// Pesos `[accion][componente_psi]` para `ActionPolicy::PsiArgmax`. Una
    /// matriz 6×4 — fila `a` = qué tan atractiva es la acción `a` para cada
    /// componente del psi. Cuando la política es `Fixed` se ignora.
    ///
    /// Default semánticamente plausible (independiente del comportamiento
    /// histórico porque sólo se consulta con `PsiArgmax`):
    /// - `Mover` (0): premia CURIOSIDAD, penaliza MIEDO.
    /// - `Extraer` (1): premia ORDEN y CORRUPTIBILIDAD.
    /// - `Sincronizar` (2): premia CURIOSIDAD.
    /// - `Intercambiar` (3): premia ORDEN, penaliza MIEDO.
    /// - `Replicar` (4): premia ORDEN.
    /// - `Degradar` (5): premia CORRUPTIBILIDAD, penaliza MIEDO.
    #[serde(default = "default_action_weights")]
    pub action_weights: [[f32; 4]; 6],
    /// Cada cuántos ticks reelige la acción la `ActionPolicy::PsiArgmax`.
    /// `0` deshabilita la reelección incluso si la política es `PsiArgmax`
    /// (failsafe: la matriz sólo "se enciende" cuando hay periodo). Valores
    /// típicos: 10..200. Períodos chicos pueden volver al sistema neurótico
    /// (cambia de oficio cada poco); muy grandes, inerte.
    #[serde(default)]
    pub policy_reeval_period: u32,
    /// Radio de influencia social (Fase B): cada agente acerca su
    /// `vector_psi` al promedio del psi de los vecinos que estén a
    /// distancia euclidiana ≤ `social_radius`. `0.0` (default) deshabilita
    /// el contagio — el motor histórico no paga nada.
    ///
    /// **Costo**: O(N²) determinista, aceptable hasta ~10k agentes por la
    /// grilla típica. Sin índice espacial: para poblaciones masivas habría
    /// que indexar celdas por agente — pendiente para Fase B.2.
    #[serde(default)]
    pub social_radius: f32,
    /// Tasa de convergencia del contagio social (Fase B). Cada tick, los
    /// agentes en el radio acercan su psi al promedio local por
    /// `psi_nuevo = psi + rate · (psi_local − psi)`. `0.0` (default) =
    /// sin contagio incluso si `social_radius > 0`. Rango útil 0.01..0.20:
    /// valores grandes producen conformismo brutal (todos convergen al
    /// mismo psi), valores chicos preservan diversidad.
    #[serde(default)]
    pub contagion_rate: f32,
    /// Umbral de homofilia (Fase B.2): un vecino dentro del `social_radius`
    /// sólo influye al agente si su distancia psi euclidiana es menor a
    /// este umbral. Mismo psi → siempre influye; psi muy distinto → no
    /// influye en absoluto. Es el "sólo escucho a los míos" canónico de la
    /// psicología social.
    ///
    /// `0.0` (default) = sin filtro de homofilia → contagio universal
    /// (motor B.1: produce homogeneización con tasas altas). Rango útil
    /// 0.3..1.0 — con threshold chico emergen **tribus aisladas** y la
    /// polarización **sube** en vez de bajar; con threshold grande, recae
    /// al comportamiento de B.1.
    #[serde(default)]
    pub homophily_threshold: f32,
    /// Activa el modelo Big Five (5 dimensiones) en vez de las 4 históricas.
    /// Cuando es `true`:
    /// - El contagio social incluye la quinta dimensión (Extraversion).
    /// - La política `PsiArgmax` consulta `action_weights_ext` además de
    ///   `action_weights`.
    /// - Las métricas `PsiMetrics` calculan `polarization_ext` y `moran_i_ext`.
    /// - La homofilia mide distancia en 5D (en vez de 4D).
    ///
    /// `false` (default) → bit-exacto al motor histórico Big Four.
    #[serde(default)]
    pub big_five: bool,
    /// Columna extendida de `action_weights` para la quinta dimensión del
    /// psi (Extraversion). Sólo se consulta cuando `big_five = true`. Default
    /// cero → la 5ª dimensión empieza neutra y el caller la sintoniza.
    ///
    /// Default semánticamente plausible:
    /// - Mover (0), Sincronizar (2), Intercambiar (3): premian extraversión.
    /// - Extraer (1), Replicar (4), Degradar (5): neutrales.
    #[serde(default = "default_action_weights_ext")]
    pub action_weights_ext: [f32; 6],
    /// **Tope duro de población** (freno defensivo anti-cuelgue). Cuando la
    /// población viva alcanza este valor, NINGÚN lemming se replica más —
    /// ni por `act_replicar` directo ni por el side-effect de abundancia.
    /// `0` (default) = sin tope → motor histórico bit-exacto. Es la red de
    /// seguridad que garantiza que el overshoot exponencial no congele el
    /// tick O(N²): el N nunca cruza un techo conocido. Ver `density_cap`
    /// para el freno *ecológico* (suave) que hace innecesario llegar acá.
    #[serde(default)]
    pub max_population: u32,
    /// **Capacidad de carga local** — lado en celdas del bloque NxN sobre el
    /// que se mide el hacinamiento. La reproducción se bloquea cuando el
    /// bloque que ocupa el agente ya tiene `density_cap` lemmings o más.
    /// `0` (default) = densidad-dependencia DESACTIVADA → la réplica sólo
    /// mira la energía individual (motor histórico). Un valor típico para
    /// grilla 240² es 8..16. Este es el fix "de verdad": hace que `N*`
    /// emerja de la capacidad del territorio sin overshoot exponencial,
    /// porque un agente saciado en una zona ya poblada NO se replica.
    #[serde(default)]
    pub density_block: u32,
    /// Máximo de lemmings por bloque `density_block × density_block` para que
    /// se permita la réplica. Si la cuenta del bloque local `>= density_cap`,
    /// el agente no replica (ni directo ni por abundancia). Sólo se consulta
    /// cuando `density_block > 0`. `0` con `density_block > 0` bloquearía toda
    /// réplica — usar valores ≥ 1.
    #[serde(default)]
    pub density_cap: u32,
}

/// Default de `SimParams::action_weights_ext` — peso por acción para la 5ª
/// dimensión del psi (Big Five Extraversion). Acciones sociales (Mover,
/// Sincronizar, Intercambiar) premian extraversión; las solitarias o
/// agresivas (Extraer, Replicar, Degradar) son neutrales.
fn default_action_weights_ext() -> [f32; 6] {
    // 0 Mover, 1 Extraer, 2 Sincronizar, 3 Intercambiar, 4 Replicar, 5 Degradar
    [0.4, 0.0, 0.6, 0.8, 0.0, -0.2]
}

/// Default de `SimParams::action_weights` — fila por acción, columna por
/// componente del `vector_psi` (`[ORDEN, MIEDO, CURIOSIDAD, CORRUPTIBILIDAD]`).
fn default_action_weights() -> [[f32; 4]; 6] {
    [
        // 0 Mover         O    M    C    K
        [0.0, -0.5, 1.0, 0.0],
        // 1 Extraer       O    M    C    K
        [0.6, 0.0, 0.0, 0.8],
        // 2 Sincronizar   O    M    C    K
        [0.0, 0.0, 1.0, 0.0],
        // 3 Intercambiar  O    M    C    K
        [1.0, -0.4, 0.0, 0.0],
        // 4 Replicar      O    M    C    K
        [1.0, 0.0, 0.0, 0.0],
        // 5 Degradar      O    M    C    K
        [0.0, -0.8, 0.0, 1.0],
    ]
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
            // Default: psi NO modula efectos → bit-exacto al motor histórico.
            // Subir lentamente (0.3..0.7) para que la psicología empiece a
            // sentirse sin reventar las calibraciones del Default.
            psi_effect_modulation: 0.0,
            // Default: política fija → la acción no se reelige por psi. Esto
            // preserva tests existentes y todos los packs históricos.
            action_policy: ActionPolicy::Fixed,
            action_weights: default_action_weights(),
            // Failsafe: con período 0, ni siquiera `PsiArgmax` reelige.
            policy_reeval_period: 0,
            // Fase B: contagio social desactivado por default. El motor
            // histórico no recorre vecinos sociales, mantiene perf O(N).
            social_radius: 0.0,
            contagion_rate: 0.0,
            // Fase B.2: sin filtro de homofilia → contagio universal cuando
            // se enciende (semántica de B.1).
            homophily_threshold: 0.0,
            // Big Five off por default — el motor mantiene los 4 ejes
            // históricos bit-exacto.
            big_five: false,
            action_weights_ext: default_action_weights_ext(),
            // Frenos de población desactivados por default → el motor
            // histórico y todos los tests que usan `SimParams::default()`
            // siguen exactamente igual (réplica sólo por energía individual,
            // sin tope). La APP los enciende; ver `dominium-app-llimphi`.
            max_population: 0,
            density_block: 0,
            density_cap: 0,
        }
    }
}
