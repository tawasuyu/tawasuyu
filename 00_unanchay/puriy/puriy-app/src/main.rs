//! puriy-app — binario del navegador.
//!
//! CLI: `puriy [URL] [--profile NAME] [--target wayland|framebuffer]`.
//! Detección automática: WAYLAND_DISPLAY → mirada; sino framebuffer wawa.
//!
//! Fase 4: pendiente.

use clap::Parser;

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
    println!(
        "puriy stub: url={:?} profile={} target={:?}",
        cli.url, cli.profile, cli.target
    );
}
