//! `nakui-explorer` — panel GPUI que renderea el event log de un
//! repo Nakui: timeline de seeds + morphisms con sus parámetros y
//! breakdown por entity type.
//!
//! ## Diseño
//!
//! Standalone, lee un archivo `.jsonl` (formato append-only del
//! `nakui_core::event_log::EventLog`). Refresh por polling cada 2s
//! para detectar nuevos eventos appended (típico de un nakui ERP en
//! producción que va escribiendo). Sin discovery dinámico vía broker
//! brahman porque nakui hoy es CLI/library/demos, no daemon — cuando
//! se daemonice, sustituir el lector de archivo por un sidecar
//! consumer (mismo patrón que `nouser-explorer`).
//!
//! ## Uso
//!
//! ```sh
//! # Path explícito:
//! NAKUI_EVENT_LOG=/tmp/nakui-demo.jsonl cargo run -p nakui-explorer
//!
//! # Default si la env no está: ./nakui.jsonl en pwd.
//! cargo run -p nakui-explorer
//! ```

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    div, prelude::*, px, rgb, Context, IntoElement, Render, SharedString, Window,
};
use nakui_core::event_log::{EventLog, LogEntry};
use yahweh_launcher::launch_app;
use yahweh_meta_runtime::{preview_value, short_hash, short_uuid};
use yahweh_theme::Theme;
use yahweh_widget_app_header::app_header;
use yahweh_widget_banner::{banner_themed, Banner};
use yahweh_widget_card::card_themed;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

fn main() {
    launch_app("Nakui — Event Log", (900., 640.), Explorer::new);
}

/// Estado de la vista. `entries` se reescribe en cada tick (el log
/// es append-only así que reload completo es seguro y barato hasta
/// decenas de miles de entries; optimizar a delta cuando duela).
struct Explorer {
    log_path: PathBuf,
    entries: Vec<LogEntry>,
    error: Option<SharedString>,
    last_load_ms: u64,
}

impl Explorer {
    fn new(cx: &mut Context<Self>) -> Self {
        let log_path = std::env::var("NAKUI_EVENT_LOG")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("nakui.jsonl"));

        // Loop de refresh.
        let path_for_loop = log_path.clone();
        cx.spawn(async move |this, cx| {
            let timer = cx.background_executor().clone();
            loop {
                let started = std::time::Instant::now();
                let result = load_log(&path_for_loop);
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let _ = this.update(cx, |me, cx| {
                    match result {
                        Ok(entries) => {
                            me.entries = entries;
                            me.error = None;
                        }
                        Err(e) => {
                            me.error = Some(SharedString::from(format!(
                                "no pude leer {}: {}",
                                me.log_path.display(),
                                e
                            )));
                        }
                    }
                    me.last_load_ms = elapsed_ms;
                    cx.notify();
                });
                timer.timer(REFRESH_INTERVAL).await;
            }
        })
        .detach();

        Self {
            log_path,
            entries: Vec::new(),
            error: None,
            last_load_ms: 0,
        }
    }

    fn breakdown(&self) -> (usize, usize, Vec<(String, usize)>) {
        let mut seeds = 0;
        let mut morphisms = 0;
        let mut entity_counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for e in &self.entries {
            match e {
                LogEntry::Seed { entity, .. } => {
                    seeds += 1;
                    *entity_counts.entry(entity.clone()).or_default() += 1;
                }
                LogEntry::Morphism { morphism, .. } => {
                    morphisms += 1;
                    *entity_counts.entry(format!("→ {}", morphism)).or_default() += 1;
                }
            }
        }
        let mut ranked: Vec<_> = entity_counts.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        (seeds, morphisms, ranked)
    }
}

fn load_log(path: &std::path::Path) -> Result<Vec<LogEntry>, String> {
    let log = EventLog::open(path).map_err(|e| format!("open: {e}"))?;
    log.entries().map_err(|e| format!("read: {e}"))
}

impl Render for Explorer {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Colores cromáticos del chrome del app vienen del Theme
        // global (instalado en main). Los acentos por kind (seed
        // azul, morphism verde) siguen siendo locales: son señales
        // semánticas del log, no del chrome.
        let theme = Theme::global(cx).clone();
        let bg = theme.bg_app.clone();
        let text = theme.fg_text;
        let text_dim = theme.fg_muted;
        let accent_seed = rgb(0x88c0d0);
        let accent_morphism = rgb(0xa3be8c);

