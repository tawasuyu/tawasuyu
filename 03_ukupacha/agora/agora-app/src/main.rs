//! `agora-app` — UI Llimphi del ágora.
//!
//! Una ventana con tiles draggables sobre el mismo `TrustGraph`. Drag de la
//! title bar de un tile sobre otro los intercambia. Dos familias de tiles:
//!
//! - **Sustrato social** — identidades · compositor · atestaciones · política
//!   · multifirma: construyen y evalúan la web-of-trust por-lector.
//! - **Plano de control de wawa** (el norte, ver `ARQUITECTURA.md`) — release ·
//!   capacidad: firman y verifican los sobres Ed25519 que el kernel honra
//!   (`ManifiestoFirmado`, `ConcesionCapacidad`), reusando `agora-channel`.
//!
//! ## Cómo arranca
//!
//! - Lee `~/.local/share/agora/graph.json` si existe; si no, parte vacío.
//! - Abre el `Keystore` en `~/.local/share/agora/keys/`.
//! - La passphrase sale de `AGORA_PASSPHRASE`; sin esa env y con keystore no
//!   vacío, muestra la pantalla de unlock. Keystore vacío ⇒ entra directo.
//!
//! El código vive partido en módulos: [`model`] (estado + mensajes), [`ui`]
//! (helpers/paletas/hex + pantallas transversales) y [`tiles`] (un archivo por
//! tile). Este archivo deja sólo el `App` impl, el `update` y el watcher.

mod model;
mod tiles;
mod ui;

use std::path::PathBuf;

use agora_core::{Attestation, Claim, IdentityKind, Keypair, MultiSignature};
use agora_graph::{TrustGraph, TrustPolicy};
use agora_keystore::Keystore;
use format::{ConcesionCapacidad, ManifiestoFirmado};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, FlexDirection, Size, Style};
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_motion::{animate, motion, Tween};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_text_input::TextInputState;
use llimphi_widget_tiled::{tiled_view_reorderable, TileSpec, TiledPalette};
use rand::RngCore;

use crate::model::{
    ComposeField, FocusedInput, Model, Msg, Screen, StatusLevel, Tile, TILES_INICIALES,
};
use crate::ui::{bytes_to_hex, hex_to_bytes, parse_hash32, status_layout, unlock_view};

struct AgoraApp;

