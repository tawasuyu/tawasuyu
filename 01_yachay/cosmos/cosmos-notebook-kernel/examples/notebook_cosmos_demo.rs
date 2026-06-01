//! Showcase CLI del `CosmosKernel` — versión extendida con los seis
//! lenguajes nuevos sobre cosmos-skywatch + extractos.
//!
//! Notebook hardcoded:
//!
//! ```text
//!   tdb ─┬─► positions, helio, distance(mars)
//!        ├─► skywatch (alt/az desde Location)
//!        ├─► sundial  (sombra del gnomon)
//!        ├─► tides    (mareas)
//!        ├─► rise-set (agenda celeste)
//!        ├─► eclipses (4 años solar)
//!        └─► transits (15 años)
//!   location ─► (alimenta skywatch, sundial, tides, rise-set)
//! ```
//!
//! Editar la celda TDB o LOCATION y re-correr `run_all` muta toda la
//! cadena. Mismo patrón reactivo que kernel-dominium.
//!
//! Corré con: `cargo run -p cosmos-notebook-kernel --example
//! notebook_cosmos_demo --release`.

use pluma_notebook_core::{CellId, CellKind, Notebook, OutputPayload};
use pluma_notebook_exec::run_all;
use cosmos_notebook_kernel::CosmosKernel;

#[tokio::main]
async fn main() {
    let mut nb = Notebook::new();
    let t = code(&mut nb, "cosmos-tdb", "2026-05-27T00:00:00");
    let l = code(&mut nb, "cosmos-location", "-12.05 -77.05 150");
    let p = code(&mut nb, "cosmos-positions", "");
    let h = code(&mut nb, "cosmos-helio", "");
    let d = code(&mut nb, "cosmos-distance", "mars");
    let sw = code(&mut nb, "cosmos-skywatch", "sun moon jupiter saturn");
    let sd = code(&mut nb, "cosmos-sundial", "");
    let td = code(&mut nb, "cosmos-tides", "");
    let rs = code(&mut nb, "cosmos-rise-set", "sun moon jupiter");
    let ec = code(&mut nb, "cosmos-eclipses", "4 solar");
    let tr = code(&mut nb, "cosmos-transits", "15");
    nb.add_dependency(p, t);
    nb.add_dependency(h, t);
    nb.add_dependency(d, t);
    nb.add_dependency(sw, t);
    nb.add_dependency(sw, l);
    nb.add_dependency(sd, t);
    nb.add_dependency(sd, l);
    nb.add_dependency(td, t);
    nb.add_dependency(td, l);
    nb.add_dependency(rs, t);
    nb.add_dependency(rs, l);
    nb.add_dependency(ec, t);
    nb.add_dependency(tr, t);

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
