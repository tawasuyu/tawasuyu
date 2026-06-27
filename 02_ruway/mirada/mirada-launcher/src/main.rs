//! `mirada-launcher` — un lanzador de aplicaciones para mirada.
//!
//! Escanea los archivos `.desktop` del sistema (el estándar XDG), los
//! lista en la terminal y lanza el que elijas. No tiene dependencias: la
//! interfaz es una lista numerada que se filtra escribiendo.
//!
//! Pensado para correr dentro de una terminal pequeña que el compositor
//! abre con un atajo — p. ej. atando `Super+d` a
//! `spawn:foot -e mirada-launcher` en el keymap de mirada. Al elegir una
//! aplicación, la lanza y termina (la terminal se cierra sola); el
//! programa lanzado queda corriendo, reparentado a init.
//!
//! También sirve suelto: `mirada-launcher` en cualquier terminal.

use std::collections::HashSet;
use std::io::{self, Write};
use std::path::PathBuf;

/// Una aplicación lista para lanzar, sacada de un `.desktop`.
struct DesktopApp {
    /// Nombre visible (`Name=`).
    name: String,
    /// Comando a ejecutar, ya sin los códigos de campo (`%u`, `%F`…).
    exec: String,
    /// `true` si la app necesita una terminal (`Terminal=true`).
    needs_terminal: bool,
}

fn main() {
    bitacora::abrir("mirada");
    let mut apps = scan_apps();
    apps.sort_by_key(|a| a.name.to_lowercase());
    if apps.is_empty() {
        eprintln!("mirada-launcher · no encontré ninguna aplicación .desktop.");
        std::process::exit(1);
    }
    run_ui(&apps);
}

// ---------------------------------------------------------------------
// Escaneo de los .desktop
// ---------------------------------------------------------------------

/// Recorre los directorios XDG de aplicaciones y devuelve las que se
/// pueden lanzar. Un `.desktop` de un directorio de mayor prioridad
/// tapa a otro con el mismo nombre de archivo en uno de menor.
fn scan_apps() -> Vec<DesktopApp> {
    let mut apps = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for dir in application_dirs() {
        collect_desktop_files(&dir, &dir, &mut seen, &mut apps);
    }
    apps
}

/// Los directorios `applications/` del estándar XDG, en orden de
/// prioridad: primero el del usuario, luego los del sistema.
fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    if let Some(home) = data_home {
        dirs.push(home.join("applications"));
    }

    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    for d in data_dirs.split(':').filter(|s| !s.is_empty()) {
        dirs.push(PathBuf::from(d).join("applications"));
    }
    dirs
}

/// Recoge los `.desktop` de `dir` (y subdirectorios) sin repetir id.
fn collect_desktop_files(
    root: &PathBuf,
    dir: &PathBuf,
    seen: &mut HashSet<String>,
    apps: &mut Vec<DesktopApp>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_desktop_files(root, &path, seen, apps);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        // El id XDG: la ruta relativa al directorio raíz, con `/` → `-`.
        let id = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('/', "-");
        if !seen.insert(id) {
            continue; // ya lo tapó un directorio de más prioridad
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(app) = parse_desktop(&text) {
                apps.push(app);
            }
        }
    }
}

/// Extrae una [`DesktopApp`] del texto de un `.desktop`. `None` si no es
/// una aplicación lanzable o está marcada para no mostrarse.
fn parse_desktop(text: &str) -> Option<DesktopApp> {
    let mut in_entry = false;
    let (mut name, mut exec, mut kind) = (None, None, None);
    let (mut no_display, mut hidden, mut terminal) = (false, false, false);

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            // Sólo nos interesa el grupo principal; otros (acciones,
            // etc.) se ignoran.
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Name" => name = Some(value.to_string()),
            "Exec" => exec = Some(value.to_string()),
            "Type" => kind = Some(value.to_string()),
            "NoDisplay" => no_display = value == "true",
            "Hidden" => hidden = value == "true",
            "Terminal" => terminal = value == "true",
            _ => {} // Name[es], Icon, Categories…: no los usamos
        }
    }

    if no_display || hidden {
        return None;
    }
    if kind.as_deref() != Some("Application") {
        return None;
    }
    let name = name?;
    let exec = strip_field_codes(&exec?);
    if name.is_empty() || exec.is_empty() {
        return None;
    }
    Some(DesktopApp { name, exec, needs_terminal: terminal })
}

