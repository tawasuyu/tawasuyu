//! puriy-app — binario del navegador.
//!
//! CLI: `puriy [URL] [--profile NAME] [--target wayland|framebuffer|headless]`.
//! Detección automática: `WAYLAND_DISPLAY` o `DISPLAY` → `wayland` (abre
//! ventana Llimphi); sino → `headless` (dumpea box tree por stdout).
//!
//! Cada `--profile NAME` vive en
//! `$XDG_CONFIG_HOME/puriy/profiles/NAME/` (fallback `~/.config/...`)
//! y guarda ahí `profile.json` (historial + bookmarks + sesión) y
//! `cache.bin` (bytes-cache). Profiles distintos quedan aislados.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use clap::Parser;
use puriy_core::Profile;
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
    diagnose_fonts();

    // Carga (o crea) el Profile del usuario.
    let (profile_dir, profile_path, profile) = load_or_create_profile(&cli.profile);
    eprintln!(
        "[puriy] profile_dir={} history={} bookmarks={}",
        profile_dir.display(),
        profile.history.len(),
        profile.bookmarks.len()
    );

    // La cache de bytes vive dentro del profile_dir, así perfiles
    // distintos no comparten contenido cacheado.
    puriy_engine::cache::set_persist_path(profile_dir.join("cache.bin"));
    puriy_engine::cache::load_from_disk();

    let profile = Arc::new(Mutex::new(profile));

    match target.as_str() {
        "headless" => {
            run_headless(&url, &profile);
            persist_all(&profile_path, &profile);
        }
        // wayland / framebuffer ambos abren ventana Llimphi en Fase 3;
        // el split real entre WinitSurface y FramebufferSurface llega
        // cuando puriy se mueva a wawa bare-metal.
        "wayland" | "framebuffer" => {
            puriy_llimphi::run_with_profile(url, profile.clone(), profile_path.clone());
            persist_all(&profile_path, &profile);
        }
        other => {
            eprintln!("[puriy] target desconocido: {other}");
            std::process::exit(2);
        }
    }
}

/// Resuelve `$XDG_CONFIG_HOME/puriy/profiles/NAME/` (fallback `$HOME/.config/...`).
/// Crea el directorio si no existe. Carga `profile.json` desde ahí; si
/// no existe (o falla la deserialización), arranca con `Profile::nuevo`.
fn load_or_create_profile(name: &str) -> (PathBuf, PathBuf, Profile) {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("puriy").join("profiles").join(name);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("profile.json");
    let profile = match puriy_core::store::load(&path) {
        Ok(p) => p,
        Err(_) => Profile::nuevo(name),
    };
    (dir, path, profile)
}

/// Persiste cache + profile. Best-effort — errores van a stderr para
/// que el usuario sepa si algo no se guardó, sin abortar el shutdown.
fn persist_all(profile_path: &std::path::Path, profile: &Arc<Mutex<Profile>>) {
    puriy_engine::cache::flush();
    if let Ok(p) = profile.lock() {
        if let Err(e) = puriy_core::store::save(profile_path, &p) {
            eprintln!("[puriy] no se pudo guardar profile.json: {e}");
        }
    }
}

/// Chequea si el sistema tiene fuentes con cobertura razonable de
/// símbolos / CJK / emoji. Sin estas, parley/fontique dibuja `□` (tofu)
/// para cualquier glifo no cubierto por la fuente base. Probamos rutas
/// típicas de Linux; si están vacías, sugerimos los paquetes.
fn diagnose_fonts() {
    let common_paths = [
        "/usr/share/fonts/noto",
        "/usr/share/fonts/google-noto",
        "/usr/share/fonts/TTF/NotoSans-Regular.ttf",
        "/usr/share/fonts/noto-cjk",
        "/usr/share/fonts/noto-emoji",
        "/usr/share/fonts/truetype/noto",
    ];
    let found = common_paths
        .iter()
        .any(|p| std::path::Path::new(p).exists());
    if !found {
        eprintln!(
            "[puriy] aviso: no se detectaron fuentes Noto en el sistema. \
             Si ves cuadrados □ en lugar de glifos (símbolos math, CJK, emoji), \
             instalá: noto-fonts noto-fonts-cjk noto-fonts-emoji noto-fonts-extra"
        );
    }
}

fn detect_target() -> &'static str {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some() {
        "wayland"
    } else {
        "headless"
    }
}

fn run_headless(url: &str, profile: &Arc<Mutex<Profile>>) {
    let engine = Engine::new();
    let doc = match engine.load(url) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[puriy] error cargando {url}: {e}");
            std::process::exit(1);
        }
    };
    if let Ok(mut p) = profile.lock() {
        p.history.record(&doc.url, &doc.title, puriy_core::now());
    }
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