impl App for AgoraApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "ágora · raíz de confianza ejecutable"
    }

    fn initial_size() -> (u32, u32) {
        (1280, 820)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let data_dir = directories::ProjectDirs::from("net", "tawasuyu", "agora")
            .map(|p| p.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        std::fs::create_dir_all(&data_dir).ok();
        let store_path = data_dir.join("graph.json");

        // Watcher del directorio padre — vigila renames y crear/borrar del
        // archivo aunque aún no exista (agora-store::save es tmp+rename).
        arranca_watcher(handle.clone(), data_dir.clone());

        let env_pass = std::env::var("AGORA_PASSPHRASE").ok();
        let keystore = Keystore::open_default()
            .unwrap_or_else(|e| panic!("agora-app: no pude abrir el keystore: {e}"));
        let graph = if store_path.exists() {
            agora_store::load(&store_path).unwrap_or_else(|e| {
                eprintln!("agora-app: no pude cargar el grafo ({e}); empiezo vacío.");
                TrustGraph::new()
            })
        } else {
            TrustGraph::new()
        };

        let ids_keystore = keystore.list().unwrap_or_default();
        let necesita_unlock = !ids_keystore.is_empty() && env_pass.is_none();

        let mut model = Model {
            graph,
            keystore,
            seeds: std::collections::HashMap::new(),
            passphrase: env_pass.unwrap_or_else(|| "agora-dev".to_string()),
            store_path,
            screen: if necesita_unlock {
                Screen::Unlock {
                    input: TextInputState::masked(),
                    status: String::new(),
                }
            } else {
                Screen::Main
            },
            tiles_order: TILES_INICIALES.to_vec(),
            focused_subject: None,
            active_signer: None,
            selected_attestation: None,
            compose_predicate: TextInputState::new(),
            compose_value: TextInputState::new(),
            focused_input: FocusedInput::Compose(ComposeField::Predicate),
            compose_status: String::new(),
            policy: TrustPolicy::default(),
            multi_message: TextInputState::new(),
            multi_selected: std::collections::BTreeSet::new(),
            multi_threshold: 1,
            multi_current: None,
            release_hash: TextInputState::new(),
            release_paste: TextInputState::new(),
            release_current: None,
            release_status: String::new(),
            cap_bytecode: TextInputState::new(),
            cap_permisos: 0,
            cap_paste: TextInputState::new(),
            cap_current: None,
            cap_status: String::new(),
            status: None,
            menu_open: None,
            edit_menu: None,
            clipboard: llimphi_clipboard::SystemClipboard::new(),
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
        };

        if !necesita_unlock {
            model.desbloquear_seeds_silencioso();
            model.active_signer = model.seeds.keys().next().copied();
            model.registrar_identidades_huerfanas();
        }
        model
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::SwapTile(from, to) => {
                if from != to && from < model.tiles_order.len() && to < model.tiles_order.len() {
                    model.tiles_order.swap(from, to);
                }
            }

            Msg::NuevaIdentidad => {
                let mut seed = [0u8; 32];
                rand::thread_rng().fill_bytes(&mut seed);
                let kp = Keypair::from_seed(seed);
                let id = kp.identity_id();
                match model.keystore.save(id, &seed, &model.passphrase) {
                    Ok(()) => {
                        model.seeds.insert(id, seed);
                        let n = model.graph.identity_count();
                        model
                            .graph
                            .register(kp.identity(IdentityKind::Person, format!("yo {}", n + 1)));
                        if model.active_signer.is_none() {
                            model.active_signer = Some(id);
                        }
                        model.save_graph();
                        model.compose_status = format!("identidad nueva: {id}");
                    }
                    Err(e) => {
                        model.compose_status = format!("no pude guardar la seed: {e}");
                    }
                }
            }

            Msg::FocoSujeto(id) => model.focused_subject = Some(id),

            Msg::ActuarComo(id) => {
                if model.seeds.contains_key(&id) {
                    model.active_signer = Some(id);
                    // Las firmas vigentes eran del firmante anterior.
                    model.release_current = None;
                    model.cap_current = None;
                }
            }

            Msg::SeleccionarAtestacion(idx) => {
                if idx < model.graph.attestations().len() {
                    model.selected_attestation = Some(idx);
                }
            }

            Msg::Foco(f) => model.focused_input = f,

            Msg::EditFocused(ev) => model.edit_focused(&ev),

            Msg::Atestar => {
                let signer = model.signer_keypair();
                let subject = model.focused_subject;
                let predicate = model.compose_predicate.text();
                let value = model.compose_value.text();
                let predicate = predicate.trim();
                let value = value.trim();
                match (signer, subject) {
                    (None, _) => {
                        model.compose_status =
                            "no hay firmante activo (creá una identidad o seleccioná \"actuar como\")".into();
                    }
                    (_, None) => {
                        model.compose_status = "elegí un sujeto en el tile de identidades".into();
                    }
                    _ if predicate.is_empty() || value.is_empty() => {
                        model.compose_status = "predicate y value son obligatorios".into();
                    }
                    (Some(kp), Some(subject)) => {
                        let now = ahora();
                        let claim = Claim::new(subject, predicate, value, now);
                        let att = Attestation::create(&kp, claim);
                        match model.graph.add_attestation(att.clone()) {
                            Ok(()) => {
                                model.compose_predicate.clear();
                                model.compose_value.clear();
                                model.compose_status = "atestación agregada y persistida".into();
                                if let Err(e) =
                                    agora_store::append_attestation(&model.store_path, &att)
                                {
                                    model.set_status(
                                        StatusLevel::Error,
                                        format!("no pude appendear la atestación: {e}"),
                                    );
                                }
                            }
                            Err(e) => model.compose_status = format!("rechazada: {e}"),
                        }
                    }
                }
            }

            Msg::SliderMinThird(_phase, dv) => {
                let cur = model.policy.min_third_party as f32 + dv;
                model.policy.min_third_party = cur.clamp(0.0, 5.0).round() as usize;
            }
            Msg::ToggleAcceptSelf => model.policy.accept_self = !model.policy.accept_self,
            Msg::CycleKind => {
                model.policy.min_attesters_of_kind = match model.policy.min_attesters_of_kind {
                    None => Some((IdentityKind::Person, 1)),
                    Some((IdentityKind::Person, n)) => Some((IdentityKind::Community, n)),
                    Some((IdentityKind::Community, n)) => Some((IdentityKind::Alliance, n)),
                    Some((IdentityKind::Alliance, n)) => Some((IdentityKind::Institution, n)),
                    Some((IdentityKind::Institution, _)) => None,
                };
            }
            Msg::SliderMinKind(_phase, dv) => {
                if let Some((kind, n)) = model.policy.min_attesters_of_kind {
                    let cur = n as f32 + dv;
                    model.policy.min_attesters_of_kind = Some((kind, cur.clamp(1.0, 5.0).round() as usize));
                }
            }
            Msg::CycleMaxAge => {
                model.policy.max_age_secs = match model.policy.max_age_secs {
                    None => Some(60),
                    Some(60) => Some(300),
                    Some(300) => Some(3_600),
                    Some(3_600) => Some(86_400),
                    Some(86_400) => Some(604_800),
                    _ => None,
                };
            }

            Msg::UnlockKey(ev) => {
                if let Screen::Unlock { input, .. } = &mut model.screen {
                    input.apply_key(&ev);
                }
            }
            Msg::UnlockSubmit => {
                let pass_intentada = if let Screen::Unlock { input, .. } = &model.screen {
                    Some(input.text())
                } else {
                    None
                };
                if let Some(pass) = pass_intentada {
                    if model.intentar_unlock(&pass) {
                        model.registrar_identidades_huerfanas();
                        model.active_signer = model.seeds.keys().next().copied();
                        model.screen = Screen::Main;
                    } else if let Screen::Unlock { input, status } = &mut model.screen {
                        *status = "passphrase incorrecta".into();
                        input.clear();
                    }
                }
            }

            Msg::ArchivoCambio => recargar_grafo(&mut model),

            Msg::DescartarStatus => model.status = None,

            // ---- Multifirma ----------------------------------------------
            Msg::ToggleMultiFirmante(id) => {
                if !model.seeds.contains_key(&id) {
                    return model; // sólo identidades propias
                }
                if !model.multi_selected.insert(id) {
                    model.multi_selected.remove(&id);
                }
                model.multi_current = None;
                let n = model.multi_selected.len();
                if n == 0 {
                    model.multi_threshold = 1;
                } else if model.multi_threshold > n {
                    model.multi_threshold = n;
                } else if model.multi_threshold == 0 {
                    model.multi_threshold = 1;
                }
            }
            Msg::SliderMultiUmbral(_phase, dv) => {
                let n = model.multi_selected.len().max(1);
                let cur = model.multi_threshold as f32 + dv;
                model.multi_threshold = cur.clamp(1.0, n as f32).round() as usize;
            }
            Msg::FirmarMulti => {
                if model.multi_selected.is_empty() {
                    model.set_status(StatusLevel::Error, "elegí al menos una identidad propia como firmante");
                    return model;
                }
                let mensaje_txt = model.multi_message.text();
                if mensaje_txt.trim().is_empty() {
                    model.set_status(StatusLevel::Error, "el mensaje de la multifirma no puede estar vacío");
                    return model;
                }
                let pares: Vec<Keypair> = model
                    .multi_selected
                    .iter()
                    .filter_map(|id| model.seeds.get(id).copied().map(Keypair::from_seed))
                    .collect();
                let refs: Vec<&Keypair> = pares.iter().collect();
                model.multi_current = Some(MultiSignature::create(&refs, mensaje_txt.as_bytes()));
            }
            Msg::LimpiarMulti => {
                model.multi_selected.clear();
                model.multi_threshold = 1;
                model.multi_current = None;
                model.multi_message.clear();
            }
            Msg::ExportarMulti => match model.multi_current.as_ref() {
                None => model.set_status(StatusLevel::Error, "no hay multifirma vigente — firmá primero"),
                Some(multi) => match postcard::to_allocvec(multi) {
                    Ok(bytes) => {
                        let hex = bytes_to_hex(&bytes);
                        model.set_status(
                            StatusLevel::Info,
                            format!("multifirma postcard ({} bytes): {hex}", bytes.len()),
                        );
                    }
                    Err(e) => model.set_status(StatusLevel::Error, format!("no pude serializar la multifirma: {e}")),
                },
            },

            // ---- Release (plano de control wawa) -------------------------
            Msg::FirmarRelease => {
                let kp = match model.signer_keypair() {
                    Some(kp) => kp,
                    None => {
                        model.release_status = "no hay firmante activo — creá una identidad".into();
                        return model;
                    }
                };
                match parse_hash32(&model.release_hash.text()) {
                    Some(hash) => {
                        model.release_current = Some(agora_channel::firmar_manifiesto(&kp, &hash));
                        model.release_status = "✓ release firmado (exportá el postcard para mudanza)".into();
                    }
                    None => {
                        model.release_status = "hash inválido — esperaba 64 dígitos hex".into();
                    }
                }
            }
            Msg::VerificarRelease => {
                let bytes = match hex_to_bytes(&model.release_paste.text()) {
                    Some(b) => b,
                    None => {
                        model.release_status = "✗ hex inválido (longitud impar o dígito no-hex)".into();
                        return model;
                    }
                };
                model.release_status = match ManifiestoFirmado::deserializar(&bytes) {
                    Ok(mf) => match agora_channel::verificar_manifiesto(&mf) {
                        Ok(()) => format!("✓ firma válida · autor {}…", &bytes_to_hex(&mf.autor)[..16]),
                        Err(_) => "✗ firma rota — el autor no firmó este hash".into(),
                    },
                    Err(e) => format!("✗ no parsea como ManifiestoFirmado: {e}"),
                };
            }
            Msg::ExportarRelease => match model.release_current.as_ref() {
                None => model.set_status(StatusLevel::Error, "no hay release vigente — firmá primero"),
                Some(mf) => match mf.serializar() {
                    Ok(bytes) => {
                        let hex = bytes_to_hex(&bytes);
                        model.set_status(
                            StatusLevel::Info,
                            format!("ManifiestoFirmado postcard ({} bytes): {hex}", bytes.len()),
                        );
                    }
                    Err(e) => model.set_status(StatusLevel::Error, format!("no pude serializar el release: {e}")),
                },
            },
            Msg::LimpiarRelease => {
                model.release_hash.clear();
                model.release_paste.clear();
                model.release_current = None;
                model.release_status.clear();
            }

            // ---- Capacidad (§14.1.3) -------------------------------------
            Msg::ToggleCapPermiso(bit) => {
                model.cap_permisos ^= bit;
                model.cap_current = None;
            }
            Msg::FirmarCapacidad => {
                let kp = match model.signer_keypair() {
                    Some(kp) => kp,
                    None => {
                        model.cap_status = "no hay firmante activo — creá una identidad".into();
                        return model;
                    }
                };
                match parse_hash32(&model.cap_bytecode.text()) {
                    Some(hash) => {
                        model.cap_current =
                            Some(agora_channel::firmar_capacidad(&kp, &hash, model.cap_permisos));
                        model.cap_status = "✓ concesión firmada (viaja con el bytecode)".into();
                    }
                    None => {
                        model.cap_status = "hash inválido — esperaba 64 dígitos hex".into();
                    }
                }
            }
            Msg::VerificarCapacidad => {
                let bytes = match hex_to_bytes(&model.cap_paste.text()) {
                    Some(b) => b,
                    None => {
                        model.cap_status = "✗ hex inválido (longitud impar o dígito no-hex)".into();
                        return model;
                    }
                };
                model.cap_status = match ConcesionCapacidad::deserializar(&bytes) {
                    Ok(c) => match agora_channel::verificar_capacidad(&c) {
                        Ok(()) => format!("✓ firma válida · autor {}…", &bytes_to_hex(&c.autor)[..16]),
                        Err(_) => "✗ firma rota — no cubre (bytecode, permisos)".into(),
                    },
                    Err(e) => format!("✗ no parsea como ConcesionCapacidad: {e}"),
                };
            }
            Msg::ExportarCapacidad => match model.cap_current.as_ref() {
                None => model.set_status(StatusLevel::Error, "no hay concesión vigente — concedé primero"),
                Some(c) => match c.serializar() {
                    Ok(bytes) => {
                        let hex = bytes_to_hex(&bytes);
                        model.set_status(
                            StatusLevel::Info,
                            format!("ConcesionCapacidad postcard ({} bytes): {hex}", bytes.len()),
                        );
                    }
                    Err(e) => model.set_status(StatusLevel::Error, format!("no pude serializar la concesión: {e}")),
                },
            },
            Msg::LimpiarCapacidad => {
                model.cap_bytecode.clear();
                model.cap_paste.clear();
                model.cap_permisos = 0;
                model.cap_current = None;
                model.cap_status.clear();
            }

            // ---- Menús ---------------------------------------------------
            Msg::MenuOpen(idx) => {
                model.menu_open = idx;
                model.edit_menu = None;
                model.menu_active = usize::MAX;
                if idx.is_some() {
                    model.menu_anim =
                        Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::CloseMenus => {
                model.menu_open = None;
                model.edit_menu = None;
                model.menu_active = usize::MAX;
                model.edit_active = usize::MAX;
            }
            Msg::MenuCommand(cmd) => {
                model.menu_open = None;
                if let Some(real) = menu_command_to_msg(&model, &cmd) {
                    return AgoraApp::update(model, real, handle);
                }
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
                        model.menu_open = None;
                        if let Some(real) = menu_command_to_msg(&model, &cmd) {
                            return AgoraApp::update(model, real, handle);
                        }
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::EditNav(dir) => {
                let editor = model.focused_input_ref().editor();
                let masked = model.focused_input_ref().is_masked();
                let flags = EditFlags::from_editor(editor, masked);
                model.edit_active = editmenu::edit_menu_step(flags, model.edit_active, dir);
            }
            Msg::EditActivate => {
                let editor = model.focused_input_ref().editor();
                let masked = model.focused_input_ref().is_masked();
                let flags = EditFlags::from_editor(editor, masked);
                if let Some(a) = editmenu::edit_menu_action_at(flags, model.edit_active) {
                    model.edit_menu = None;
                    model.apply_edit_menu_action(a);
                }
            }
            Msg::EditMenuOpen(x, y) => {
                model.edit_menu = Some((x, y));
                model.menu_open = None;
                model.edit_active = usize::MAX;
                model.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(handle, motion::FAST, || Msg::MenuTick);
            }
            Msg::EditMenuAction(action) => {
                model.edit_menu = None;
                model.apply_edit_menu_action(action);
            }
        }
        model
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
        // cierra. Tiene prioridad sobre todo lo demás.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
            match &event.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => {
                    return Some(Msg::MenuOpen(Some((mi + n - 1) % n)));
                }
                Key::Named(NamedKey::ArrowRight) => {
                    return Some(Msg::MenuOpen(Some((mi + 1) % n)));
                }
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => return Some(Msg::MenuActivate),
                _ => return None,
            }
        }
        // Menú de edición abierto: ↑/↓ navegan, Enter ejecuta, Esc cierra.
        if model.edit_menu.is_some() {
            match &event.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowDown) => return Some(Msg::EditNav(1)),
                Key::Named(NamedKey::ArrowUp) => return Some(Msg::EditNav(-1)),
                Key::Named(NamedKey::Enter) => return Some(Msg::EditActivate),
                _ => return None,
            }
        }
        // Unlock: todas las teclas al input; Enter desbloquea.
        if matches!(model.screen, Screen::Unlock { .. }) {
            if let Key::Named(NamedKey::Enter) = &event.key {
                return Some(Msg::UnlockSubmit);
            }
            return Some(Msg::UnlockKey(event.clone()));
        }
        // Tab cicla los campos del compositor; desde cualquier otro foco,
        // lleva al predicado.
        if let Key::Named(NamedKey::Tab) = &event.key {
            let destino = match model.focused_input {
                FocusedInput::Compose(ComposeField::Predicate) => {
                    FocusedInput::Compose(ComposeField::Value)
                }
                _ => FocusedInput::Compose(ComposeField::Predicate),
            };
            return Some(Msg::Foco(destino));
        }
        // Enter dispara la acción primaria del input focado.
        if let Key::Named(NamedKey::Enter) = &event.key {
            return Some(match model.focused_input {
                FocusedInput::Compose(_) => Msg::Atestar,
                FocusedInput::MultiMessage => Msg::FirmarMulti,
                FocusedInput::ReleaseHash => Msg::FirmarRelease,
                FocusedInput::ReleasePaste => Msg::VerificarRelease,
                FocusedInput::CapBytecode => Msg::FirmarCapacidad,
                FocusedInput::CapPaste => Msg::VerificarCapacidad,
            });
        }
        Some(Msg::EditFocused(event.clone()))
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        if matches!(model.screen, Screen::Unlock { .. }) {
            return unlock_view(&model.screen, &theme);
        }
        let palette = TiledPalette::from_theme(&theme);

        let tiles: Vec<TileSpec<Msg>> = model
            .tiles_order
            .iter()
            .map(|t| {
                let (label, content) = match t {
                    Tile::Identidades => ("identidades", tiles::identidades_view(model, &theme)),
                    Tile::Compositor => ("compositor", tiles::compositor_view(model, &theme)),
                    Tile::Atestaciones => ("atestaciones", tiles::atestaciones_view(model, &theme)),
                    Tile::Politica => ("política", tiles::politica_view(model, &theme)),
                    Tile::Multifirma => ("multifirma", tiles::multifirma_view(model, &theme)),
                    Tile::Release => ("release · wawa", tiles::release_view(model, &theme)),
                    Tile::Capacidad => ("capacidad · wawa", tiles::capacidad_view(model, &theme)),
                };
                TileSpec {
                    label: label.into(),
                    content,
                }
            })
            .collect();

        let tiled =
            tiled_view_reorderable(tiles, |from, to| Some(Msg::SwapTile(from, to)), &palette);

        let body = match &model.status {
            None => tiled,
            Some(banner) => status_layout(&theme, tiled, banner),
        };

        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));

        // Right-click en la raíz (origen 0,0 → coords locales == coords de
        // ventana) abre el menú de edición sobre el input focuseado.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::EditMenuOpen(x, y)))
        .children(vec![menubar, ui::grow(body)])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        if matches!(model.screen, Screen::Unlock { .. }) {
            return None;
        }
        let theme = Theme::dark();
        // El menú de edición tiene prioridad sobre el dropdown principal.
        if let Some((x, y)) = model.edit_menu {
            let editor = model.focused_input_ref().editor();
            let masked = model.focused_input_ref().is_masked();
            let flags = EditFlags::from_editor(editor, masked);
            let (w, h) = AgoraApp::initial_size();
            let mut spec = editmenu::edit_context_menu(
                (x, y),
                (w as f32, h as f32),
                &theme,
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            );
            spec.active = model.edit_active;
            return Some(context_menu_view_ex(
                spec,
                ContextMenuExtras { appear: model.edit_anim.value(), ..Default::default() },
            ));
        }
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &theme),
            model.menu_active,
            model.menu_anim.value(),
        )
    }
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(
    menu: &'a app_bus::AppMenu,
    model: &Model,
    theme: &'a Theme,
) -> MenuBarSpec<'a, Msg> {
    let (w, h) = AgoraApp::initial_size();
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: (w as f32, h as f32),
        height: MENU_H,
        on_open: std::sync::Arc::new(Msg::MenuOpen),
        on_command: std::sync::Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// Construye el menú principal reflejando el estado real: el submenú
