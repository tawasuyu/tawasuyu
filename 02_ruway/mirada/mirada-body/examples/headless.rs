//! `headless` — un Cuerpo de carmen sin gráficos, guiado por stdin.
//!
//! Es el banco de pruebas del Cerebro: implementa el lado del Cuerpo del
//! protocolo (escucha en un socket, lleva un [`BodyState`], manda
//! [`BodyEvent`]s y ejecuta —imprimiéndolas— las [`BodyOp`]s) sin tocar
//! `smithay` ni el hardware. Así se ejercita el bucle entero
//! Cerebro↔Cuerpo desde una terminal.
//!
//! ```text
//!   # terminal 1 — el Cuerpo escucha
//!   cargo run -p mirada-body --example headless -- /tmp/mirada.sock
//!   # terminal 2 — el Cerebro se conecta
//!   MIRADA_SOCKET=/tmp/mirada.sock cargo run -p mirada
//! ```
//!
//! Órdenes de stdin: `output <w> <h>`, `open <app>`, `close <id>`,
//! `title <id> <texto>`, `key <combo>`, `pointer <id>`, `tick`, `quit`.

use std::io::BufRead;
use std::time::Duration;

use mirada_body::BodyState;
use mirada_link::BodyLink;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/mirada.sock".to_string());
    let _ = std::fs::remove_file(&path);

    println!("Cuerpo headless · escuchando en {path} — esperando al Cerebro…");
    let mut link: BodyLink = match BodyLink::listen(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("no se pudo escuchar en {path}: {e}");
            std::process::exit(1);
        }
    };
    println!("Cerebro conectado. Órdenes: output / open / close / title / key / pointer / tick / quit");

    let mut body = BodyState::new();
    let mut next_id: u64 = 1;
    let stdin = std::io::stdin();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let mut parts = line.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();

        // Cada orden o bien manda un evento al Cerebro, o no manda nada.
        let event = match cmd {
            "output" if rest.len() == 2 => {
                match (rest[0].parse(), rest[1].parse()) {
                    (Ok(w), Ok(h)) => Some(body.add_output(0, w, h)),
                    _ => {
                        eprintln!("uso: output <ancho> <alto>");
                        None
                    }
                }
            }
            "open" if !rest.is_empty() => {
                let app = rest[0];
                let id = next_id;
                next_id += 1;
                println!("  → ventana {id} ({app})");
                Some(body.open_surface(id, format!("org.brahman.{app}"), format!("{app} {id}")))
            }
            "close" if rest.len() == 1 => match rest[0].parse() {
                Ok(id) => body.close_surface(id),
                Err(_) => {
                    eprintln!("uso: close <id>");
                    None
                }
            },
            "title" if rest.len() >= 2 => match rest[0].parse() {
                Ok(id) => body.retitle_surface(id, rest[1..].join(" ")),
                Err(_) => {
                    eprintln!("uso: title <id> <texto>");
                    None
                }
            },
            "key" if rest.len() == 1 => Some(body.keybind(rest[0])),
            "pointer" if rest.len() == 1 => match rest[0].parse() {
                Ok(id) => Some(body.pointer_enter(id)),
                Err(_) => {
                    eprintln!("uso: pointer <id>");
                    None
                }
            },
            "tick" => None,
            "quit" | "exit" => break,
            "" => None,
            other => {
                eprintln!("orden desconocida: {other}");
                None
            }
        };

        if let Some(ev) = event {
            if link.send(&ev).is_err() {
                eprintln!("el Cerebro cerró la conexión.");
                break;
            }
        }

        // Deja que el Cerebro responda y ejecuta lo que ordene.
        std::thread::sleep(Duration::from_millis(40));
        for command in link.drain() {
            for op in body.apply(command) {
                println!("  · op: {op:?}");
            }
        }
    }

    println!("Cuerpo headless · adiós.");
    let _ = std::fs::remove_file(&path);
}
