//! `panels` — los constructores de vistas en flujo: cabecera, cajón de
//! notas (lista + búsqueda), panel de recibir (pares P2P), y el editor de
//! la nota (título/cuerpo/etiquetas + stats). Frontends puros sobre el
//! `Model`; la lógica vive en la raíz y en `map`.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Rect, Size, Style},
    AlignItems, Dimension, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_text_editor::{text_editor_view, EditorMetrics, EditorPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};
use khipu_core::{Note, NoteId};

use crate::{
    current_mass, now_secs, Focus, Model, Msg, EDITOR_VISIBLE_LINES, FIELD_LABEL_SIZE, HEADER_H,
    LIST_WIDTH, ROW_H,
};

pub(crate) fn header_view(model: &Model) -> View<Msg> {
    let title = format!("khipu · {} notas", model.store.len());
    let title_node = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(title, 14.0, model.theme.fg_text, Alignment::Start);

    let list_label = if model.show_list { "ocultar notas" } else { "☰ notas" };
    let list_btn = button(
        list_label,
        model.theme.bg_button,
        if model.show_list { model.theme.accent } else { model.theme.fg_muted },
        Msg::ToggleList,
    );
    let new_btn = button(
        "+ nueva  (Ctrl+N)",
        model.theme.bg_button,
        model.theme.fg_text,
        Msg::NewNote,
    );
    let archive_label = if model.show_archive {
        "ocultar archivo"
    } else {
        "ver archivo"
    };
    let archive_btn = button(
        archive_label,
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::ToggleArchive,
    );
    let del_btn = button(
        "borrar",
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::DeleteSelected,
    );
    let export_btn = button(
        "exportar",
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::Export,
    );
    let import_btn = button(
        "importar",
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::Import,
    );
    let publish_label = if model.publishing {
        "publicando"
    } else {
        "publicar"
    };
    let publish_btn = button(
        publish_label,
        model.theme.bg_button,
        if model.publishing {
            model.theme.accent
        } else {
            model.theme.fg_muted
        },
        Msg::Publish,
    );
    let receive_btn = button(
        "recibir",
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::Receive,
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(HEADER_H),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(0.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(model.theme.bg_panel_alt)
    .children(vec![
        title_node,
        list_btn,
        new_btn,
        archive_btn,
        del_btn,
        export_btn,
        import_btn,
        publish_btn,
        receive_btn,
    ])
}

pub(crate) fn button(label: &str, bg: Color, fg: Color, msg: Msg) -> View<Msg> {
    // El ancho crece con el largo del texto — los labels más
    // explícitos («+ nueva (Ctrl+N)», «ocultar archivo») piden más
    // espacio que un «borrar» seco.
    let chars = label.chars().count() as f32;
    let width = (chars * 7.2 + 22.0).max(86.0);
    View::new(Style {
        size: Size {
            width: length(width),
            height: length(26.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(4.0)
    .text_aligned(label.to_string(), 11.0, fg, Alignment::Center)
    .on_click(msg)
}

pub(crate) fn list_panel(
    model: &Model,
    palette: &ListPalette,
    input_palette: &TextInputPalette,
) -> View<Msg> {
    let now = now_secs();
    let query = model.search.text();
    let q = query.trim();

    // Particionamos en horizonte vs archivo y ordenamos cada parte por
    // masa viva decreciente. Si hay query, ambas listas quedan
    // pre-filtradas por coincidencia en título/cuerpo/etiquetas.
    let mut visible: Vec<(NoteId, f32, &Note)> = Vec::new();
    let mut archive: Vec<(NoteId, f32, &Note)> = Vec::new();
    let mut hidden_by_query = 0usize;
    for id in &model.order {
        let Some(n) = model.store.get(*id) else {
            continue;
        };
        if !q.is_empty() && !note_matches(n, q) {
            hidden_by_query += 1;
            continue;
        }
        let m = current_mass(&model.gravity, n, now);
        if model.gravity.is_visible(m) {
            visible.push((*id, m, n));
        } else {
            archive.push((*id, m, n));
        }
    }
    let by_mass_desc = |a: &(NoteId, f32, &Note), b: &(NoteId, f32, &Note)| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(core::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    };
    visible.sort_by(by_mass_desc);
    archive.sort_by(by_mass_desc);

    let mut chain: Vec<(NoteId, f32, &Note)> = visible.clone();
    if model.show_archive {
        chain.extend(archive.iter().cloned());
    }

    let rows: Vec<ListRow<Msg>> = chain
        .into_iter()
        .map(|(id, mass, n)| ListRow {
            label: row_label(n, mass),
            selected: Some(id) == model.selected,
            on_click: Msg::SelectNote(id),
        })
        .collect();

    let caption = if !q.is_empty() {
        format!(
            "buscar «{}» · {}/{} coinciden",
            q,
            visible.len() + if model.show_archive { archive.len() } else { 0 },
            visible.len() + archive.len() + hidden_by_query
        )
    } else if archive.is_empty() {
        format!("notas · {}", visible.len())
    } else if model.show_archive {
        format!(
            "notas · {} horizonte + {} archivo",
            visible.len(),
            archive.len()
        )
    } else {
        format!(
            "notas · {} horizonte (+{} archivo)",
            visible.len(),
            archive.len()
        )
    };

    let spec = ListSpec {
        total: rows.len(),
        rows,
        caption: Some(caption),
        truncated_hint: None,
        row_height: ROW_H,
        palette: *palette,
    };

    let search_input = text_input_view(
        &model.search,
        "buscar (título, cuerpo, etiquetas)",
        model.focus == Focus::Search,
        input_palette,
        Msg::Focus(Focus::Search),
    );
    let search_row = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(model.theme.bg_panel_alt)
    .children(vec![search_input]);

    let list_wrap = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![list_view(spec)]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(LIST_WIDTH),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![search_row, list_wrap])
}

/// Panel izquierdo en modo "recibir": arriba un input de dirección manual
/// (`host:puerto`, habilita WAN) con botones jalar/cancelar; debajo, la
/// lista de pares descubiertos en la LAN (click ⇒ jalar de él). Reemplaza
/// transitoriamente la lista de notas.
pub(crate) fn receive_panel(
    model: &Model,
    palette: &ListPalette,
    input_palette: &TextInputPalette,
) -> View<Msg> {
    // Fila de dirección manual + jalar.
    let addr_input = text_input_view(
        &model.peer_input,
        "host:puerto  o  /ip4/…/p2p/…",
        model.focus == Focus::PeerAddr,
        input_palette,
        Msg::Focus(Focus::PeerAddr),
    );
    let addr_wrap = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(26.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![addr_input]);
    let jalar = button(
        "jalar",
        model.theme.bg_button,
        model.theme.accent,
        Msg::FetchManual,
    );
    let addr_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(model.theme.bg_panel_alt)
    .children(vec![addr_wrap, jalar]);

    let cancel = button(
        "cancelar",
        model.theme.bg_button,
        model.theme.fg_muted,
        Msg::CancelPeers,
    );
    let cancel_row = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(2.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(model.theme.bg_panel_alt)
    .children(vec![cancel]);

    let rows: Vec<ListRow<Msg>> = model
        .peers
        .iter()
        .map(|p| ListRow {
            label: p.label.clone(),
            selected: false,
            on_click: Msg::FetchFrom(p.addr.clone()),
        })
        .collect();
    let caption = if model.peers.is_empty() {
        "pares en la LAN: ninguno aún".to_string()
    } else {
        format!("pares en la LAN · {} (click para jalar)", model.peers.len())
    };
    let spec = ListSpec {
        total: rows.len(),
        rows,
        caption: Some(caption),
        truncated_hint: None,
        row_height: ROW_H,
        palette: *palette,
    };
    let list_wrap = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![list_view(spec)]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(LIST_WIDTH),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![addr_row, cancel_row, list_wrap])
}

/// Coincidencia sobre título, cuerpo y etiquetas. Case-insensitive.
pub(crate) fn note_matches(n: &Note, query: &str) -> bool {
    if n.matches(query) {
        return true;
    }
    let q = query.to_lowercase();
    n.tags.iter().any(|t| t.to_lowercase().contains(&q))
}

pub(crate) fn row_label(n: &Note, mass: f32) -> String {
    let title = if n.title.is_empty() {
        "(sin título)"
    } else {
        n.title.as_str()
    };
    // Una barra de tres bloques visualiza la masa (0..1.5 mapeada a
    // 0..3). Sobre el horizonte se ve llena; cayendo, se vacía.
    let bars = (mass.clamp(0.0, 1.5) / 0.5).round() as usize;
    let glyph: String = (0..3)
        .map(|i| if i < bars { '▮' } else { '▯' })
        .collect();
    format!("{glyph}  {title}")
}

pub(crate) fn editor_panel(
    model: &Model,
    input_palette: &TextInputPalette,
    editor_palette: &EditorPalette,
) -> View<Msg> {
    let none_view = || -> View<Msg> {
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(model.theme.bg_panel)
        .text_aligned(
            "selecciona o crea una nota".to_string(),
            12.0,
            model.theme.fg_muted,
            Alignment::Center,
        )
    };

    if model.selected.is_none() {
        return wrap_panel(model, none_view());
    }

    let metrics = EditorMetrics::for_font_size(13.0);

    let title_field = field(
        model,
        "título",
        text_input_view(
            &model.title,
            "(sin título)",
            model.focus == Focus::Title,
            input_palette,
            Msg::Focus(Focus::Title),
        ),
    );

    let body_input = text_editor_view(
        &model.body,
        editor_palette,
        metrics,
        EDITOR_VISIBLE_LINES,
        |ev| Some(Msg::EditorPointer(ev)),
    );
    let body_field = body_field_view(model, body_input);

    let tags_field = field(
        model,
        "etiquetas (coma separadas)",
        text_input_view(
            &model.tags,
            "p. ej. cocina, jardín",
            model.focus == Focus::Tags,
            input_palette,
            Msg::Focus(Focus::Tags),
        ),
    );

    let stats = stats_view(model);

    let column = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(model.theme.bg_panel)
    .children(vec![title_field, body_field, tags_field, stats]);

    wrap_panel(model, column)
}

pub(crate) fn wrap_panel(_model: &Model, child: View<Msg>) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![child])
}

pub(crate) fn field(model: &Model, label: &str, control: View<Msg>) -> View<Msg> {
    let label_node = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        label.to_string(),
        FIELD_LABEL_SIZE,
        model.theme.fg_muted,
        Alignment::Start,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_shrink: 0.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(vec![label_node, control])
}

pub(crate) fn body_field_view(model: &Model, editor: View<Msg>) -> View<Msg> {
    let label_node = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "cuerpo (wiki-links con [[Título]])".to_string(),
        FIELD_LABEL_SIZE,
        model.theme.fg_muted,
        Alignment::Start,
    );

    let focused = model.focus == Focus::Body;
    let border = if focused {
        model.theme.border_focus
    } else {
        model.theme.border
    };

    let editor_wrap = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(1.0_f32),
            right: length(1.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        ..Default::default()
    })
    .border(1.0, border)
    .radius(4.0)
    .on_click(Msg::Focus(Focus::Body))
    .children(vec![editor]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(vec![label_node, editor_wrap])
}

pub(crate) fn stats_view(model: &Model) -> View<Msg> {
    let Some(id) = model.selected else {
        return View::new(Style::default());
    };
    let fwd = model.store.forward_links(id);
    let back = model.store.backlinks(id);
    let fwd_titles: Vec<String> = fwd
        .iter()
        .filter_map(|i| model.store.get(*i).map(|n| n.title.clone()))
        .collect();
    let back_titles: Vec<String> = back
        .iter()
        .filter_map(|i| model.store.get(*i).map(|n| n.title.clone()))
        .collect();
    let nearest: Vec<String> = model
        .field
        .nearest(id, 3)
        .into_iter()
        .filter_map(|(nid, score)| {
            model
                .store
                .get(nid)
                .map(|n| format!("{} ({:.2})", n.title, score))
        })
        .collect();

    let mut lines = vec![
        format!("→ enlaza a: {}", join_or_dash(&fwd_titles)),
        format!("← backlinks: {}", join_or_dash(&back_titles)),
        format!("∼ vecinos: {}", join_or_dash(&nearest)),
    ];
    // Procedencia: si la nota llegó por compartir, lleva una etiqueta
    // `de:<autor>`. La mostramos explícita.
    if let Some(n) = model.store.get(id) {
        let autores: Vec<&str> = n
            .tags
            .iter()
            .filter_map(|t| t.strip_prefix("de:"))
            .collect();
        if !autores.is_empty() {
            lines.push(format!("✎ de: {}", autores.join(", ")));
        }
    }

    let nodes: Vec<View<Msg>> = lines
        .into_iter()
        .map(|s| {
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(16.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(s, 11.0, model.theme.fg_muted, Alignment::Start)
        })
        .collect();

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(nodes)
}

pub(crate) fn join_or_dash(items: &[String]) -> String {
    if items.is_empty() {
        "—".to_string()
    } else {
        items.join(", ")
    }
}
