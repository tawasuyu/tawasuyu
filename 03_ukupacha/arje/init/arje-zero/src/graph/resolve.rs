//! Planificador de arranque del genesis.
//!
//! Convierte un conjunto de Cards (las hijas de la Semilla) en un ORDEN de
//! arranque que respeta sus contratos de dependencia:
//!
//!   - **Satisfacibilidad** (punto fijo): una Card cuyos `requires`/`Any`/
//!     `Quorum` no se pueden cumplir ni con TODO el conjunto vivo, o cuyo
//!     `Conflicts` choca con una capacidad presente, se rechaza. Rechazar una
//!     Card le quita sus `provides` al universo, lo que puede volver imposibles
//!     a otras ⇒ se itera hasta estabilizar.
//!   - **Orden topológico** (Kahn): proveedores antes que consumidores, usando
//!     `Card::ordering_deps()` (requires + Any + Quorum.of + After). Determinista
//!     por índice. Lo que queda fuera del orden está en un **ciclo** ⇒ se rechaza.
//!
//! Es una función PURA (sin estado del Init) ⇒ testeable en aislamiento. El
//! grafo la usa en `instantiate_seed_dependencies`; la satisfacibilidad puntual
//! de un spawn dinámico la cubre `Card::deps_satisfied` en `authorize_and_spawn`.

use arje_card::{Capability, EntityCard, UnmetContract};
use std::collections::{BTreeMap, BTreeSet};

/// Resultado de planificar el arranque de un conjunto de Cards.
#[derive(Debug, Default)]
pub struct SpawnPlan {
    /// Índices de las Cards (en `cards`) en orden de arranque: cada Card va
    /// después de los proveedores de las capacidades de las que depende.
    pub order: Vec<usize>,
    /// Cards que NO se pueden arrancar, con el motivo. El índice referencia
    /// `cards`.
    pub rejected: Vec<(usize, RejectReason)>,
}

/// Por qué una Card quedó fuera del plan de arranque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// Sus contratos no se pueden satisfacer ni con todo el conjunto vivo
    /// disponible (capacidad inalcanzable, quórum imposible, o conflicto con
    /// una capacidad presente).
    Unsatisfiable(UnmetContract),
    /// Participa en un ciclo de dependencias: no hay orden de arranque válido.
    Cycle,
}

/// Capacidades alcanzables: las de los proveedores externos (Semilla + entes ya
/// vivos) más las que proveen las Cards aún candidatas.
fn reachable_caps(
    cards: &[EntityCard],
    alive: &[bool],
    external: &BTreeSet<Capability>,
) -> BTreeSet<Capability> {
    let mut s = external.clone();
    for (i, card) in cards.iter().enumerate() {
        if alive[i] {
            s.extend(card.provides.iter().cloned());
        }
    }
    s
}

