//! Plataforma de hipótesis canónicas — el cuerpo experimental de dominium.
//!
//! Cada hipótesis es una **aserción cuantitativa** sobre el motor: "si encendés
//! X, esperás que Y suba/baje/no cambie". Acá las codificamos como tests
//! Monte Carlo: corremos N réplicas con seeds distintos, calculamos la media
//! del estadístico y comparamos contra la rama de control con tolerancia
//! holgada (el simulador es determinista por seed pero ruidoso entre seeds).
//!
//! No son tests de "no rompe" (los `--lib` ya cubren eso). Son **falsadores
//! de fenómenos emergentes**: si alguien rompe la mecánica del contagio o de
//! la homofilia, estos tests caen aunque el binario compile.
//!
//! Convención: cada hipótesis vive en un test con nombre `hipotesis_*`. El
//! nombre describe la causalidad esperada ("homofilia_sube_morans_i"). El
//! cuerpo monta dos configuraciones — control y tratamiento — corre N
//! réplicas con seeds distintos y reporta la estadística agregada con
//! `assert!(...)` sobre la diferencia de medias.

use dominium_core::{
    ActionPolicy, PsiMetrics, SimParams, World, WorldStats, MORANS_RADIUS_DEFAULT,
};
use dominium_physics::tick::run;

/// LCG mínimo determinista — el mismo que usa la app, pero local al test
/// para no acoplar a sus internals.
struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Cantidad de réplicas Monte Carlo. 8 es suficiente para distinguir efectos
/// macroscópicos en estos sistemas; subir si las medias quedan cerca.
const MC_REPS: usize = 8;
/// Lado de la grilla cuadrada usada en los experimentos.
const GRID: usize = 40;
/// Población inicial por experimento. Suficientemente grande para que las
/// métricas tengan señal, suficientemente chica para que 8 réplicas × 200
/// ticks no demoren más de un par de segundos.
const POP: usize = 200;
/// Pasos de simulación por réplica. 200 alcanza para que el contagio sature
/// y la polarización converja en sus regímenes característicos.
const STEPS: usize = 200;

/// Construye un mundo con `POP` lemmings dispersos uniformemente y psi
/// también uniforme en `[0, 1]`. El seed controla *todo* el ruido: misma
/// seed → misma poblacion → mismo trayectoria.
fn build_world(seed: u64) -> World {
    let mut w = World::new(GRID, GRID);
    let mut rng = Lcg::new(seed);
    // Pequeña materia uniforme para que los Extractores no se mueran.
    for c in w.grid.materia.iter_mut() {
        *c = 5.0;
    }
    for _ in 0..POP {
        let x = rng.next_f32() * (GRID as f32 - 1.0);
        let y = rng.next_f32() * (GRID as f32 - 1.0);
        let psi = [
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
        ];
        let i = w.lemmings.spawn_big5(x, y, 50.0, psi, rng.next_f32());
        // Asignación de acciones balanceada para que no todos hagan lo mismo.
        w.lemmings.accion[i] = (rng.next_u32() % 6) as u8;
    }
    w
}

/// Corre `steps` ticks y devuelve las métricas finales.
fn run_and_measure(w: &mut World, p: &SimParams, steps: usize) -> (PsiMetrics, WorldStats) {
    run(w, p, steps);
    (PsiMetrics::from_world(w), WorldStats::from_world(w))
}

/// Promedia una métrica escalar sobre `MC_REPS` réplicas.
fn mean_over_reps<F>(mut compute: F) -> f64
where
    F: FnMut(u64) -> f32,
{
    let mut sum: f64 = 0.0;
    for r in 0..MC_REPS {
        let seed = 0xD0_31_31_07u64.wrapping_add(r as u64 * 0x9E37_79B9);
        sum += compute(seed) as f64;
    }
    sum / MC_REPS as f64
}

// ─────────────────────── Hipótesis 1 ────────────────────────
//
// **↑ homofilia ⇒ ↑ Moran's I.**
//
// Cuando la homofilia es fuerte, los agentes sólo se influyen con los
// psicológicamente parecidos. Las tribus emergen y se vuelven espacialmente
// segregadas → la autocorrelación espacial (Moran's I) del psi sube.
// Sin homofilia, el contagio universal homogeneiza y Moran's I tiende a 0.
//
// Estadístico: promedio de `moran_i[0]` (ORDEN) sobre 8 réplicas.

