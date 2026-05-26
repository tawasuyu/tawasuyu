//! `nakui-explorer-llimphi` — panel Llimphi que renderea el event log de
//! un repo Nakui: timeline de seeds + morphisms con sus parámetros y
//! breakdown por entity type.
//!
//! ## Diseño
//!
//! Standalone, lee un archivo `.jsonl` (format append-only del
//! `nakui_core::event_log::EventLog`). Refresh por polling cada 2 s vía
//! `Handle::spawn_periodic` para detectar nuevos eventos appended
//! (típico de un nakui ERP en producción que va escribiendo). Sin
//! discovery dinámico vía broker brahman porque nakui hoy es
//! CLI/library/demos, no daemon — cuando se daemonice, sustituir el
//! lector de archivo por un sidecar consumer.
//!
//! ## Uso
//!
//! ```sh
//! # Path explícito:
//! NAKUI_EVENT_LOG=/tmp/nakui-demo.jsonl cargo run -p nakui-explorer-llimphi
//!
//! # Default si la env no está: ./nakui.jsonl en pwd.
//! cargo run -p nakui-explorer-llimphi
//! ```

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_card::{card_view, CardOptions, CardPalette};

use nahual_meta_runtime::format::{preview_value, short_hash, short_uuid};
use nakui_core::event_log::{EventLog, LogEntry};

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const MAX_VISIBLE: usize = 80;
const ROW_GAP: f32 = 6.0;
const ACCENT_SEED: Color = Color::from_rgba8(0x88, 0xc0, 0xd0, 0xff);
const ACCENT_MORPHISM: Color = Color::from_rgba8(0xa3, 0xbe, 0x8c, 0xff);

#[derive(Clone)]
enum Msg {
    Reload,
}

struct Model {
    log_path: PathBuf,
    /// Compartido con el callback periódico que reescribe los entries
    /// fuera del lock del Model. `Msg::Reload` es la señal de "una
    /// pasada ocurrió, leé la versión nueva".
    shared: Arc<Mutex<SharedState>>,
}

struct SharedState {
    entries: Vec<LogEntry>,
    error: Option<String>,
    last_load_ms: u64,
}

struct Explorer;

impl App for Explorer {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Nakui — Event Log"
    }

    fn initial_size() -> (u32, u32) {
        (900, 640)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let log_path = std::env::var("NAKUI_EVENT_LOG")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("nakui.jsonl"));

        let shared = Arc::new(Mutex::new(SharedState {
            entries: Vec::new(),
            error: None,
            last_load_ms: 0,
        }));

        // Primera lectura síncrona para que la primera frame ya tenga
        // contenido sin esperar 2 s.
        reload_into(&log_path, &shared);

        let path_for_loop = log_path.clone();
        let shared_for_loop = shared.clone();
        handle.spawn_periodic(REFRESH_INTERVAL, move || {
            reload_into(&path_for_loop, &shared_for_loop);
            Msg::Reload
        });

        Model { log_path, shared }
    }

    fn update(model: Model, _: Msg, _: &Handle<Msg>) -> Model {
        // El sampler ya escribió en `shared` antes de despachar. El
        // update sólo necesita disparar el re-render — el `view` lee
        // del `shared` lockeando.
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let snapshot = model.shared.lock().unwrap();
        let entries = &snapshot.entries;

        let (seed_count, morphism_count, top_breakdown) = breakdown(entries);

        let header_text = rimay_localize::t_args(
            "nakui-explorer-header",
            &[
                ("path", model.log_path.display().to_string().into()),
                ("entries", entries.len().to_string().into()),
                ("seeds", seed_count.to_string().into()),
                ("morphisms", morphism_count.to_string().into()),
                ("ms", snapshot.last_load_ms.to_string().into()),
            ],
        );
        let header = app_header::<Msg>(
            header_text,
            Vec::new(),
            &AppHeaderPalette::from_theme(&theme),
        );

        let mut chrome: Vec<View<Msg>> = vec![header];

        let breakdown_line = if top_breakdown.is_empty() {
            None
        } else {
            let parts: Vec<String> = top_breakdown
                .iter()
                .take(5)
                .map(|(k, v)| format!("{k}({v})"))
                .collect();
            Some(rimay_localize::t_args(
                "nakui-explorer-breakdown",
                &[("parts", parts.join(", ").into())],
            ))
        };
        if let Some(line) = breakdown_line {
            chrome.push(
                View::new(Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(22.0_f32),
                    },
                    padding: Rect {
                        left: length(16.0_f32),
                        right: length(16.0_f32),
                        top: length(4.0_f32),
                        bottom: length(4.0_f32),
                    },
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                })
                .fill(theme.bg_panel_alt)
                .text_aligned(line, 11.0, theme.fg_muted, Alignment::Start),
            );
        }

        if let Some(err) = &snapshot.error {
            chrome.push(banner_view::<Msg>(BannerKind::Error, err.clone()));
        }

        // Renderea las últimas N entries (la timeline crece hacia abajo
        // en append-order; mostramos las más recientes primero para que
        // el usuario vea actividad reciente sin scroll).
        let card_palette = CardPalette::from_theme(&theme);
        let cards: Vec<View<Msg>> = entries
            .iter()
            .rev()
            .take(MAX_VISIBLE)
            .map(|e| entry_card(e, &theme, &card_palette))
            .collect();

        let body = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(8.0_f32),
                bottom: length(8.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(ROW_GAP),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .clip(true)
        .children(cards);

        chrome.push(body);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(chrome)
    }
}

