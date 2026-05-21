//! Un Cerebro *headless* para probar el API de control sin gráficos.
//!
//! Abre el socket de `mirada-ctl`, arranca un [`Desktop`] con una pantalla
//! y unas ventanas de muestra, y atiende peticiones en bucle, imprimiendo
//! el estado tras cada una. Útil para ejercitar `mirada-ctl` en modo
//! desatendido.
//!
//! ```sh
//! cargo run -p mirada-brain --example headless-ctl   # terminal 1
//! mirada-ctl windows                                 # terminal 2
//! mirada-ctl focus-next
//! mirada-ctl focus-window 2
//! ```

use std::thread;
use std::time::Duration;

use mirada_brain::ctl::{self, CtlReply, CtlRequest, CtlServer};
use mirada_brain::{BodyEvent, BrainCommand, Desktop};

fn main() {
    let path = ctl::default_socket_path();
    let server = match CtlServer::bind(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Cerebro headless · no pude abrir el control: {e}");
            std::process::exit(1);
        }
    };
    println!("Cerebro headless · control en {}", path.display());

    // Una pantalla y tres ventanas de muestra.
    let mut desktop = Desktop::new();
    desktop.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
    for id in 1..=3 {
        desktop.on_event(BodyEvent::WindowOpened {
            id,
            app_id: format!("org.brahman.app{id}"),
            title: format!("ventana {id}"),
        });
    }
    print_state(&desktop);
    println!("   esperando a mirada-ctl …");

    loop {
        if let Some(mut conn) = server.poll() {
            if let Ok(Some(req)) = conn.read_request() {
                let reply = match req {
                    CtlRequest::Do(action) => {
                        let cmds = desktop.apply(action);
                        // Sin Cuerpo: simulamos nosotros el cierre.
                        for cmd in cmds {
                            if let BrainCommand::Close(id) | BrainCommand::Kill(id) = cmd {
                                desktop.on_event(BodyEvent::WindowClosed { id });
                            }
                        }
                        println!("· {action}");
                        print_state(&desktop);
                        CtlReply::Ok
                    }
                    CtlRequest::ListWindows => CtlReply::Windows(desktop.window_lines()),
                };
                let _ = conn.reply(&reply);
            }
        }
        thread::sleep(Duration::from_millis(16));
    }
}

fn print_state(d: &Desktop) {
    println!(
        "  escritorio {} · foco {:?} · ventanas/escritorio {:?}",
        d.active_index() + 1,
        d.focused_window(),
        d.workspace_loads(),
    );
}
