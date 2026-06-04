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
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_motion::{animate, motion, Tween};
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey};
use wawa_config_llimphi::theme_from_wawa;

use app_bus::{AppMenu, Menu, MenuItem};

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
    /// El bus `wawa-config` publicó una versión nueva.
    WawaConfigChanged(Box<wawa_config::WawaConfig>),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Navegación por teclado en el dropdown del menú principal
    /// (`+1` baja, `-1` sube).
    MenuNav(i32),
    /// Ejecuta la fila activa del menú principal (Enter).
    MenuActivate,
    /// Tick de animación del dropdown (sólo re-render).
    MenuTick,
    /// Cicla el tema claro/oscuro.
    CycleTheme,
    /// Selecciona una entrada por índice en la lista RENDERIZADA (más
    /// recientes primero). Resalta y habilita el menú contextual.
    SelectEntry(usize),
    /// Right-click en la raíz → abre el menú contextual anclado en
    /// `(x, y)` de ventana sobre la entrada seleccionada. Sin selección
    /// es no-op.
    ContextMenuOpen(f32, f32),
    /// Fuerza una relectura síncrona del log (Refrescar del menú).
    ForceReload,
}

struct Model {
    log_path: PathBuf,
    /// Compartido con el callback periódico que reescribe los entries
    /// fuera del lock del Model. `Msg::Reload` es la señal de "una
    /// pasada ocurrió, leé la versión nueva".
    shared: Arc<Mutex<SharedState>>,
    theme: Theme,
    /// Suscripción al bus de configuración del SO.
    _wawa_watcher: Option<wawa_config::ConfigWatcher>,
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado).
    menu_open: Option<usize>,
    /// Fila activa (resaltada por teclado) en el dropdown abierto.
    /// `usize::MAX` = ninguna.
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal.
    menu_anim: Tween<f32>,
    /// Entrada seleccionada — índice en la lista RENDERIZADA (rev, las
    /// más recientes primero, capada a `MAX_VISIBLE`). El explorer es de
    /// sólo lectura; la selección sólo resalta y habilita el contextual.
    selected: Option<usize>,
    /// Menú contextual sobre una entrada: `(idx_render, x, y)` ancla en
    /// ventana. `None` cerrado.
    context_menu: Option<(usize, f32, f32)>,
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

        // Bus de configuración del SO: theme + locale en vivo.
        let cfg = wawa_config::WawaConfig::load();
        let theme = theme_from_wawa(&cfg, &Theme::dark());
        let _ = rimay_localize::set_locale(&cfg.lang);
        let handle_clone = handle.clone();
        let watcher = wawa_config::ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::WawaConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("nakui-explorer · wawa-config watcher: {e}"))
        .ok();

        Model {
            log_path,
            shared,
            theme,
            _wawa_watcher: watcher,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            selected: None,
            context_menu: None,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Reload => {
                // El sampler ya escribió en `shared` antes de
                // despachar. El update sólo dispara el re-render — el
                // `view` lee del `shared` lockeando. Si la selección
                // quedó fuera de rango tras el refresh, la descartamos.
                let count = visible_count(&m.shared);
                if m.selected.map(|i| i >= count).unwrap_or(false) {
                    m.selected = None;
                    m.context_menu = None;
                }
            }
            Msg::WawaConfigChanged(cfg) => {
                m.theme = theme_from_wawa(&cfg, &m.theme);
                if cfg.lang != rimay_localize::current_locale() {
                    let _ = rimay_localize::set_locale(&cfg.lang);
                }
            }
            Msg::MenuOpen(which) => {
                m.menu_open = which;
                m.menu_active = usize::MAX;
                // Abrir un menú raíz cierra cualquier contextual.
                m.context_menu = None;
                // Animación de aparición/swap del dropdown.
                if which.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu();
                    m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu();
                    if let Some(cmd) = menubar_command_at(&menu, mi, m.menu_active) {
                        m.menu_open = None;
                        m.menu_active = usize::MAX;
                        return handle_menu_command(m, &cmd, handle);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::CloseMenus => {
                m.menu_open = None;
                m.menu_active = usize::MAX;
                m.context_menu = None;
            }
            Msg::MenuCommand(cmd) => {
                m.menu_open = None;
                m.menu_active = usize::MAX;
                return handle_menu_command(m, &cmd, handle);
            }
            Msg::CycleTheme => {
                m.theme = Theme::next_after(m.theme.name);
            }
            Msg::ForceReload => {
                reload_into(&m.log_path, &m.shared);
                handle.dispatch(Msg::Reload);
            }
            Msg::SelectEntry(i) => {
                m.selected = Some(i);
                m.context_menu = None;
            }
            Msg::ContextMenuOpen(x, y) => {
                let count = visible_count(&m.shared);
                if let Some(i) = m.selected.filter(|i| *i < count) {
                    m.menu_open = None;
                    m.context_menu = Some((i, x, y));
                }
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
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

        let mut chrome: Vec<View<Msg>> = vec![menubar, header];

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
            .enumerate()
            .map(|(i, e)| {
                let card = entry_card(e, &theme, &card_palette).on_click(Msg::SelectEntry(i));
                if model.selected == Some(i) {
                    // Resalte sutil de la entrada seleccionada.
                    card.fill(theme.bg_selected)
                } else {
                    card
                }
            })
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
        // Right-click en la raíz (origen 0,0 ⇒ local == ventana) abre el
        // menú contextual sobre la entrada seleccionada.
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(chrome)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // El menú contextual sobre la entrada tiene prioridad si está
        // abierto.
        if let Some((idx, x, y)) = model.context_menu {
            let t = rimay_localize::t;
            let header = {
                let snap = model.shared.lock().unwrap();
                // `idx` es índice en la lista renderizada (rev). Mapear al
                // entry real para el header del menú.
                snap.entries
                    .iter()
                    .rev()
                    .nth(idx)
                    .map(entry_label)
                    .unwrap_or_else(|| t("nakui-explorer-ctx-entry-fallback"))
            };
            let viewport = viewport_of(model);
            // Acciones reales: el explorer es de sólo lectura, no
            // inventamos edición. Seleccionar/refrescar son las únicas
            // acciones reales que existen.
            let items = vec![
                ContextMenuItem::action(t("nakui-explorer-ctx-view-detail")),
                ContextMenuItem::action(t("nakui-explorer-ctx-refresh-log")),
            ];
            let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> =
                Arc::new(move |i: usize| match i {
                    0 => Msg::SelectEntry(idx),
                    _ => Msg::ForceReload,
                });
            return Some(context_menu_view(ContextMenuSpec {
                anchor: (x, y),
                viewport,
                header: Some(header),
                items,
                active: usize::MAX,
                on_pick,
                on_dismiss: Msg::CloseMenus,
                palette: ContextMenuPalette::from_theme(&model.theme),
            }));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu();
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &model.theme),
            model.menu_active,
            model.menu_anim.value(),
        )
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // Menú principal abierto: ←/→ cambian de menú raíz (con wrap),
        // ↑/↓ mueven la fila activa, Enter ejecuta, Esc cierra.
        if let Some(mi) = model.menu_open {
            let n = app_menu().menus.len().max(1);
            return Some(match &event.key {
                Key::Named(NamedKey::Escape) => Msg::CloseMenus,
                Key::Named(NamedKey::ArrowLeft) => Msg::MenuOpen(Some((mi + n - 1) % n)),
                Key::Named(NamedKey::ArrowRight) => Msg::MenuOpen(Some((mi + 1) % n)),
                Key::Named(NamedKey::ArrowDown) => Msg::MenuNav(1),
                Key::Named(NamedKey::ArrowUp) => Msg::MenuNav(-1),
                Key::Named(NamedKey::Enter) => Msg::MenuActivate,
                _ => return None,
            });
        }
        None
    }
}

/// Viewport para clampear overlays: el explorer no trackea el tamaño de
/// ventana, así que usamos `initial_size()`.
fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = Explorer::initial_size();
    (w as f32, h as f32)
}

