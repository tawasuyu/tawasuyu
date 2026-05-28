//! `agora-app` — UI Llimphi del ágora.
//!
//! Cuatro tiles draggables sobre el mismo `TrustGraph`: identidades,
//! atestaciones, compositor y política. Drag de la title bar de un
//! tile sobre otro los intercambia.
//!
//! ## Cómo arranca
//!
//! - Lee `~/.local/share/agora/graph.json` si existe; si no, parte vacío.
//! - Abre el [`agora_keystore::Keystore`] en `~/.local/share/agora/keys/`.
//! - La passphrase se toma de la env `AGORA_PASSPHRASE` o `"agora-dev"`
//!   por defecto (MVP — un unlock screen real queda para una iteración
//!   siguiente).
//!
//! ## Flujo típico
//!
//! 1. Tile **Identidades**: botón "nueva identidad" genera una seed
//!    CSPRNG, la cifra en el keystore y registra la pubkey en el grafo.
//!    Click en una fila la marca como *sujeto enfocado*; click en
//!    "actuar como" (visible sólo en identidades con seed propia) la
//!    elige como *firmante activo*.
//! 2. Tile **Compositor**: con un sujeto y un firmante seleccionados,
//!    edita predicado y valor; "atestar" firma y agrega al grafo.
//! 3. Tile **Atestaciones**: lista verificada del grafo. Click sobre
//!    una fila la selecciona para que la política aplique sobre su claim.
//! 4. Tile **Política**: slider `min_third_party` (0..=5) + toggle
//!    `accept_self`; veredicto en vivo abajo, basado en el claim de la
//!    atestación seleccionada.

use std::path::PathBuf;

use agora_core::{Attestation, Claim, IdentityKind, Keypair};
use agora_graph::{Corroboration, TrustGraph, TrustPolicy};
use agora_keystore::Keystore;
use agora_core::IdentityId;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_tiled::{tiled_view_reorderable, TileSpec, TiledPalette};
use rand::RngCore;

// =============================================================================
//  Modelo
// =============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tile {
    Identidades,
    Compositor,
    Atestaciones,
    Politica,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ComposeField {
    Predicate,
    Value,
}

struct Model {
    graph: TrustGraph,
    keystore: Keystore,
    /// Seeds en RAM para las identidades "mías" (las que tienen archivo
    /// en el keystore). Se desbloquean al arrancar con la passphrase y
    /// se mantienen aquí mientras corre el proceso. No persisten al
    /// salir — siguen viviendo cifradas en el keystore.
    seeds: std::collections::HashMap<IdentityId, [u8; 32]>,
    passphrase: String,
    store_path: PathBuf,
    tiles_order: Vec<Tile>,

    /// Identidad seleccionada como sujeto (objetivo del próximo claim).
    focused_subject: Option<IdentityId>,
    /// Identidad firmante activa (debe estar en `seeds`).
    active_signer: Option<IdentityId>,
    /// Atestación seleccionada en el tile de atestaciones, por índice.
    selected_attestation: Option<usize>,

    compose_predicate: TextInputState,
    compose_value: TextInputState,
    compose_focus: ComposeField,
    /// Último mensaje al pie del compositor (éxito, error, hint).
    compose_status: String,

    policy: TrustPolicy,
}

impl Model {
    fn save_graph(&self) {
        if let Err(e) = agora_store::save(&self.store_path, &self.graph) {
            eprintln!("agora-app: no pude persistir el grafo: {e}");
        }
    }

    fn is_mine(&self, id: IdentityId) -> bool {
        self.seeds.contains_key(&id)
    }

    fn signer_keypair(&self) -> Option<Keypair> {
        self.active_signer
            .and_then(|id| self.seeds.get(&id).copied())
            .map(Keypair::from_seed)
    }
}

#[derive(Clone)]
enum Msg {
    /// Reordenar tiles por drag.
    SwapTile(usize, usize),

    /// Genera una identidad nueva con un seed CSPRNG, la guarda en el
    /// keystore y la registra en el grafo.
    NuevaIdentidad,
    /// Selecciona el sujeto enfocado (objetivo del próximo claim).
    FocoSujeto(IdentityId),
    /// Cambia el firmante activo (entre las identidades mías).
    ActuarComo(IdentityId),
    /// Selecciona una atestación para que la política evalúe su claim.
    SeleccionarAtestacion(usize),

