//! Demo de `atipay`: muestra el catálogo tal como lo ve la IA y despacha una
//! invocación a un plan ejecutable (sin ejecutarlo — eso es trabajo de shuma).
//!
//! ```sh
//! cargo run -p atipay --example catalogo
//! ```

use atipay::{Catalogo, Invocacion};

fn main() {
    let cat = Catalogo::estandar();

    println!("== Catálogo de capacidades ({} en total) ==\n", cat.capacidades().len());
    for c in cat.capacidades() {
        let ps: Vec<String> = c.params.iter().map(|p| format!("<{}>", p.nombre)).collect();
        println!("  {:<28} [{:?}] {} {}", c.id, c.peligro, c.resumen, ps.join(" "));
    }

    println!("\n== Menú para prompt (lo que shuma :hacé incrusta en el system prompt) ==\n");
    print!("{}", cat.prompt_menu());

    println!("\n== Definiciones tool-use (lo que recibe el LLM vía pluma-llm) ==\n");
    println!("{}", serde_json::to_string_pretty(&cat.as_tools()).unwrap());

    println!("\n== Despacho de invocaciones de ejemplo (intención → plan) ==\n");
    let ejemplos = [
        Invocacion::nueva("mirada.workspace").con("n", "3"),
        Invocacion::nueva("mirada.layout").con("modo", "grid"),
        Invocacion::nueva("sandokan.stop").con("id", "01J9XABC"),
        Invocacion::nueva("mirada.workspace").con("n", "tres"), // inválida a propósito
    ];
    for inv in ejemplos {
        match cat.plan(&inv) {
            Ok(p) => println!("  {:<32} → {} {}  [{:?}]", inv.id, p.programa, p.args.join(" "), p.peligro),
            Err(e) => println!("  {:<32} → ⚠ {}", inv.id, e),
        }
    }
}
