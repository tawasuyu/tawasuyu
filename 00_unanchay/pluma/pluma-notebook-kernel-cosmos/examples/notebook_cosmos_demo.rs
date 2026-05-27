//! Showcase CLI del `CosmosKernel`.
//!
//! Notebook hardcoded:
//!
//! ```text
//!   tdb ─┬─► positions (todos)
//!        ├─► helio (todos)
//!        └─► distance(mars)
//! ```
//!
//! Cambiar el TDB y re-correr `run_all` muta toda la cadena con el
//! nuevo instante. El notebook tiene un `digest` reproducible — dos
//! corridas con el mismo TDB dan el mismo digest.
//!
//! Corré con: `cargo run -p pluma-notebook-kernel-cosmos --example
//! notebook_cosmos_demo --release`.

use pluma_notebook_core::{CellId, CellKind, Notebook, OutputPayload};
use pluma_notebook_exec::run_all;
use pluma_notebook_kernel_cosmos::CosmosKernel;

#[tokio::main]
async fn main() {
    let mut nb = Notebook::new();
    let t = code(&mut nb, "cosmos-tdb", "2026-05-27T00:00:00");
    let p = code(&mut nb, "cosmos-positions", "");
    let h = code(&mut nb, "cosmos-helio", "");
    let d = code(&mut nb, "cosmos-distance", "mars");
    nb.add_dependency(p, t);
    nb.add_dependency(h, t);
    nb.add_dependency(d, t);

    let kernel = CosmosKernel::new();
    let report = run_all(&mut nb, &kernel).await.expect("notebook sin ciclo");
    println!("=== notebook_cosmos_demo — corrida completa ===");
    println!(
        "ejecutadas: {} · falladas: {} · saltadas: {}\n",
        report.executed.len(),
        report.failed.len(),
        report.skipped.len()
    );

    for cell in nb.cells() {
        let lang = match &cell.kind {
            CellKind::Code { language } => language.as_str(),
            _ => "n/a",
        };
        let stdout = cell
            .last_output
            .as_ref()
            .map(|o| o.stdout.as_str())
            .unwrap_or("(sin output)");
        println!(
            "--- celda {} [{lang}] state={:?} ---",
            cell.id, cell.state
        );
        println!("source: {}", cell.source.replace('\n', " ⏎ "));
        println!("{stdout}");
        println!();
    }

    if let Some(dig) = nb.notebook_digest() {
        println!(
            "notebook_digest = {}",
            dig.iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
    }

    let d_cell = nb.cell(d).unwrap();
    if let Some(out) = &d_cell.last_output {
        if let OutputPayload::Scalar(v) = out.payload {
            println!("\nd_geo(mars) al 2026-05-27 = {v:.10} au");
        }
    }
}

fn code(nb: &mut Notebook, language: &str, source: &str) -> CellId {
    nb.push(
        CellKind::Code { language: language.to_string() },
        source.to_string(),
    )
}