    /// Cambia el campo focado en el compositor.
    FocoCompose(ComposeField),
    /// Tecla aplicada al input focado.
    EditCompose(KeyEvent),
    /// Firma + agrega la atestación con los valores actuales y persiste.
    Atestar,

    /// Drag del slider de `min_third_party`. Acumula el delta.
    SliderMinThird(DragPhase, f32),
    /// Toggle de `accept_self`.
    ToggleAcceptSelf,
}

// =============================================================================
//  App
// =============================================================================

struct AgoraApp;

impl App for AgoraApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "ágora · red de confianza federada"
    }

    fn initial_size() -> (u32, u32) {
        (1200, 760)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let data_dir = directories::ProjectDirs::from("net", "gioser", "agora")
            .map(|p| p.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        std::fs::create_dir_all(&data_dir).ok();
        let store_path = data_dir.join("graph.json");

        let passphrase = std::env::var("AGORA_PASSPHRASE").unwrap_or_else(|_| {
            eprintln!(
                "agora-app: usando passphrase de desarrollo \"agora-dev\". \
                 Setear AGORA_PASSPHRASE para producción."
            );
            "agora-dev".to_string()
        });

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

        // Desbloquear todas las identidades mías al arrancar. Si una
        // falla, se omite — el usuario verá que no puede firmar con ella.
        let mut seeds = std::collections::HashMap::new();
        for id in keystore.list().unwrap_or_default() {
            match keystore.load(id, &passphrase) {
                Ok(seed) => {
                    seeds.insert(id, seed);
                }
                Err(e) => {
                    eprintln!("agora-app: no pude desbloquear {id}: {e}");
                }
            }
        }

        // Si el grafo no registra una identidad mía conocida (p. ej. el
        // archivo se borró pero el keystore sobrevivió), la registramos
        // de nuevo como Person con un nombre genérico.
        let to_register: Vec<_> = seeds
            .iter()
            .filter(|(id, _)| graph.identity(**id).is_none())
            .map(|(_id, seed)| {
                let kp = Keypair::from_seed(*seed);
                let n = graph.identity_count();
                kp.identity(IdentityKind::Person, format!("yo {}", n + 1))
            })
            .collect();
        let mut graph = graph;
        for ident in to_register {
            graph.register(ident);
        }

        let active_signer = seeds.keys().next().copied();

        Model {
            graph,
            keystore,
            seeds,
            passphrase,
            store_path,
            tiles_order: vec![
                Tile::Identidades,
                Tile::Compositor,
                Tile::Atestaciones,
                Tile::Politica,
            ],
            focused_subject: None,
            active_signer,
            selected_attestation: None,
            compose_predicate: TextInputState::new(),
            compose_value: TextInputState::new(),
            compose_focus: ComposeField::Predicate,
            compose_status: String::new(),
            policy: TrustPolicy::default(),
        }
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
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

            Msg::FocoSujeto(id) => {
                model.focused_subject = Some(id);
            }

            Msg::ActuarComo(id) => {
                if model.seeds.contains_key(&id) {
                    model.active_signer = Some(id);
                }
            }

            Msg::SeleccionarAtestacion(idx) => {
                if idx < model.graph.attestations().len() {
                    model.selected_attestation = Some(idx);
                }
            }

            Msg::FocoCompose(field) => {
                model.compose_focus = field;
            }

            Msg::EditCompose(ev) => match model.compose_focus {
                ComposeField::Predicate => {
                    model.compose_predicate.apply_key(&ev);
                }
                ComposeField::Value => {
                    model.compose_value.apply_key(&ev);
                }
            },

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
                        model.compose_status =
                            "predicate y value son obligatorios".into();
                    }
                    (Some(kp), Some(subject)) => {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let claim = Claim::new(subject, predicate, value, now);
                        let att = Attestation::create(&kp, claim);
                        match model.graph.add_attestation(att) {
                            Ok(()) => {
                                model.compose_predicate.clear();
                                model.compose_value.clear();
                                model.compose_status = "atestación agregada y persistida".into();
                                model.save_graph();
                            }
                            Err(e) => {
                                model.compose_status = format!("rechazada: {e}");
                            }
                        }
                    }
                }
            }

            Msg::SliderMinThird(_phase, dv) => {
                let cur = model.policy.min_third_party as f32 + dv;
                let new = cur.clamp(0.0, 5.0).round() as usize;
                model.policy.min_third_party = new;
            }

            Msg::ToggleAcceptSelf => {
                model.policy.accept_self = !model.policy.accept_self;
            }
        }
        model
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // Tab cicla campo focado en el compositor.
        if let Key::Named(NamedKey::Tab) = &event.key {
            return Some(Msg::FocoCompose(match model.compose_focus {
                ComposeField::Predicate => ComposeField::Value,
                ComposeField::Value => ComposeField::Predicate,
            }));
        }
        // Enter sobre el compositor firma.
        if let Key::Named(NamedKey::Enter) = &event.key {
            return Some(Msg::Atestar);
        }
        Some(Msg::EditCompose(event.clone()))
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = TiledPalette::from_theme(&theme);

        let tiles: Vec<TileSpec<Msg>> = model
            .tiles_order
            .iter()
            .map(|t| match t {
                Tile::Identidades => TileSpec {
                    label: "identidades".into(),
                    content: identidades_view(model, &theme),
                },
                Tile::Compositor => TileSpec {
                    label: "compositor".into(),
                    content: compositor_view(model, &theme),
                },
                Tile::Atestaciones => TileSpec {
                    label: "atestaciones".into(),
                    content: atestaciones_view(model, &theme),
                },
                Tile::Politica => TileSpec {
                    label: "política".into(),
                    content: politica_view(model, &theme),
                },
            })
            .collect();

        tiled_view_reorderable(
            tiles,
            |from, to| Some(Msg::SwapTile(from, to)),
            &palette,
        )
    }
}

