//! `pluma-reactor` — el esqueleto reactivo del multilienzo de pluma.
//!
//! La unificación de fondo (2026-06-19): el documento de pluma **es una hoja**
//! tipo nakui/yupay vista como sankey. Cada **sección** es una **celda** con
//! coordenada `(lienzo, sección)` = [`CellRef`] (columna = lienzo, fila =
//! sección). Una **relación-función** entre lienzos es una **fórmula** yupay:
//!
//! ```text
//! inglés!sec0 = TRADUCIR(español!sec0)      // en A1:  B1 = TRADUCIR(A1)
//! ```
//!
//! El reactor:
//!   - guarda la fórmula de cada celda-destino ([`Reactor::set_formula`]),
//!   - mantiene el grafo de dependencias (extraído con [`yupay_core::dependencies`]),
//!   - da el **orden topológico** de recálculo cuando una celda cambia
//!     ([`Reactor::downstream`]).
//!
//! El **cómputo del valor** (texto) lo hace el caller:
//!   - **síncrono** vía [`Reactor::eval`] — funciones baratas y tests;
//!   - **asíncrono** (traducción/resumen por LLM) — el caller lee `downstream` +
//!     la fórmula y dispara el `Transformacion` de pluma; al volver, mete el
//!     texto en su resolver y propaga otra vez.
//!
//! Las referencias son **arbitrarias** (cualquier celda puede apuntar a
//! cualquier otra): las relaciones/sankeys **pueden cruzar**, el orden de
//! secciones **no** es obligatorio.

use std::collections::{HashMap, HashSet, VecDeque};

use yupay_core::{compile, dependencies, eval_formula, FormulaExpr};

// Re-exportados para que los callers (el puente de pluma) no dependan de
// `yupay-core` directamente.
pub use yupay_core::{CellRef, CellResolver, FuncDispatch, ParseError, SheetValue};

/// Grafo de relaciones-fórmula entre secciones-celda + recálculo propagado.
#[derive(Default)]
pub struct Reactor {
    /// Fórmula compilada de cada celda-destino.
    formulas: HashMap<CellRef, FormulaExpr>,
    /// Dependencias directas de cada celda (las celdas que su fórmula lee).
    deps: HashMap<CellRef, Vec<CellRef>>,
    /// Inverso: `dependents[d]` = celdas cuya fórmula referencia a `d`.
    dependents: HashMap<CellRef, HashSet<CellRef>>,
}

impl Reactor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Define (o redefine) la celda `target` con la fórmula `src` (sintaxis
    /// yupay, p.ej. `"TRADUCIR(A1)"`). Extrae sus dependencias y actualiza el
    /// grafo, reemplazando las relaciones viejas de `target`. Error si no
    /// parsea.
    pub fn set_formula(&mut self, target: CellRef, src: &str) -> Result<(), ParseError> {
        let expr = compile(src)?;
        let nuevas = dependencies(&expr);
        // Sacar las aristas viejas de `target` (si redefinimos).
        if let Some(viejas) = self.deps.remove(&target) {
            for d in viejas {
                if let Some(s) = self.dependents.get_mut(&d) {
                    s.remove(&target);
                }
            }
        }
        // Plantar las nuevas.
        for d in &nuevas {
            self.dependents.entry(*d).or_default().insert(target);
        }
        self.deps.insert(target, nuevas);
        self.formulas.insert(target, expr);
        Ok(())
    }

    /// Quita la fórmula (y las aristas) de `target`. No-op si no tenía.
    pub fn clear_formula(&mut self, target: CellRef) {
        self.formulas.remove(&target);
        if let Some(viejas) = self.deps.remove(&target) {
            for d in viejas {
                if let Some(s) = self.dependents.get_mut(&d) {
                    s.remove(&target);
                }
            }
        }
    }

    /// La fórmula compilada de `target`, si tiene.
    pub fn formula(&self, target: CellRef) -> Option<&FormulaExpr> {
        self.formulas.get(&target)
    }

    /// Celdas a **recomputar** cuando `changed` cambia, en **orden topológico**
    /// (cada celda aparece después de todas las que la alimentan dentro del
    /// conjunto afectado). No incluye a `changed`. Si hubiera un ciclo, las
    /// celdas del ciclo quedan fuera del resultado (no se cuelga).
    pub fn downstream(&self, changed: CellRef) -> Vec<CellRef> {
        // 1) Alcance: todo lo aguas-abajo de `changed` por `dependents`.
        let mut afectadas: HashSet<CellRef> = HashSet::new();
        let mut cola: VecDeque<CellRef> = VecDeque::from([changed]);
        while let Some(c) = cola.pop_front() {
            if let Some(deps) = self.dependents.get(&c) {
                for &t in deps {
                    if afectadas.insert(t) {
                        cola.push_back(t);
                    }
                }
            }
        }
        // 2) Orden topológico (Kahn) DENTRO del conjunto afectado: el in-degree
        //    cuenta sólo dependencias que también están afectadas (las externas
        //    ya están computadas).
        let mut indeg: HashMap<CellRef, usize> = HashMap::with_capacity(afectadas.len());
        for &c in &afectadas {
            let n = self
                .deps
                .get(&c)
                .map_or(0, |ds| ds.iter().filter(|d| afectadas.contains(*d)).count());
            indeg.insert(c, n);
        }
        // Cola de listos, ordenada por (col, row) para un orden determinista.
        let mut listos: Vec<CellRef> = indeg
            .iter()
            .filter(|(_, &n)| n == 0)
            .map(|(&c, _)| c)
            .collect();
        ordenar(&mut listos);
        let mut orden: Vec<CellRef> = Vec::with_capacity(afectadas.len());
        let mut i = 0;
        while i < listos.len() {
            let c = listos[i];
            i += 1;
            orden.push(c);
            if let Some(ts) = self.dependents.get(&c) {
                let mut nuevos: Vec<CellRef> = Vec::new();
                for &t in ts {
                    if let Some(n) = indeg.get_mut(&t) {
                        *n -= 1;
                        if *n == 0 {
                            nuevos.push(t);
                        }
                    }
                }
                ordenar(&mut nuevos);
                listos.extend(nuevos);
            }
        }
        orden
    }

    /// Evalúa la fórmula de `target` **síncronamente** con un resolver (los
    /// valores ya computados de sus dependencias — el orden de `downstream` lo
    /// garantiza) + un dispatcher de funciones. Para funciones LLM async el
    /// caller NO usa esto: lee [`Reactor::formula`] y dispara su
    /// `Transformacion`. `None` si `target` no tiene fórmula.
    pub fn eval(
        &self,
        target: CellRef,
        resolver: &dyn CellResolver,
        funcs: &dyn FuncDispatch,
    ) -> Option<SheetValue> {
        self.formulas
            .get(&target)
            .map(|e| eval_formula(e, resolver, funcs))
    }
}