        let (seed_count, morphism_count, top_breakdown) = self.breakdown();

        let header_text = format!(
            "Log: {}  ·  {} entries ({} seeds, {} morphisms)  ·  reload {} ms",
            self.log_path.display(),
            self.entries.len(),
            seed_count,
            morphism_count,
            self.last_load_ms,
        );

        // Header standard via widget compartido yahweh-widget-app-header
        // (label flex_grow + theme switcher derecha + bg panel + border
        // bottom + text styling consistente).
        let header = app_header(cx, header_text);

        let breakdown_line = if top_breakdown.is_empty() {
            String::new()
        } else {
            let parts: Vec<String> = top_breakdown
                .iter()
                .take(5)
                .map(|(k, v)| format!("{k}({v})"))
                .collect();
            format!("breakdown: {}", parts.join(", "))
        };

        let breakdown_div = (!breakdown_line.is_empty()).then(|| {
            div()
                .px(px(16.))
                .py(px(6.))
                .bg(theme.bg_panel_alt.clone())
                .text_color(text_dim)
                .text_size(px(11.))
                .child(breakdown_line)
        });

        // Banner de error themed: deriva (bg, fg) del Theme actual
        // según `Banner::Error` + `is_dark`. Padding extra (16/8)
        // del header preservado via overrides del builder.
        let error_banner = self.error.as_ref().map(|e| {
            banner_themed(cx, Banner::Error, e.clone())
                .px(px(16.))
                .py(px(8.))
                .text_size(px(12.))
        });

        // Renderea las últimas N entries (la timeline crece hacia abajo
        // en append-order; mostramos las más recientes primero para
        // que el usuario vea actividad reciente sin scroll).
        const MAX_VISIBLE: usize = 200;
        let visible: Vec<_> = self
            .entries
            .iter()
            .rev()
            .take(MAX_VISIBLE)
            .collect();

        let cards: Vec<gpui::AnyElement> = visible
            .iter()
            .map(|e| match e {
                LogEntry::Seed {
                    seq,
                    entity,
                    id,
                    data,
                    schema_hash,
                } => {
                    let data_preview = preview_value(data, 80);
                    let schema_label = schema_hash
                        .as_ref()
                        .map(|h| format!("schema={}", short_hash(h)))
                        .unwrap_or_else(|| "schema=(legacy)".into());
                    card_themed(cx)
                        .border_l_4()
                        .border_color(accent_seed)
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(8.))
                                .items_center()
                                .child(
                                    div()
                                        .text_color(accent_seed)
                                        .text_size(px(11.))
                                        .child(format!("[#{seq} seed]")),
                                )
                                .child(
                                    div()
                                        .text_color(text)
                                        .text_size(px(13.))
                                        .child(entity.clone()),
                                )
                                .child(
                                    div()
                                        .text_color(text_dim)
                                        .text_size(px(10.))
                                        .child(format!("id={}", short_uuid(id))),
                                ),
                        )
                        .child(
                            div()
                                .text_color(text_dim)
                                .text_size(px(11.))
                                .child(data_preview),
                        )
                        .child(
                            div()
                                .text_color(text_dim)
                                .text_size(px(10.))
                                .child(schema_label),
                        )
                        .into_any_element()
                }
                LogEntry::Morphism {
                    seq,
                    morphism,
                    inputs,
                    params,
                    ops,
                    schema_hash,
                } => {
                    let inputs_line = if inputs.is_empty() {
                        String::new()
                    } else {
                        let parts: Vec<String> = inputs
                            .iter()
                            .map(|(name, id)| format!("{name}={}", short_uuid(id)))
                            .collect();
                        format!("inputs: {}", parts.join(", "))
                    };
                    let params_line = preview_value(params, 80);
                    let ops_line = format!("{} op(s)", ops.len());
                    let schema_label = schema_hash
                        .as_ref()
                        .map(|h| format!("schema={}", short_hash(h)))
                        .unwrap_or_else(|| "schema=(legacy)".into());
                    card_themed(cx)
                        .border_l_4()
                        .border_color(accent_morphism)
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(8.))
                                .items_center()
                                .child(
                                    div()
                                        .text_color(accent_morphism)
                                        .text_size(px(11.))
                                        .child(format!("[#{seq} morph]")),
                                )
                                .child(
                                    div()
                                        .text_color(text)
                                        .text_size(px(13.))
                                        .child(morphism.clone()),
                                )
                                .child(
                                    div()
                                        .text_color(text_dim)
                                        .text_size(px(10.))
                                        .child(ops_line),
                                ),
                        )
                        .when(!inputs_line.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_color(text_dim)
                                    .text_size(px(11.))
                                    .child(inputs_line),
                            )
                        })
                        .when(!params_line.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_color(text_dim)
                                    .text_size(px(11.))
                                    .child(format!("params: {params_line}")),
                            )
                        })
                        .child(
                            div()
                                .text_color(text_dim)
                                .text_size(px(10.))
                                .child(schema_label),
                        )
                        .into_any_element()
                }
            })
            .collect();

        let body = div()
            .flex()
            .flex_col()
            .p(px(12.))
            .overflow_hidden()
            .children(cards);

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(bg)
            .child(header)
            .when_some(breakdown_div, |d, b| d.child(b))
            .when_some(error_banner, |d, b| d.child(b))
            .child(body)
    }
}