/// Editar se grisa según el input focuseado; Archivo/Ver exponen firmar,
/// verificar, exportar, limpiar y los toggles de tile. Sólo comandos que
/// mapean a `Msg` reales (ver [`menu_command_to_msg`]).
fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};

    let editor = model.focused_input_ref().editor();
    let masked = model.focused_input_ref().is_masked();
    let has_sel = editor.has_selection();
    let can_undo = editor.can_undo();
    let can_redo = editor.can_redo();
    let has_text = !editor.is_empty();
    let can_copy = has_sel && !masked;

    let mut undo = MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z");
    if !can_undo {
        undo = undo.disabled();
    }
    let mut redo = MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Y");
    if !can_redo {
        redo = redo.disabled();
    }
    let mut cut = MenuItem::new("Cortar", "edit.cut").shortcut("Ctrl+X").separated();
    let mut copy = MenuItem::new("Copiar", "edit.copy").shortcut("Ctrl+C");
    if !can_copy {
        cut = cut.disabled();
        copy = copy.disabled();
    }
    let paste = MenuItem::new("Pegar", "edit.paste").shortcut("Ctrl+V");
    let mut sel_all = MenuItem::new("Seleccionar todo", "edit.selectall")
        .shortcut("Ctrl+A")
        .separated();
    if !has_text {
        sel_all = sel_all.disabled();
    }

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Nueva identidad", "file.nueva_identidad"))
                .item(MenuItem::new("Firmar release", "file.firmar_release").separated())
                .item(MenuItem::new("Verificar release", "file.verificar_release"))
                .item(MenuItem::new("Exportar release", "file.exportar_release"))
                .item(MenuItem::new("Firmar capacidad", "file.firmar_capacidad").separated())
                .item(MenuItem::new("Verificar capacidad", "file.verificar_capacidad"))
                .item(MenuItem::new("Exportar capacidad", "file.exportar_capacidad"))
                .item(MenuItem::new("Firmar multifirma", "file.firmar_multi").separated())
                .item(MenuItem::new("Exportar multifirma", "file.exportar_multi")),
        )
        .menu(
            Menu::new("Editar")
                .item(undo)
                .item(redo)
                .item(cut)
                .item(copy)
                .item(paste)
                .item(sel_all)
                .item(MenuItem::new("Atestar", "edit.atestar").separated()),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Limpiar release", "view.limpiar_release"))
                .item(MenuItem::new("Limpiar capacidad", "view.limpiar_capacidad"))
                .item(MenuItem::new("Limpiar multifirma", "view.limpiar_multi").separated())
                .item(MenuItem::new("Cerrar aviso de estado", "view.descartar_status")),
        )
        .menu(
            Menu::new("Ayuda")
                .item(MenuItem::new("Recargar grafo desde disco", "help.recargar")),
        )
}