/// Orden estable por (columna, fila) — determinismo para tests y para que el
/// recálculo recorra siempre igual.
fn ordenar(cells: &mut [CellRef]) {
    cells.sort_by_key(|c| (c.col, c.row));
}

#[cfg(test)]
mod tests {
    use super::*;
    use yupay_core::FormulaArg;

    fn c(s: &str) -> CellRef {
        s.parse().unwrap()
    }

    /// Dispatcher de prueba: `TRADUCIR` = mayúsculas, `RESUMIR` = primera
    /// palabra. (En la app real estas son transformaciones LLM async.)
    struct Funcs;
    impl FuncDispatch for Funcs {
        fn call(&self, name: &str, args: &[FormulaArg]) -> SheetValue {
            let txt = match args.first() {
                Some(FormulaArg::Value(v)) => v.to_display_string(),
                _ => String::new(),
            };
            match name {
                "TRADUCIR" => SheetValue::Text(txt.to_uppercase()),
                "RESUMIR" => {
                    SheetValue::Text(txt.split_whitespace().next().unwrap_or("").to_string())
                }
                _ => SheetValue::Empty,
            }
        }
    }

    #[test]
    fn relacion_simple_y_eval() {
        let mut r = Reactor::new();
        // inglés!s0 = TRADUCIR(español!s0)  →  B1 = TRADUCIR(A1)
        r.set_formula(c("B1"), "TRADUCIR(A1)").unwrap();
        // Cambiar A1 obliga a recomputar B1.
        assert_eq!(r.downstream(c("A1")), vec![c("B1")]);
        // El valor: A1 = "hola mundo" → B1 = "HOLA MUNDO".
        let mut hoja: HashMap<CellRef, SheetValue> = HashMap::new();
        hoja.insert(c("A1"), SheetValue::Text("hola mundo".into()));
        let v = r.eval(c("B1"), &hoja, &Funcs).unwrap();
        assert_eq!(v, SheetValue::Text("HOLA MUNDO".into()));
    }

    #[test]
    fn cadena_propaga_en_orden_topologico() {
        let mut r = Reactor::new();
        // A1 → B1 → C1 : B1=TRADUCIR(A1), C1=RESUMIR(B1)
        r.set_formula(c("B1"), "TRADUCIR(A1)").unwrap();
        r.set_formula(c("C1"), "RESUMIR(B1)").unwrap();
        // Cambiar A1 recomputa B1 y luego C1, en ese orden.
        assert_eq!(r.downstream(c("A1")), vec![c("B1"), c("C1")]);
        // Cambiar B1 recomputa sólo C1.
        assert_eq!(r.downstream(c("B1")), vec![c("C1")]);
    }

    #[test]
    fn redefinir_quita_la_relacion_vieja() {
        let mut r = Reactor::new();
        r.set_formula(c("B1"), "TRADUCIR(A1)").unwrap();
        assert_eq!(r.downstream(c("A1")), vec![c("B1")]);
        // Redefinir B1 para que dependa de otra celda → A1 ya no lo afecta.
        r.set_formula(c("B1"), "TRADUCIR(X9)").unwrap();
        assert!(r.downstream(c("A1")).is_empty());
        assert_eq!(r.downstream(c("X9")), vec![c("B1")]);
    }

    #[test]
    fn las_relaciones_pueden_cruzar() {
        // El orden de secciones NO es obligatorio: B1 puede ser función de A2 y
        // B2 de A1 (los sankeys se cruzan).
        let mut r = Reactor::new();
        r.set_formula(c("B1"), "TRADUCIR(A2)").unwrap();
        r.set_formula(c("B2"), "TRADUCIR(A1)").unwrap();
        assert_eq!(r.downstream(c("A1")), vec![c("B2")]);
        assert_eq!(r.downstream(c("A2")), vec![c("B1")]);
    }

    #[test]
    fn una_fuente_a_muchas_dependientes() {
        // 1-n: A1 alimenta a B1, C1 y D1.
        let mut r = Reactor::new();
        r.set_formula(c("B1"), "TRADUCIR(A1)").unwrap();
        r.set_formula(c("C1"), "TRADUCIR(A1)").unwrap();
        r.set_formula(c("D1"), "TRADUCIR(A1)").unwrap();
        let abajo = r.downstream(c("A1"));
        assert_eq!(abajo.len(), 3);
        assert!([c("B1"), c("C1"), c("D1")].iter().all(|x| abajo.contains(x)));
    }
}
