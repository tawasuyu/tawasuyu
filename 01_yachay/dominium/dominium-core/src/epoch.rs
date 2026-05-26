//! Clasificación cualitativa del estado del mundo — "qué época estamos
//! viviendo" a partir de las métricas agregadas.
//!
//! No es una capa del motor: el motor sigue ignorando que existen "edades de
//! oro". Esto es un **lector** que toma `WorldStats` y traduce los números a
//! una etiqueta legible para mostrar en el HUD o etiquetar filas del CSV.
//!
//! Las heurísticas son honestas y pocas: seis arquetipos con umbrales fijos,
//! orden de prelación explícito. Cuando el mundo no encaja en ningún
//! arquetipo, cae a [`Epoch::Equilibrio`].

use crate::metrics::WorldStats;
use serde::{Deserialize, Serialize};

/// Arquetipos macro que el mundo puede atravesar. La clasificación corre
/// sobre la `WorldStats` instantánea — no hay memoria, así que el "Auge" no
/// implica que la pob esté creciendo (no tenemos derivadas), sino que el
/// estado actual *parece un auge*: mucha materia, mucha energía, Gini bajo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Epoch {
    /// Mundo vacío o casi — la población se extinguió o está al borde.
    Colapso,
    /// Mucha gente, poca materia/energía promedio: hay hambre.
    Hambruna,
    /// Gini extremo: pocos concentran la energía, la mayoría malvive.
    Imperio,
    /// Mucha materia, mucha energía, Gini moderado: prosperidad amplia.
    EdadDeOro,
    /// Más materia que pob., reservas creciendo: el motor está "respirando".
    Auge,
    /// Default: nada extremo, el sistema flota.
    Equilibrio,
}

impl Epoch {
    /// Etiqueta corta y legible para HUD/CSV. Sin tilde donde podría romper
    /// renderers ASCII-only.
    pub fn label(self) -> &'static str {
        match self {
            Epoch::Colapso => "colapso",
            Epoch::Hambruna => "hambruna",
            Epoch::Imperio => "imperio",
            Epoch::EdadDeOro => "edad-de-oro",
            Epoch::Auge => "auge",
            Epoch::Equilibrio => "equilibrio",
        }
    }

    /// Clasifica el mundo según `stats`. Orden de prelación: colapso →
    /// hambruna → imperio → edad-de-oro → auge → equilibrio. La primera
    /// regla que matchea gana — los umbrales están elegidos para que sólo
    /// una matchee a la vez en la práctica.
    pub fn classify(stats: &WorldStats) -> Epoch {
        // 1. Colapso: muy poca gente — o se está extinguiendo o ya pasó.
        if stats.n < 5 {
            return Epoch::Colapso;
        }
        let nf = stats.n as f32;
        let energia_por_capita = stats.total_energia / nf;
        let materia_por_capita = stats.total_materia / nf;

        // 2. Hambruna: muchos, poca energía y poca materia disponible.
        if energia_por_capita < 8.0 && materia_por_capita < 30.0 {
            return Epoch::Hambruna;
        }
        // 3. Imperio: concentración brutal de energía aunque haya recursos.
        if stats.gini_energia > 0.55 {
            return Epoch::Imperio;
        }
        // 4. Edad de oro: holgura energética y suelo fértil, sin grandes
        //    diferencias. El umbral de materia es absoluto para que mundos
        //    chicos no clasifiquen como "edad de oro" a falta de masa.
        if energia_por_capita > 25.0
            && materia_por_capita > 60.0
            && stats.gini_energia < 0.35
        {
            return Epoch::EdadDeOro;
        }
        // 5. Auge: materia abundante sin que la energía haya explotado aún.
        //    Si materia_por_capita es alto pero no llegamos al combo de
        //    "edad de oro" es porque la energía aún no se reparte — está
        //    pasando algo bueno pero todavía no llegó a la mesa.
        if materia_por_capita > 80.0 {
            return Epoch::Auge;
        }
        // 6. Default.
        Epoch::Equilibrio
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;

    fn stats_from(
        n: usize,
        energia: f32,
        materia: f32,
        gini: f32,
    ) -> WorldStats {
        WorldStats {
            n,
            gini_energia: gini,
            var_psi: [0.0; 4],
            action_counts: [0; 6],
            total_materia: materia,
            total_psique: 0.0,
            total_poder: 0.0,
            total_oro: 0.0,
            total_degradacion: 0.0,
            mean_edad: 0.0,
            total_energia: energia,
        }
    }

    #[test]
    fn colapso_when_population_is_tiny() {
        let s = stats_from(2, 100.0, 100.0, 0.0);
        assert_eq!(Epoch::classify(&s), Epoch::Colapso);
    }

    #[test]
    fn hambruna_when_per_capita_is_low() {
        // 50 lemmings, 50 energía total (1.0/cap), 100 materia total (2.0/cap).
        let s = stats_from(50, 50.0, 100.0, 0.1);
        assert_eq!(Epoch::classify(&s), Epoch::Hambruna);
    }

    #[test]
    fn imperio_when_gini_is_high() {
        // Hay recursos pero un puñado concentra todo.
        let s = stats_from(50, 5000.0, 5000.0, 0.7);
        assert_eq!(Epoch::classify(&s), Epoch::Imperio);
    }

    #[test]
    fn edad_de_oro_when_abundant_and_egalitarian() {
        // 30/cap energía, 100/cap materia, gini bajo.
        let s = stats_from(50, 1500.0, 5000.0, 0.20);
        assert_eq!(Epoch::classify(&s), Epoch::EdadDeOro);
    }

    #[test]
    fn auge_when_materia_abundant_but_energy_modest() {
        // Mucha materia/cap pero energía/cap insuficiente para edad de oro.
        let s = stats_from(50, 600.0, 5000.0, 0.30);
        assert_eq!(Epoch::classify(&s), Epoch::Auge);
    }

    #[test]
    fn equilibrio_is_the_default() {
        let s = stats_from(50, 700.0, 2000.0, 0.30);
        assert_eq!(Epoch::classify(&s), Epoch::Equilibrio);
    }

    #[test]
    fn empty_world_collapses() {
        let w = World::new(4, 4);
        let s = WorldStats::from_world(&w);
        assert_eq!(Epoch::classify(&s), Epoch::Colapso);
    }

    #[test]
    fn label_is_stable_and_ascii_safe() {
        // Sanity para CSV/logs: ningún arquetipo emite cadena vacía.
        for e in [
            Epoch::Colapso,
            Epoch::Hambruna,
            Epoch::Imperio,
            Epoch::EdadDeOro,
            Epoch::Auge,
            Epoch::Equilibrio,
        ] {
            let l = e.label();
            assert!(!l.is_empty());
            assert!(l.is_ascii());
        }
    }
}