// =============================================================================
//  Tile: Identidades
// =============================================================================

fn identidades_view(model: &Model, theme: &Theme) -> View<Msg> {
    let list_palette = ListPalette::from_theme(theme);

    // Orden estable: por id bytes, ya que `graph.identities()` itera
    // sobre el HashMap interno.
    let mut idents: Vec<_> = model.graph.identities().collect();
    idents.sort_by(|a, b| a.id().as_bytes().cmp(b.id().as_bytes()));

    let rows: Vec<ListRow<Msg>> = idents
        .iter()
        .map(|ident| {
            let id = ident.id();
            let prefix = if model.is_mine(id) { "★ " } else { "  " };
            let active = Some(id) == model.active_signer;
            let mark_active = if active { " ← activa" } else { "" };
            let kind = match ident.kind {
                IdentityKind::Person => "persona",
                IdentityKind::Community => "comunidad",
                IdentityKind::Alliance => "alianza",
                IdentityKind::Institution => "institución",
            };
            ListRow {
                label: format!(
                    "{prefix}{id}  {kind}  {name}{mark_active}",
                    name = ident.display_name
                ),
                selected: model.focused_subject == Some(id),
                on_click: Msg::FocoSujeto(id),
            }
        })
        .collect();

    let caption = format!(
        "{} identidades · {} mías · enfocada: {}",
        idents.len(),
        model.seeds.len(),
        model
            .focused_subject
            .map(|id| format!("{id}"))
            .unwrap_or_else(|| "—".into())
    );

    let list = list_view(ListSpec {
        rows,
        total: idents.len(),
        caption: Some(caption),
        truncated_hint: None,
        row_height: 22.0,
        palette: list_palette,
    });

    let mut footer_buttons: Vec<View<Msg>> = vec![button_styled(
        "+ nueva identidad",
        Style {
            size: Size {
                width: percent(0.5_f32),
                height: length(30.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_primary(theme),
        Msg::NuevaIdentidad,
    )];

    // Botón "actuar como" — sólo se habilita si la enfocada es mía y
    // distinta de la activa actual.
    let can_act_as = model
        .focused_subject
        .filter(|id| model.is_mine(*id) && Some(*id) != model.active_signer);
    if let Some(id) = can_act_as {
        footer_buttons.push(button_styled(
            "actuar como ★ enfocada",
            Style {
                size: Size {
                    width: percent(0.5_f32),
                    height: length(30.0_f32),
                },
                padding: edge_padding(10.0, 0.0),
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            },
            Alignment::Center,
            &button_palette_secondary(theme),
            Msg::ActuarComo(id),
        ));
    }

    let footer = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(38.0_f32),
        },
        flex_shrink: 0.0,
        padding: edge_padding(8.0, 4.0),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(footer_buttons);

    column(vec![grow(list), footer])
}

// =============================================================================
//  Tile: Compositor
// =============================================================================

fn compositor_view(model: &Model, theme: &Theme) -> View<Msg> {
    let input_palette = TextInputPalette::from_theme(theme);

    let signer_line = format!(
        "yo: {}",
        model
            .active_signer
            .map(|id| format!("★ {id}"))
            .unwrap_or_else(|| "(ninguna — creá una identidad)".into())
    );
    let subject_line = format!(
        "sobre: {}",
        model
            .focused_subject
            .map(|id| format!("{id}"))
            .unwrap_or_else(|| "(elegí una identidad en el tile de la izquierda)".into())
    );

    let header_signer = label_line(&signer_line, 13.0, theme.fg_text);
    let header_subject = label_line(&subject_line, 13.0, theme.fg_text);

    let label_predicate = label_line("predicado", 10.0, theme.fg_muted);
    let label_value = label_line("valor", 10.0, theme.fg_muted);

    let input_predicate = input_row(
        &model.compose_predicate,
        "nacionalidad / miembro-de / habilidad …",
        model.compose_focus == ComposeField::Predicate,
        &input_palette,
        Msg::FocoCompose(ComposeField::Predicate),
    );
    let input_value = input_row(
        &model.compose_value,
        "venezolana / El Valle / soldadura …",
        model.compose_focus == ComposeField::Value,
        &input_palette,
        Msg::FocoCompose(ComposeField::Value),
    );

    let firmar = button_styled(
        "atestar (Enter)",
        Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(34.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_primary(theme),
        Msg::Atestar,
    );

    let status_color = if model.compose_status.starts_with("atestación") {
        theme.accent
    } else if model.compose_status.is_empty() {
        theme.fg_muted
    } else {
        theme.fg_destructive
    };
    let status = label_line(
        if model.compose_status.is_empty() {
            "Tab cicla campos · Enter firma"
        } else {
            &model.compose_status
        },
        11.0,
        status_color,
    );

    column(vec![
        spacer(6.0),
        header_signer,
        header_subject,
        spacer(8.0),
        label_predicate,
        input_predicate,
        spacer(4.0),
        label_value,
        input_value,
        spacer(8.0),
        firmar,
        spacer(6.0),
        status,
        grow(empty()),
    ])
}

fn input_row(
    state: &TextInputState,
    placeholder: &str,
    focused: bool,
    palette: &TextInputPalette,
    on_focus: Msg,
) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(32.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![text_input_view(state, placeholder, focused, palette, on_focus)])
}

// =============================================================================
//  Tile: Atestaciones
// =============================================================================

fn atestaciones_view(model: &Model, theme: &Theme) -> View<Msg> {
    let list_palette = ListPalette::from_theme(theme);

    let rows: Vec<ListRow<Msg>> = model
        .graph
        .attestations()
        .iter()
        .enumerate()
        .map(|(idx, att)| {
            let mark = if att.is_self_attested() { "[self]" } else { "      " };
            let attester_name = model
                .graph
                .identity(att.attester)
                .map(|i| i.display_name.as_str())
                .unwrap_or("?");
            ListRow {
                label: format!(
                    "{mark}  {att}  ←  {attester} · {pred} = {val}",
                    att = att.attester,
                    attester = attester_name,
                    pred = att.claim.predicate,
                    val = att.claim.value,
                ),
                selected: model.selected_attestation == Some(idx),
                on_click: Msg::SeleccionarAtestacion(idx),
            }
        })
        .collect();

    let total = rows.len();
    let caption = format!(
        "{total} atestaciones verificadas · seleccioná una para evaluar política"
    );

    list_view(ListSpec {
        rows,
        total,
        caption: Some(caption),
        truncated_hint: None,
        row_height: 22.0,
        palette: list_palette,
    })
}

// =============================================================================
//  Tile: Política
// =============================================================================

fn politica_view(model: &Model, theme: &Theme) -> View<Msg> {
    let slider_palette = SliderPalette::from_theme(theme);

    let slider = slider_view(
        "min terceros",
        model.policy.min_third_party as f32,
        0.0,
        5.0,
        &slider_palette,
        |phase, dv| Some(Msg::SliderMinThird(phase, dv)),
    );

    let toggle_label = format!(
        "accept_self: {}",
        if model.policy.accept_self { "sí" } else { "no" }
    );
    let toggle = button_styled(
        toggle_label,
        Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(30.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_secondary(theme),
        Msg::ToggleAcceptSelf,
    );

    // Veredicto sobre la atestación seleccionada.
    let verdict_block = match model.selected_attestation.and_then(|i| {
        let atts = model.graph.attestations();
        atts.get(i).cloned()
    }) {
        None => column(vec![label_line(
            "seleccioná una atestación para ver el veredicto",
            12.0,
            theme.fg_muted,
        )]),
        Some(att) => {
            let cor: Corroboration = model.graph.corroboration(
                att.claim.subject,
                &att.claim.predicate,
                &att.claim.value,
            );
            let ok = model.policy.accepts(&cor);
            let veredicto_color = if ok { theme.accent } else { theme.fg_destructive };
            let veredicto = label_line(
                if ok { "ACEPTA" } else { "rechaza" },
                26.0,
                veredicto_color,
            );

            column(vec![
                label_line(
                    &format!("claim: {} = {}", att.claim.predicate, att.claim.value),
                    12.0,
                    theme.fg_text,
                ),
                label_line(
                    &format!("sujeto: {}", att.claim.subject),
                    11.0,
                    theme.fg_muted,
                ),
                spacer(4.0),
                label_line(
                    &format!(
                        "atestadores: {} (terceros: {}, auto: {})",
                        cor.total(),
                        cor.third_party(),
                        if cor.self_attested { "sí" } else { "no" }
                    ),
                    11.0,
                    theme.fg_muted,
                ),
                spacer(6.0),
                veredicto,
            ])
        }
    };

    column(vec![
        spacer(8.0),
        slider,
        spacer(8.0),
        toggle,
        spacer(12.0),
        verdict_block,
        grow(empty()),
    ])
}

// =============================================================================
//  Helpers de layout y paletas
// =============================================================================

fn column<Msg: 'static>(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: edge_padding(10.0, 6.0),
        gap: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

fn grow<Msg: 'static>(v: View<Msg>) -> View<Msg> {
    let mut v = v;
    v.style.flex_grow = 1.0;
    v.style.flex_basis = length(0.0_f32);
    v.style.min_size = Size {
        width: length(0.0_f32),
        height: length(0.0_f32),
    };
    v
}

fn empty<Msg: 'static>() -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
}

fn spacer<Msg: 'static>(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
}

fn label_line<Msg: 'static>(text: &str, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(size + 8.0),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text.to_string(), size, color, Alignment::Start)
}

fn edge_padding(h: f32, v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(h),
        right: length(h),
        top: length(v),
        bottom: length(v),
    }
}

fn button_palette_primary(t: &Theme) -> ButtonPalette {
    ButtonPalette {
        bg: t.accent,
        bg_hover: t.bg_button_hover,
        fg: t.bg_app,
        radius: 4.0,
    }
}

fn button_palette_secondary(t: &Theme) -> ButtonPalette {
    ButtonPalette {
        bg: t.bg_button,
        bg_hover: t.bg_button_hover,
        fg: t.fg_text,
        radius: 4.0,
    }
}

// =============================================================================
//  Entrypoint
// =============================================================================

fn main() {
    llimphi_ui::run::<AgoraApp>();
}
