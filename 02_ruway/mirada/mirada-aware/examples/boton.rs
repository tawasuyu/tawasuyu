//! Ejemplo mínimo de una app **mirada-aware**: registra un par de botones en su
//! barra de título y reacciona a los clicks. Corré una app cliente cualquiera
//! con el mismo `app_id` (el que fija por `xdg_toplevel.set_app_id`) y este
//! proceso le pone los botones y escucha.
//!
//! ```sh
//! cargo run -p mirada-aware --example boton -- com.tu.app
//! ```

use std::time::Duration;

use mirada_aware::{Aware, AwareItem, AwareSide};

fn main() {
    let app_id = std::env::args().nth(1).unwrap_or_else(|| "com.ejemplo.app".to_string());
    let aware = Aware::new(&app_id);

    // Dos botones propios: uno a la derecha (▶ correr) y uno a la izquierda (★).
    let items = vec![
        AwareItem::new("run", "▶", "Correr todo"),
        AwareItem { id: "fav".into(), glyph: "★".into(), label: "Favorito".into(), side: AwareSide::Left },
    ];
    match aware.register(items) {
        Ok(()) => println!("registrado para app_id={app_id}; esperando clicks…"),
        Err(e) => {
            eprintln!("no pude registrar (¿está mirada corriendo?): {e}");
            return;
        }
    }

    // Lazo: poleá los clicks cada 100 ms y reaccioná.
    loop {
        match aware.poll_clicks() {
            Ok(clicks) => {
                for c in clicks {
                    println!("click en «{}» (ventana «{}»)", c.item_id, c.window_title);
                    match c.item_id.as_str() {
                        "run" => println!("  → acá correrías todo el cuaderno"),
                        "fav" => println!("  → acá lo marcarías como favorito"),
                        _ => {}
                    }
                }
            }
            Err(e) => {
                eprintln!("se cortó la conexión con mirada: {e}");
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
