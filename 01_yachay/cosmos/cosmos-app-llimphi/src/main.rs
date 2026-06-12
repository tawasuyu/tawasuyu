//! `cosmos-app-llimphi` — shell astronómico/astrológico sobre Llimphi.
//!
//! IDE de cartas: barra de menú principal arriba (`Archivo`/`Vista`/
//! `Capas`/`Armónico`/`Ayuda`), árbol de navegación a la izquierda
//! (biblioteca de cartas + catálogo de gráficas astrológicas y
//! astronómicas), pestañas en el área central (una por gráfica abierta)
//! y barra de estado abajo. Click derecho sobre la rueda abre un menú
//! contextual con las opciones del wheel. Todo lo configurable vive en la
//! vista `Configuración` y en los menús `Capas`/`Armónico`.
//!
//! Módulos: `model` (estado + mensajes + taxonomías), `persist`
//! (UI-state + cartas + watcher), `engine` (compose del wheel),
//! `astroview` (cómputo + gráficas astronómicas), `view` (paneles
//! astrológicos), `chrome` (menú/árbol/pestañas/estado/contextuales),
//! `astrocarto` (mapa equirectangular), `format` (símbolos),
//! `update` (bucle Elm + helpers de transición), `nav_ops` (árbol),
//! `dialog_ops` (diálogos modales), `rectify_ops` (rectificador).

mod astrocarto;
mod astroview;
mod chrome;
mod dialog;
mod dialog_ops;
mod engine;
mod format;
mod glyphs;
mod library;
mod model;
mod nav_ops;
mod persist;
mod print;
mod rectify_ops;
mod tools;
mod update;
mod view;

use std::sync::Arc;

use cosmos_engine::Corpus;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::{App, DragPhase, Handle, Key, KeyState, NamedKey, View};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use wawa_config_llimphi::theme_from_wawa;

use crate::astroview::compute_astro;
use crate::chrome::MenuCmd;
use crate::engine::{compute, sample_chart};
use crate::model::{MenuKind, Model, Msg, OpenTab};
use crate::persist::{
    load_chart_from_disk, load_ui_state, save_chart_to_disk, save_ui_state, spawn_chart_watcher,
    UiState,
};

const CORPUS_DEFAULT_RON: &str = include_str!("../../cosmos-corpus/ejemplo.ron");

struct Cosmos;

