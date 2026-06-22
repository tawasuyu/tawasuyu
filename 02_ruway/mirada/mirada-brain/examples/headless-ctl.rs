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
    eprintln!("Cerebro headless · control en {}", path.display());

    // Dos pantallas y tres ventanas de muestra.
    let mut desktop = Desktop::new();
    desktop.on_event(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 });
    desktop.on_event(BodyEvent::OutputAdded { id: 1, width: 1920, height: 1080 });
    for id in 1..=3 {
        desktop.on_event(BodyEvent::WindowOpened {
            id,
            app_id: format!("org.brahman.app{id}"),
            title: format!("ventana {id}"),
        });
    }
    print_state(&desktop);
    eprintln!("   esperando a mirada-ctl …");

    loop {
        if let Some(mut conn) = server.poll() {
            if let Ok(Some(req)) = conn.read_request() {
                let reply = match req {
                    CtlRequest::Do(action) => {
                        eprintln!("· {action}");
                        for cmd in desktop.apply(action) {
                            match cmd {
                                // La geometría que el Cerebro mandaría al Cuerpo.
                                BrainCommand::Place(places) => {
                                    for p in places {
                                        eprintln!(
                                            "    win {} → {:>5}×{:<4} @ ({:>5},{:>4}){}{}",
                                            p.id,
                                            p.rect.w,
                                            p.rect.h,
                                            p.rect.x,
                                            p.rect.y,
                                            if p.fullscreen {
                                                "  ~pantalla"
                                            } else if p.floating {
                                                "  ~flotante"
                                            } else {
                                                ""
                                            },
                                            if p.focused { "  *" } else { "" },
                                        );
                                    }
                                }
                                // Sin Cuerpo: simulamos nosotros el cierre.
                                BrainCommand::Close(id) | BrainCommand::Kill(id) => {
                                    desktop.on_event(BodyEvent::WindowClosed { id });
                                }
                                _ => {}
                            }
                        }
                        print_state(&desktop);
                        CtlReply::Ok
                    }
                    CtlRequest::ListWindows => CtlReply::Windows(desktop.window_lines()),
                    CtlRequest::Workspaces => CtlReply::Workspaces(mirada_brain::WorkspacesState {
                        active: desktop.active_index() + 1,
                        loads: desktop.workspace_loads(),
                        layout: mirada_brain::layout_slug(
                            desktop.active_workspace().params().mode,
                        )
                        .to_string(),
                        on_other_outputs: desktop.workspaces_on_other_outputs(),
                    }),
                    // Las zonas son del Cuerpo (compositor); este ejemplo
                    // headless del Cerebro no las tiene.
                    CtlRequest::CycleZones => CtlReply::Ok,
                };
                let _ = conn.reply(&reply);
            }
        }
        thread::sleep(Duration::from_millis(16));
    }
}

fn print_state(d: &Desktop) {
    let ws = d.active_workspace();
    eprintln!(
        "  activo: escritorio {} · {:?} (maestra {:.0}%) · foco {:?}",
        d.active_index() + 1,
        ws.params().mode,
        ws.params().master_ratio * 100.0,
        d.focused_window(),
    );
    for (i, o) in d.outputs().iter().enumerate() {
        let mark = if i == d.focused_output() { '*' } else { ' ' };
        eprintln!(
            "  {mark} salida {} {}×{} @ x{} → escritorio {}",
            o.id,
            o.rect.w,
            o.rect.h,
            o.rect.x,
            o.workspace + 1,
        );
    }
}
