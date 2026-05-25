//! Yahweh — bootstrap GPUI.
//!
//! Fase 6: además del LayoutModel, la Shell crea un `AppBus` (Entity) y se
//! lo pasa al LayoutHost. El bus circula a viewers (TextViewer,
//! ImageViewer) que se subscriben directo, y el LayoutHost forwardea los
//! eventos tipados de los explorers (FileExplorer, DatabaseExplorer)
//! traducidos a AppEvent.

mod brahman_client;
mod hot_reload;
mod layout_host;
mod layout_model;
mod managed_tree;
mod persister;
mod status_panel;

use gpui::{App, Application, Bounds, WindowBounds, WindowOptions, prelude::*, px, size};

use nahual_bus::AppBus;
use nahual_core::LayerConfig;
use nahual_theme::Theme;

use crate::layout_host::LayoutHost;
use crate::layout_model::LayoutModel;
use crate::persister::Persister;

const LAYOUT_PATH: &str = "layout.json";

fn main() {
    // Sidecar brahman: nahual se presenta al Init antes de levantar GPUI.
    // No bloquea: si el Init no está, el thread loggea y termina.
    brahman_client::spawn();

    Application::new().run(|cx: &mut App| {
        Theme::install_default(cx);

        let config = LayerConfig::load_or_default(LAYOUT_PATH);
        let bounds = Bounds::centered(None, size(px(1300.), px(800.)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_w, cx| {
                let model = cx.new(|_| LayoutModel::new(config.clone()));
                let bus = cx.new(|_| AppBus);
                let persister = cx.new(|cx| {
                    Persister::new(LAYOUT_PATH.into(), model.clone(), cx)
                });
                // Hot-reload: notify watcher en el dir del JSON. El
                // watcher debe mantenerse vivo (drop ⇒ stop), así que lo
                // movemos a una static atómica vía Box::leak.
                match hot_reload::spawn_watch(LAYOUT_PATH.into(), model.clone(), cx) {
                    Ok(watcher) => {
                        Box::leak(Box::new(watcher));
                    }
                    Err(e) => {
                        eprintln!("[hot_reload] no se pudo iniciar watcher: {}", e);
                    }
                }
                cx.new(|cx| LayoutHost::new(model, bus, persister, cx))
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