/// Traduce el `command` del menú principal al `Msg` real de la app. `None`
/// si el comando no aplica (p. ej. Atestar sin sujeto se resuelve en el
/// propio `update`). Mantener en sync con [`app_menu`].
fn menu_command_to_msg(_model: &Model, command: &str) -> Option<Msg> {
    Some(match command {
        "file.nueva_identidad" => Msg::NuevaIdentidad,
        "file.firmar_release" => Msg::FirmarRelease,
        "file.verificar_release" => Msg::VerificarRelease,
        "file.exportar_release" => Msg::ExportarRelease,
        "file.firmar_capacidad" => Msg::FirmarCapacidad,
        "file.verificar_capacidad" => Msg::VerificarCapacidad,
        "file.exportar_capacidad" => Msg::ExportarCapacidad,
        "file.firmar_multi" => Msg::FirmarMulti,
        "file.exportar_multi" => Msg::ExportarMulti,
        "edit.undo" => Msg::EditMenuAction(EditAction::Undo),
        "edit.redo" => Msg::EditMenuAction(EditAction::Redo),
        "edit.cut" => Msg::EditMenuAction(EditAction::Cut),
        "edit.copy" => Msg::EditMenuAction(EditAction::Copy),
        "edit.paste" => Msg::EditMenuAction(EditAction::Paste),
        "edit.selectall" => Msg::EditMenuAction(EditAction::SelectAll),
        "edit.atestar" => Msg::Atestar,
        "view.limpiar_release" => Msg::LimpiarRelease,
        "view.limpiar_capacidad" => Msg::LimpiarCapacidad,
        "view.limpiar_multi" => Msg::LimpiarMulti,
        "view.descartar_status" => Msg::DescartarStatus,
        "help.recargar" => Msg::ArchivoCambio,
        _ => return None,
    })
}

