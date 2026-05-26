//! puriy-app — binario del navegador.
//!
//! CLI: `puriy [URL] [--profile NAME] [--target wayland|framebuffer|headless]`.
//! Detección automática: `WAYLAND_DISPLAY` o `DISPLAY` → `wayland` (abre
//! ventana Llimphi); sino → `headless` (dumpea box tree por stdout).

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
        eprintln!("uso: puriy <URL> [--profile NAME] [--target wayland|framebuffer|headless]");
        std::process::exit(2);
    };

    eprintln!("[puriy] profile={} target={} url={}", cli.profile, target, url);

    match target.as_str() {
        "headless" => run_headless(&url),
        // wayland / framebuffer ambos abren ventana Llimphi en Fase 3;
        // el split real entre WinitSurface y FramebufferSurface llega
        // cuando puriy se mueva a wawa bare-metal.
        "wayland" | "framebuffer" => puriy_llimphi::run(url),
        other => {
            eprintln!("[puriy] target desconocido: {other}");
            std::process::exit(2);
        }
    }
}

fn detect_target() -> &'static str {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some() {
        "wayland"
    } else {
        "headless"
    }
}

fn run_headless(url: &str) {
    let engine = Engine::new();
    let doc = match engine.load(url) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[puriy] error cargando {url}: {e}");
            std::process::exit(1);
        }
    };
    println!("título: {}", doc.title);
    println!("boxes : {}", doc.box_tree.descendants_count());
    println!("---");
    dump(&doc.box_tree.root, 0);
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
