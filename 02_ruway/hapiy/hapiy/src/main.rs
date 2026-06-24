//! `hapiy` — la captura de pantalla de la suite.
//!
//! Atrapa lo que mirada pinta: pantalla completa, un monitor o una región;
//! guarda un PNG y, con `--edit`, lo abre en **tullpu** para anotar/recortar.
//!
//! ```text
//! hapiy                      # captura y guarda en ~/Pictures/hapiy-<ts>.png
//! hapiy -o /tmp/foo.png      # destino explícito
//! hapiy --region 100,80,640,480
//! hapiy --display eDP-1
//! hapiy --edit               # captura y la abre en tullpu
//! hapiy --list-displays
//! hapiy --backend grim|native|auto
//! ```
//!
//! Backends: `native` = cliente `zwlr_screencopy` propio (feature `wayland`),
//! `grim` = el binario grim. `auto` (default) prueba el nativo y cae a grim.

mod grim;
#[cfg(feature = "wayland")]
mod wayland;

use hapiy_core::{default_dir, default_filename, tullpu_launch, Capturer, Region};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

enum Backend {
    Auto,
    Native,
    Grim,
}

struct Args {
    display: Option<String>,
    region: Option<Region>,
    dest: Option<PathBuf>,
    edit: bool,
    list: bool,
    backend: Backend,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Some(a)) => a,
        Ok(None) => return ExitCode::SUCCESS, // --help
        Err(e) => {
            eprintln!("hapiy: {e}\nProbá `hapiy --help`.");
            return ExitCode::FAILURE;
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hapiy: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), String> {
    let cap = make_capturer(&args.backend)?;

    if args.list {
        for o in cap.outputs()? {
            println!("{}\t{}x{}", o.name, o.width, o.height);
        }
        return Ok(());
    }

    let shot = cap.capture(args.display.as_deref())?;
    let shot = match args.region {
        Some(r) => shot.crop(r).ok_or("la región queda fuera de la captura")?,
        None => shot,
    };

    let path = args
        .dest
        .unwrap_or_else(|| default_dir().join(default_filename(&stamp())));
    shot.save_png(&path)?;
    println!("captura guardada en {}", path.display());

    if args.edit {
        let (prog, a) = tullpu_launch(&path);
        Command::new(&prog)
            .args(&a)
            .spawn()
            .map_err(|e| format!("no se pudo abrir tullpu ({prog}): {e}"))?;
        println!("abriendo en tullpu para anotar…");
    }
    Ok(())
}

fn make_capturer(backend: &Backend) -> Result<Box<dyn Capturer>, String> {
    match backend {
        Backend::Grim => Ok(Box::new(grim::GrimCapturer)),
        Backend::Native => native_capturer(),
        Backend::Auto => match native_capturer() {
            Ok(c) => Ok(c),
            Err(e) => {
                eprintln!("hapiy: backend nativo no disponible ({e}); uso grim.");
                Ok(Box::new(grim::GrimCapturer))
            }
        },
    }
}

#[cfg(feature = "wayland")]
fn native_capturer() -> Result<Box<dyn Capturer>, String> {
    Ok(Box::new(wayland::WaylandCapturer::connect()?))
}

#[cfg(not(feature = "wayland"))]
fn native_capturer() -> Result<Box<dyn Capturer>, String> {
    Err("compilado sin el backend nativo (feature `wayland`)".into())
}

/// Sello para el nombre de archivo: segundos desde epoch (suficiente para no
/// pisar capturas seguidas; el núcleo lo formatea como `hapiy-<stamp>.png`).
fn stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = Args {
        display: None,
        region: None,
        dest: None,
        edit: false,
        list: false,
        backend: Backend::Auto,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(None);
            }
            "-o" | "--output" => {
                args.dest = Some(PathBuf::from(next(&mut it, "-o necesita una ruta")?));
            }
            "--display" => {
                args.display = Some(next(&mut it, "--display necesita un nombre de salida")?);
            }
            "--region" | "-g" => {
                args.region = Some(parse_region(&next(&mut it, "--region necesita x,y,w,h")?)?);
            }
            "--edit" | "-e" => args.edit = true,
            "--list-displays" | "--list" => args.list = true,
            "--backend" => {
                args.backend = match next(&mut it, "--backend necesita un valor")?.as_str() {
                    "auto" => Backend::Auto,
                    "native" => Backend::Native,
                    "grim" => Backend::Grim,
                    other => return Err(format!("backend desconocido «{other}» (auto|native|grim)")),
                };
            }
            other => return Err(format!("argumento desconocido «{other}»")),
        }
    }
    Ok(Some(args))
}

fn next(it: &mut impl Iterator<Item = String>, msg: &str) -> Result<String, String> {
    it.next().ok_or_else(|| msg.to_string())
}

/// `x,y,w,h` o `x,y wxh` (estilo grim/slurp) → [`Region`].
fn parse_region(s: &str) -> Result<Region, String> {
    let norm = s.replace([' ', 'x'], ",");
    let parts: Vec<u32> = norm
        .split(',')
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u32>().map_err(|_| format!("región inválida «{s}»")))
        .collect::<Result<_, _>>()?;
    match parts[..] {
        [x, y, w, h] => Ok(Region { x, y, w, h }),
        _ => Err(format!("región «{s}»: se esperaban 4 números (x,y,w,h)")),
    }
}

fn print_help() {
    println!(
        "hapiy — captura de pantalla de la suite\n\n\
         USO:\n  hapiy [opciones]\n\n\
         OPCIONES:\n\
         \x20 -o, --output <ruta>     destino del PNG (default ~/Pictures/hapiy-<ts>.png)\n\
         \x20     --display <nombre>  capturar sólo esa salida (ver --list-displays)\n\
         \x20 -g, --region x,y,w,h    recortar a esa región\n\
         \x20 -e, --edit             abrir la captura en tullpu para anotar\n\
         \x20     --list-displays    listar las salidas (monitores)\n\
         \x20     --backend <b>      auto|native|grim (default auto)\n\
         \x20 -h, --help             esta ayuda"
    );
}
