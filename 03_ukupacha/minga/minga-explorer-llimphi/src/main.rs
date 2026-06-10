//! `minga-explorer-llimphi` — dashboard Llimphi del repo Minga (VCS
//! semántico P2P).
//!
//! Polling cada 2s contra `MINGA_REPO` (env, default `./.minga`), abre
//! el `PersistentRepo` (sled, sin passphrase porque los counts son
//! lectura pública) y muestra:
//! - Cantidad de nodos AST almacenados.
//! - Cantidad de atestaciones firmadas.
//! - Cantidad de claves del MST (Merkle Search Tree).
//!
//! No requiere keypair descifrado — eso queda para el CLI
//! (`minga status`) cuando hace falta el DID. El explorer foco es
//! observabilidad rápida.
//!
//! Stack visual: llimphi-theme + llimphi-widget-app-header +
//! llimphi-widget-banner + llimphi-widget-stat-card. Mismo patrón que
//! `nakui-explorer-llimphi`.
//!
//! Uso:
//! ```sh
//! cargo run -p minga-explorer-llimphi
//! # con repo custom:
//! MINGA_REPO=/path/to/.minga cargo run -p minga-explorer-llimphi
//! ```

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_motion::{animate, motion, Tween};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_stat_card::{stat_card_view, StatCardPalette};
use minga_store::PersistentRepo;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const REPO_DIRNAME: &str = "repo";

/// Cuántos items recientes mostrar por sección. Los stores no tienen
/// orden cronológico (sled ordena lexicográfico por hash); los
/// "recent" acá son simplemente los primeros del iter — sirve como
/// sample, no como log temporal.
const RECENT_LIMIT: usize = 5;

// Los tipos del estado son `pub(crate)`: el example headless
// `pantallazo_minga` incluye este archivo por `#[path]` y necesita
// construir el `Model` real para llamar la misma `view`.
#[derive(Clone, Default, Debug)]
pub(crate) struct RepoSnapshot {
    pub nodes: usize,
    pub attestations: usize,
    pub mst_keys: usize,
    pub recent_nodes: Vec<(String, String)>,
    pub recent_attestations: Vec<(String, String)>,
    pub recent_mst_keys: Vec<String>,
}

pub(crate) struct Model {
    pub theme: Theme,
    pub repo_path: PathBuf,
    pub snapshot: Option<RepoSnapshot>,
    pub error: Option<String>,
    pub last_load_ms: u64,
    /// Mantenemos vivo el watcher para que su thread no muera. No se
    /// usa después de crearlo (consume su sí mismo cuando se dropea).
    pub _wawa_watcher: Option<wawa_config::ConfigWatcher>,
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado).
    pub menu_open: Option<usize>,
    /// Fila activa dentro del dropdown abierto (`usize::MAX` = ninguna).
    pub menu_active: usize,
    /// Animación de aparición del dropdown.
    pub menu_anim: Tween<f32>,
    /// Menú contextual sobre el dashboard: `(x, y)` ancla en ventana.
    /// `None` cerrado. El explorer es de sólo lectura — el contextual
    /// sólo ofrece acciones de observación (refrescar / tema).
    pub context_menu: Option<(f32, f32)>,
}

#[derive(Clone)]
pub(crate) enum Msg {
    /// Tick del scheduler: corre `load_snapshot` y dispatcha el
    /// resultado como `Refresh`.
    Tick,
    /// Resultado de un refresh: snapshot exitoso o mensaje de error,
    /// junto al tiempo que tardó el load.
    Refresh {
        result: Result<RepoSnapshot, String>,
        elapsed_ms: u64,
    },
    /// El bus de wawa-config cambió: re-aplicar theme/accent/idioma.
    WawaChanged(wawa_config::WawaConfig),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Navega la fila activa del dropdown (+1/-1).
    MenuNav(i32),
    /// Ejecuta el comando de la fila activa (Enter).
    MenuActivate,
    /// No-op: sólo fuerza re-render durante la animación del dropdown.
    MenuTick,
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click en la raíz → abre el menú contextual anclado en
    /// `(x, y)` de ventana.
    ContextMenuOpen(f32, f32),
    /// Cicla el tema claro/oscuro localmente (override del de wawa hasta
    /// el próximo cambio del bus).
    CycleTheme,
}

