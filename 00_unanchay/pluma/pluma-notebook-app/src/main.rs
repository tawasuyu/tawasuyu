//! `pluma_notebook_app` — demostración de notebooks reproducibles.
//!
//! Arma un notebook con prosa, código y un embed de otro módulo
//! brahman; imprime el orden de ejecución y el digest Merkle; luego
//! edita una celda intermedia y muestra cómo la obsolescencia y el
//! digest se propagan. `cargo run -p pluma_notebook_app`.

use pluma_notebook_core::{CellId, CellKind, CellState, Notebook};

/// Etiqueta de la clase de una celda.
fn label(nb: &Notebook, id: CellId) -> &'static str {
    match nb.cell(id).map(|c| &c.kind) {
        Some(CellKind::Markdown) => "markdown",
        Some(CellKind::Code { .. }) => "código  ",
        Some(CellKind::Embed { .. }) => "embed   ",
        None => "?       ",
    }
}

/// Primeros bytes de un digest, en hex — suficiente para distinguirlos.
fn short(digest: Option<[u8; 32]>) -> String {
    match digest {
        Some(d) => d[..6].iter().map(|b| format!("{b:02x}")).collect(),
        None => "—(ciclo)".to_string(),
    }
}

fn main() {
    let mut nb = Notebook::new();

    let intro = nb.push(CellKind::Markdown, "# Cosecha de auyama\nAnálisis del rendimiento.");
    let datos = nb.push(
        CellKind::Code { language: "rust".into() },
        "let kilos = vec![12.0, 18.0, 9.5, 21.0];",
    );
    let media = nb.push(
        CellKind::Code { language: "rust".into() },
        "let media = kilos.iter().sum::<f64>() / kilos.len() as f64;",
    );
    let grafico = nb.push(
        CellKind::Embed { module: "pineal".into() },
        "barras: kilos por semana",
    );

    // DAG: media depende de datos; el gráfico depende de ambos.
    nb.add_dependency(media, datos);
    nb.add_dependency(grafico, datos);
    nb.add_dependency(grafico, media);

    println!("\n  pluma_notebook_app · notebook reproducible — {} celdas\n", nb.len());

    println!("  orden de ejecución (según el DAG de dependencias):");
    if let Some(order) = nb.execution_order() {
        for (step, id) in order.iter().enumerate() {
            println!(
                "    {}. [{}] celda {}   digest {}",
                step + 1,
                label(&nb, *id),
                id,
                short(nb.digest(*id))
            );
        }
    }

    let digest_inicial = nb.notebook_digest();
    println!("\n  digest del notebook: {}", short(digest_inicial));

    // Marca todo Fresh y luego edita la celda de datos.
    for c in nb.cells().iter().map(|c| c.id).collect::<Vec<_>>() {
        nb.set_state(c, CellState::Fresh);
    }
    println!("\n  ── se edita la celda «datos» ──────────────────────────");
    nb.set_source(datos, "let kilos = vec![12.0, 18.0, 9.5, 21.0, 30.0];");

    println!("\n  estado de las celdas tras la edición:");
    for id in [intro, datos, media, grafico] {
        let st = match nb.cell(id).unwrap().state {
            CellState::Fresh => "fresca",
            CellState::Stale => "OBSOLETA",
            CellState::Failed => "fallida",
        };
        println!("    [{}] celda {}  → {}", label(&nb, id), id, st);
    }

    println!("\n  digest del notebook: {}", short(nb.notebook_digest()));
    println!(
        "  {} la edición cambió el digest — la corrida anterior ya no\n  \
         es reproducible bit a bit; hay que re-ejecutar lo obsoleto.\n",
        if digest_inicial != nb.notebook_digest() { "✔" } else { "✘" }
    );
}
