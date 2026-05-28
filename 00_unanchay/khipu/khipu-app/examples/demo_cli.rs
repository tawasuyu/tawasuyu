//! `demo_cli` — demostración no gráfica del cuaderno.
//!
//! Siembra un cuaderno personal, imprime el grafo de wiki-links
//! (forward-links, backlinks, huérfanas, enlaces colgantes) y luego la
//! gravedad semántica: los clústeres por afinidad y los vecinos más
//! cercanos de una nota.
//!
//! Los vectores semánticos van a mano —tres tópicos: cocina, jardín,
//! oficina— para que el clustering se vea con claridad. En la app real
//! los produce `verbo`. Corre con
//! `cargo run -p khipu-app --example demo_cli --release`.

use khipu_core::{NoteId, NoteStore};
use khipu_gravity::{GravityConfig, SemanticField};

/// Vector de tópico con un leve sesgo — notas del mismo tema quedan
/// afines sin ser idénticas.
fn topic(base: [f32; 3], nudge: f32) -> Vec<f32> {
    vec![base[0] + nudge, base[1] + nudge * 0.3, base[2] - nudge * 0.2]
}

fn main() {
    let cocina = [1.0, 0.0, 0.0];
    let jardin = [0.0, 1.0, 0.0];
    let oficina = [0.0, 0.0, 1.0];

    let mut store = NoteStore::new();
    let mut field = SemanticField::new();

    let seed: [(&str, &str, &[&str], Vec<f32>); 7] = [
        (
            "Índice",
            "mi cuaderno: [[Recetas de la abuela]], [[Jardín]] y [[Oficina]]",
            &["meta"],
            topic(cocina, 0.0),
        ),
        (
            "Recetas de la abuela",
            "sopa de auyama; ver también [[Lista del mercado]]",
            &["cocina"],
            topic(cocina, 0.05),
        ),
        (
            "Lista del mercado",
            "auyama, cilantro, pan; vuelve al [[Índice]]",
            &["cocina"],
            topic(cocina, 0.10),
        ),
        (
            "Jardín",
            "riego semanal; las [[Semillas de cilantro]] van en marzo",
            &["jardín"],
            topic(jardin, 0.05),
        ),
        (
            "Semillas de cilantro",
            "germinan en diez días",
            &["jardín"],
            topic(jardin, 0.10),
        ),
        (
            "Oficina",
            "[[Reunión del lunes]] y pendientes varios",
            &["trabajo"],
            topic(oficina, 0.05),
        ),
        (
            "Diario sin enlaces",
            "una nota suelta, no la enlaza nadie y enlaza a [[Algo Perdido]]",
            &["personal"],
            topic(oficina, 0.50),
        ),
    ];

    let mut ids: Vec<(NoteId, String)> = Vec::new();
    for (title, body, tags, vector) in seed {
        let tags = tags.iter().map(|t| t.to_string()).collect();
        let id = store.create(title, body, tags, 1_700_000_000);
        field.insert(id, vector);
        ids.push((id, title.to_string()));
    }

    let name = |id: NoteId| {
        ids.iter()
            .find(|(i, _)| *i == id)
            .map(|(_, n)| n.as_str())
            .unwrap_or("?")
    };

    println!("\n  khipu · cuaderno de notas — {} notas\n", store.len());

    println!("  grafo de enlaces:");
    for note in store.iter() {
        let fwd: Vec<&str> = store.forward_links(note.id).into_iter().map(name).collect();
        let back: Vec<&str> = store.backlinks(note.id).into_iter().map(name).collect();
        println!("    «{}»", note.title);
        println!("       enlaza a  : {}", fmt_list(&fwd));
        println!("       backlinks : {}", fmt_list(&back));
    }

    let orphans: Vec<&str> = store.orphans().iter().map(|n| n.title.as_str()).collect();
    println!("\n  notas huérfanas (sin backlinks): {}", fmt_list(&orphans));
    let dangling_owned = store.dangling_links();
    let dangling: Vec<&str> = dangling_owned.iter().map(|s| s.as_str()).collect();
    println!("  enlaces colgantes (destino inexistente): {}", fmt_list(&dangling));

    println!("\n  gravedad semántica — clústeres (afinidad ≥ 0.85):");
    for (n, cluster) in field.clusters(0.85).iter().enumerate() {
        let titles: Vec<&str> = cluster.iter().map(|id| name(*id)).collect();
        println!("    grupo {}: {}", n + 1, fmt_list(&titles));
    }

    let pivot = ids[1].0; // "Recetas de la abuela"
    println!("\n  vecinos más afines a «{}»:", name(pivot));
    for (id, score) in field.nearest(pivot, 3) {
        println!("    {:.3}  {}", score, name(id));
    }

    let layout = field.gravity_layout(&GravityConfig::default());
    println!("\n  layout 2D por gravedad ({} posiciones):", layout.len());
    for p in &layout {
        println!("    ({:7.1}, {:7.1})  {}", p.x, p.y, name(p.id));
    }
    println!();
}

/// Formatea una lista de nombres, o `—` si está vacía.
fn fmt_list(items: &[&str]) -> String {
    if items.is_empty() {
        "—".to_string()
    } else {
        items.join(", ")
    }
}
