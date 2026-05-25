//! `pluma-notebook-exec` — ejecución de notebooks sobre un [`Kernel`] abstracto.
//!
//! Recorre el DAG en orden topológico, ejecuta cada celda de código contra
//! el kernel, marca el estado resultante (`Fresh` o `Failed`) y propaga el
//! skip a los descendientes de cualquier celda que falle. Markdown y Embed
//! se consideran "ejecutadas" trivialmente (pasan a `Fresh` sin tocar kernel).
//!
//! El kernel es abstracto a propósito: una implementación de referencia
//! mock vive aquí mismo; las reales (rust via `evcxr`, python via stdio,
//! lo-que-sea) viven en crates separados.

#![forbid(unsafe_code)]

use std::collections::HashSet;

use async_trait::async_trait;
use pluma_notebook_core::{CellId, CellKind, CellState, Notebook};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Salida de la ejecución de una celda de código en el kernel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelOutput {
    /// Texto stdout/stderr concatenado.
    pub stdout: String,
    /// Representación textual del último valor evaluado, si lo hay.
    pub value: Option<String>,
}

#[derive(Debug, Error)]
pub enum KernelError {
    #[error("error de ejecución: {0}")]
    Runtime(String),
}

#[async_trait]
pub trait Kernel: Send + Sync {
    /// Ejecuta `source` en el lenguaje dado. Si la celda no compila o
    /// rompe en runtime, devuelve `Err`. Si compila y corre, devuelve
    /// `Ok(output)` aunque imprima nada.
    async fn execute(&self, source: &str, language: &str) -> Result<KernelOutput, KernelError>;
}

/// Kernel de tests: una función pura `(source, language) -> Result`.
pub struct KernelMock<F>
where
    F: Fn(&str, &str) -> Result<KernelOutput, KernelError> + Send + Sync,
{
    f: F,
}

impl<F> KernelMock<F>
where
    F: Fn(&str, &str) -> Result<KernelOutput, KernelError> + Send + Sync,
{
    pub fn new(f: F) -> Self {
        Self { f }
    }
}

#[async_trait]
impl<F> Kernel for KernelMock<F>
where
    F: Fn(&str, &str) -> Result<KernelOutput, KernelError> + Send + Sync,
{
    async fn execute(&self, source: &str, language: &str) -> Result<KernelOutput, KernelError> {
        (self.f)(source, language)
    }
}

/// Reporte de una corrida del notebook.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub executed: Vec<CellId>,
    pub failed: Vec<CellId>,
    /// Celdas no ejecutadas porque alguna de sus dependencias (directa o
    /// transitiva) falló en esta corrida.
    pub skipped: Vec<CellId>,
}

/// Recorre el DAG en topo-order y ejecuta cada celda. Devuelve `None` si
/// el notebook tiene un ciclo (no hay orden de ejecución).
pub async fn run_all<K: Kernel>(notebook: &mut Notebook, kernel: &K) -> Option<RunReport> {
    let order = notebook.execution_order()?;
    let mut report = RunReport::default();
    let mut failed: HashSet<CellId> = HashSet::new();

    for id in order {
        let cell = notebook.cell(id).expect("id viene de execution_order");
        let upstream_failed = cell.depends_on.iter().any(|dep| failed.contains(dep));
        if upstream_failed {
            failed.insert(id);
            report.skipped.push(id);
            continue;
        }

        let kind = cell.kind.clone();
        let source = cell.source.clone();
        match kind {
            CellKind::Markdown | CellKind::Embed { .. } => {
                notebook.set_state(id, CellState::Fresh);
                report.executed.push(id);
            }
            CellKind::Code { language } => match kernel.execute(&source, &language).await {
                Ok(_) => {
                    notebook.set_state(id, CellState::Fresh);
                    report.executed.push(id);
                }
                Err(_) => {
                    notebook.set_state(id, CellState::Failed);
                    failed.insert(id);
                    report.failed.push(id);
                }
            },
        }
    }

    Some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_kernel() -> KernelMock<impl Fn(&str, &str) -> Result<KernelOutput, KernelError> + Send + Sync> {
        KernelMock::new(|_src, _lang| Ok(KernelOutput { stdout: String::new(), value: None }))
    }

    fn falla_si_contiene(token: &'static str) -> KernelMock<impl Fn(&str, &str) -> Result<KernelOutput, KernelError> + Send + Sync> {
        KernelMock::new(move |src, _lang| {
            if src.contains(token) {
                Err(KernelError::Runtime(format!("contiene {token}")))
            } else {
                Ok(KernelOutput { stdout: String::new(), value: None })
            }
        })
    }

    fn notebook_cadena() -> (Notebook, CellId, CellId, CellId) {
        let mut nb = Notebook::new();
        let a = nb.push(CellKind::Code { language: "rust".into() }, "let x = 1;");
        let b = nb.push(CellKind::Code { language: "rust".into() }, "let y = 2;");
        let c = nb.push(CellKind::Code { language: "rust".into() }, "let z = 3;");
        nb.add_dependency(b, a);
        nb.add_dependency(c, b);
        (nb, a, b, c)
    }

    #[tokio::test]
    async fn run_all_marca_fresh_cuando_todo_ok() {
        let (mut nb, a, b, c) = notebook_cadena();
        let report = run_all(&mut nb, &ok_kernel()).await.unwrap();
        assert_eq!(report.executed.len(), 3);
        assert!(report.failed.is_empty());
        for id in [a, b, c] {
            assert_eq!(nb.cell(id).unwrap().state, CellState::Fresh);
        }
    }

    #[tokio::test]
    async fn falla_en_b_skipea_a_c() {
        let (mut nb, a, b, c) = notebook_cadena();
        let report = run_all(&mut nb, &falla_si_contiene("y =")).await.unwrap();
        assert_eq!(report.executed, vec![a]);
        assert_eq!(report.failed, vec![b]);
        assert_eq!(report.skipped, vec![c]);
        assert_eq!(nb.cell(a).unwrap().state, CellState::Fresh);
        assert_eq!(nb.cell(b).unwrap().state, CellState::Failed);
        // c se queda Stale (no Fresh, no Failed) porque no se ejecutó.
        assert_eq!(nb.cell(c).unwrap().state, CellState::Stale);
    }

    #[tokio::test]
    async fn markdown_no_invoca_al_kernel() {
        // Si markdown invocara al kernel, este test panicearía.
        let mut nb = Notebook::new();
        nb.push(CellKind::Markdown, "# hola");
        let k = KernelMock::new(|_, _| panic!("no debería llamarse"));
        let report = run_all(&mut nb, &k).await.unwrap();
        assert_eq!(report.executed.len(), 1);
    }

    #[tokio::test]
    async fn embed_no_invoca_al_kernel() {
        let mut nb = Notebook::new();
        nb.push(CellKind::Embed { module: "pineal".into() }, "barras");
        let k = KernelMock::new(|_, _| panic!("no debería llamarse"));
        let report = run_all(&mut nb, &k).await.unwrap();
        assert_eq!(report.executed.len(), 1);
    }
}
