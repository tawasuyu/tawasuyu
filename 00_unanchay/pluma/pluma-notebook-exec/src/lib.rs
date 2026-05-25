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
pub use pluma_notebook_core::{CellOutput, OutputPayload};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Salida de la ejecución de una celda de código en el kernel — alias
/// de [`pluma_notebook_core::CellOutput`]. Vive en core ahora para que
/// `Cell::last_output` pueda persistirlo sin invertir la dependencia.
pub type KernelOutput = CellOutput;

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
    run_subset(notebook, kernel, None).await
}

/// Recomputación **reactiva mínima**: ejecuta `root` y sus dependientes
/// transitivos en topo-order, sin tocar nada que esté fuera del cono.
/// Si la celda raíz no existe devuelve `None`. Si el notebook tiene un
/// ciclo devuelve `None`. Equivale al "edit-cell-and-propagate-only" de
/// los notebooks reactivos: ningún ancestro vuelve a correr.
pub async fn run_from<K: Kernel>(
    notebook: &mut Notebook,
    kernel: &K,
    root: CellId,
) -> Option<RunReport> {
    if notebook.cell(root).is_none() {
        return None;
    }
    let mut cone: HashSet<CellId> =
        notebook.dependents_transitive(root).into_iter().collect();
    cone.insert(root);
    run_subset(notebook, kernel, Some(cone)).await
}

/// Núcleo compartido: `subset = None` corre todo el notebook; `Some(s)`
/// restringe la corrida a los ids de `s` (en topo-order del DAG global).
async fn run_subset<K: Kernel>(
    notebook: &mut Notebook,
    kernel: &K,
    subset: Option<HashSet<CellId>>,
) -> Option<RunReport> {
    let order = notebook.execution_order()?;
    let mut report = RunReport::default();
    let mut failed: HashSet<CellId> = HashSet::new();

    for id in order {
        if let Some(s) = &subset {
            if !s.contains(&id) {
                continue;
            }
        }
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
                Ok(out) => {
                    notebook.set_last_output(id, Some(out));
                    notebook.set_state(id, CellState::Fresh);
                    report.executed.push(id);
                }
                Err(e) => {
                    notebook.set_last_output(
                        id,
                        Some(CellOutput {
                            stdout: String::new(),
                            value: Some(e.to_string()),
                            payload: OutputPayload::Text(e.to_string()),
                        }),
                    );
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
        KernelMock::new(|_src, _lang| Ok(KernelOutput::empty()))
    }

    fn falla_si_contiene(token: &'static str) -> KernelMock<impl Fn(&str, &str) -> Result<KernelOutput, KernelError> + Send + Sync> {
        KernelMock::new(move |src, _lang| {
            if src.contains(token) {
                Err(KernelError::Runtime(format!("contiene {token}")))
            } else {
                Ok(KernelOutput::empty())
            }
        })
    }

    /// Kernel que registra cada source que recibió, para chequear que
    /// `run_from` deja en paz a los ancestros.
    fn kernel_grabador() -> (
        std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        KernelMock<impl Fn(&str, &str) -> Result<KernelOutput, KernelError> + Send + Sync>,
    ) {
        let log: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let cloned = log.clone();
        let kernel = KernelMock::new(move |src, _lang| {
            cloned.lock().unwrap().push(src.to_string());
            Ok(KernelOutput::empty())
        });
        (log, kernel)
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

    #[tokio::test]
    async fn run_from_solo_toca_el_cono() {
        // a → b → c, d suelta — al correr desde b, sólo b y c se ejecutan.
        let (mut nb, a, b, c) = notebook_cadena();
        let d = nb.push(CellKind::Code { language: "rust".into() }, "let suelta = 0;");
        for id in [a, b, c, d] {
            nb.set_state(id, CellState::Fresh);
        }
        let (log, k) = kernel_grabador();
        let report = run_from(&mut nb, &k, b).await.unwrap();

        assert_eq!(report.executed, vec![b, c]);
        // a y d no se ejecutaron: ni el kernel los vio ni cambiaron de estado.
        let visto = log.lock().unwrap().clone();
        assert_eq!(visto.len(), 2);
        assert!(visto.iter().any(|s| s.contains("let y = 2;")));
        assert!(visto.iter().any(|s| s.contains("let z = 3;")));
        assert!(!visto.iter().any(|s| s.contains("let x = 1;")));
        assert!(!visto.iter().any(|s| s.contains("suelta")));
        assert_eq!(nb.cell(a).unwrap().state, CellState::Fresh);
        assert_eq!(nb.cell(d).unwrap().state, CellState::Fresh);
    }

    #[tokio::test]
    async fn run_from_inexistente_devuelve_none() {
        let (mut nb, ..) = notebook_cadena();
        assert!(run_from(&mut nb, &ok_kernel(), 999).await.is_none());
    }

    #[tokio::test]
    async fn run_from_de_una_hoja_solo_corre_esa() {
        let (mut nb, a, b, c) = notebook_cadena();
        for id in [a, b, c] {
            nb.set_state(id, CellState::Stale);
        }
        let (log, k) = kernel_grabador();
        let report = run_from(&mut nb, &k, c).await.unwrap();
        assert_eq!(report.executed, vec![c]);
        assert_eq!(log.lock().unwrap().len(), 1);
        assert_eq!(nb.cell(a).unwrap().state, CellState::Stale);
        assert_eq!(nb.cell(b).unwrap().state, CellState::Stale);
        assert_eq!(nb.cell(c).unwrap().state, CellState::Fresh);
    }

    #[tokio::test]
    async fn run_all_persiste_output_en_la_celda() {
        let mut nb = Notebook::new();
        let id = nb.push(CellKind::Code { language: "rust".into() }, "x");
        let k = KernelMock::new(|_, _| Ok(KernelOutput {
            stdout: "log".into(),
            value: Some("7".into()),
            payload: OutputPayload::Scalar(7.0),
        }));
        run_all(&mut nb, &k).await.unwrap();
        let saved = nb.cell(id).unwrap().last_output.as_ref().unwrap();
        assert_eq!(saved.stdout, "log");
        assert_eq!(saved.value.as_deref(), Some("7"));
        assert!(matches!(saved.payload, OutputPayload::Scalar(n) if (n - 7.0).abs() < 1e-9));
    }

    #[tokio::test]
    async fn falla_persiste_el_error_como_output() {
        let mut nb = Notebook::new();
        let id = nb.push(CellKind::Code { language: "rust".into() }, "x");
        let k = KernelMock::new(|_, _| Err(KernelError::Runtime("explotó".into())));
        run_all(&mut nb, &k).await.unwrap();
        let saved = nb.cell(id).unwrap().last_output.as_ref().unwrap();
        assert!(saved.value.as_ref().unwrap().contains("explotó"));
    }
}
