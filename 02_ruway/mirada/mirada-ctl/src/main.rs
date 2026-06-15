//! `mirada-ctl` — el control del compositor carmen por línea de comandos.
//!
//! Al estilo de `swaymsg` / `hyprctl`: dispara una acción de escritorio o
//! consulta el estado, hablando con el Cerebro por su socket de control
//! ([`mirada_brain::ctl`]). El Cerebro es la app `mirada`, o
//! `mirada-compositor` cuando lleva el Cerebro embebido.
//!
//! ```sh
//! mirada-ctl focus-next            # cambia el foco
//! mirada-ctl focus-window 5        # enfoca una ventana concreta
//! mirada-ctl workspace 3           # va al escritorio 3
//! mirada-ctl layout grid           # fija el modo de teselado
//! mirada-ctl windows               # lista las ventanas
//! mirada-ctl actions               # lista las acciones
//! ```

use std::process::ExitCode;

use std::path::PathBuf;

use mirada_brain::ctl::{self, CtlReply, CtlRequest, WindowLine, WorkspacesState};
use mirada_brain::{DesktopAction, KeymapProfiles};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("mirada-ctl: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => {
            print_help();
            Ok(())
        }
        Some("actions") => {
            print_actions();
            Ok(())
        }
        Some("windows") => match request(CtlRequest::ListWindows)? {
            CtlReply::Windows(ws) => {
                // `--porcelain`: una línea TAB-separada por ventana, para que la
                // consuma la barra (`pata`) sin parsear la tabla humana.
                if args.iter().any(|a| a == "--porcelain") {
                    print_windows_porcelain(&ws);
                } else {
                    print_windows(&ws);
                }
                Ok(())
            }
            CtlReply::Error(e) => Err(e),
            _ => Err("respuesta inesperada del Cerebro".into()),
        },
        Some("workspaces") => match request(CtlRequest::Workspaces)? {
            CtlReply::Workspaces(st) => {
                print_workspaces(&st);
                Ok(())
            }
            CtlReply::Error(e) => Err(e),
            _ => Err("respuesta inesperada del Cerebro".into()),
        },
        // Cicla al siguiente preset de zonas de arrastre (config.ron). Bindealo
        // a un atajo lanzando `mirada-ctl cycle-zones`.
        Some("cycle-zones") => match request(CtlRequest::CycleZones)? {
            CtlReply::Ok => Ok(()),
            CtlReply::Error(e) => Err(e),
            _ => Err("respuesta inesperada del Cerebro".into()),
        },
        // Perfiles de atajos: gestión de la biblioteca de keymaps. Son
        // operaciones de archivo (profiles.ron + keymap.ron); el compositor
        // recarga en caliente vía su FileWatch — no necesita socket.
        Some("profile" | "profiles") => run_profile(&args[1..]),
        // Vistas: presets de escritorio completo (look + decoraciones + layout +
        // teclas + barra de pata). Operación de archivo: escribe config.ron,
        // keymap.ron y el launcher.toml de pata; el compositor y pata recargan
        // en caliente. Es lo que hace alcanzable el «panel de control» (las
        // vistas) desde la sesión real, sin la app de simulación.
        Some("vista" | "vistas") => run_vista(&args[1..]),
        // Todo lo demás es una acción. `focus-window 5` y `workspace 3`
        // se unen con `:` a la forma canónica (`focus-window:5`).
        Some(_) => {
            let spec = args.join(":");
            let action: DesktopAction = spec
                .parse()
                .map_err(|e| format!("{e}\n  lista de acciones:  mirada-ctl actions"))?;
            match request(CtlRequest::Do(action))? {
                CtlReply::Ok => Ok(()),
                CtlReply::Error(e) => Err(e),
                _ => Err("respuesta inesperada del Cerebro".into()),
            }
        }
    }
}

