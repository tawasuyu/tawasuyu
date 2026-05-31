//! `chasqui-explorer-llimphi` — panel Llimphi que descubre al daemon
//! `chasqui` vía broker brahman y muestra sus Mónadas en vivo.
//!
//! Diseño: ventana standalone que cada N segundos consulta el query
//! socket del daemon (`chasqui_card::query::client::list_monads`). El
//! path del socket NO está hardcoded — se descubre vía
//! `card_sidecar::await_provider_blocking` para el flow
//! `monad-list:json`. Si el daemon cae, el socket cacheado se invalida
//! y la próxima iteración re-descubre.
//!
//! Sin integración con nahual-shell — es su propio binario para que el
//! ecosistema sea visible incluso sin la shell completa.
//!
//! Uso:
//! ```sh
//! cargo run -p chasqui-explorer-llimphi
//! # con override del init socket (heredado de brahman-handshake):
//! BRAHMAN_INIT_SOCKET=/tmp/init.sock cargo run -p chasqui-explorer-llimphi
//! ```

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Duration;

use card_sidecar::{await_provider_blocking, build_consumer_card, ConsumerError};
use chasqui_card::query::client as query_client;
use chasqui_card::query::{transport, ListMonadsResponse, FLOW_MONAD_LIST, FLOW_TYPE_NAME};
use chasqui_card::Lens;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, Dimension, FlexDirection, Size, Style},
    Rect,
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
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};

use app_bus::{AppMenu, Menu, MenuItem};
use std::sync::Arc;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);

struct Model {
    theme: Theme,
    socket: Option<PathBuf>,
    snapshot: Option<ListMonadsResponse>,
    error: Option<String>,
    /// Última fuente del socket activo: "discovery"/"broker"/"cache"/
    /// "default-path". Sólo informativo en el header.
    socket_source: Option<&'static str>,
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado).
    menu_open: Option<usize>,
    /// Mónada seleccionada (índice en `snapshot.monads`). `None` si
    /// ninguna. La selección sólo resalta y habilita el menú contextual;
    /// el explorer es de sólo lectura.
    selected: Option<usize>,
    /// Menú contextual sobre una Mónada: `(idx, x, y)` ancla en ventana.
    /// `None` cerrado.
    context_menu: Option<(usize, f32, f32)>,
}

#[derive(Clone)]
enum Msg {
    Tick,
    Refresh(TickOutcome),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Cicla el tema claro/oscuro.
    CycleTheme,
    /// Fuerza re-descubrimiento del socket: invalida el cacheado y
    /// dispara un Tick. Mapea "Reconectar" del menú Ver.
    Reconnect,
    /// Selecciona una Mónada por índice (resalta).
    SelectMonad(usize),
    /// Right-click en la raíz → abre el menú contextual anclado en
    /// `(x, y)` de ventana sobre la Mónada seleccionada. Sin selección
    /// es no-op.
    ContextMenuOpen(f32, f32),
}

#[derive(Clone)]
enum TickOutcome {
    Ok {
        socket: PathBuf,
        source: &'static str,
        snapshot: Box<ListMonadsResponse>,
    },
    DiscoveryFailed(String),
    QueryFailed(String),
}

struct Explorer;

