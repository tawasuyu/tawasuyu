//! ente-localed-compat: shim de `org.freedesktop.locale1`.
//!
//! GNOME settings panel "Region & Language" llama aquí. Properties leen
//! /etc/locale.conf y /etc/vconsole.conf; setters log + forward.

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::Capability;
use arje_compat::{atomic_write, conf_entries, merge_kv, parse_kv};
use std::sync::Mutex;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use zbus::{fdo, interface};

const BUS_NAME: &str = "org.freedesktop.locale1";
const OBJ_PATH: &str = "/org/freedesktop/locale1";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    info!("ente-localed-compat: arrancando");
    announce_to_fractal().await;

    let manager = LocaleManager::default();
    let conn_result = zbus::connection::Builder::system()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJ_PATH, manager));
    match conn_result {
        Ok(builder) => match builder.build().await {
            Ok(_conn) => {
                info!(name = BUS_NAME, "name acquired, sirviendo");
                wait_for_term().await
            }
            Err(e) => {
                warn!(?e, "build conn falló — modo idle");
                wait_for_term().await
            }
        },
        Err(e) => {
            warn!(?e, "builder D-Bus falló — modo idle");
            wait_for_term().await
        }
    }
}

#[derive(Default)]
struct LocaleManager {
    transient_locale: Mutex<Option<Vec<String>>>,
}

#[interface(name = "org.freedesktop.locale1")]
impl LocaleManager {
    /// Locale actual como array de "KEY=value" (LANG=en_US.UTF-8, LC_TIME=...).
    /// Default: leer /etc/locale.conf.
    #[zbus(property)]
    async fn locale(&self) -> Vec<String> {
        if let Some(v) = self.transient_locale.lock().unwrap().clone() {
            return v;
        }
        match std::fs::read_to_string("/etc/locale.conf") {
            Ok(c) => conf_entries(&c),
            Err(_) => vec!["LANG=C.UTF-8".into()],
        }
    }

    #[zbus(property)]
    async fn x11layout(&self) -> String {
        read_kv("/etc/X11/xorg.conf.d/00-keyboard.conf", "XkbLayout").unwrap_or_default()
    }

    #[zbus(property)]
    async fn x11model(&self) -> String {
        read_kv("/etc/X11/xorg.conf.d/00-keyboard.conf", "XkbModel").unwrap_or_default()
    }

    #[zbus(property)]
    async fn x11variant(&self) -> String {
        read_kv("/etc/X11/xorg.conf.d/00-keyboard.conf", "XkbVariant").unwrap_or_default()
    }

    #[zbus(property)]
    async fn x11options(&self) -> String {
        read_kv("/etc/X11/xorg.conf.d/00-keyboard.conf", "XkbOptions").unwrap_or_default()
    }

    #[zbus(property)]
    async fn vconsole_keymap(&self) -> String {
        read_vconsole("KEYMAP").unwrap_or_default()
    }

    #[zbus(property)]
    async fn vconsole_keymap_toggle(&self) -> String {
        read_vconsole("KEYMAP_TOGGLE").unwrap_or_default()
    }

    async fn set_locale(&self, locale: Vec<String>, _interactive: bool) -> fdo::Result<()> {
        // Validar format KEY=value en cada entry.
        for entry in &locale {
            if !entry.contains('=') {
                return Err(fdo::Error::InvalidArgs(
                    format!("locale entry inválido (sin '='): {entry}")
                ));
            }
        }
        let content: String = locale.iter()
            .map(|s| format!("{s}\n"))
            .collect();
        atomic_write("/etc/locale.conf", content.as_bytes())
            .map_err(|e| fdo::Error::Failed(format!("write /etc/locale.conf: {e}")))?;
        *self.transient_locale.lock().unwrap() = Some(locale.clone());
        info!(?locale, "SetLocale → /etc/locale.conf");
        Ok(())
    }

    async fn set_vconsole_keymap(
        &self,
        keymap: String,
        keymap_toggle: String,
        _convert: bool,
        _interactive: bool,
    ) -> fdo::Result<()> {
        let existing = std::fs::read_to_string("/etc/vconsole.conf").unwrap_or_default();
        let mut out = merge_kv(&existing, "KEYMAP", &keymap);
        if !keymap_toggle.is_empty() {
            out = merge_kv(&out, "KEYMAP_TOGGLE", &keymap_toggle);
        }
        atomic_write("/etc/vconsole.conf", out.as_bytes())
            .map_err(|e| fdo::Error::Failed(format!("write /etc/vconsole.conf: {e}")))?;
        info!(%keymap, %keymap_toggle, "SetVConsoleKeymap → /etc/vconsole.conf");
        Ok(())
    }

