//! Showcase end-to-end del `DominiumKernel` sobre el motor de
//! notebooks.
//!
//! Arma un notebook hardcoded con la cadena:
//!
//! ```text
//!   world ───┐
//!   params ──┼─► tick(0) ─► tick(50) ─► tick(50) ─► stats
//!   seed ────┘
//! ```
//!
//! - `world`: resetea la grilla a 32×24.
//! - `seed`: siembra 150 lemmings con `seed=7` (determinista).
//! - `params`: ajusta tres campos escalares.
//! - `tick(0)`: snapshot inicial — corre 0 ticks (ya con seed + params),
//!   imprime stats t=0.
//! - `tick(50)` y `tick(50)` encadenados: avanzan 100 ticks en total
//!   en dos celdas para mostrar reactividad parcial.
//! - `stats`: lectura final sin avanzar el reloj.
//!
//! Corré con: `cargo run -p dominium-notebook-kernel --example
//! notebook_dominium_demo --release`.
//!
//! El demo no abre ventana; imprime el stdout de cada celda. Para
//! visualizar el DAG arrástralo a `pluma-notebook-graph-llimphi`
//! (consume el mismo `Notebook`).

use pluma_notebook_core::{CellId, CellKind, Notebook, OutputPayload};
use pluma_notebook_exec::run_all;
use dominium_notebook_kernel::DominiumKernel;

#[tokio::main]
async fn main() {
    let mut nb = Notebook::new();

    let world = code(&mut nb, "dominium-world", "32 24");
    let seed = code(&mut nb, "dominium-seed", "150 7");
    let params = code(
        &mut nb,
        "dominium-param",
        "move_speed=0.4\nsync_rate=0.05\nclimb_cost=0.1",
    );
    let tick0 = code(&mut nb, "dominium-tick", "0");
    let tick50_a = code(&mut nb, "dominium-tick", "50");
    let tick50_b = code(&mut nb, "dominium-tick", "50");
    let stats = code(&mut nb, "dominium-stats", "");

    // Edges del DAG: world se setea primero; seed y params dependen de
    // world (ambos lo necesitan listo); tick(0) depende de seed + params;
    // los dos tick(50) van en cadena; stats al final.
    assert!(nb.add_dependency(seed, world));
    assert!(nb.add_dependency(params, world));
    assert!(nb.add_dependency(tick0, seed));
    assert!(nb.add_dependency(tick0, params));
    assert!(nb.add_dependency(tick50_a, tick0));
    assert!(nb.add_dependency(tick50_b, tick50_a));
    assert!(nb.add_dependency(stats, tick50_b));

    let kernel = DominiumKernel::new();
    let report = run_all(&mut nb, &kernel).await.expect("notebook sin ciclo");

    println!("=== notebook_dominium_demo — corrida completa ===");
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

    // Imprime también el digest reproducible — dos corridas idénticas
    // dan el mismo número en cualquier laptop.
    if let Some(d) = nb.notebook_digest() {
        println!(
            "notebook_digest = {}",
            d.iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
    }

    // Sanity: la última celda debe ser una tabla con la fila "n".
    let stats_cell = nb.cell(stats).unwrap();
    if let Some(out) = &stats_cell.last_output {
        if let OutputPayload::Table { rows, .. } = &out.payload {
            let n_row = rows.iter().find(|r| r[0] == "n").unwrap();
            println!("\nlemmings vivos al final: {}", n_row[1]);
        }
    }
}

fn code(nb: &mut Notebook, language: &str, source: &str) -> CellId {
    nb.push(
        CellKind::Code { language: language.to_string() },
        source.to_string(),
    )
}
