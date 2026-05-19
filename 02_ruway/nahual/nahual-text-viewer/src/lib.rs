//! `nahual_text_viewer` — visor de texto plano.
//!
//! Suscribe al `AppBus` y, en cada `EntitySelected` / `EntityOpened`,
//! decide si el `provider` corresponde a uno que sabe leer (por ahora
//! `local_fs` y `sqlite_db`); si sí, dispara `cx.spawn` con el provider
//! correspondiente para traer el contenido. Mientras carga muestra
//! "(cargando…)"; al terminar lo pinta como texto con saltos de línea
//! preservados.
//!
//! Si el contenido no es válido UTF-8 (binario), muestra los primeros
//! N bytes en hex — útil para preview no ciego sin pretender ser un
//! editor de binarios.

use std::sync::Arc;

use gpui::{
    Context, Entity, IntoElement, Render, SharedString, Window, div, prelude::*, px,
};

use nahual_bus::{AppBus, AppEvent};
use nahual_core::DataProvider;
use nahual_provider_fs::{FileDataProvider, PROVIDER_ID as FS_PROVIDER_ID};
use nahual_provider_sqlite::{PROVIDER_ID as SQL_PROVIDER_ID, SqliteDataProvider};
use nahual_theme::Theme;

const PREVIEW_HEX_BYTES: usize = 256;
const MAX_TEXT_BYTES: usize = 256 * 1024;

pub struct TextViewer {
    /// Última entidad mostrada. `None` ⇒ pantalla en estado "vacío".
    current: Option<CurrentEntity>,
    /// Contenido renderizado. Si está cargando se muestra el estado en
    /// `current`.
    content: Content,
    /// Generación monotónica — al cambiar `current` la incrementamos para
    /// descartar resultados de loads previos que vuelvan tarde.
    generation: u64,
}

#[derive(Clone, Debug)]
struct CurrentEntity {
    provider: String,
    provider_path: Option<String>,
    id: String,
    loading: bool,
}

#[derive(Clone)]
enum Content {
    Empty,
    Loading,
    Text(SharedString),
    HexPreview(SharedString),
    Error(SharedString),
    Unsupported(SharedString),
}

impl TextViewer {
    pub fn new(bus: Entity<AppBus>, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();

        cx.subscribe(&bus, |this: &mut TextViewer, _, ev, cx| {
            this.on_app_event(ev, cx);
        })
        .detach();

        Self {
            current: None,
            content: Content::Empty,
            generation: 0,
        }
    }

    fn on_app_event(&mut self, event: &AppEvent, cx: &mut Context<Self>) {
        let (provider, provider_path, id) = match event {
            AppEvent::EntitySelected { provider, provider_path, id }
            | AppEvent::EntityOpened { provider, provider_path, id } => {
                (provider.clone(), provider_path.clone(), id.clone())
            }
        };

        // Comparar con el actual para evitar reload de lo mismo.
        if let Some(cur) = &self.current {
            if cur.provider == provider && cur.id == id && cur.provider_path == provider_path {
                return;
            }
        }

        self.generation = self.generation.wrapping_add(1);
        let gen = self.generation;
        self.current = Some(CurrentEntity {
            provider: provider.clone(),
            provider_path: provider_path.clone(),
            id: id.clone(),
            loading: true,
        });
        self.content = Content::Loading;
        cx.notify();

        // Dispatch por provider.
        if provider == FS_PROVIDER_ID {
            self.spawn_load_fs(id, gen, cx);
        } else if provider == SQL_PROVIDER_ID {
            self.spawn_load_sqlite(provider_path, id, gen, cx);
        } else {
            self.content = Content::Unsupported(
                format!("provider '{}' no soportado por TextViewer", provider).into(),
            );
            if let Some(cur) = &mut self.current {
                cur.loading = false;
            }
            cx.notify();
        }
    }

    fn spawn_load_fs(&self, path: String, gen: u64, cx: &mut Context<Self>) {
        let provider = Arc::new(FileDataProvider::new());
        cx.spawn(async move |this, cx| {
            let result = provider.get_data(&path).await;
            let _ = this.update(cx, |this, cx| this.on_loaded(gen, result, cx));
        })
        .detach();
    }