/// Cuántas entradas se renderizan (rev, capadas a `MAX_VISIBLE`). Define
/// el rango válido de la selección.
fn visible_count(shared: &Arc<Mutex<SharedState>>) -> usize {
    shared.lock().unwrap().entries.len().min(MAX_VISIBLE)
}

/// Etiqueta corta de un entry para el header del menú contextual.
fn entry_label(entry: &LogEntry) -> String {
    match entry {
        LogEntry::Seed { seq, entity, .. } => format!("#{seq} seed · {entity}"),
        LogEntry::Morphism { seq, morphism, .. } => format!("#{seq} morph · {morphism}"),
    }
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(menu: &'a AppMenu, model: &Model, theme: &'a Theme) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: viewport_of(model),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// El menú principal del explorer. Archivo / Ver / Idioma / Ayuda — sólo
/// comandos que mapean a acciones reales (refrescar log, tema, salir). Sin
/// "Editar": el explorer no tiene campos de texto editables.
fn app_menu() -> AppMenu {
    let t = rimay_localize::t;

    // Menú de idioma: autónimos sin traducir (convención del SO). El item
    // activo lleva ✔. El comando `lang.<code>` lo resuelve
    // `handle_menu_command` → set_locale + persiste en wawa-config.
    let cur = rimay_localize::current_locale();
    let lang_item = |label: &str, code: &str| {
        let mut it = MenuItem::new(label, format!("lang.{code}"));
        if cur == code {
            it = it.icon("\u{2714}");
        }
        it
    };

    AppMenu::new()
        .menu(
            Menu::new(t("file"))
                .item(MenuItem::new(t("nakui-explorer-menu-refresh-log"), "file.refresh").shortcut("Ctrl+R"))
                .item(MenuItem::new(t("exit"), "file.quit").shortcut("Ctrl+Q").separated()),
        )
        .menu(Menu::new(t("view")).item(MenuItem::new(t("cycle-theme"), "view.theme")))
        .menu(
            Menu::new(t("language"))
                .item(lang_item("Español", "es-PE"))
                .item(lang_item("English", "en-US"))
                .item(lang_item("Runasimi", "qu-PE")),
        )
        .menu(Menu::new(t("help")).item(MenuItem::new(t("about"), "help.about")))
}

/// Traduce un command id del menú principal al `Msg`/efecto real.
fn handle_menu_command(model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    // Cambio de idioma desde el menú "Idioma": aplica el locale en caliente
    // y lo persiste en la capa de usuario de wawa-config.
    if let Some(code) = cmd.strip_prefix("lang.") {
        let _ = rimay_localize::set_locale(code);
        let mut cfg = wawa_config::WawaConfig::load();
        cfg.lang = code.to_string();
        let _ = cfg.save();
        return model;
    }
    match cmd {
        "file.refresh" => {
            handle.dispatch(Msg::ForceReload);
            model
        }
        "file.quit" => std::process::exit(0),
        "view.theme" => {
            handle.dispatch(Msg::CycleTheme);
            model
        }
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => model,
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
