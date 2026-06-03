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

use mirada_brain::ctl::{self, CtlReply, CtlRequest, WindowLine};
use mirada_brain::DesktopAction;

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
                print_windows(&ws);
                Ok(())
            }
            CtlReply::Error(e) => Err(e),
            CtlReply::Ok => Err("respuesta inesperada del Cerebro".into()),
        },
        // Cicla al siguiente preset de zonas de arrastre (config.ron). Bindealo
        // a un atajo lanzando `mirada-ctl cycle-zones`.
        Some("cycle-zones") => match request(CtlRequest::CycleZones)? {
            CtlReply::Ok => Ok(()),
            CtlReply::Error(e) => Err(e),
            CtlReply::Windows(_) => Err("respuesta inesperada del Cerebro".into()),
        },
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
                CtlReply::Windows(_) => Err("respuesta inesperada del Cerebro".into()),
            }
        }
    }
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

fn print_help() {
    println!(
        "mirada-ctl — control del compositor carmen\n\
         \n\
         USO:\n  \
           mirada-ctl <acción>      aplica una acción de escritorio\n  \
           mirada-ctl windows       lista las ventanas\n  \
           mirada-ctl cycle-zones   cicla el preset de zonas de arrastre\n  \
           mirada-ctl actions       lista las acciones disponibles\n\
         \n\
         EJEMPLOS:\n  \
           mirada-ctl focus-next\n  \
           mirada-ctl focus-window 5\n  \
           mirada-ctl workspace 3\n  \
           mirada-ctl layout grid"
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