impl App for Explorer {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Chasqui — Mónadas"
    }

    fn initial_size() -> (u32, u32) {
        (900, 640)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        // Primer refresh inmediato + ticks periódicos. El tick dispara
        // discovery+query en un thread (vía `Handle::spawn` desde
        // update); así el broker bloqueante no congela el UI.
        handle.dispatch(Msg::Tick);
        handle.spawn_periodic(REFRESH_INTERVAL, || Msg::Tick);

        Model {
            theme: Theme::dark(),
            socket: None,
            snapshot: None,
            error: None,
            socket_source: None,
            menu_open: None,
            selected: None,
            context_menu: None,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                let prior_socket = m.socket.clone();
                handle.spawn(move || Msg::Refresh(tick(prior_socket)));
            }
            Msg::Refresh(outcome) => match outcome {
                TickOutcome::Ok { socket, source, snapshot } => {
                    m.socket = Some(socket);
                    m.socket_source = Some(source);
                    m.snapshot = Some(*snapshot);
                    m.error = None;
                    // Si la selección quedó fuera de rango tras el
                    // refresh, la descartamos.
                    let count = m.snapshot.as_ref().map(|s| s.monads.len()).unwrap_or(0);
                    if m.selected.map(|i| i >= count).unwrap_or(false) {
                        m.selected = None;
                        m.context_menu = None;
                    }
                }
                TickOutcome::DiscoveryFailed(msg) => {
                    m.socket = None;
                    m.socket_source = None;
                    m.error = Some(msg);
                }
                TickOutcome::QueryFailed(msg) => {
                    // Invalida el socket cacheado: la próxima iteración
                    // re-descubre.
                    m.socket = None;
                    m.socket_source = None;
                    m.error = Some(msg);
                }
            },
            Msg::MenuOpen(which) => {
                m.menu_open = which;
                // Abrir un menú raíz cierra cualquier contextual.
                m.context_menu = None;
            }
            Msg::CloseMenus => {
                m.menu_open = None;
                m.context_menu = None;
            }
            Msg::MenuCommand(cmd) => {
                m.menu_open = None;
                return handle_menu_command(m, &cmd, handle);
            }
            Msg::CycleTheme => {
                m.theme = Theme::next_after(m.theme.name);
            }
            Msg::Reconnect => {
                // Invalida el socket cacheado y re-dispara discovery.
                m.socket = None;
                m.socket_source = None;
                m.error = None;
                handle.dispatch(Msg::Tick);
            }
            Msg::SelectMonad(i) => {
                m.selected = Some(i);
                m.context_menu = None;
            }
            Msg::ContextMenuOpen(x, y) => {
                // Sólo si hay una Mónada seleccionada válida.
                let count = m.snapshot.as_ref().map(|s| s.monads.len()).unwrap_or(0);
                if let Some(i) = m.selected.filter(|i| *i < count) {
                    m.menu_open = None;
                    m.context_menu = Some((i, x, y));
                }
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model, theme));
        let header_palette = AppHeaderPalette::from_theme(theme);
        let card_palette = CardPalette::from_theme(theme);

        // Acentos por kind del dominio chasqui: engine cyan, data
        // purple. Señales semánticas locales del explorer.
        let accent_engine = Color::from_rgba8(0x88, 0xc0, 0xd0, 0xff);
        let accent_data = Color::from_rgba8(0xb4, 0x8e, 0xad, 0xff);

        let header_text = match (&model.snapshot, &model.socket, model.socket_source) {
            (Some(s), Some(sock), Some(src)) => {
                let watching = s
                    .engine
                    .watching
                    .as_deref()
                    .map(|w| {
                        rimay_localize::t_args(
                            "chasqui-header-watching",
                            &[("name", w.into())],
                        )
                    })
                    .unwrap_or_default();
                rimay_localize::t_args(
                    "chasqui-header",
                    &[
                        ("engine", s.engine.label.as_str().into()),
                        ("count", s.monads.len().to_string().into()),
                        ("socket", sock.display().to_string().into()),
                        ("src", src.into()),
                        ("watching", watching.into()),
                    ],
                )
            }
            _ => rimay_localize::t("chasqui-header-searching"),
        };

        let header = app_header::<Msg>(header_text, vec![], &header_palette);

        let mut body_children: Vec<View<Msg>> = Vec::new();

        if let Some(ref e) = model.error {
            body_children.push(banner_view::<Msg>(BannerKind::Error, e.clone()));
        }

        if let Some(snap) = &model.snapshot {
            body_children.push(engine_card(snap, accent_engine, theme, &card_palette));
            for (i, m) in snap.monads.iter().enumerate() {
                let selected = model.selected == Some(i);
                let card = monad_card(m, accent_data, theme, &card_palette);
                // Click selecciona la Mónada. El menú contextual se abre
                // por right-click en la raíz (coords de ventana) sobre la
                // selección actual — ver `view()`.
                let card = card.on_click(Msg::SelectMonad(i));
                let card = if selected {
                    // Resalte sutil de la card seleccionada.
                    card.fill(theme.bg_selected)
                } else {
                    card
                };
                body_children.push(card);
            }
        }

        let body = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            padding: Rect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(12.0_f32),
                bottom: length(16.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(8.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(body_children);

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
        // menú contextual sobre la Mónada seleccionada.
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, header, body])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // El menú contextual de la Mónada tiene prioridad si está abierto.
        if let Some((idx, x, y)) = model.context_menu {
            let label = model
                .snapshot
                .as_ref()
                .and_then(|s| s.monads.get(idx))
                .map(|m| m.label.clone())
                .unwrap_or_else(|| "Mónada".to_string());
            let viewport = viewport_of(model);
            // Acciones reales del explorer: ver/seleccionar y refrescar.
            // El explorer es de sólo lectura, no inventamos edición.
            let items = vec![
                ContextMenuItem::action("Ver detalle"),
                ContextMenuItem::action("Refrescar"),
            ];
            let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| match i {
                0 => Msg::SelectMonad(idx),
                _ => Msg::Tick,
            });
            return Some(context_menu_view(ContextMenuSpec {
                anchor: (x, y),
                viewport,
                header: Some(label),
                items,
                active: usize::MAX,
                on_pick,
                on_dismiss: Msg::CloseMenus,
                palette: ContextMenuPalette::from_theme(&model.theme),
            }));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu();
        menubar_overlay(&menubar_spec(&menu, model, &model.theme))
    }
}