    async fn set_x11_keyboard(
        &self,
        layout: String,
        model: String,
        variant: String,
        options: String,
        _convert: bool,
        _interactive: bool,
    ) -> fdo::Result<()> {
        let conf = format_x11_keyboard_conf(&layout, &model, &variant, &options);
        atomic_write("/etc/X11/xorg.conf.d/00-keyboard.conf", conf.as_bytes())
            .map_err(|e| fdo::Error::Failed(format!("write 00-keyboard.conf: {e}")))?;
        info!(%layout, %model, %variant, %options, "SetX11Keyboard → 00-keyboard.conf");
        Ok(())
    }
}

/// Lee el valor de un `Option "Clave" "valor"` de un snippet
/// `xorg.conf.d` — el valor es la SEGUNDA cadena entre comillas.
fn parse_xorg_option(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with(&format!("Option \"{key}\"")) {
            return t.split('"').nth(3).map(str::to_string);
        }
    }
    None
}

/// Genera el snippet `xorg.conf.d` que fija el teclado X11 — el mismo
/// `InputClass` que escribe systemd-localed. Omite los campos vacíos.
fn format_x11_keyboard_conf(layout: &str, model: &str, variant: &str, options: &str) -> String {
    let mut s = String::from("# Generado por arje-localed-compat\n");
    s.push_str("Section \"InputClass\"\n");
    s.push_str("        Identifier \"system-keyboard\"\n");
    s.push_str("        MatchIsKeyboard \"on\"\n");
    for (opt, val) in [
        ("XkbLayout", layout),
        ("XkbModel", model),
        ("XkbVariant", variant),
        ("XkbOptions", options),
    ] {
        if !val.is_empty() {
            s.push_str(&format!("        Option \"{opt}\" \"{val}\"\n"));
        }
    }
    s.push_str("EndSection\n");
    s
}

fn read_kv(path: &str, key: &str) -> Option<String> {
    parse_xorg_option(&std::fs::read_to_string(path).ok()?, key)
}

fn read_vconsole(key: &str) -> Option<String> {
    parse_kv(&std::fs::read_to_string("/etc/vconsole.conf").ok()?, key)
}

async fn announce_to_fractal() {
    if let Ok(mut client) = BusClient::from_env().await {
        let req = BusRequest::Announce {
            capabilities: vec![Capability::Endpoint {
                interface: arje_card::InterfaceId([0xa2; 16]),
                version: 1,
            }],
        };
        match client.call(req).await {
            Ok(BusResponse::Ok) => info!("Announce → bus interno OK"),
            Ok(other) => warn!(?other, "Announce respuesta inesperada"),
            Err(e) => warn!(?e, "Announce falló"),
        }
    }
}

async fn wait_for_term() -> anyhow::Result<()> {
    let mut term = signal(SignalKind::terminate())?;
    let mut int_ = signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = term.recv() => info!("SIGTERM"),
        _ = int_.recv() => info!("SIGINT"),
    }
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("arje_localed_compat=info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_xorg_option_toma_la_segunda_cadena() {
        let c = "Section \"InputClass\"\n        Option \"XkbLayout\" \"us\"\n        Option \"XkbVariant\" \"intl\"\nEndSection\n";
        assert_eq!(parse_xorg_option(c, "XkbLayout").as_deref(), Some("us"));
        assert_eq!(parse_xorg_option(c, "XkbVariant").as_deref(), Some("intl"));
        assert_eq!(parse_xorg_option(c, "XkbModel"), None);
    }

    #[test]
    fn format_x11_keyboard_conf_omite_los_campos_vacios() {
        let conf = format_x11_keyboard_conf("us", "pc105", "", "");
        assert!(conf.contains("Option \"XkbLayout\" \"us\""));
        assert!(conf.contains("Option \"XkbModel\" \"pc105\""));
        assert!(!conf.contains("XkbVariant"), "el variant vacío se omite");
        assert!(conf.contains("Section \"InputClass\""));
        assert!(conf.contains("EndSection"));
    }
}
