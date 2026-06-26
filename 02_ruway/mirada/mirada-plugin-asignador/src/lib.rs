//! Plugin reactor **asignador**: enruta cada ventana nueva por su `app_id` a un
//! escritorio fijo y/o la hace flotar, según reglas que vienen en su **config**
//! (el campo `config:` del manifest, editable a mano o desde wawa-panel).
//!
//! Es el patrón más pedido en un WM teselante ("mandá el navegador al 2, el
//! chat al 3, flotá la calculadora") y el primer plugin que usa la **config por
//! plugin**: sin reglas no hace nada; con reglas, enruta al abrir cada ventana.
//!
//! ## Formato de la config
//!
//! Una regla por línea. `#` comenta. Cada regla es un **substring del `app_id`**
//! (sin distinguir mayúsculas) seguido de su destino — un número de escritorio
//! `1..9`, la palabra `float`, o ambos:
//!
//! ```text
//! firefox      2          # Firefox → escritorio 2
//! Alacritty    1          # la terminal → escritorio 1
//! pavucontrol  float      # el mezclador, flotando
//! calc         5 float    # la calculadora → escritorio 5, flotando
//! ```
//!
//! Gana la **primera** regla que case. Usa sólo `CAP_ACTIONS`: emite
//! `toggle-float` y `send-to-workspace:N` como acciones de escritorio.

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use mirada_plugin_sdk::{export_reactor_plugin, BodyEvent, Ctx, ReactorPlugin};

/// Una regla de enrutado: si el `app_id` contiene `pat`, mandar al escritorio
/// `ws` (si lo hay) y/o flotar.
struct Rule {
    /// Substring del `app_id` a buscar, ya en minúsculas.
    pat: String,
    /// Escritorio destino (1..9), si la regla lo fija.
    ws: Option<u8>,
    /// Si la ventana debe flotar.
    float: bool,
}

#[derive(Default)]
struct Asignador {
    rules: Vec<Rule>,
}

impl Asignador {
    /// Parsea el texto de config a reglas. Líneas vacías o `#…` se ignoran;
    /// una línea sin destino útil (ni escritorio ni `float`) se descarta.
    fn parse(config: &str) -> Vec<Rule> {
        let mut rules = Vec::new();
        for line in config.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut toks = line.split_whitespace();
            let Some(pat) = toks.next() else { continue };
            let mut ws = None;
            let mut float = false;
            for t in toks {
                if t.eq_ignore_ascii_case("float") {
                    float = true;
                } else if let Ok(n) = t.parse::<u8>() {
                    if (1..=9).contains(&n) {
                        ws = Some(n);
                    }
                }
            }
            if ws.is_some() || float {
                rules.push(Rule { pat: pat.to_ascii_lowercase(), ws, float });
            }
        }
        rules
    }
}

impl ReactorPlugin for Asignador {
    fn configure(&mut self, config: &str) {
        self.rules = Self::parse(config);
    }

    fn on_event(&mut self, event: BodyEvent, ctx: &mut Ctx) {
        let BodyEvent::WindowOpened { app_id, .. } = event else {
            return;
        };
        let id = app_id.to_ascii_lowercase();
        let Some(rule) = self.rules.iter().find(|r| id.contains(r.pat.as_str())) else {
            return;
        };
        // La ventana recién abierta tiene el foco. Flotamos PRIMERO (mientras
        // sigue enfocada) y recién después la mandamos: `send-to-workspace` opera
        // sobre la enfocada, y `toggle-float` no le quita el foco.
        if rule.float {
            ctx.act("toggle-float");
        }
        if let Some(n) = rule.ws {
            ctx.act(&format!("send-to-workspace:{n}"));
        }
    }
}

export_reactor_plugin!(Asignador::default());