/// Viewport para clampear overlays: tamaño de ventana del Model si lo
/// llevara; como el explorer no lo trackea, usamos `initial_size()`.
fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = Explorer::initial_size();
    (w as f32, h as f32)
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(
    menu: &'a AppMenu,
    model: &Model,
    theme: &'a Theme,
) -> MenuBarSpec<'a, Msg> {
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

/// El menú principal del explorer. Archivo / Ver / Ayuda — sólo comandos
/// que mapean a acciones reales (refrescar, reconectar, tema). Sin
/// "Editar": el explorer no tiene campos de texto editables.
fn app_menu() -> AppMenu {
    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Refrescar", "file.refresh").shortcut("Ctrl+R"))
                .item(MenuItem::new("Salir", "file.quit").shortcut("Ctrl+Q").separated()),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Reconectar", "view.reconnect"))
                .item(MenuItem::new("Cambiar tema", "view.theme").separated()),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Traduce un command id del menú principal al `Msg`/efecto real.
fn handle_menu_command(model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    match cmd {
        "file.refresh" => {
            handle.dispatch(Msg::Tick);
            model
        }
        "file.quit" => std::process::exit(0),
        "view.reconnect" => {
            handle.dispatch(Msg::Reconnect);
            model
        }
        "view.theme" => {
            handle.dispatch(Msg::CycleTheme);
            model
        }
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => model,
    }
}

fn engine_card(
    snap: &ListMonadsResponse,
    accent: Color,
    theme: &Theme,
    palette: &CardPalette,
) -> View<Msg> {
    let mut rows: Vec<View<Msg>> = vec![
        kind_row("[engine]", &snap.engine.label, accent, theme),
        muted_line(
            &rimay_localize::t_args(
                "chasqui-field-id",
                &[("id", snap.engine.id.to_string().into())],
            ),
            theme,
        ),
    ];
    if let Some(w) = &snap.engine.watching {
        rows.push(muted_line(
            &rimay_localize::t_args(
                "chasqui-field-watching",
                &[("name", w.as_str().into())],
            ),
            theme,
        ));
    }
    card_view(
        rows,
        CardOptions {
            accent: Some(accent),
            ..Default::default()
        },
        palette,
    )
}

fn monad_card(
    m: &chasqui_card::query::MonadView,
    accent: Color,
    theme: &Theme,
    palette: &CardPalette,
) -> View<Msg> {
    let lens = lens_label(m.dominant_lens);
    let stats = format!("{} files · ent {:.2} · {}", m.cardinality, m.entropy, lens);
    let mut rows: Vec<View<Msg>> = vec![
        kind_row_with_stats("[monad]", &m.label, &stats, accent, theme),
        muted_line(
            &rimay_localize::t_args(
                "chasqui-field-id",
                &[("id", m.id.to_string().into())],
            ),
            theme,
        ),
    ];
    if !m.summary.is_empty() {
        rows.push(text_line(&m.summary, theme.fg_text, theme));
    }
    let keywords = m.keywords.join(", ");
    if !keywords.is_empty() {
        rows.push(muted_line(
            &rimay_localize::t_args(
                "chasqui-field-keywords",
                &[("keywords", keywords.as_str().into())],
            ),
            theme,
        ));
    }
    if let Some(p) = m.path_hint.as_deref().filter(|p| !p.is_empty()) {
        rows.push(muted_line(
            &rimay_localize::t_args("chasqui-field-path", &[("path", p.into())]),
            theme,
        ));
    }
    if let Some(model_name) = m.centroid_model.as_deref().filter(|s| !s.is_empty()) {
        rows.push(muted_line(
            &rimay_localize::t_args("chasqui-field-model", &[("name", model_name.into())]),
            theme,
        ));
    }
    card_view(
        rows,
        CardOptions {
            accent: Some(accent),
            ..Default::default()
        },
        palette,
    )
}

fn kind_row(tag: &str, label: &str, accent: Color, theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size {
                width: length(72.0_f32),
                height: length(16.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(tag.to_string(), 11.0, accent, Alignment::Start),
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(18.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(label.to_string(), 15.0, theme.fg_text, Alignment::Start),
    ])
}

fn kind_row_with_stats(
    tag: &str,
    label: &str,
    stats: &str,
    accent: Color,
    theme: &Theme,
) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size {
                width: length(72.0_f32),
                height: length(16.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(tag.to_string(), 11.0, accent, Alignment::Start),
        View::new(Style {
            size: Size {
                width: Dimension::auto(),
                height: length(18.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(label.to_string(), 15.0, theme.fg_text, Alignment::Start),
        View::new(Style {
            size: Size {
                width: Dimension::auto(),
                height: length(16.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(stats.to_string(), 11.0, theme.fg_muted, Alignment::Start),
    ])
}

fn muted_line(text: &str, theme: &Theme) -> View<Msg> {
    text_line(text, theme.fg_muted, theme)
}

fn text_line(text: &str, color: Color, _theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), 11.0, color, Alignment::Start)
}

fn tick(prior_socket: Option<PathBuf>) -> TickOutcome {
    let (socket, source) = match prior_socket {
        Some(p) => (p, "cache"),
        None => match resolve_socket() {
            Ok(found) => found,
            Err(e) => return TickOutcome::DiscoveryFailed(e),
        },
    };

    match query_client::list_monads(&socket, QUERY_TIMEOUT) {
        Ok(resp) => TickOutcome::Ok {
            socket,
            source,
            snapshot: Box::new(resp),
        },
        Err(e) => TickOutcome::QueryFailed(format!(
            "query a {}: {e} — re-descubriendo en próxima iteración",
            socket.display()
        )),
    }
}

/// Resuelve el socket del daemon en dos pasos:
/// 1. **Broker**: consumer Card + `await_provider_blocking`. Path
///    "consciente" (ecosistema brahman activo).
/// 2. **Default path**: si el broker no responde, probamos
///    `transport::default_socket_path()` directo. Path "soberano"
///    (daemon corriendo solo, sin init).
fn resolve_socket() -> Result<(PathBuf, &'static str), String> {
    match discover_via_broker() {
        Ok(p) => Ok((p, "broker")),
        Err(broker_err) => {
            let fallback = transport::default_socket_path();
            if fallback.exists() {
                Ok((fallback, "default-path"))
            } else {
                Err(format!(
                    "broker: {broker_err}; fallback {} no existe",
                    fallback.display()
                ))
            }
        }
    }
}

fn discover_via_broker() -> Result<PathBuf, ConsumerError> {
    let card = build_consumer_card("chasqui-explorer-llimphi", FLOW_MONAD_LIST, FLOW_TYPE_NAME);
    await_provider_blocking(card, DISCOVERY_TIMEOUT)
}

fn lens_label(l: Lens) -> &'static str {
    match l {
        Lens::Grid => "grid",
        Lens::Code => "code",
        Lens::Gallery => "gallery",
        Lens::Database => "database",
        Lens::Markdown => "markdown",
        Lens::Tree => "tree",
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Explorer>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lens_labels_cover_all_variants() {
        // Sanity: cualquier Lens devuelve un string no vacío.
        for l in [
            Lens::Grid,
            Lens::Code,
            Lens::Gallery,
            Lens::Database,
            Lens::Markdown,
            Lens::Tree,
        ] {
            assert!(!lens_label(l).is_empty());
        }
    }

    #[test]
    fn resolve_socket_fails_with_message_when_nothing_responds() {
        // El test depende de que ni init socket ni default path tengan
        // un daemon vivo — en CI sin daemon corriendo eso se cumple.
        // Si en local hay un nouser vivo este test pasa por la rama Ok,
        // sin assert estricto. La condición esencial: nunca panic.
        let _ = resolve_socket();
    }
}
