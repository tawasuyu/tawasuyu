//! `app` — implementación de [`llimphi_ui::App`] para khipu.
//!
//! Contiene la estructura `KhipuApp` y los métodos del ciclo Elm:
//! `init`, `update`, `view`, `view_overlay` y `on_key`.
//! La lógica de negocio se delega a los submódulos `estado`, `menu`,
//! `map`, `panels` y `net`.

use llimphi_motion::{animate, motion, Tween};
use llimphi_ui::llimphi_hal::winit::keyboard::{Key, NamedKey};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    Dimension,
};
use llimphi_ui::{App, Handle, KeyEvent, KeyState, View};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_list::ListPalette;
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view,
};
use llimphi_widget_text_editor::{EditorMetrics, EditorPalette, PointerEvent};
use llimphi_widget_text_input::TextInputPalette;

use crate::estado::{
    commit_edits, data_file_path, deselect, first_visible, from_state, load_state, persist,
    reinforce_and_touch, schedule_embedding, seeded_model, select, start_unlock, unlock_identity,
    now_secs,
};
use crate::map::{
    gravity_panel, name_region_chip, naming_input, node_card, node_screen_pos, overlay_left,
    overlay_right, pick_note, pinned, unnamed_cluster_centroids, world_screen, place_note,
};
use crate::menu::{app_menu, focused_edit_flags, focused_editor, menubar_spec};
use crate::modelo::{Embedder, Focus, Model, Msg, EDITOR_OVERLAY_W, LIST_WIDTH, ZOOM_INJECT};
use crate::net::{ensure_p2p, export_notebook, import_notebook, peer_addr, start_publishing};
use crate::panels::{
    editor_panel, header_view, list_panel, receive_panel, status_bar, unlock_view,
};

/// Tamaño inicial de la ventana. Compartido por `menubar_spec` y el App trait.
const INITIAL_W: u32 = 1280;
const INITIAL_H: u32 = 760;

/// La aplicación Llimphi de khipu. Sin estado propio: todo vive en `Model`.
pub(crate) struct KhipuApp;

// =====================================================================
// Helpers locales de menú (no pueden vivir en menu.rs por ciclo de módulos)
// =====================================================================

/// Traduce el `command` del menú principal al `Msg` real y lo redespacha
/// por el `update`. Cierra el menú antes de actuar.
fn handle_menu_command(mut model: Model, command: String, h: &Handle<Msg>) -> Model {
    model.menu_open = None;
    let target = match command.as_str() {
        "note.new" => Some(Msg::NewNote),
        "note.delete" => Some(Msg::DeleteSelected),
        "note.archive" => Some(Msg::ToggleArchive),
        "share.export" => Some(Msg::Export),
        "share.import" => Some(Msg::Import),
        "share.publish" => Some(Msg::Publish),
        "share.receive" => Some(Msg::Receive),
        "view.search" => Some(Msg::Focus(Focus::Search)),
        "edit.undo" => Some(Msg::EditMenuAction(EditAction::Undo)),
        "edit.redo" => Some(Msg::EditMenuAction(EditAction::Redo)),
        "edit.cut" => Some(Msg::EditMenuAction(EditAction::Cut)),
        "edit.copy" => Some(Msg::EditMenuAction(EditAction::Copy)),
        "edit.paste" => Some(Msg::EditMenuAction(EditAction::Paste)),
        "edit.selectall" => Some(Msg::EditMenuAction(EditAction::SelectAll)),
        "help.about" => {
            model.status = Some("khipu · cuaderno de notas P2P soberano".into());
            None
        }
        _ => None,
    };
    match target {
        Some(msg) => KhipuApp::update(model, msg, h),
        None => model,
    }
}