#[test]
fn hipotesis_homofilia_sube_morans_i() {
    let mean_baseline = mean_over_reps(|seed| {
        let mut w = build_world(seed);
        let mut p = SimParams::default();
        p.social_radius = 5.0;
        p.contagion_rate = 0.15;
        p.homophily_threshold = 0.0; // sin homofilia → contagio universal
        let (m, _) = run_and_measure(&mut w, &p, STEPS);
        m.moran_i[0].abs()
    });
    let mean_treatment = mean_over_reps(|seed| {
        let mut w = build_world(seed);
        let mut p = SimParams::default();
        p.social_radius = 5.0;
        p.contagion_rate = 0.15;
        p.homophily_threshold = 0.4; // homofilia fuerte → tribus
        let (m, _) = run_and_measure(&mut w, &p, STEPS);
        m.moran_i[0].abs()
    });
    eprintln!(
        "[H1] Moran_i[ORDEN]: baseline {:.4} vs homofilia {:.4}",
        mean_baseline, mean_treatment
    );
    // El contagio universal ya produce algo de clustering por la geografía
    // (radius 5 << diagonal del grid 40×40), así que el baseline arranca
    // alto. La homofilia debe seguir levantándolo de forma consistente —
    // umbral mínimo 0.05 absoluto sobre el baseline.
    assert!(
        mean_treatment > mean_baseline + 0.05,
        "homofilia no levantó Moran's I: {} ≤ {} + 0.05",
        mean_treatment,
        mean_baseline
    );
}

// ─────────────────────── Hipótesis 2 ────────────────────────
//
// **↑ contagion_rate con radio que cubre toda la grilla ⇒ ↓ varianza
// poblacional del psi.**
//
// Con contagio fuerte y radio suficiente para conectar a todos los agentes,
// la población converge a su promedio global y la varianza colapsa. Usamos
// `var_psi` en vez de polarización porque Esteban-Ray normaliza por span:
// con radios chicos pueden quedar clusters locales y la polarización subir;
// la varianza es la métrica honesta de "convergencia al consenso".

#[test]
fn hipotesis_contagio_universal_reduce_varianza() {
    let mean_baseline = mean_over_reps(|seed| {
        let mut w = build_world(seed);
        let mut p = SimParams::default();
        p.social_radius = 0.0;
        p.contagion_rate = 0.0;
        let (_, s) = run_and_measure(&mut w, &p, STEPS);
        s.var_psi[0]
    });
    let mean_treatment = mean_over_reps(|seed| {
        let mut w = build_world(seed);
        let mut p = SimParams::default();
        // Radio que cubre la diagonal del grid (40·√2 ≈ 57) → todos vecinos.
        p.social_radius = 60.0;
        p.contagion_rate = 0.30;
        p.homophily_threshold = 0.0;
        let (_, s) = run_and_measure(&mut w, &p, STEPS);
        s.var_psi[0]
    });
    eprintln!(
        "[H2] var(psi[ORDEN]): baseline {:.6} vs contagio universal {:.6}",
        mean_baseline, mean_treatment
    );
    // Contagio verdaderamente universal debe colapsar la varianza al menos
    // a la mitad (medirla por debajo del 50% del baseline).
    assert!(
        mean_treatment < mean_baseline * 0.5,
        "contagio no colapsó la varianza: {} ≥ 0.5 × {}",
        mean_treatment,
        mean_baseline
    );
}

// ─────────────────────── Hipótesis 3 ────────────────────────
//
// **PsiArgmax + psi_modulation ⇒ |corr(psi, accion)| más alta que Fixed.**
//
// Con la política psicológica encendida, la acción del agente se vuelve
// función del psi. La correlación punto-biserial entre cada componente del
// psi y la acción mayoritaria que esa componente premia debe crecer.
// Estadístico: max sobre `(k, a)` del valor absoluto de `psi_action_corr[k][a]`.

#[test]
fn hipotesis_psi_argmax_aumenta_correlacion_psi_accion() {
    fn max_abs_corr(corr: &[[f32; 6]; 4]) -> f32 {
        let mut m: f32 = 0.0;
        for k in 0..4 {
            for a in 0..6 {
                let v = corr[k][a].abs();
                if v > m {
                    m = v;
                }
            }
        }
        m
    }
    let mean_baseline = mean_over_reps(|seed| {
        let mut w = build_world(seed);
        let p = SimParams::default(); // Fixed
        let (m, _) = run_and_measure(&mut w, &p, STEPS);
        max_abs_corr(&m.psi_action_corr)
    });
    let mean_treatment = mean_over_reps(|seed| {
        let mut w = build_world(seed);
        let mut p = SimParams::default();
        p.action_policy = ActionPolicy::PsiArgmax;
        p.policy_reeval_period = 5; // reelige cada 5 ticks
        p.psi_effect_modulation = 0.5;
        let (m, _) = run_and_measure(&mut w, &p, STEPS);
        max_abs_corr(&m.psi_action_corr)
    });
    eprintln!(
        "[H3] max |corr(psi, accion)|: baseline {:.4} vs PsiArgmax {:.4}",
        mean_baseline, mean_treatment
    );
    assert!(
        mean_treatment > mean_baseline + 0.10,
        "PsiArgmax no aumentó correlación: {} no supera {} + 0.10",
        mean_treatment,
        mean_baseline
    );
}

// ─────────────────────── Hipótesis 4 ────────────────────────
//
// **regrowth_rate > 0 ⇒ población sostenida vs sin regrowth.**
//
// Sin regrowth, la materia se agota por Extraer y la población colapsa.
// Con regrowth, el cierre termodinámico mantiene un punto fijo `N* > 0`.