/// Las rutas de la biblioteca de perfiles y del keymap activo.
fn profile_paths() -> Result<(PathBuf, PathBuf), String> {
    let profiles = KeymapProfiles::default_path()
        .ok_or("no pude determinar ~/.config/mirada (profiles.ron)")?;
    let keymap =
        KeymapProfiles::keymap_path().ok_or("no pude determinar ~/.config/mirada (keymap.ron)")?;
    Ok((profiles, keymap))
}

/// Aplica/lista **vistas** de escritorio completo (`mirada-ctl vista …`).
/// `use` escribe config.ron (decoraciones+layout+tema) + keymap.ron (vía el
/// perfil de la vista) + el launcher.toml de pata (barra de la vista). El
/// compositor y pata recargan en caliente.
fn run_vista(args: &[String]) -> Result<(), String> {
    let sub = args.first().map(String::as_str);
    if matches!(sub, None | Some("list" | "ls")) {
        for name in mirada_brain::VISTA_NAMES {
            println!("  {name:<14} {}", mirada_brain::Vista::label_for(name));
        }
        return Ok(());
    }
    match sub {
        Some("use" | "set") => {
            let name = args
                .get(1)
                .map(String::as_str)
                .ok_or("uso: mirada-ctl vista use <nombre>  (ver: mirada-ctl vista list)")?;
            let v = mirada_brain::Vista::by_name(name).ok_or_else(|| {
                format!("vista desconocida «{name}» (ver: mirada-ctl vista list)")
            })?;
            // 1. Decoraciones + layout + tema → config.ron (lo vigila el compositor).
            let cfgp = mirada_brain::Config::default_path()
                .ok_or("no pude determinar ~/.config/mirada/config.ron")?;
            v.config.save(&cfgp).map_err(|e| format!("config: {e}"))?;
            // 2. Teclas: el keymap de la vista como perfil activo → keymap.ron.
            let (ppath, kpath) = profile_paths()?;
            let mut profs = KeymapProfiles::load_or_init(&ppath);
            profs.set_active(v.keymap).map_err(|e| e.to_string())?;
            profs.save(&ppath).map_err(|e| e.to_string())?;
            profs.write_active_keymap(&kpath).map_err(|e| e.to_string())?;
            // 3. Barra: el preset de barra de la vista → launcher.toml (lo vigila pata).
            if let Some(bar) = pata_core::Config::vista_preset(name) {
                pata_config::save(&bar).map_err(|e| format!("barra: {e}"))?;
            }
            println!(
                "vista «{}» aplicada — decoraciones + teclas + barra (recarga en caliente)",
                v.label
            );
            Ok(())
        }
        Some(other) => Err(format!(
            "subcomando de vista desconocido: «{other}»\n  use: list · use <nombre>"
        )),
        None => unreachable!("list lo maneja la rama de arriba"),
    }
}