/// Segundos UNIX actuales (0 si el reloj está antes de la época).
fn ahora() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Recarga `graph.json` tras un cambio externo (típicamente `agora-cli`).
/// `agora-store::save` es atómico (tmp+rename); si falla, deja el grafo en
/// memoria intacto y reporta por el banner.
fn recargar_grafo(model: &mut Model) {
    match agora_store::load(&model.store_path) {
        Ok(g) => {
            if let Some(id) = model.focused_subject {
                if g.identity(id).is_none() {
                    model.focused_subject = None;
                }
            }
            if let Some(idx) = model.selected_attestation {
                if idx >= g.attestations().len() {
                    model.selected_attestation = None;
                }
            }
            let antes_atts = model.graph.attestation_count();
            let antes_idents = model.graph.identity_count();
            model.graph = g;
            let delta_atts = model.graph.attestation_count() as isize - antes_atts as isize;
            let delta_idents = model.graph.identity_count() as isize - antes_idents as isize;
            if delta_atts != 0 || delta_idents != 0 {
                model.set_status(
                    StatusLevel::Info,
                    format!(
                        "grafo recargado desde disco · {delta_atts:+} atestaciones · {delta_idents:+} identidades"
                    ),
                );
            }
        }
        Err(e) => model.set_status(
            StatusLevel::Error,
            format!("no pude recargar graph.json ({e}); sigo con el grafo en memoria"),
        ),
    }
}

/// Lanza un watcher de filesystem sobre `dir` y dispatcha `Msg::ArchivoCambio`
/// en cada evento OK. El callback es coarse — el `update` decide si el cambio
/// es relevante reintentando el `load`. El `Watcher` se "leakea" con
/// `mem::forget`: vive lo que vive el proceso y es `!Send` en algunos backends.
fn arranca_watcher(handle: Handle<Msg>, dir: PathBuf) {
    use notify::{RecursiveMode, Watcher};
    let watcher_res = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            handle.dispatch(Msg::ArchivoCambio);
        }
    });
    let mut watcher = match watcher_res {
        Ok(w) => w,
        Err(e) => {
            eprintln!("agora-app: no pude arrancar el watcher ({e}); cambios externos al grafo no se reflejarán hasta reiniciar.");
            return;
        }
    };
    if let Err(e) = watcher.watch(&dir, RecursiveMode::NonRecursive) {
        eprintln!("agora-app: no pude vigilar {} ({e}); cambios externos al grafo no se reflejarán.", dir.display());
        return;
    }
    std::mem::forget(watcher);
}

fn main() {
    llimphi_ui::run::<AgoraApp>();
}