/// Planifica el arranque de `cards` dado `external` = capacidades que ya proveen
/// entes fuera de este conjunto (típicamente la Semilla: `Spawn`, `Journal`, …).
///
/// Política de conflictos: la Card que DECLARA el `Conflicts` es la que cede
/// (se rechaza) si la capacidad excluida está presente — los proveedores que no
/// declararon nada siguen vivos.
pub fn plan_spawn(cards: &[EntityCard], external: &BTreeSet<Capability>) -> SpawnPlan {
    let n = cards.len();
    let mut alive = vec![true; n];
    let mut rejected: Vec<(usize, RejectReason)> = Vec::new();

    // --- Fase 1: satisfacibilidad a punto fijo. ---
    loop {
        let universe = reachable_caps(cards, &alive, external);
        let mut changed = false;
        for i in 0..n {
            if !alive[i] {
                continue;
            }
            if let Err(unmet) = cards[i].deps_satisfied(&universe) {
                alive[i] = false;
                rejected.push((i, RejectReason::Unsatisfiable(unmet)));
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // --- Fase 2: orden topológico (Kahn) sobre las Cards vivas. ---
    // cap → proveedores vivos de esa cap (dentro del conjunto).
    let mut provided_by: BTreeMap<Capability, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        if !alive[i] {
            continue;
        }
        for cap in &cards[i].provides {
            provided_by.entry(cap.clone()).or_default().push(i);
        }
    }
    // Aristas proveedor → consumidor. Capacidades provistas sólo por `external`
    // NO generan arista (ya están disponibles antes de arrancar nada).
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg: Vec<usize> = vec![0; n];
    let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
    for c in 0..n {
        if !alive[c] {
            continue;
        }
        for cap in cards[c].ordering_deps() {
            if let Some(provs) = provided_by.get(&cap) {
                for &p in provs {
                    if p != c && seen.insert((p, c)) {
                        adj[p].push(c);
                        indeg[c] += 1;
                    }
                }
            }
        }
    }
    // Kahn determinista (menor índice primero, vía BTreeSet).
    let mut queue: BTreeSet<usize> = (0..n).filter(|&i| alive[i] && indeg[i] == 0).collect();
    let mut order = Vec::new();
    while let Some(&node) = queue.iter().next() {
        queue.remove(&node);
        order.push(node);
        for &succ in &adj[node] {
            indeg[succ] -= 1;
            if indeg[succ] == 0 {
                queue.insert(succ);
            }
        }
    }
    // Los vivos que no entraron al orden están en un ciclo.
    let in_order: BTreeSet<usize> = order.iter().copied().collect();
    for i in 0..n {
        if alive[i] && !in_order.contains(&i) {
            rejected.push((i, RejectReason::Cycle));
        }
    }

    SpawnPlan { order, rejected }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arje_card::{Capability, DepContract, EntityCard};
    use std::collections::BTreeSet;

    fn caps(it: impl IntoIterator<Item = Capability>) -> BTreeSet<Capability> {
        it.into_iter().collect()
    }

    fn card(label: &str) -> EntityCard {
        EntityCard::new(label)
    }

    /// Orden topológico: el proveedor arranca antes que el consumidor aunque se
    /// declaren en orden inverso.
    #[test]
    fn ordena_proveedor_antes_que_consumidor() {
        let mut consumer = card("consumer");
        consumer.requires = caps([Capability::LegacyLogind]);
        let mut provider = card("provider");
        provider.provides = caps([Capability::LegacyLogind]);

        // Declaradas en orden INVERSO (consumer primero).
        let cards = vec![consumer, provider];
        let plan = plan_spawn(&cards, &caps([Capability::Spawn]));
        assert!(plan.rejected.is_empty(), "ninguna debe rechazarse");
        // provider (idx 1) debe ir antes que consumer (idx 0).
        let pos = |i| plan.order.iter().position(|&x| x == i).unwrap();
        assert!(pos(1) < pos(0), "el proveedor arranca primero");
    }

    /// "A o B": basta un proveedor de cualquiera de las alternativas.
    #[test]
    fn any_se_satisface_con_una_alternativa() {
        let mut greeter = card("greeter");
        greeter.contracts = vec![DepContract::Any(caps([
            Capability::LegacyLogind,
            Capability::Journal,
        ]))];
        let mut provider = card("logind");
        provider.provides = caps([Capability::LegacyLogind]);

        let plan = plan_spawn(&[greeter, provider], &caps([]));
        assert!(plan.rejected.is_empty());
        assert_eq!(plan.order.len(), 2);
    }

    /// "A o B" sin ningún proveedor de las alternativas ⇒ insatisfacible.
    #[test]
    fn any_sin_proveedor_se_rechaza() {
        let mut greeter = card("greeter");
        greeter.contracts = vec![DepContract::Any(caps([
            Capability::LegacyLogind,
            Capability::Journal,
        ]))];
        let plan = plan_spawn(&[greeter], &caps([]));
        assert_eq!(plan.order.len(), 0);
        assert!(matches!(
            plan.rejected.as_slice(),
            [(0, RejectReason::Unsatisfiable(_))]
        ));
    }

    /// Quórum N-de-M con suficientes proveedores.
    #[test]
    fn quorum_con_suficientes_proveedores() {
        let mut q = card("quorum");
        q.contracts = vec![DepContract::Quorum {
            of: caps([Capability::Journal, Capability::LegacyLogind]),
            at_least: 2,
        }];
        let mut a = card("a");
        a.provides = caps([Capability::Journal]);
        let mut b = card("b");
        b.provides = caps([Capability::LegacyLogind]);
        let plan = plan_spawn(&[q, a, b], &caps([]));
        assert!(plan.rejected.is_empty());
        // q (idx 0) arranca después de a y b.
        let pos = |i| plan.order.iter().position(|&x| x == i).unwrap();
        assert!(pos(0) > pos(1) && pos(0) > pos(2));
    }

    /// Conflicts: la Card que declara el conflicto cede si la cap está presente.
    #[test]
    fn conflicts_rechaza_al_declarante() {
        let mut exclusivo = card("exclusivo");
        exclusivo.contracts = vec![DepContract::Conflicts(caps([Capability::LegacyLogind]))];
        let mut otro = card("provee-logind");
        otro.provides = caps([Capability::LegacyLogind]);

        let plan = plan_spawn(&[exclusivo, otro], &caps([]));
        // exclusivo (idx 0) se rechaza; otro (idx 1) sobrevive.
        assert!(matches!(
            plan.rejected.as_slice(),
            [(0, RejectReason::Unsatisfiable(UnmetContract::Conflict(_)))]
        ));
        assert_eq!(plan.order, vec![1]);
    }

    /// Ciclo: A requiere algo que provee B, B requiere algo que provee A.
    #[test]
    fn ciclo_se_detecta_y_rechaza() {
        let mut a = card("a");
        a.provides = caps([Capability::Journal]);
        a.requires = caps([Capability::LegacyLogind]);
        let mut b = card("b");
        b.provides = caps([Capability::LegacyLogind]);
        b.requires = caps([Capability::Journal]);

        let plan = plan_spawn(&[a, b], &caps([]));
        // Ambas satisfacen en el universo (cada cap la provee la otra) pero el
        // orden es imposible ⇒ ciclo.
        assert!(plan.order.is_empty());
        assert_eq!(plan.rejected.len(), 2);
        assert!(plan
            .rejected
            .iter()
            .all(|(_, r)| *r == RejectReason::Cycle));
    }

    /// Cascada: si B se rechaza, A —que dependía de B— también cae.
    #[test]
    fn rechazo_en_cascada() {
        // C requiere una cap que nadie provee ⇒ se rechaza.
        let mut c = card("c");
        c.requires = caps([Capability::FilesystemRoot]);
        c.provides = caps([Capability::Journal]);
        // A depende de Journal, que sólo proveería C (rechazada) ⇒ A cae también.
        let mut a = card("a");
        a.requires = caps([Capability::Journal]);

        let plan = plan_spawn(&[a, c], &caps([]));
        assert!(plan.order.is_empty());
        assert_eq!(plan.rejected.len(), 2);
    }

    /// Caso feliz sin contratos: todas arrancan, orden = declaración.
    #[test]
    fn sin_contratos_arrancan_todas() {
        let plan = plan_spawn(&[card("a"), card("b"), card("c")], &caps([Capability::Spawn]));
        assert!(plan.rejected.is_empty());
        assert_eq!(plan.order, vec![0, 1, 2]);
    }
}