// Helpers `short_uuid`, `short_hash`, `preview_value` viven en
// `yahweh_meta_runtime::format`. Se usan acá via el `use` de arriba.

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_sample_log() -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        // 3 seeds + 2 morphisms, formato canónico de event_log.
        let lines = [
            r#"{"kind":"seed","seq":0,"entity":"product","id":"00000000-0000-0000-0000-000000000001","data":{"sku":"A"}}"#,
            r#"{"kind":"seed","seq":1,"entity":"product","id":"00000000-0000-0000-0000-000000000002","data":{"sku":"B"}}"#,
            r#"{"kind":"seed","seq":2,"entity":"customer","id":"00000000-0000-0000-0000-000000000003","data":{"name":"Acme"}}"#,
            r#"{"kind":"morphism","seq":3,"morphism":"sale.create","inputs":{"product":"00000000-0000-0000-0000-000000000001"},"params":{"qty":1},"ops":[]}"#,
            r#"{"kind":"morphism","seq":4,"morphism":"sale.refund","inputs":{},"params":{},"ops":[]}"#,
        ];
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        f.flush().unwrap();
        f
    }

    #[test]
    fn load_log_returns_all_entries_in_order() {
        let f = write_sample_log();
        let entries = load_log(f.path()).expect("load");
        assert_eq!(entries.len(), 5);
        for (i, e) in entries.iter().enumerate() {
            assert_eq!(e.seq(), i as u64, "seqs should be 0..4 contiguous");
        }
    }

    #[test]
    fn breakdown_counts_seeds_morphisms_and_buckets() {
        let f = write_sample_log();
        let entries = load_log(f.path()).unwrap();
        let me = Explorer {
            log_path: f.path().to_path_buf(),
            entries,
            error: None,
            last_load_ms: 0,
        };
        let (seeds, morphisms, ranked) = me.breakdown();
        assert_eq!(seeds, 3);
        assert_eq!(morphisms, 2);
        // Buckets esperados: product (2), customer (1), → sale.create (1),
        // → sale.refund (1).
        assert_eq!(ranked.len(), 4);
        let map: std::collections::BTreeMap<_, _> = ranked.into_iter().collect();
        assert_eq!(map.get("product"), Some(&2));
        assert_eq!(map.get("customer"), Some(&1));
        assert_eq!(map.get("→ sale.create"), Some(&1));
        assert_eq!(map.get("→ sale.refund"), Some(&1));
    }

    #[test]
    fn load_missing_file_yields_empty_not_error() {
        // EventLog::open de un archivo inexistente no falla; entries() devuelve [].
        let path = std::env::temp_dir().join("nakui-explorer-missing-test.jsonl");
        let _ = std::fs::remove_file(&path);
        let result = load_log(&path).expect("missing path is OK per EventLog::open contract");
        assert!(result.is_empty());
    }

    // Tests de `short_uuid` / `short_hash` / `preview_value` viven
    // en `yahweh-meta-runtime::format` tras la migración. Si esos se
    // vuelven a romper, los tests específicos del crate runtime los
    // capturan; acá no duplicamos.
}