/// Aplica una acción del menú de edición al editor del campo focuseado,
/// usando el portapapeles del sistema. Cierra el menú de edición.
fn apply_edit_menu_action(mut model: Model, action: EditAction, h: &Handle<Msg>) -> Model {
    model.edit_menu = None;
    let focus = model.focus;
    let clip = &mut model.clipboard;
    let result = match focus {
        Focus::Body => Some(editmenu::apply(&mut model.body, action, clip)),
        Focus::Title => Some(editmenu::apply(model.title.editor_mut(), action, clip)),
        Focus::Tags => Some(editmenu::apply(model.tags.editor_mut(), action, clip)),
        Focus::Search => Some(editmenu::apply(model.search.editor_mut(), action, clip)),
        Focus::PeerAddr => Some(editmenu::apply(model.peer_input.editor_mut(), action, clip)),
        Focus::Region => Some(editmenu::apply(model.region_input.editor_mut(), action, clip)),
        Focus::Passphrase => Some(editmenu::apply(model.passphrase.editor_mut(), action, clip)),
        Focus::None => None,
    };
    // Si la acción cambió un campo persistente de la nota, corremos el commit.
    if let Some(r) = result {
        if r.changed() && matches!(focus, Focus::Body | Focus::Title | Focus::Tags) {
            commit_edits(&mut model, h);
        }
    }
    model
}

// =====================================================================
// App impl
// =====================================================================

impl App for KhipuApp {
    type Model = Model;
    type Msg = Msg;

    fn init(handle: &Handle<Msg>) -> Model {
        // Conectamos al daemon una sola vez al arrancar.
        let embedder = Embedder::connect();
        let data_path = data_file_path();
        let mut model = match data_path.as_ref().and_then(load_state) {
            Some(state) => from_state(state, embedder),
            None => seeded_model(embedder),
        };
        model.data_path = data_path;
        // Identidad: si `KHIPU_PASSPHRASE` está en el entorno, desbloqueamos
        // sin prompt — útil headless.
        model.keypair = std::env::var("KHIPU_PASSPHRASE")
            .ok()
            .and_then(|p| unlock_identity(&p));
        model.theme = llimphi_theme::Theme::dark();
        // Con bootstrap configurado, arrancamos el nodo libp2p ya.
        if std::env::var("KHIPU_BOOTSTRAP").is_ok() {
            ensure_p2p(&mut model);
        }
        // Elegimos la primera nota más pesada (decayendo on-the-fly).
        let first = first_visible(&model).or_else(|| model.order.first().copied());
        if let Some(id) = first {
            reinforce_and_touch(&mut model, id);
            select(&mut model, id);
        }
        persist(&model);
        // Latido cada 30 s — la masa decae en disco como en pantalla.
        handle.spawn_periodic(std::time::Duration::from_secs(30), || Msg::Tick);
        model
    }