/// Gestiona la biblioteca de perfiles de atajos (`mirada-ctl profile …`).
fn run_profile(args: &[String]) -> Result<(), String> {
    let (ppath, kpath) = profile_paths()?;
    let sub = args.first().map(String::as_str);
    // `list` y la forma sin subcomando sólo leen.
    if matches!(sub, None | Some("list" | "ls")) {
        let profs = KeymapProfiles::load_or_init(&ppath);
        for name in profs.names() {
            let mark = if name == profs.active() { '*' } else { ' ' };
            let kind = if mirada_brain::Keymap::is_builtin_name(&name) {
                "(fábrica)"
            } else {
                ""
            };
            let n = profs.get(&name).map(|k| k.len()).unwrap_or(0);
            println!("{mark} {name:<16} {n:>2} atajos {kind}");
        }
        return Ok(());
    }

    let mut profs = KeymapProfiles::load_or_init(&ppath);
    let arg = |i: usize| args.get(i).map(String::as_str);
    let mut switched = false;
    match sub {
        Some("use" | "switch") => {
            let name = arg(1).ok_or("uso: mirada-ctl profile use <nombre>")?;
            profs.set_active(name).map_err(|e| e.to_string())?;
            switched = true;
        }
        Some("new" | "create") => {
            // `new <nombre>` (desde dwm) o `new <nombre> from <preset>`.
            let name = arg(1).ok_or("uso: mirada-ctl profile new <nombre> [from <preset>]")?;
            let preset = match (arg(2), arg(3)) {
                (Some("from"), Some(p)) => p,
                (None, _) => "dwm",
                _ => return Err("uso: mirada-ctl profile new <nombre> [from <preset>]".into()),
            };
            profs.create_from_preset(name, preset).map_err(|e| e.to_string())?;
            println!("perfil «{name}» creado desde «{preset}»");
        }
        Some("dup" | "duplicate") => {
            let (src, name) = (
                arg(1).ok_or("uso: mirada-ctl profile dup <origen> <nombre>")?,
                arg(2).ok_or("uso: mirada-ctl profile dup <origen> <nombre>")?,
            );
            profs.duplicate(src, name).map_err(|e| e.to_string())?;
            println!("perfil «{name}» duplicado de «{src}»");
        }
        Some("rename" | "mv") => {
            let (from, to) = (
                arg(1).ok_or("uso: mirada-ctl profile rename <origen> <nombre>")?,
                arg(2).ok_or("uso: mirada-ctl profile rename <origen> <nombre>")?,
            );
            profs.rename(from, to).map_err(|e| e.to_string())?;
            println!("perfil «{from}» renombrado a «{to}»");
        }
        Some("rm" | "remove" | "delete") => {
            let name = arg(1).ok_or("uso: mirada-ctl profile rm <nombre>")?;
            profs.remove(name).map_err(|e| e.to_string())?;
            println!("perfil «{name}» borrado");
        }
        Some(other) => {
            return Err(format!(
                "subcomando de perfil desconocido: «{other}»\n  \
                 use: list · use · new · dup · rename · rm"
            ))
        }
        None => unreachable!("list lo maneja la rama de arriba"),
    }

    profs.save(&ppath).map_err(|e| e.to_string())?;
    // Conmutar (o borrar/renombrar el activo) cambia el keymap efectivo: lo
    // volcamos a keymap.ron y el compositor lo recarga en caliente.
    profs.write_active_keymap(&kpath).map_err(|e| e.to_string())?;
    if switched {
        println!("perfil activo: «{}» (recargado)", profs.active());
    }
    Ok(())
}

/// Manda una petición al Cerebro y devuelve su respuesta.
fn request(req: CtlRequest) -> Result<CtlReply, String> {
    let path = ctl::default_socket_path();
    ctl::send_request(&path, &req).map_err(|e| {
        format!(
            "no pude hablar con el Cerebro en {} ({e})\n  \
             ¿está corriendo `mirada` o `mirada-compositor`?",
            path.display()
        )
    })
}

/// Imprime la lista de ventanas, marcando la enfocada con `*`.
fn print_windows(windows: &[WindowLine]) {
    if windows.is_empty() {
        println!("(no hay ventanas)");
        return;
    }
    for w in windows {
        let mark = if w.focused { '*' } else { ' ' };
        // El escritorio 0 es el scratchpad (ventana guardada).
        let ws = if w.workspace == 0 {
            "scratch".to_string()
        } else {
            w.workspace.to_string()
        };
        println!("{mark} id {:<4} esc {:<7} {:<24} {}", w.id, ws, w.app_id, w.title);
    }
}

/// Imprime las ventanas en formato **porcelain**: una línea por ventana, campos
/// separados por TAB, pensada para que la consuma un *task manager* (la barra de
/// `pata` en el backend winit) sin parsear la tabla humana:
/// `id\tworkspace\tfocused\tminimized\tapp_id\ttitle`. El título puede llevar
/// espacios pero no tabs, así que el separador es estable aunque el `app_id`
/// esté vacío.
fn print_windows_porcelain(windows: &[WindowLine]) {
    for w in windows {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            w.id, w.workspace, w.focused as u8, w.minimized as u8, w.app_id, w.title
        );
    }
}

