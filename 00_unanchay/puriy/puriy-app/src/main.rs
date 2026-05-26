//! puriy-app — binario del navegador.
//!
//! CLI: `puriy [URL] [--profile NAME] [--target wayland|framebuffer]`.
//! Detección automática: WAYLAND_DISPLAY → mirada; sino framebuffer wawa.
//!
//! Fase 4: el target gráfico aún es stub (sólo dumpea el box tree).

use clap::Parser;
use puriy_engine::{BoxNode, Engine};

#[derive(Parser)]
#[command(name = "puriy", about = "Navegador web soberano sobre Llimphi")]
struct Cli {
    /// URL inicial a cargar
    url: Option<String>,
    /// Nombre del perfil (default: "default")
    #[arg(long, default_value = "default")]
    profile: String,
    /// Target de salida: wayland | framebuffer | headless
    #[arg(long)]
    target: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    let target = cli.target.clone().unwrap_or_else(|| detect_target().to_string());
    let Some(url) = cli.url else {
        eprintln!("uso: puriy <URL> [--profile NAME] [--target wayland|framebuffer]");
        std::process::exit(2);
    };

    println!("[puriy] profile={} target={} url={}", cli.profile, target, url);

    let engine = Engine::new();
    let doc = match engine.load(&url) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[puriy] error cargando {url}: {e}");
            std::process::exit(1);
        }
    };

    println!("[puriy] título: {}", doc.title);
    println!("[puriy] boxes : {}", doc.box_tree.descendants_count());
    println!("---");
    dump(&doc.box_tree.root, 0);
}

fn detect_target() -> &'static str {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        "wayland"
    } else {
        "framebuffer"
    }
}

fn dump(b: &BoxNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let tag = b.tag.as_deref().unwrap_or("·");
    let txt = b.text.as_deref().unwrap_or("");
    println!("{indent}<{tag} {:?}> {}", b.display, txt);
    for c in &b.children {
        dump(c, depth + 1);
    }
}
