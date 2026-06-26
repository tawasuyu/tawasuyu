//! Plugin reactor **efecto por app**: fija opacidad y sombra de cada ventana
//! según su `app_id`, por reglas de la config. Mismo patrón que el `asignador`
//! (enruta por app_id), pero emitiendo `CAP_EFFECTS` en vez de acciones — p. ej.
//! la terminal semi-transparente, el navegador opaco, el visor sin sombra.
//!
//! ## Formato de la config
//!
//! Una regla por línea. `#` comenta. Cada regla es `<app_id-substring>  <opacidad
//! 0-100>  [shadow|noshadow]` (sin distinguir mayúsculas en el substring):
//!
//! ```text
//! Alacritty   88            # la terminal al 88% de opacidad (con sombra)
//! foot        85  noshadow  # otra terminal, sin sombra
//! mpv         100 noshadow  # el reproductor opaco y sin sombra
//! ```
//!
//! Opacidad por defecto si se omite: 100 (opaca). Sombra por defecto: sí. Gana
//! la **primera** regla que case. Se aplica al abrir cada ventana
//! (`WindowOpened`). Sin reglas no hace nada.

#![no_std]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use mirada_plugin_sdk::{export_reactor_plugin, BodyEvent, Ctx, ReactorPlugin, WindowEffects};

/// Una regla de efecto: si el `app_id` contiene `pat`, fijar `opacity`/`shadow`.
struct Rule {
    /// Substring del `app_id` a buscar, ya en minúsculas.
    pat: String,
    /// Opacidad de composición (`0`=transparente, `255`=opaca).
    opacity: u8,
    shadow: bool,
}

#[derive(Default)]
struct EfectoPorApp {
    rules: Vec<Rule>,
}

impl EfectoPorApp {
    fn parse(config: &str) -> Vec<Rule> {
        let mut rules = Vec::new();
        for line in config.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut toks = line.split_whitespace();
            let Some(pat) = toks.next() else { continue };
            let mut opacity_pct: u32 = 100;
            let mut shadow = true;
            for t in toks {
                if t.eq_ignore_ascii_case("noshadow") {
                    shadow = false;
                } else if t.eq_ignore_ascii_case("shadow") {
                    shadow = true;
                } else if let Ok(n) = t.parse::<u32>() {
                    opacity_pct = n.min(100);
                }
            }
            // 0-100 → 0-255.
            let opacity = ((opacity_pct * 255 + 50) / 100) as u8;
            rules.push(Rule { pat: pat.to_ascii_lowercase(), opacity, shadow });
        }
        rules
    }
}

impl ReactorPlugin for EfectoPorApp {
    fn configure(&mut self, config: &str) {
        self.rules = Self::parse(config);
    }

    fn on_event(&mut self, event: BodyEvent, ctx: &mut Ctx) {
        let BodyEvent::WindowOpened { id, app_id, .. } = event else {
            return;
        };
        let id_lc = app_id.to_ascii_lowercase();
        if let Some(rule) = self.rules.iter().find(|r| id_lc.contains(r.pat.as_str())) {
            ctx.set_effects(id, WindowEffects { opacity: rule.opacity, shadow: rule.shadow });
        }
    }
}

export_reactor_plugin!(EfectoPorApp::default());
