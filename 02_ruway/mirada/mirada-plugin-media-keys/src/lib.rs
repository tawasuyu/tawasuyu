//! Plugin reactor **teclas de medios**: cablea las teclas multimedia del teclado
//! (las `XF86…`, sin modificador) a comandos de sistema. Volumen y mute por
//! `wpctl` (PipeWire), brillo por `brightnessctl`, transporte por `playerctl`,
//! captura de pantalla por `grim`. Las teclas se interceptan vía `CAP_KEYS` y el
//! comando se lanza por `CAP_SPAWN` (`sh -c`).
//!
//! Trae **defaults sensatos** que andan sin config. La config (opcional) los
//! ajusta: una línea `<tecla>  <comando…>` agrega o reemplaza un bind; una línea
//! con sólo la tecla lo **borra**. `#` comenta.
//!
//! ```text
//! # subir el volumen de a 10% en vez de 5%:
//! XF86AudioRaiseVolume  wpctl set-volume @DEFAULT_AUDIO_SINK@ 10%+
//! # capturar a un área con slurp en vez de pantalla entera:
//! Print  grim -g "$(slurp)" ~/Pictures/recorte.png
//! # desactivar el bind de micrófono:
//! XF86AudioMicMute
//! ```

#![no_std]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use mirada_plugin_sdk::{export_reactor_plugin, BodyEvent, Ctx, ReactorPlugin};

/// Binds de fábrica `(tecla XF86, comando sh)`. Andan sin config.
const DEFAULTS: &[(&str, &str)] = &[
    ("XF86AudioRaiseVolume", "wpctl set-volume -l 1.5 @DEFAULT_AUDIO_SINK@ 5%+"),
    ("XF86AudioLowerVolume", "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"),
    ("XF86AudioMute", "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"),
    ("XF86AudioMicMute", "wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle"),
    ("XF86MonBrightnessUp", "brightnessctl set 5%+"),
    ("XF86MonBrightnessDown", "brightnessctl set 5%-"),
    ("XF86AudioPlay", "playerctl play-pause"),
    ("XF86AudioPause", "playerctl play-pause"),
    ("XF86AudioNext", "playerctl next"),
    ("XF86AudioPrev", "playerctl previous"),
    ("XF86AudioStop", "playerctl stop"),
    ("Print", "grim ~/Pictures/captura-mirada.png"),
];

/// Una tecla → comando a lanzar.
struct Bind {
    key: String,
    cmd: String,
}

#[derive(Default)]
struct MediaKeys {
    binds: Vec<Bind>,
    keys: Vec<String>,
}

impl MediaKeys {
    /// Reemplaza (o agrega) el bind de `key` con `cmd`.
    fn set(binds: &mut Vec<Bind>, key: &str, cmd: &str) {
        if let Some(b) = binds.iter_mut().find(|b| b.key == key) {
            b.cmd = cmd.to_string();
        } else {
            binds.push(Bind { key: key.to_string(), cmd: cmd.to_string() });
        }
    }

    /// Defaults + overrides de la config. Una línea `<tecla> <cmd…>` agrega o
    /// reemplaza; una línea con sólo la tecla la borra.
    fn build(config: &str) -> Vec<Bind> {
        let mut binds: Vec<Bind> = DEFAULTS
            .iter()
            .map(|(k, c)| Bind { key: k.to_string(), cmd: c.to_string() })
            .collect();
        for line in config.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line.splitn(2, char::is_whitespace);
            let Some(key) = it.next() else { continue };
            match it.next().map(str::trim).filter(|s| !s.is_empty()) {
                Some(cmd) => Self::set(&mut binds, key, cmd),
                None => binds.retain(|b| b.key != key), // sólo la tecla → borra
            }
        }
        binds
    }
}

impl ReactorPlugin for MediaKeys {
    fn configure(&mut self, config: &str) {
        self.binds = Self::build(config);
        self.keys = self.binds.iter().map(|b| b.key.clone()).collect();
    }

    fn on_event(&mut self, event: BodyEvent, ctx: &mut Ctx) {
        // `configure` siempre corre antes del primer evento (el SDK la llama al
        // construir, con la config del manifest aunque esté vacía), así que
        // `self.binds` ya trae los defaults + overrides acá.
        ctx.grab_keys(&self.keys);
        let BodyEvent::Keybind(combo) = event else {
            return;
        };
        if let Some(b) = self.binds.iter().find(|b| b.key == combo) {
            ctx.spawn(&b.cmd);
        }
    }
}

export_reactor_plugin!(MediaKeys::default());