/// Imprime el estado de los escritorios en **una línea key=value** estable —
/// pensada para que la consuma un *workspace switcher* (la barra de `pata`) sin
/// parsear texto humano: `active=2 count=9 loads=1,0,3,0,0,0,0,0,0`.
fn print_workspaces(st: &WorkspacesState) {
    let loads = st
        .loads
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "active={} count={} loads={} layout={}",
        st.active,
        st.loads.len(),
        loads,
        st.layout
    );
}

fn print_help() {
    println!(
        "mirada-ctl — control del compositor mirada\n\
         \n\
         USO:\n  \
           mirada-ctl <acción>      aplica una acción de escritorio\n  \
           mirada-ctl windows       lista las ventanas (--porcelain: TAB-separado)\n  \
           mirada-ctl workspaces    estado de los escritorios (active/count/loads)\n  \
           mirada-ctl cycle-zones   cicla el preset de zonas de arrastre\n  \
           mirada-ctl profile …     biblioteca de perfiles de atajos (ver abajo)\n  \
           mirada-ctl vista …       vistas de escritorio completo (ver abajo)\n  \
           mirada-ctl actions       lista las acciones disponibles\n\
         \n\
         VISTAS (look + decoraciones + layout + teclas + barra):\n  \
           mirada-ctl vista list            lista las vistas\n  \
           mirada-ctl vista use <nombre>    aplica una vista (recarga en caliente)\n  \
           vistas: mirada · windows-xp · windows-3.1 · mac · kde · solaris · hyprland · dwm\n\
         \n\
         PERFILES DE ATAJOS:\n  \
           mirada-ctl profile list              lista los perfiles (* = activo)\n  \
           mirada-ctl profile use <nombre>      conmuta el activo (recarga en caliente)\n  \
           mirada-ctl profile new <nombre> [from <preset>]   crea desde un preset\n  \
           mirada-ctl profile dup <origen> <nombre>          duplica uno existente\n  \
           mirada-ctl profile rename <origen> <nombre>       renombra uno propio\n  \
           mirada-ctl profile rm <nombre>       borra un perfil propio\n  \
           presets de fábrica: dwm · i3 · hyprland\n\
         \n\
         EJEMPLOS:\n  \
           mirada-ctl focus-next\n  \
           mirada-ctl focus-window 5\n  \
           mirada-ctl workspace 3\n  \
           mirada-ctl layout grid\n  \
           mirada-ctl profile use i3"
    );
}

fn print_actions() {
    // Cadena multilínea literal: la indentación de cada línea es la que
    // se imprime (el `\` tras la comilla se come sólo el primer salto).
    print!(
        "\
Acciones de mirada-ctl:
  focus-next                 mueve el foco a la siguiente ventana
  focus-prev                 mueve el foco a la anterior
  focus-window <id>          enfoca la ventana <id>  (ver: mirada-ctl windows)
  move-forward               adelanta la ventana enfocada en el teselado
  move-backward              la atrasa
  close-focused              cierra la ventana enfocada
  close-window <id>          cierra la ventana <id>  (ver: mirada-ctl windows)
  toggle-float               alterna flotante / teselada la enfocada
  toggle-fullscreen          alterna pantalla completa en la enfocada
  send-to-scratchpad         guarda la ventana enfocada en el scratchpad
  toggle-scratchpad          invoca u oculta la ventana del scratchpad
  cycle-layout               pasa al siguiente modo de teselado
  layout <modo>              master-stack · centered-master · spiral
                             grid · columns · rows · monocle
  grow-master                agranda el área de la ventana maestra
  shrink-master              la encoge
  inc-master / dec-master    nº de ventanas en el área maestra (nmaster)
  promote-to-master          la ventana enfocada al puesto maestro
  workspace <n>              activa el escritorio n (1..9)
  send-to-workspace <n>      manda la enfocada al escritorio n
  focus-output-next          pasa el foco al siguiente monitor
  quit                       apaga el compositor
"
    );
}
