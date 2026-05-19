//! `nahual_image_viewer` — visor de imágenes.
//!
//! Suscribe al `AppBus` y, en cada `EntitySelected` cuyo provider sea
//! `local_fs` y la extensión sugiera imagen (jpg, png, webp, gif), pasa el
//! path a `gpui::img(...)` que se encarga del decode + cache. Para otros
//! providers o extensiones desconocidas, muestra un mensaje neutro sin
//! intentar render (evita binarios random pasando por el decoder).
//!
//! Detección por extensión: lista de extensiones soportadas en
//! [`is_image_path`]. Para discriminar por mime real (sin importar la
//! extensión) habría que invocar `image::guess_format` en un task —
//! valdrá la pena cuando carguemos imágenes desde SQLite blobs.

use std::path::Path;

use gpui::{
    Context, Entity, IntoElement, Render, SharedString, Window, div, img, prelude::*, px,
};

use nahual_bus::{AppBus, AppEvent};
use nahual_theme::Theme;

const FS_PROVIDER: &str = "local_fs";

pub struct ImageViewer {
    /// Path actualmente mostrado (si lo hay).
    current_path: Option<String>,
    /// Mensaje a mostrar cuando no se puede renderear (extensión no
    /// reconocida, provider sin soporte, etc.).
    notice: Option<SharedString>,
}

impl ImageViewer {
    pub fn new(bus: Entity<AppBus>, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.subscribe(&bus, |this: &mut ImageViewer, _, ev, cx| {
            this.on_app_event(ev, cx);
        })
        .detach();
        Self {
            current_path: None,
            notice: None,
        }
    }

    fn on_app_event(&mut self, event: &AppEvent, cx: &mut Context<Self>) {
        let (provider, id) = match event {
            AppEvent::EntitySelected { provider, id, .. }
            | AppEvent::EntityOpened { provider, id, .. } => (provider, id),
        };

        if provider != FS_PROVIDER {
            self.current_path = None;
            self.notice = Some(
                format!("provider '{}' no soportado por ImageViewer", provider).into(),
            );
            cx.notify();
            return;
        }

        if !is_image_path(id) {
            self.current_path = None;
            self.notice = Some("(no es una imagen reconocible)".into());
            cx.notify();
            return;
        }

        self.current_path = Some(id.clone());
        self.notice = None;
        cx.notify();
    }
}

fn is_image_path(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico" | "tiff" | "tif"
    )
}

impl Render for ImageViewer {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();

        let header_text = match (&self.current_path, &self.notice) {
            (Some(p), _) => format!("[image] {}", p),
            (None, Some(n)) => n.to_string(),
            (None, None) => "(ninguna imagen seleccionada)".to_string(),
        };

        let body: gpui::AnyElement = match (&self.current_path, &self.notice) {
            (Some(path), _) => {
                let path_buf = std::path::PathBuf::from(path);
                div()
                    .flex_grow()
                    .min_h(px(0.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(12.0))
                    .child(img(path_buf).max_w_full().max_h_full())
                    .into_any_element()
            }
            (None, Some(n)) => div()
                .flex_grow()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.fg_muted)
                .text_size(px(11.0))
                .child(n.clone())
                .into_any_element(),
            (None, None) => div()
                .flex_grow()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.fg_muted)
                .text_size(px(11.0))
                .child("doble click sobre una imagen en el FileExplorer.")
                .into_any_element(),
        };

        div()
            .size_full()
            .bg(theme.bg_panel.clone())
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(28.0))
                    .px(px(10.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .items_center()
                    .text_size(px(11.0))
                    .text_color(theme.fg_muted)
                    .child(SharedString::from(header_text)),
            )
            .child(body)
    }
}