pub(crate) struct Explorer;

impl App for Explorer {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Minga — Repo"
    }

    fn initial_size() -> (u32, u32) {
        (800, 560)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let repo_path = std::env::var("MINGA_REPO")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".minga"));

        // Primer refresh inmediato + ticks periódicos. El `Tick` dispara
        // el load en un thread aparte (vía `Handle::spawn` desde update);
        // así el sled no bloquea el hilo de UI.
        handle.dispatch(Msg::Tick);
        handle.spawn_periodic(REFRESH_INTERVAL, || Msg::Tick);

        // Cargar config wawa una vez y aplicarla; suscribirse a cambios.
        let initial_cfg = wawa_config::WawaConfig::load();
        let theme = theme_from_wawa(&initial_cfg);
        apply_lang_from_wawa(&initial_cfg);

        let handle_clone = handle.clone();
        let watcher = wawa_config::ConfigWatcher::spawn(move |cfg| {
            handle_clone.dispatch(Msg::WawaChanged(cfg));
        })
        .ok();

        Model {
            theme,
            repo_path,
            snapshot: None,
            error: None,
            last_load_ms: 0,
            _wawa_watcher: watcher,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            context_menu: None,
        }
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        if let Some(mi) = model.menu_open {
            let n = app_menu().menus.len().max(1);
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        None
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                let path = m.repo_path.clone();
                handle.spawn(move || {
                    let started = std::time::Instant::now();
                    let result = load_snapshot(&path);
                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    Msg::Refresh { result, elapsed_ms }
                });
            }
            Msg::Refresh { result, elapsed_ms } => {
                match result {
                    Ok(snap) => {
                        m.snapshot = Some(snap);
                        m.error = None;
                    }
                    Err(e) => {
                        m.error = Some(rimay_localize::t_args(
                            "minga-error-read",
                            &[
                                ("path", m.repo_path.display().to_string().into()),
                                ("err", e.to_string().into()),
                            ],
                        ));
                    }
                }
                m.last_load_ms = elapsed_ms;
            }
            Msg::WawaChanged(cfg) => {
                m.theme = theme_from_wawa(&cfg);
                apply_lang_from_wawa(&cfg);
            }
            Msg::MenuOpen(which) => {
                m.menu_open = which;
                // Abrir un menú raíz cierra cualquier contextual.
                m.context_menu = None;
                m.menu_active = usize::MAX;
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
            Msg::ContextMenuOpen(x, y) => {
                m.menu_open = None;
                m.context_menu = Some((x, y));
            }
            Msg::CycleTheme => {
                m.theme = Theme::next_after(m.theme.name);
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model, theme));
        let header_palette = AppHeaderPalette::from_theme(theme);
        let stat_palette = StatCardPalette::from_theme(theme);

        // Acentos por kind del dashboard: nodos azul, atestaciones
        // verde, MST purple. Señales semánticas del dominio Minga.
        let accent_nodes = Color::from_rgba8(0x88, 0xc0, 0xd0, 0xff);
        let accent_attestations = Color::from_rgba8(0xa3, 0xbe, 0x8c, 0xff);
        let accent_mst = Color::from_rgba8(0xb4, 0x8e, 0xad, 0xff);

        let header_text = match &model.snapshot {
            Some(_) => rimay_localize::t_args(
                "minga-header-loaded",
                &[
                    ("path", model.repo_path.display().to_string().into()),
                    ("ms", model.last_load_ms.to_string().into()),
                ],
            ),
            None => rimay_localize::t_args(
                "minga-header-searching",
                &[("path", model.repo_path.display().to_string().into())],
            ),
        };

        let header = app_header::<Msg>(header_text, vec![], &header_palette);

        let mut body_children: Vec<View<Msg>> = Vec::new();

        if let Some(ref e) = model.error {
            body_children.push(banner_view::<Msg>(BannerKind::Error, e.clone()));
        }

        match &model.snapshot {
            None => {
                body_children.push(empty_message(theme));
            }
            Some(snap) => {
                let node_items: Vec<String> = snap
                    .recent_nodes
                    .iter()
                    .map(|(h, k)| format!("{h}  {k}"))
                    .collect();
                let attestation_items: Vec<String> = snap
                    .recent_attestations
                    .iter()
                    .map(|(h, did)| format!("{h}  ←  {did}"))
                    .collect();
                let mst_items: Vec<String> = snap.recent_mst_keys.clone();

                body_children.push(stat_card_view::<Msg>(
                    &rimay_localize::t("minga-card-nodes-title"),
                    &snap.nodes.to_string(),
                    &rimay_localize::t("minga-card-nodes-desc"),
                    accent_nodes,
                    &node_items,
                    &stat_palette,
                ));
                body_children.push(stat_card_view::<Msg>(
                    &rimay_localize::t("minga-card-attestations-title"),
                    &snap.attestations.to_string(),
                    &rimay_localize::t("minga-card-attestations-desc"),
                    accent_attestations,
                    &attestation_items,
                    &stat_palette,
                ));
                body_children.push(stat_card_view::<Msg>(
                    &rimay_localize::t("minga-card-mst-title"),
                    &snap.mst_keys.to_string(),
                    &rimay_localize::t("minga-card-mst-desc"),
                    accent_mst,
                    &mst_items,
                    &stat_palette,
                ));
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
        // menú contextual de observación.
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, header, body])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // El menú contextual tiene prioridad si está abierto.
        if let Some((x, y)) = model.context_menu {
            let viewport = viewport_of(model);
            // Acciones reales del explorer: refrescar el snapshot y ciclar
            // el tema. El explorer es de sólo lectura — no inventamos
            // edición.
            let items = vec![
                ContextMenuItem::action(&rimay_localize::t("minga-menu-refresh")),
                ContextMenuItem::action(&rimay_localize::t("minga-menu-theme")),
            ];
            let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> =
                Arc::new(move |i: usize| match i {
                    0 => Msg::Tick,
                    _ => Msg::CycleTheme,
                });
            return Some(context_menu_view(ContextMenuSpec {
                anchor: (x, y),
                viewport,
                header: Some(rimay_localize::t("minga-menu-context-title")),
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
}

/// Viewport para clampear overlays: el explorer no trackea el tamaño de
/// ventana, así que usamos `initial_size()`.
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
/// que mapean a acciones reales (refrescar, tema). Sin "Editar": el
/// explorer no tiene campos de texto editables.
fn app_menu() -> AppMenu {
    AppMenu::new()
        .menu(
            Menu::new(&rimay_localize::t("minga-menu-file"))
                .item(
                    MenuItem::new(&rimay_localize::t("minga-menu-refresh"), "file.refresh")
                        .shortcut("Ctrl+R"),
                )
                .item(
                    MenuItem::new(&rimay_localize::t("minga-menu-quit"), "file.quit")
                        .shortcut("Ctrl+Q")
                        .separated(),
                ),
        )
        .menu(
            Menu::new(&rimay_localize::t("minga-menu-view"))
                .item(MenuItem::new(&rimay_localize::t("minga-menu-theme"), "view.theme")),
        )
        .menu(
            Menu::new(&rimay_localize::t("minga-menu-help"))
                .item(MenuItem::new(&rimay_localize::t("minga-menu-about"), "help.about")),
        )
}

/// Traduce un command id del menú principal al `Msg`/efecto real.
fn handle_menu_command(model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    match cmd {
        "file.refresh" => {
            handle.dispatch(Msg::Tick);
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

fn empty_message(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        rimay_localize::t("minga-empty"),
        13.0,
        theme.fg_muted,
        Alignment::Start,
    )
}

/// Lee el repo sled `<repo_path>/repo` y devuelve los 3 counts.
/// Falla si: el dir no existe, sled rebota al abrir, o cualquier
/// store falla a `len()`. Ningún error es fatal — la UI muestra el
/// banner y mantiene el último snapshot bueno.
pub(crate) fn load_snapshot(repo_path: &std::path::Path) -> Result<RepoSnapshot, String> {
    let inner = repo_path.join(REPO_DIRNAME);
    if !inner.exists() {
        return Err(format!(
            "directorio del repo sled no existe: {}",
            inner.display()
        ));
    }
    let repo = PersistentRepo::open(&inner).map_err(|e| format!("open: {e}"))?;

    let nodes = repo.nodes.len();
    let attestations = repo.attestations.len();
    let mst_keys = repo.mst.len();

    let recent_nodes: Vec<(String, String)> = repo
        .nodes
        .iter()
        .filter_map(|r| r.ok())
        .take(RECENT_LIMIT)
        .map(|(hash, stored)| (short_hash(&hash.to_string()), stored.kind))
        .collect();

    let recent_attestations: Vec<(String, String)> = repo
        .attestations
        .iter()
        .filter_map(|r| r.ok())
        .take(RECENT_LIMIT)
        .map(|att| {
            (
                short_hash(&att.content.to_string()),
                short_hash(&att.author.to_string()),
            )
        })
        .collect();

    let recent_mst_keys: Vec<String> = repo
        .mst
        .iter()
        .filter_map(|r| r.ok())
        .take(RECENT_LIMIT)
        .map(|h| short_hash(&h.to_string()))
        .collect();

    Ok(RepoSnapshot {
        nodes,
        attestations,
        mst_keys,
        recent_nodes,
        recent_attestations,
        recent_mst_keys,
    })
}

/// Trunca un hex string a sus primeros 12 chars. Convención cross-app
/// para mostrar hashes/dids/contenthash compactos sin perder
/// distintividad práctica (12 hex = 48 bits, colisión improbable
/// dentro de un repo single-machine).
fn short_hash(s: &str) -> String {
    s.chars().take(12).collect()
}

/// Construye un `Theme` a partir de la config wawa: matchea el variant
/// canónico contra `Theme::by_name`, aplica el accent si está definido.
/// Cualquier campo no reconocido cae al default dark sin romper.
fn theme_from_wawa(cfg: &wawa_config::WawaConfig) -> Theme {
    let mut t = wawa_config::canonical_theme_name(&cfg.theme_variant)
        .and_then(Theme::by_name)
        .unwrap_or_else(Theme::dark);
    if let Some([r, g, b]) = wawa_config::accent_rgb(&cfg.accent) {
        let c = Color::from_rgba8(r, g, b, 0xff);
        t.accent = c;
        t.border_focus = c;
    }
    t
}

/// Aplica el `lang` de wawa a `rimay_localize`. Errores (locale
/// desconocido) se ignoran — la traducción cae a la cadena default
/// silenciosamente, no vale tumbar la UI por eso.
fn apply_lang_from_wawa(cfg: &wawa_config::WawaConfig) {
    let _ = rimay_localize::set_locale(&cfg.lang);
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Explorer>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_snapshot_errors_on_missing_dir() {
        let p = std::env::temp_dir().join(format!(
            "minga-explorer-llimphi-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let err = load_snapshot(&p).unwrap_err();
        assert!(
            err.contains("no existe"),
            "msg debe explicar el missing: {err}"
        );
    }

    #[test]
    fn snapshot_default_is_zeros_and_empty_lists() {
        let s = RepoSnapshot::default();
        assert_eq!(s.nodes, 0);
        assert_eq!(s.attestations, 0);
        assert_eq!(s.mst_keys, 0);
        assert!(s.recent_nodes.is_empty());
        assert!(s.recent_attestations.is_empty());
        assert!(s.recent_mst_keys.is_empty());
    }

    #[test]
    fn short_hash_takes_first_12_chars() {
        let s = "a1b2c3d4e5f6789012345678901234567890123456789012345678901234abcd";
        assert_eq!(short_hash(s), "a1b2c3d4e5f6");
        assert_eq!(short_hash(s).len(), 12);
    }

    #[test]
    fn short_hash_handles_empty_or_shorter() {
        assert_eq!(short_hash(""), "");
        assert_eq!(short_hash("abc"), "abc");
    }
}