fn entry_card(entry: &LogEntry, theme: &Theme, palette: &CardPalette) -> View<Msg> {
    match entry {
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

            let head = text_row(
                format!(
                    "[#{seq} seed]  {entity}  ·  id={}",
                    short_uuid(id)
                ),
                12.0,
                theme.fg_text,
            );
            let preview = text_row(data_preview, 11.0, theme.fg_muted);
            let schema = text_row(schema_label, 10.0, theme.fg_muted);

            card_view::<Msg>(
                vec![head, preview, schema],
                CardOptions {
                    accent: Some(ACCENT_SEED),
                    ..Default::default()
                },
                palette,
            )
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

            let head = text_row(
                format!("[#{seq} morph]  {morphism}  ·  {ops_line}"),
                12.0,
                theme.fg_text,
            );
            let mut children = vec![head];
            if !inputs_line.is_empty() {
                children.push(text_row(inputs_line, 11.0, theme.fg_muted));
            }
            if !params_line.is_empty() {
                children.push(text_row(
                    format!("params: {params_line}"),
                    11.0,
                    theme.fg_muted,
                ));
            }
            children.push(text_row(schema_label, 10.0, theme.fg_muted));

            card_view::<Msg>(
                children,
                CardOptions {
                    accent: Some(ACCENT_MORPHISM),
                    ..Default::default()
                },
                palette,
            )
        }
    }
}

fn text_row(text: String, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(size + 6.0),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text, size, color, Alignment::Start)
}

fn reload_into(path: &Path, shared: &Arc<Mutex<SharedState>>) {
    let started = Instant::now();
    let result = load_log(path);
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let mut guard = shared.lock().unwrap();
    match result {
        Ok(entries) => {
            guard.entries = entries;
            guard.error = None;
        }
        Err(e) => {
            guard.error = Some(format!("no pude leer {}: {}", path.display(), e));
        }
    }
    guard.last_load_ms = elapsed_ms;
}

fn load_log(path: &Path) -> Result<Vec<LogEntry>, String> {
    let log = EventLog::open(path).map_err(|e| format!("open: {e}"))?;
    log.entries().map_err(|e| format!("read: {e}"))
}

fn breakdown(entries: &[LogEntry]) -> (usize, usize, Vec<(String, usize)>) {
    let mut seeds = 0;
    let mut morphisms = 0;
    let mut entity_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for e in entries {
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

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Explorer>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_sample_log() -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
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
        let (seeds, morphisms, ranked) = breakdown(&entries);
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
        let path = std::env::temp_dir().join("nakui-explorer-llimphi-missing-test.jsonl");
        let _ = std::fs::remove_file(&path);
        let result = load_log(&path).expect("missing path is OK per EventLog::open contract");
        assert!(result.is_empty());
    }
}
