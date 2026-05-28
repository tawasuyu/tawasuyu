//! Hito Fase 2 — `cargo run -p puriy-engine --example load_example_com`.
//!
//! Pipeline completo: fetch HTTP → parse HTML → box tree → dump.
//!
//! Si no hay red, podés pasar `--offline` para usar un HTML fixture:
//!     cargo run -p puriy-engine --example load_example_com -- --offline

use puriy_engine::{BoxNode, Engine};

const FIXTURE: &str = r#"<!doctype html>
<html>
  <head>
    <title>Example Domain</title>
    <style>
      body { background: #f0f0f2; color: #333; padding: 16px; }
      h1 { font-size: 24px; }
      p { font-size: 16px; }
    </style>
  </head>
  <body>
    <h1>Example Domain</h1>
    <p>This domain is for use in illustrative examples in documents.</p>
    <p>You may use this domain in literature without prior coordination or asking for permission.</p>
  </body>
</html>"#;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let offline = args.iter().any(|a| a == "--offline");

    let engine = Engine::new();
    let doc = if offline {
        engine.load_html("about:example", FIXTURE)
    } else {
        match engine.load("https://example.com") {
            Ok(d) => d,
            Err(e) => {
                eprintln!("fetch falló ({e}); cayendo a fixture offline");
                engine.load_html("about:example", FIXTURE)
            }
        }
    };

    println!("URL    : {}", doc.url);
    println!("título : {}", doc.title);
    println!("boxes  : {}", doc.box_tree.descendants_count());
    println!("---");
    dump(&doc.box_tree.root, 0);
}

fn dump(b: &BoxNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let tag = b.tag.as_deref().unwrap_or("·");
    let txt = b.text.as_deref().unwrap_or("");
    let bg = match b.background {
        Some(c) => format!(" bg=#{:02x}{:02x}{:02x}", c.r, c.g, c.b),
        None => String::new(),
    };
    println!(
        "{indent}<{tag} {:?} fs={} m={:?} p={:?}{bg}> {}",
        b.display, b.font_size, b.margin, b.padding, txt
    );
    for c in &b.children {
        dump(c, depth + 1);
    }
}
