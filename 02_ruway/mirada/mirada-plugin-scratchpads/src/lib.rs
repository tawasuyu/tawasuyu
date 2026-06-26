//! Plugin reactor **scratchpads con nombre**: cajones ocultos que aparecen y se
//! esconden con un atajo, estilo Hyprland `special:nombre` / sway scratchpad —
//! pero con **varios, cada uno con su nombre y su tecla**. Generaliza el
//! scratchpad único del core (`Super+Shift+~`) y el dropterm.
//!
//! El core ya sabe hacer el trabajo pesado: las acciones de escritorio
//! `toggle-special:NOMBRE` (muestra/oculta el especial flotando sobre el activo)
//! y `move-to-special:NOMBRE` (aparta la ventana enfocada a ese especial). Este
//! plugin sólo **registra atajos** (`CAP_KEYS`) y los traduce a esas acciones
//! (`CAP_ACTIONS`) — toda la política vive en el `Desktop` autoritativo.
//!
//! ## Formato de la config
//!
//! Una entrada por línea. `#` comenta. Cada línea es `<tecla>  [verbo]  <nombre>`:
//!
//! - **verbo** `toggle` (default) → muestra/oculta el cajón `<nombre>`.
//! - **verbo** `send` (o `+`) → manda la ventana enfocada al cajón `<nombre>`
//!   (la oculta ahí hasta que lo invoques).
//!
//! ```text
//! Super+grave         dev          # Super+` muestra/oculta el cajón «dev»
//! Super+Shift+grave   send  dev    # Super+Shift+` manda la enfocada a «dev»
//! Super+n             notas        # otro cajón, con su propia tecla
//! Super+Shift+n       send  notas
//! ```
//!
//! Sin config no hace nada (no registra atajos). Es la forma recomendada de los
//! scratchpads con nombre: el core ya trae el genérico sin nombre.

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use mirada_plugin_sdk::{export_reactor_plugin, BodyEvent, Ctx, ReactorPlugin};

/// Un atajo → acción sobre un escritorio especial con nombre.
struct Bind {
    /// La combinación a interceptar (`"Super+grave"`, canon del Cuerpo).
    key: String,
    /// Nombre del especial (cajón).
    name: String,
    /// `true` = mandar la enfocada (`move-to-special`); `false` = mostrar/ocultar
    /// (`toggle-special`).
    send: bool,
}

#[derive(Default)]
struct Scratchpads {
    binds: Vec<Bind>,
    /// Caché de las teclas a registrar (las claves de `binds`), para no
    /// reconstruirla en cada evento.
    keys: Vec<String>,
}

impl Scratchpads {
    /// Parsea la config a binds. Líneas vacías o `#…` se ignoran; una línea sin
    /// nombre de cajón se descarta.
    fn parse(config: &str) -> Vec<Bind> {
        let mut binds = Vec::new();
        for line in config.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut toks = line.split_whitespace();
            let Some(key) = toks.next() else { continue };
            let mut send = false;
            let mut name: Option<&str> = None;
            for t in toks {
                if t.eq_ignore_ascii_case("send") || t == "+" {
                    send = true;
                } else if t.eq_ignore_ascii_case("toggle") {
                    send = false;
                } else if name.is_none() {
                    name = Some(t);
                }
            }
            if let Some(name) = name {
                binds.push(Bind { key: key.to_string(), name: name.to_string(), send });
            }
        }
        binds
    }
}

impl ReactorPlugin for Scratchpads {
    fn configure(&mut self, config: &str) {
        self.binds = Self::parse(config);
        self.keys = self.binds.iter().map(|b| b.key.clone()).collect();
    }

    fn on_event(&mut self, event: BodyEvent, ctx: &mut Ctx) {
        // Re-registramos en cada evento: el host une y deduplica nuestros atajos
        // con los del Desktop (idem que el reactor de ejemplo). Sin esto, las
        // teclas no quedarían interceptadas hasta el primer GrabKeys.
        if !self.keys.is_empty() {
            ctx.grab_keys(&self.keys);
        }
        let BodyEvent::Keybind(combo) = event else {
            return;
        };
        for b in &self.binds {
            if b.key == combo {
                if b.send {
                    ctx.act(&format!("move-to-special:{}", b.name));
                } else {
                    ctx.act(&format!("toggle-special:{}", b.name));
                }
            }
        }
    }
}

export_reactor_plugin!(Scratchpads::default());
