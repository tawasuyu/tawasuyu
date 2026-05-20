//! ente-localed-compat: shim de `org.freedesktop.locale1`.
//!
//! GNOME settings panel "Region & Language" llama aquí. Properties leen
//! /etc/locale.conf y /etc/vconsole.conf; setters log + forward.

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::Capability;
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
            Ok(c) => c.lines()
                .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
                .map(|s| s.trim().to_string())
                .collect(),
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
        // Validar formato KEY=value en cada entry.
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
        info!(%keymap, %keymap_toggle, "SetVConsoleKeymap (stub)");
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
        info!(%layout, %model, %variant, %options, "SetX11Keyboard (stub)");
        Ok(())
    }
}

fn atomic_write(path: &str, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let p = std::path::Path::new(path);
    if let Some(parent) = p.parent() { let _ = std::fs::create_dir_all(parent); }
    let tmp = p.with_extension("tmp");
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true).write(true).truncate(true)
            .mode(0o644)
            .open(&tmp)?;
        f.write_all(content)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, p)?;
    Ok(())
}

fn read_kv(path: &str, key: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&format!("Option \"{key}\"")) || trimmed.starts_with(key) {
            // Best-effort parse: tomar lo que está entre comillas.
            if let Some(start) = trimmed.find('"') {
                let rest = &trimmed[start + 1..];
                if let Some(end) = rest.find('"') {
                    return Some(rest[..end].to_string());
                }
            }
        }
    }
    None
}

fn read_vconsole(key: &str) -> Option<String> {
    let content = std::fs::read_to_string("/etc/vconsole.conf").ok()?;
    for line in content.lines() {
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
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