impl App for Cosmos {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "cosmos · canvas (llimphi)"
    }

    /// El `app_id` Wayland: pata lo usa para correlacionar foco ↔ dientes en el
    /// rail hospedado, así que el `HostClient` registra con este mismo string.
    fn app_id() -> Option<&'static str> {
        Some("tawasuyu.cosmos")
    }

    fn initial_size() -> (u32, u32) {
        (1200, 860)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let cfg_wawa = wawa_config::WawaConfig::load();
        let _ = rimay_localize::set_locale(&cfg_wawa.lang);

        let handle_clone = handle.clone();
        let watcher = wawa_config::ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::WawaConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("cosmos · wawa-config watcher: {e}"))
        .ok();

        let chart = load_chart_from_disk().unwrap_or_else(|| {
            let c = sample_chart();
            save_chart_to_disk(&c);
            c
        });
        let ui = load_ui_state();
        // En modo impresión el tema B/N gana y no acepta el tinte de
        // wawa-config (la hoja tiene que ser blanca sí o sí). En claro/
        // oscuro, el tinte del SO se aplica como siempre.
        let theme = if ui.cfg.print_mode {
            ui.cfg.active_theme()
        } else {
            let base = if ui.cfg.theme_dark {
                Theme::dark()
            } else {
                Theme::light()
            };
            theme_from_wawa(&cfg_wawa, &base)
        };
        // El render de la carta es barato → síncrono. El astro (orto/ocaso/
        // efemérides) es el caro: arranca en `None` ("calculando…") y se
        // computa en un worker que reentra con `AstroComputed`. `init` corre
        // en winit DESPUÉS de crear la ventana, así que un cómputo pesado aquí
        // congelaría la ventana recién abierta. Generación 1 = la del arranque.
        let (render, error) = compute(&chart, &ui.overlays, ui.harmonic, ui.cfg.minor_aspects, 0);
        let astro = None;
        {
            let (c, use_now) = (chart.clone(), ui.cfg.use_now);
            handle.spawn(move || Msg::AstroComputed(1, Arc::new(compute_astro(&c, use_now))));
        }
        let corpus = Corpus::desde_ron(CORPUS_DEFAULT_RON).unwrap_or_default();
        let chart_watcher = spawn_chart_watcher(handle);

        // Árbol de datos sobre cosmos-store: abrir, sembrar/migrar y armar
        // el snapshot jerárquico. Todo expandido en la primera carga.
        let store = library::open_store();
        if let Some(s) = &store {
            library::ensure_seed(s, &chart);
        }
        // La rama fija «Efemérides → Hoy» (sintética) va al tope; luego el store.
        let mut nav_nodes = library::hoy_nodes(&ui.cfg.user_location, &ui.cfg.hoy_locations);
        if let Some(s) = &store {
            nav_nodes.extend(library::snapshot(s));
        }
        let nav_expanded = library::container_keys(&nav_nodes).into_iter().collect();

        // Refresco horario de las cartas «Hoy» al instante actual.
        handle.spawn_periodic(std::time::Duration::from_secs(3600), || Msg::HoyTick);

        // Una pestaña inicial con la carta de trabajo (scratch, sin id).
        let open = vec![OpenTab {
            id: None,
            chart: chart.clone(),
            render: render.clone(),
        }];

        // Rail hospedado: si `COSMOS_DELEGATE_SIDEBAR` está set, cosmos delega su
        // sidebar a pata — publica sus dientes y queda puro canvas.
        let delegated = std::env::var_os("COSMOS_DELEGATE_SIDEBAR").is_some();
        let host = if delegated {
            let teeth: Vec<pata_host::HostedTooth> = ui
                .dock_left
                .iter()
                .chain(&ui.dock_right)
                .map(|i| dock_item_tooth(*i))
                .collect();
            let h = handle.clone();
            pata_host::HostClient::connect("tawasuyu.cosmos", "Cosmos", teeth, move |id| {
                h.dispatch(Msg::HostActivate(id))
            })
        } else {
            None
        };

        Model {
            chart,
            overlays: ui.overlays,
            harmonic: ui.harmonic,
            render,
            astro,
            astro_dirty: false,
            astro_gen: 1,
            corpus,
            cfg: ui.cfg,
            theme,
            error,
            status_note: None,
            open,
            active_tab: 0,
            tile_mode: ui.tile_mode,
            selected_card: None,
            selected_body: None,
            store,
            nav_nodes,
            nav_expanded,
            nav_selected: None,
            nav_rename: None,
            rename_input: llimphi_widget_text_input::TextInputState::new(),
            nav_cut: None,
            sphere_yaw: ui.sphere_yaw,
            sphere_pitch: ui.sphere_pitch,
            sky_nadir: ui.sky_nadir,
            wheel_zoom: 1.0,
            wheel_pan: (0.0, 0.0),
            dial_rot: 0.0,
            carto_rect: Arc::new(std::sync::Mutex::new(None)),
            viewport: model::VIEWPORT,
            tools_scroll: 0.0,
            nav_w: ui.nav_w,
            tools_w: ui.tools_w,
            nav_open: ui.nav_open,
            tools_open: ui.tools_open,
            chart_view: ui.chart_view,
            tool_cat: ui.tool_cat,
            expanded_panels: ui.expanded_panels,
            active_left: ui.dock_left.first().copied(),
            active_right: ui.dock_right.first().copied(),
            dock_expanded: None,
            dock_left: ui.dock_left,
            dock_right: ui.dock_right,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: llimphi_motion::Tween::idle(1.0),
            ctx_open: None,
            nav_ctx: None,
            nav_scroll: 0.0,
            print_scroll: 0.0,
            hoy_active: None,
            rectify_offset_min: 0,
            rectify_events: Vec::new(),
            rectify_result: None,
            rectify_naibod: true,
            rectify_age: 30.0,
            rectify_triggers: Vec::new(),
            dialog: None,
            dialog_field: dialog::DialogField::Name,
            dialog_input: llimphi_widget_text_input::TextInputState::new(),
            delegated,
            _host: host,
            _wawa_watcher: watcher,
            _chart_watcher: chart_watcher,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        update::update(model, msg, handle)
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let menu = chrome::menu_bar(model, &theme);
        let status = chrome::status_bar(model, &theme);
        let sp = SplitterPalette::from_theme(&theme);

        let center = chrome::center_view(model, &theme);

        // Dock: los **rails** flotan como overlay sobre el centro (los
        // dibuja `center_view`), así la rueda usa todo el hueco. Acá sólo
        // colocamos los **paneles** de contenido en panes resizables; la
        // barra azul queda pegada al panel. Angosto → sólo rails; clic en
        // un diente despliega ese lado (estilo web).
        let collapsed = chrome::dock_collapsed(model);
        // En modo delegado los rails los pinta pata; el panel de un lado aparece
        // sólo cuando ese lado está expandido (un diente hospedado activo).
        let (left_show, right_show) = if model.delegated {
            (
                model.dock_expanded == Some(model::DockSide::Left),
                model.dock_expanded == Some(model::DockSide::Right),
            )
        } else {
            (
                !collapsed || model.dock_expanded == Some(model::DockSide::Left),
                !collapsed || model.dock_expanded == Some(model::DockSide::Right),
            )
        };
        let left_panel = if left_show {
            chrome::dock_panel_for(model::DockSide::Left, model, &theme)
        } else {
            None
        };
        let right_panel = if right_show {
            chrome::dock_panel_for(model::DockSide::Right, model, &theme)
        } else {
            None
        };

        let mut core = center;
        if let Some(rp) = right_panel {
            core = splitter_two(
                Direction::Row,
                core,
                PaneSize::Flex,
                rp,
                PaneSize::Fixed(model.tools_w),
                |phase, dx| match phase {
                    DragPhase::Move => Some(Msg::SetToolsWidth(dx)),
                    DragPhase::End => Some(Msg::PersistLayout),
                },
                &sp,
            );
        }
        if let Some(lp) = left_panel {
            core = splitter_two(
                Direction::Row,
                lp,
                PaneSize::Fixed(model.nav_w),
                core,
                PaneSize::Flex,
                |phase, dx| match phase {
                    DragPhase::Move => Some(Msg::SetNavWidth(dx)),
                    DragPhase::End => Some(Msg::PersistLayout),
                },
                &sp,
            );
        }
        let body = core;

        let body_box = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            min_size: Size {
                width: length(0.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![body]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![menu, body_box, status])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // El diálogo modal tiene prioridad sobre los menús.
        dialog::dialog_overlay(model, &model.theme).or_else(|| chrome::overlay_view(model, &model.theme))
    }

    fn on_key(model: &Model, ev: &llimphi_ui::KeyEvent) -> Option<Msg> {
        // Un diálogo modal captura el teclado: Enter confirma, Escape
        // cancela, el resto alimenta el campo enfocado.
        if model.dialog.is_some() {
            if ev.state == KeyState::Pressed {
                match &ev.key {
                    Key::Named(NamedKey::Enter) => return Some(Msg::DialogConfirm),
                    Key::Named(NamedKey::Escape) => return Some(Msg::DialogCancel),
                    _ => {}
                }
            }
            return Some(Msg::DialogKey(ev.clone()));
        }
        // Renombrar un nodo del árbol captura el teclado: Enter confirma,
        // Escape cancela, el resto alimenta el buffer de texto.
        if model.nav_rename.is_some() {
            if ev.state == KeyState::Pressed {
                match &ev.key {
                    Key::Named(NamedKey::Enter) => return Some(Msg::RenameCommit),
                    Key::Named(NamedKey::Escape) => return Some(Msg::RenameCancel),
                    _ => {}
                }
            }
            return Some(Msg::RenameKey(ev.clone()));
        }
        if ev.state != KeyState::Pressed {
            return None;
        }
        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
        // cierra. El context-menu de la rueda queda mouse-only (sólo Esc).
        if let Some(kind) = model.menu_open {
            let order = MenuKind::order();
            let n = order.len().max(1);
            let cur = order.iter().position(|k| *k == kind).unwrap_or(0);
            return match &ev.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenu),
                Key::Named(NamedKey::ArrowLeft) => {
                    Some(Msg::OpenMenu(order[(cur + n - 1) % n]))
                }
                Key::Named(NamedKey::ArrowRight) => Some(Msg::OpenMenu(order[(cur + 1) % n])),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        match &ev.key {
            Key::Named(NamedKey::Escape) => {
                if model.menu_open.is_some() {
                    Some(Msg::CloseMenu)
                } else if model.ctx_open.is_some() {
                    Some(Msg::CloseCtx)
                } else {
                    None
                }
            }
            Key::Character(s) if ev.modifiers.ctrl && s.as_str().eq_ignore_ascii_case("w") => {
                Some(Msg::CloseChartTab(model.active_tab))
            }
            // Ctrl+S → guardar carta en biblioteca (espeja Archivo/Editar).
            // Resolvemos el índice contra la misma lista que pinta el menú
            // para no acoplar el atajo al orden de las entradas.
            Key::Character(s) if ev.modifiers.ctrl && s.as_str().eq_ignore_ascii_case("s") => {
                chrome::menu_entries(MenuKind::Archivo, model)
                    .iter()
                    .position(|e| matches!(e.cmd, MenuCmd::Guardar))
                    .map(|i| Msg::MenuPick(MenuKind::Archivo, i))
            }
            _ => None,
        }
    }

    fn on_resize(_model: &Model, width: u32, height: u32) -> Option<Msg> {
        Some(Msg::Resized(width as f32, height as f32))
    }

    /// Rueda del ratón sobre el lienzo central: zoom (rueda sola), paneo
    /// vertical (Ctrl) y paneo horizontal (Alt).
    fn on_wheel(
        model: &Model,
        delta: llimphi_ui::WheelDelta,
        cursor: (f32, f32),
        modifiers: llimphi_ui::Modifiers,
    ) -> Option<Msg> {
        const STEP: f32 = 40.0;
        if modifiers.ctrl {
            Some(Msg::WheelPan(0.0, -delta.y * STEP))
        } else if modifiers.alt {
            Some(Msg::WheelPan(-delta.y * STEP, 0.0))
        } else {
            // Zoom: rueda hacia arriba (delta.y < 0) acerca.
            let factor = if delta.y < 0.0 { 1.12 } else { 0.892 };
            // En astrocarto, el zoom va HACIA el cursor: ajusta el paneo
            // para que el punto del mapa bajo el puntero quede fijo.
            if matches!(model.chart_view, crate::model::ChartView::Cielo) {
                if let Ok(guard) = model.carto_rect.lock() {
                    if let Some((rx, ry, rw, rh)) = *guard {
                        let z = model.wheel_zoom;
                        let z2 = (z * factor).clamp(0.25, 8.0);
                        let f = if z > 0.0 { z2 / z } else { 1.0 };
                        let rcx = rx + rw * 0.5;
                        let rcy = ry + rh * 0.5;
                        let (cx, cy) = cursor;
                        let pan_x = (cx - rcx) * (1.0 - f) + model.wheel_pan.0 * f;
                        let pan_y = (cy - rcy) * (1.0 - f) + model.wheel_pan.1 * f;
                        return Some(Msg::WheelSetView(z2, pan_x, pan_y));
                    }
                }
                return Some(Msg::WheelZoom(factor));
            }
            if matches!(model.chart_view, crate::model::ChartView::Carto) {
                if let Ok(guard) = model.carto_rect.lock() {
                    if let Some((rx, ry, rw, rh)) = *guard {
                        let base = (rw / 320.0).min(rh / 160.0);
                        let z = model.wheel_zoom;
                        let z2 = (z * factor).clamp(0.25, 8.0);
                        let s = base * z;
                        let s2 = base * z2;
                        if s > 0.0 && base > 0.0 {
                            let (cx, cy) = cursor;
                            let rcx = rx + rw * 0.5;
                            let rcy = ry + rh * 0.5;
                            let off_x = rcx - 320.0 * s / 2.0 + model.wheel_pan.0;
                            let off_y = rcy - 160.0 * s / 2.0 + model.wheel_pan.1;
                            let wx = (cx - off_x) / s;
                            let wy = (cy - off_y) / s;
                            let pan_x = cx - wx * s2 - rcx + 320.0 * s2 / 2.0;
                            let pan_y = cy - wy * s2 - rcy + 160.0 * s2 / 2.0;
                            return Some(Msg::WheelSetView(z2, pan_x, pan_y));
                        }
                    }
                }
            }
            Some(Msg::WheelZoom(factor))
        }
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Cosmos>();
}

/// Proyecta un `DockItem` a un diente hospedado `(id, icono, etiqueta)` para
/// publicarlo en el rail de pata. El `id` codifica el `DockItem` (`to_u64`) y
/// vuelve tal cual en [`Msg::HostActivate`].
fn dock_item_tooth(item: model::DockItem) -> pata_host::HostedTooth {
    use model::{DockItem, ToolCat};
    let (icon, label): (&str, String) = match item {
        DockItem::Arbol => ("folder", "Biblioteca".to_string()),
        other => {
            let tc = other.tool_cat().unwrap_or(ToolCat::Principal);
            let icon = match tc {
                ToolCat::Astronomia => "astro",
                ToolCat::Sistema => "settings",
                _ => "tools",
            };
            (icon, tc.title().to_string())
        }
    };
    pata_host::HostedTooth::new(item.to_u64() as u32, icon, label)
}