    fn update(mut model: Model, msg: Msg, h: &Handle<Msg>) -> Model {
        match msg {
            Msg::SelectNote(id) => {
                commit_edits(&mut model, h);
                reinforce_and_touch(&mut model, id);
                select(&mut model, id);
                persist(&model);
            }
            Msg::NewNote => {
                commit_edits(&mut model, h);
                let now = now_secs();
                let id = model.store.create("Nota nueva", "", Vec::new(), now);
                model.order.push(id);
                schedule_embedding(&mut model, id, h);
                select(&mut model, id);
                persist(&model);
            }
            Msg::ToggleArchive => {
                model.show_archive = !model.show_archive;
            }
            Msg::Tick => {
                // No muta nada: el Tick existe sólo para forzar un redraw.
            }
            Msg::EmbeddingReady(id, seq, v) => {
                if model.embed_latest.get(&id) == Some(&seq)
                    && model.store.get(id).is_some()
                {
                    model.field.insert(id, v);
                    place_note(&mut model, id);
                    persist(&model);
                }
            }
            Msg::Export => {
                if model.keypair.is_none() {
                    start_unlock(&mut model, Msg::Export);
                } else {
                    commit_edits(&mut model, h);
                    model.status = Some(export_notebook(&model));
                }
            }
            Msg::Import => {
                let report = import_notebook(&mut model, h);
                persist(&model);
                model.status = Some(report);
            }
            Msg::Publish => {
                if model.keypair.is_none() {
                    start_unlock(&mut model, Msg::Publish);
                } else {
                    commit_edits(&mut model, h);
                    let _ = export_notebook(&model);
                    model.status = Some(start_publishing(&mut model, h));
                }
            }
            Msg::Receive => {
                let my_key = model.keypair.as_ref().map(|k| k.public_key());
                model.receiving = true;
                model.peers.clear();
                if model.peer_input.is_empty() {
                    model.peer_input.set_text(peer_addr());
                }
                model.focus = Focus::PeerAddr;
                model.status = Some("buscando pares (LAN + DHT)… o escribí una dirección".into());
                let dht = model.p2p.as_ref().map(|p| (p.rt.clone(), p.node.clone()));
                h.spawn(move || {
                    let mut infos: Vec<crate::modelo::PeerInfo> =
                        khipu_share::discovery::descubrir(std::time::Duration::from_secs(3))
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|p| Some(p.beacon.author) != my_key)
                            .map(|p| crate::modelo::PeerInfo {
                                addr: p.fetch_addr.to_string(),
                                label: format!(
                                    "LAN · {} · de:{} · {}",
                                    p.beacon.name,
                                    khipu_share::hex8(&p.beacon.author),
                                    p.fetch_addr
                                ),
                            })
                            .collect();
                    if let Some((rt, node)) = dht {
                        let me = node.peer_id();
                        for pid in rt.block_on(node.descubrir()) {
                            if pid == me {
                                continue;
                            }
                            let s = pid.to_string();
                            let corto: String = s.chars().rev().take(8).collect::<Vec<_>>()
                                .into_iter().rev().collect();
                            infos.push(crate::modelo::PeerInfo {
                                label: format!("DHT · …{corto}"),
                                addr: s,
                            });
                        }
                    }
                    Msg::PeersFound(infos)
                });
            }
            Msg::PeersFound(peers) => {
                if model.receiving {
                    model.status = Some(if peers.is_empty() {
                        "ningún par en la LAN — escribí una dirección y jalá".into()
                    } else {
                        format!("{} pares en la red — elegí uno o escribí una dirección", peers.len())
                    });
                    model.peers = peers;
                }
            }
            Msg::FetchManual => {
                let addr = model.peer_input.text().trim().to_string();
                if addr.is_empty() {
                    model.status = Some("escribí una dirección host:puerto".into());
                } else {
                    h.dispatch(Msg::FetchFrom(addr));
                }
            }
            Msg::FetchFrom(addr) => {
                model.receiving = false;
                model.peers.clear();
                model.focus = Focus::None;
                let destino = addr.trim().to_string();
                if destino.starts_with('/') || !destino.contains(':') {
                    // Vía libp2p: multiaddr o peer-id pelado.
                    if ensure_p2p(&mut model) {
                        let p = model.p2p.as_ref().expect("p2p recién armado");
                        let (rt, node) = (p.rt.clone(), p.node.clone());
                        let es_multiaddr = destino.starts_with('/');
                        model.status = Some(format!("jalando por libp2p de {destino}…"));
                        h.spawn(move || {
                            let res = if es_multiaddr {
                                rt.block_on(node.fetch_addr_str(&destino))
                            } else {
                                rt.block_on(node.fetch_peer_str(&destino))
                            };
                            match res {
                                Ok(s) => Msg::Received(Ok(s)),
                                Err(e) => Msg::Received(Err(format!("p2p: {e}"))),
                            }
                        });
                    } else {
                        model.status = Some("no se pudo iniciar el nodo libp2p".into());
                    }
                } else {
                    // Dirección TCP `host:puerto` (LAN/WAN directa).
                    model.status = Some(format!("jalando de {destino}…"));
                    h.spawn(move || match khipu_share::net::fetch(&destino) {
                        Ok(s) => Msg::Received(Ok(s)),
                        Err(e) => Msg::Received(Err(format!("no se pudo recibir de {destino}: {e}"))),
                    });
                }
            }
            Msg::CancelPeers => {
                model.receiving = false;
                model.peers.clear();
                model.focus = Focus::None;
                model.status = Some("recibir cancelado".into());
            }
            Msg::Received(res) => {
                model.receiving = false;
                model.peers.clear();
                model.status = Some(match res {
                    Ok(sobre) => match khipu_share::open(&sobre) {
                        Ok(bundle) => {
                            let now = now_secs();
                            let outcome =
                                khipu_share::import_into(&mut model.store, bundle, now);
                            for id in &outcome.created {
                                model.order.push(*id);
                                schedule_embedding(&mut model, *id, h);
                            }
                            persist(&model);
                            format!(
                                "recibidas {} · omitidas {} (ya existían)",
                                outcome.created.len(),
                                outcome.skipped
                            )
                        }
                        Err(_) => "firma inválida — sobre rechazado".into(),
                    },
                    Err(e) => e,
                });
            }
            Msg::Unlock => {
                let pass = model.passphrase.text();
                match unlock_identity(&pass) {
                    Some(kp) => {
                        let id = khipu_share::hex8(&kp.public_key());
                        model.keypair = Some(kp);
                        model.unlocking = false;
                        model.passphrase.clear();
                        model.focus = Focus::None;
                        model.status = Some(format!("identidad desbloqueada · {id}"));
                        if let Some(accion) = model.pending.take() {
                            h.dispatch(*accion);
                        }
                    }
                    None => {
                        model.status =
                            Some("passphrase incorrecta o sin acceso al keystore".into());
                    }
                }
            }
            Msg::CancelUnlock => {
                model.unlocking = false;
                model.pending = None;
                model.passphrase.clear();
                model.focus = Focus::None;
                model.status = Some("desbloqueo cancelado".into());
            }
            Msg::RelayReady(addr) => {
                model.status = Some(format!("alcanzable vía relay: {addr}"));
            }
            Msg::DeleteSelected => {
                if let Some(id) = model.selected {
                    model.store.remove(id);
                    model.order.retain(|x| *x != id);
                    model.field.remove(id);
                    let next = model.order.first().copied();
                    model.selected = None;
                    model.title.clear();
                    model.body = llimphi_widget_text_editor::EditorState::default();
                    model.tags.clear();
                    if let Some(n) = next {
                        select(&mut model, n);
                    }
                    persist(&model);
                }
            }
            Msg::Focus(f) => {
                commit_edits(&mut model, h);
                model.focus = f;
            }
            Msg::Key(ev) => {
                let changed = match model.focus {
                    Focus::Title => model.title.apply_key(&ev),
                    Focus::Body => model.body.apply_key(&ev).touched(),
                    Focus::Tags => model.tags.apply_key(&ev),
                    Focus::Search => {
                        let _ = model.search.apply_key(&ev);
                        false
                    }
                    Focus::Passphrase => {
                        let _ = model.passphrase.apply_key(&ev);
                        false
                    }
                    Focus::PeerAddr => {
                        let _ = model.peer_input.apply_key(&ev);
                        false
                    }
                    Focus::Region => {
                        let _ = model.region_input.apply_key(&ev);
                        false
                    }
                    Focus::None => false,
                };
                if changed {
                    commit_edits(&mut model, h);
                }
            }
            Msg::EditorPointer(ev) => {
                let metrics = EditorMetrics::for_font_size(13.0);
                match ev {
                    PointerEvent::Click { x, y } => {
                        let (line, col) = metrics.screen_to_pos(x, y, model.body.scroll_offset);
                        model.body.set_caret_at(line, col);
                    }
                    PointerEvent::Drag { initial_x, initial_y, dx, dy } => {
                        let (l0, c0) = metrics.screen_to_pos(
                            initial_x,
                            initial_y,
                            model.body.scroll_offset,
                        );
                        let (l1, c1) = metrics.screen_to_pos(
                            initial_x + dx,
                            initial_y + dy,
                            model.body.scroll_offset,
                        );
                        model.body.set_caret_at(l0, c0);
                        model.body.extend_selection_to(l1, c1);
                    }
                }
                model.focus = Focus::Body;
            }
            Msg::MenuOpen(idx) => {
                model.menu_open = idx;
                model.menu_active = usize::MAX;
                model.edit_menu = None;
                if idx.is_some() {
                    model.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(h, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuCommand(cmd) => {
                return handle_menu_command(model, cmd, h);
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    model.menu_active = menubar_nav(&menu, mi, model.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    if let Some(cmd) = menubar_command_at(&menu, mi, model.menu_active) {
                        return handle_menu_command(model, cmd, h);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::EditNav(dir) => {
                let flags = focused_edit_flags(&model);
                model.edit_active = editmenu::edit_menu_step(flags, model.edit_active, dir);
            }
            Msg::EditActivate => {
                let flags = focused_edit_flags(&model);
                if let Some(action) = editmenu::edit_menu_action_at(flags, model.edit_active) {
                    return apply_edit_menu_action(model, action, h);
                }
            }
            Msg::EditMenuOpen(x, y) => {
                model.edit_menu = Some((x, y));
                model.edit_active = usize::MAX;
                model.menu_open = None;
                model.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(h, motion::FAST, || Msg::MenuTick);
            }
            Msg::EditMenuAction(action) => {
                return apply_edit_menu_action(model, action, h);
            }
            Msg::CloseMenus => {
                model.menu_open = None;
                model.menu_active = usize::MAX;
                model.edit_menu = None;
                model.edit_active = usize::MAX;
            }
            Msg::MapPan(dx, dy) => {
                let z = model.cam_zoom.max(0.01);
                model.cam_pan.0 += dx / z;
                model.cam_pan.1 += dy / z;
            }
            Msg::MapZoom(dy) => {
                let factor = (1.0 - dy * 0.12).clamp(0.5, 2.0);
                model.cam_zoom = (model.cam_zoom * factor).clamp(0.15, 6.0);
            }
            Msg::MapClick(lx, ly, rw, rh) => {
                model.canvas_size = (rw, rh);
                if let Some(id) = pick_note(&model, lx, ly, rw, rh) {
                    commit_edits(&mut model, h);
                    reinforce_and_touch(&mut model, id);
                    select(&mut model, id);
                    persist(&model);
                }
            }
            Msg::ToggleList => {
                model.show_list = !model.show_list;
            }
            Msg::Deselect => {
                commit_edits(&mut model, h);
                deselect(&mut model);
                persist(&model);
            }
            Msg::BeginNaming(x, y) => {
                model.naming = Some((x, y));
                model.region_input.clear();
                model.focus = Focus::Region;
            }
            Msg::CommitNaming => {
                if let Some((x, y)) = model.naming.take() {
                    let name = model.region_input.text().trim().to_string();
                    if !name.is_empty() {
                        model.regions.push(crate::modelo::Region { name, x, y });
                        persist(&model);
                    }
                }
                model.region_input.clear();
                model.focus = Focus::None;
            }
            Msg::CancelNaming => {
                model.naming = None;
                model.region_input.clear();
                model.focus = Focus::None;
            }
            Msg::EscapeMap => {
                if model.naming.is_some() {
                    model.naming = None;
                    model.region_input.clear();
                    model.focus = Focus::None;
                } else if model.selected.is_some() {
                    commit_edits(&mut model, h);
                    deselect(&mut model);
                    persist(&model);
                } else if model.show_list {
                    model.show_list = false;
                } else {
                    model.focus = Focus::None;
                }
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let palette = ListPalette::from_theme(&model.theme);
        let input_palette = TextInputPalette::from_theme(&model.theme);
        let editor_palette = EditorPalette::from_theme(&model.theme);
        let viewport = (INITIAL_W as f32, INITIAL_H as f32);

        // Prompt de passphrase: ocupa toda la ventana hasta resolverse.
        if model.unlocking {
            let mut children = vec![header_view(model), unlock_view(model, &input_palette)];
            if let Some(bar) = status_bar(model) {
                children.push(bar);
            }
            return View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                ..Default::default()
            })
            .fill(model.theme.bg_app)
            .children(children);
        }

        let header = header_view(model);

        // Zoom semántico: con nota seleccionada y el mapa lo bastante cerca,
        // el nodo se "abre" como tarjeta anclada a su coordenada.
        let inplace = model
            .selected
            .filter(|_| model.cam_zoom >= ZOOM_INJECT)
            .and_then(|id| node_screen_pos(model, id).map(|p| (id, p)));

        let mut injected: Vec<View<Msg>> = Vec::new();
        if let Some((_, (nx, ny))) = inplace {
            let editor = editor_panel(model, &input_palette, &editor_palette);
            injected.push(node_card(editor, nx, ny, model.canvas_size, &model.theme));
        }
        // Sugerencias de bautizo de clústeres sin nombre.
        if inplace.is_none() {
            for (wx, wy) in unnamed_cluster_centroids(model) {
                let (sx, sy) = world_screen(model, wx, wy);
                injected.push(pinned(
                    name_region_chip(wx, wy, &model.theme),
                    sx,
                    sy,
                    132.0,
                    24.0,
                    model.canvas_size,
                ));
            }
        }
        // Input del bautizo en curso.
        if let Some((wx, wy)) = model.naming {
            let (sx, sy) = world_screen(model, wx, wy);
            injected.push(pinned(
                naming_input(model, &input_palette),
                sx,
                sy,
                220.0,
                34.0,
                model.canvas_size,
            ));
        }
        let map = gravity_panel(model, injected);
        let mut layers: Vec<View<Msg>> = vec![map];

        if model.show_list || model.receiving {
            let drawer = if model.receiving {
                receive_panel(model, &palette, &input_palette)
            } else {
                list_panel(model, &palette, &input_palette)
            };
            layers.push(overlay_left(drawer, LIST_WIDTH));
        }

        if model.selected.is_some() && inplace.is_none() {
            let editor = editor_panel(model, &input_palette, &editor_palette);
            layers.push(overlay_right(editor, EDITOR_OVERLAY_W, &model.theme));
        }

        let body = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            min_size: Size {
                width: length(0.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(layers);

        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &model.theme, viewport));

        let mut children = vec![menubar, header, body];
        if let Some(bar) = status_bar(model) {
            children.push(bar);
        }

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(model.theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::EditMenuOpen(x, y)))
        .children(children)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        let viewport = (INITIAL_W as f32, INITIAL_H as f32);
        if model.unlocking {
            return None;
        }
        if let Some((x, y)) = model.edit_menu {
            let flags = focused_edit_flags(model);
            let mut spec = editmenu::edit_context_menu(
                (x, y),
                viewport,
                &model.theme,
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            );
            spec.active = model.edit_active;
            return Some(context_menu_view_ex(
                spec,
                ContextMenuExtras {
                    appear: model.edit_anim.value(),
                    ..Default::default()
                },
            ));
        }
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &model.theme, viewport),
            model.menu_active,
            model.menu_anim.value(),
        )
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        // Menús abiertos: las flechas navegan y tienen prioridad sobre todo.
        if event.state == KeyState::Pressed {
            if let Some(mi) = model.menu_open {
                let n = app_menu(model).menus.len().max(1);
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
            if model.edit_menu.is_some() {
                return match &event.key {
                    Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                    Key::Named(NamedKey::ArrowDown) => Some(Msg::EditNav(1)),
                    Key::Named(NamedKey::ArrowUp) => Some(Msg::EditNav(-1)),
                    Key::Named(NamedKey::Enter) => Some(Msg::EditActivate),
                    _ => None,
                };
            }
        }
        // Con el prompt de passphrase abierto, las teclas son sólo suyas.
        if model.unlocking {
            if event.state == KeyState::Pressed && !event.repeat {
                if matches!(&event.key, Key::Named(NamedKey::Enter)) {
                    return Some(Msg::Unlock);
                }
                if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                    return Some(Msg::CancelUnlock);
                }
            }
            return Some(Msg::Key(event.clone()));
        }
        // En modo recibir con foco en la dirección: Enter jala, Esc cancela.
        if model.receiving && model.focus == Focus::PeerAddr {
            if event.state == KeyState::Pressed && !event.repeat {
                if matches!(&event.key, Key::Named(NamedKey::Enter)) {
                    return Some(Msg::FetchManual);
                }
                if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                    return Some(Msg::CancelPeers);
                }
            }
            return Some(Msg::Key(event.clone()));
        }
        // Bautizando una región: Enter confirma, Esc cancela.
        if model.naming.is_some() && model.focus == Focus::Region {
            if event.state == KeyState::Pressed && !event.repeat {
                if matches!(&event.key, Key::Named(NamedKey::Enter)) {
                    return Some(Msg::CommitNaming);
                }
                if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                    return Some(Msg::CancelNaming);
                }
            }
            return Some(Msg::Key(event.clone()));
        }
        // Atajo global: Ctrl+N crea nota. Esc libera foco.
        if event.state == KeyState::Pressed && !event.repeat {
            if event.modifiers.ctrl
                && matches!(&event.key, Key::Character(s) if s.eq_ignore_ascii_case("n"))
            {
                return Some(Msg::NewNote);
            }
            if matches!(&event.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::EscapeMap);
            }
        }
        Some(Msg::Key(event.clone()))
    }

    fn title() -> &'static str {
        "khipu"
    }

    fn app_id() -> Option<&'static str> {
        Some("tawasuyu.khipu")
    }

    fn initial_size() -> (u32, u32) {
        (INITIAL_W, INITIAL_H)
    }
}