#[test]
fn hipotesis_regrowth_sostiene_poblacion() {
    let mean_baseline = mean_over_reps(|seed| {
        let mut w = build_world(seed);
        let mut p = SimParams::default();
        p.regrowth_rate = 0.0; // sin regrowth
        p.carrying_capacity = 0.0;
        let (_, s) = run_and_measure(&mut w, &p, STEPS);
        s.n as f32
    });
    let mean_treatment = mean_over_reps(|seed| {
        let mut w = build_world(seed);
        let p = SimParams::default(); // default tiene regrowth_rate > 0
        let (_, s) = run_and_measure(&mut w, &p, STEPS);
        s.n as f32
    });
    eprintln!(
        "[H4] N final: sin regrowth {:.1} vs con regrowth {:.1}",
        mean_baseline, mean_treatment
    );
    assert!(
        mean_treatment > mean_baseline + 5.0,
        "regrowth no sostuvo población: {} ≤ {} + 5",
        mean_treatment,
        mean_baseline
    );
}

// ─────────────────────── Hipótesis 5 ────────────────────────
//
// **Big Five con peso ext positivo ⇒ acciones sociales (Mover/Sync/Intercambiar)
// crecen vs Big Four bit-exacto.**
//
// Con `big_five=true` y `action_weights_ext` premiando Intercambiar/Sincronizar,
// la política argmax debería empujar a más agentes hacia esas acciones, sobre
// todo si `psi5` (Extraversion) es alta en promedio (lo es: nuestro builder
// muestrea uniforme en [0, 1] → media 0.5).

#[test]
fn hipotesis_big_five_levanta_acciones_sociales() {
    fn share_social(s: &WorldStats) -> f32 {
        let total: u32 = s.action_counts.iter().sum();
        if total == 0 {
            return 0.0;
        }
        // Mover (0) + Sincronizar (2) + Intercambiar (3)
        (s.action_counts[0] + s.action_counts[2] + s.action_counts[3]) as f32
            / total as f32
    }
    let mean_baseline = mean_over_reps(|seed| {
        let mut w = build_world(seed);
        let mut p = SimParams::default();
        p.action_policy = ActionPolicy::PsiArgmax;
        p.policy_reeval_period = 5;
        p.big_five = false;
        let (_, s) = run_and_measure(&mut w, &p, STEPS);
        share_social(&s)
    });
    let mean_treatment = mean_over_reps(|seed| {
        let mut w = build_world(seed);
        let mut p = SimParams::default();
        p.action_policy = ActionPolicy::PsiArgmax;
        p.policy_reeval_period = 5;
        p.big_five = true;
        // action_weights_ext default: Mover 0.4, Sync 0.6, Intercambiar 0.8.
        let (_, s) = run_and_measure(&mut w, &p, STEPS);
        share_social(&s)
    });
    eprintln!(
        "[H5] fracción social: Big4 {:.4} vs Big5 {:.4}",
        mean_baseline, mean_treatment
    );
    assert!(
        mean_treatment > mean_baseline + 0.05,
        "Big Five no levantó fracción social: {} ≤ {} + 0.05",
        mean_treatment,
        mean_baseline
    );
}

// ─────────────────────── Hipótesis 6 ────────────────────────
//
// **Determinismo bit-exacto:** misma seed → misma trayectoria, incluso
// con todas las mecánicas opt-in encendidas a la vez. Si esto cae, algún
// componente nuevo metió no-determinismo (HashMap iteration, RNG global,
// reducción paralela). Es el guardián más importante de la plataforma.

#[test]
fn hipotesis_determinismo_bit_exacto_con_todas_las_mecanicas() {
    let mut p = SimParams::default();
    p.social_radius = 4.0;
    p.contagion_rate = 0.10;
    p.homophily_threshold = 0.4;
    p.action_policy = ActionPolicy::PsiArgmax;
    p.policy_reeval_period = 7;
    p.psi_effect_modulation = 0.6;
    p.big_five = true;
    p.season_period = 50;
    p.season_amplitude = 0.3;
    let mut a = build_world(0xCAFE_BABE);
    let mut b = build_world(0xCAFE_BABE);
    run(&mut a, &p, 150);
    run(&mut b, &p, 150);
    assert_eq!(a.lemmings.pos_x, b.lemmings.pos_x);
    assert_eq!(a.lemmings.pos_y, b.lemmings.pos_y);
    assert_eq!(a.lemmings.energia, b.lemmings.energia);
    assert_eq!(a.lemmings.vector_psi, b.lemmings.vector_psi);
    assert_eq!(a.lemmings.psi5, b.lemmings.psi5);
    assert_eq!(a.lemmings.accion, b.lemmings.accion);
    assert_eq!(a.grid.materia, b.grid.materia);
    let _ = MORANS_RADIUS_DEFAULT; // chequeo que la constante sigue exportada
}