    fn spawn_load_sqlite(
        &self,
        provider_path: Option<String>,
        id: String,
        gen: u64,
        cx: &mut Context<Self>,
    ) {
        let db_path = provider_path.unwrap_or_else(|| "nahual.db".to_string());
        cx.spawn(async move |this, cx| {
            // El SqliteDataProvider abre la DB en su constructor — si
            // falla, reportamos error y salimos.
            let provider = match SqliteDataProvider::new(&db_path) {
                Ok(p) => p,
                Err(e) => {
                    let _ = this.update(cx, |this, cx| {
                        this.on_loaded(
                            gen,
                            Err(format!("abriendo {}: {}", db_path, e)),
                            cx,
                        )
                    });
                    return;
                }
            };
            let result = provider.get_data(&id).await;
            let _ = this.update(cx, |this, cx| this.on_loaded(gen, result, cx));
        })
        .detach();
    }

    fn on_loaded(
        &mut self,
        gen: u64,
        result: Result<Vec<u8>, String>,
        cx: &mut Context<Self>,
    ) {
        // Si el usuario cambió de selección antes de que volviera el load,
        // descartamos este resultado.
        if gen != self.generation {
            return;
        }

        if let Some(cur) = &mut self.current {
            cur.loading = false;
        }

        self.content = match result {
            Ok(bytes) => bytes_to_content(&bytes),
            Err(e) => Content::Error(format!("error: {}", e).into()),
        };
        cx.notify();
    }
}

fn bytes_to_content(bytes: &[u8]) -> Content {
    if bytes.is_empty() {
        return Content::Text("(vacío)".into());
    }
    let truncated = bytes.len() > MAX_TEXT_BYTES;
    let slice = if truncated { &bytes[..MAX_TEXT_BYTES] } else { bytes };
    match std::str::from_utf8(slice) {
        Ok(s) => {
            let mut out = s.to_string();
            if truncated {
                out.push_str("\n…(truncado)…");
            }
            Content::Text(out.into())
        }
        Err(_) => {
            // No es UTF-8: mostramos hex preview de los primeros bytes.
            let n = bytes.len().min(PREVIEW_HEX_BYTES);
            let mut hex = String::with_capacity(n * 3);
            for (i, b) in bytes[..n].iter().enumerate() {
                if i > 0 && i % 16 == 0 {
                    hex.push('\n');
                }
                hex.push_str(&format!("{:02x} ", b));
            }
            if bytes.len() > n {
                hex.push_str(&format!("\n…({} bytes más)", bytes.len() - n));
            }
            Content::HexPreview(hex.into())
        }
    }
}

impl Render for TextViewer {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();

        let header_text = match &self.current {
            None => "(ningún archivo seleccionado)".to_string(),
            Some(cur) => {
                let suffix = if cur.loading { " ⏳" } else { "" };
                format!("[{}] {}{}", cur.provider, cur.id, suffix)
            }
        };

        let body: gpui::AnyElement = match &self.content {
            Content::Empty => div()
                .text_color(theme.fg_muted)
                .text_size(px(11.0))
                .child("seleccioná un archivo en el FileExplorer o una entry en el DatabaseExplorer.")
                .into_any_element(),
            Content::Loading => div()
                .text_color(theme.fg_muted)
                .text_size(px(11.0))
                .child("(cargando…)")
                .into_any_element(),
            Content::Text(s) => div()
                .text_color(theme.fg_text)
                .text_size(px(12.0))
                .font_family("monospace")
                .child(s.clone())
                .into_any_element(),
            Content::HexPreview(s) => div()
                .text_color(theme.fg_muted)
                .text_size(px(11.0))
                .font_family("monospace")
                .child(s.clone())
                .into_any_element(),
            Content::Error(s) => div()
                .text_color(theme.accent_strong)
                .text_size(px(11.0))
                .child(s.clone())
                .into_any_element(),
            Content::Unsupported(s) => div()
                .text_color(theme.fg_muted)
                .text_size(px(11.0))
                .child(s.clone())
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
            .child(
                div()
                    .id("text-viewer-body")
                    .flex_grow()
                    .min_h(px(0.0))
                    .overflow_scroll()
                    .p(px(12.0))
                    .child(body),
            )
    }
}