/// Quita los códigos de campo de un `Exec` de `.desktop` (`%u`, `%F`,
/// `%i`…), que sólo tienen sentido al abrir archivos. `%%` queda en `%`.
fn strip_field_codes(exec: &str) -> String {
    let mut out = String::new();
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            // `%%` es un `%` literal; cualquier otro `%x` es un código de
            // campo y se descarta entero.
            if let Some('%') = chars.next() {
                out.push('%');
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

// ---------------------------------------------------------------------
// Interfaz de terminal
// ---------------------------------------------------------------------

/// Cuántas aplicaciones se listan como mucho de una vez.
const MAX_SHOWN: usize = 40;

/// El bucle de la interfaz: muestra la lista, lee una línea y según sea
/// un número lanza, texto filtra, o vacía sale.
fn run_ui(apps: &[DesktopApp]) {
    let mut filter = String::new();
    loop {
        let needle = filter.to_lowercase();
        let matches: Vec<&DesktopApp> = apps
            .iter()
            .filter(|a| needle.is_empty() || a.name.to_lowercase().contains(&needle))
            .collect();

        // Limpia la pantalla y dibuja la lista.
        print!("\x1b[2J\x1b[H");
        if filter.is_empty() {
            println!("mirada-launcher · {} aplicaciones", matches.len());
        } else {
            println!(
                "mirada-launcher · {} de {} · filtro «{filter}»",
                matches.len(),
                apps.len()
            );
        }
        println!();
        if matches.is_empty() {
            println!("  (sin coincidencias)");
        }
        for (i, a) in matches.iter().take(MAX_SHOWN).enumerate() {
            println!("  {:>2}  {}", i + 1, a.name);
        }
        if matches.len() > MAX_SHOWN {
            println!("  …  y {} más — afina el filtro", matches.len() - MAX_SHOWN);
        }
        println!();
        println!("  nº = lanzar · texto = filtrar · Enter vacío = salir");
        print!("> ");
        io::stdout().flush().ok();

        let mut line = String::new();
        if io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            return; // fin de entrada (Ctrl+D)
        }
        let line = line.trim();
        if line.is_empty() {
            return;
        }

        // ¿Un número? Lanza esa entrada de la lista visible.
        if let Ok(n) = line.parse::<usize>() {
            if (1..=matches.len().min(MAX_SHOWN)).contains(&n) {
                launch(matches[n - 1]);
                return;
            }
            continue; // número fuera de rango: vuelve a pedir
        }

        // Texto: es un filtro nuevo. Si deja una sola, lánzala directo.
        filter = line.to_string();
        let needle = filter.to_lowercase();
        let now: Vec<&DesktopApp> = apps
            .iter()
            .filter(|a| a.name.to_lowercase().contains(&needle))
            .collect();
        if now.len() == 1 {
            launch(now[0]);
            return;
        }
    }
}

/// Lanza la aplicación elegida como proceso hijo y devuelve. Hereda el
/// entorno —`WAYLAND_DISPLAY` incluido—; al terminar el lanzador, el
/// proceso queda corriendo, reparentado a init.
fn launch(app: &DesktopApp) {
    let cmd = if app.needs_terminal {
        format!("foot -e {}", app.exec)
    } else {
        app.exec.clone()
    };
    print!("\x1b[2J\x1b[H");
    println!("mirada-launcher · lanzando «{}» …", app.name);
    match std::process::Command::new("sh").arg("-c").arg(&cmd).spawn() {
        Ok(_) => {}
        Err(e) => {
            eprintln!("mirada-launcher · no pude lanzar «{cmd}»: {e}");
            std::process::exit(1);
        }
    }
}
