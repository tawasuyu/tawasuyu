//! El árbol de vistas de paloma: tres paneles + barra de estado + el modal de
//! redacción. Todo se reconstruye en cada frame desde `&Model` (Elm puro).
//!
//! El look apunta a una bandeja sobria y legible: avatares con iniciales por
//! remitente, estrella por hilo, barra de acciones en lectura (responder ·
//! reenviar · destacar · leído · papelera) y estados de selección/hover claros.

use llimphi_theme::{stable_color as avatar_color, Theme};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, LengthPercentage, Rect, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

use paloma_core::{MailStore, MailboxRole, Message, SignatureStatus};

use crate::{Compose, ComposeField, Model, Msg};

const MAILBOX_W: f32 = 200.0;
const THREADS_W: f32 = 360.0;
const TOOLBAR_H: f32 = 48.0;
const STATUS_H: f32 = 26.0;
const ROW_H: f32 = 64.0;
const AVATAR: f32 = 38.0;

/// Estilo de columna que ocupa todo el alto disponible.
fn col(width: Dimension) -> Style {
    Style {
        flex_direction: FlexDirection::Column,
        size: Size { width, height: percent(1.0_f32) },
        ..Default::default()
    }
}

fn pad(all: f32) -> Rect<LengthPercentage> {
    Rect { left: length(all), right: length(all), top: length(all), bottom: length(all) }
}

fn pad_xy(x: f32, y: f32) -> Rect<LengthPercentage> {
    Rect { left: length(x), right: length(x), top: length(y), bottom: length(y) }
}

/// La raíz: barra superior, cuerpo de tres columnas, barra de estado.
pub fn root(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let body = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![
        mailboxes_panel(model),
        if model.search.text().trim().is_empty() {
            threads_panel(model)
        } else {
            search_results_panel(model)
        },
        reading_panel(model),
    ]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![toolbar(model), body, status_bar(model)])
}

/// Barra superior: marca + buscador + Redactar + Refrescar.
fn toolbar(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let t = rimay_localize::t;
    let brand = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        padding: pad_xy(16.0, 0.0),
        ..Default::default()
    })
    .text_aligned("🕊  paloma", 18.0, theme.fg_text, Alignment::Start);

    let pal = TextInputPalette::from_theme(theme);
    let search = View::new(Style {
        size: Size { width: length(300.0_f32), height: Dimension::auto() },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![text_input_view(
        &model.search,
        &t("paloma-placeholder-search"),
        model.search_focused,
        &pal,
        Msg::SearchFocus(true),
    )]);

    let mut items = vec![
        brand,
        search,
        button(&t("paloma-btn-compose"), theme.accent, theme.bg_app, Msg::ComposeOpen),
    ];
    // Botón para ver/compartir la propia dirección del rail P2P (si hay rail).
    if model.rail_available() {
        items.push(button(
            &format!("🛰 {}", t("paloma-btn-my-rail")),
            theme.bg_button,
            theme.fg_text,
            Msg::ShowRailAddress,
        ));
    }
    items.push(button("⟳", theme.bg_button, theme.fg_text, Msg::Refresh));

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(TOOLBAR_H) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        padding: pad_xy(12.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(items)
}

/// Panel izquierdo: los buzones, con rol e indicador de no-leídos.
fn mailboxes_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let mut rows: Vec<View<Msg>> = Vec::new();
    for mb in model.store_ref().mailboxes() {
        let selected = model.selected_mailbox.as_deref() == Some(mb.name.as_str());
        let unread = model.store_ref().unread_count(&mb.name);
        rows.push(mailbox_row(theme, role_glyph(mb.role), mb.leaf_name(), unread, selected, mb.name.clone()));
    }

    // Nota de roadmap: Calendario/Contactos compartirán esta capa de cuentas.
    let t = rimay_localize::t;
    rows.push(nav_hint(theme, &format!("🗓  {}", t("paloma-nav-calendar")), &t("paloma-nav-soon")));
    rows.push(nav_hint(theme, &format!("👤  {}", t("paloma-nav-contacts")), &t("paloma-nav-soon")));

    View::new(Style {
        padding: pad(8.0),
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..col(length(MAILBOX_W))
    })
    .fill(theme.bg_panel_alt)
    .children(rows)
}

fn mailbox_row(
    theme: &Theme,
    glyph: &str,
    name: &str,
    unread: usize,
    selected: bool,
    key: String,
) -> View<Msg> {
    let bg = if selected { theme.bg_selected } else { theme.bg_panel_alt };
    let fg = if unread > 0 || selected { theme.fg_text } else { theme.fg_muted };

    let label = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(format!("{glyph}  {name}"), 14.0, fg, Alignment::Start);

    let mut children = vec![accent_bar(if selected { theme.accent } else { bg }), label];
    if unread > 0 {
        children.push(badge(theme, unread));
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        padding: pad_xy(8.0, 0.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(6.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::SelectMailbox(key))
    .children(children)
}

/// Entrada de navegación deshabilitada (Calendario/Contactos — del roadmap).
fn nav_hint(theme: &Theme, label: &str, tag: &str) -> View<Msg> {
    let name = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(label, 14.0, theme.fg_placeholder, Alignment::Start);
    let chip = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(16.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: pad_xy(6.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(8.0)
    .text(tag, 10.0, theme.fg_muted);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        padding: pad_xy(16.0, 0.0),
        ..Default::default()
    })
    .children(vec![name, chip])
}

/// Panel central: cabecera del buzón + lista de hilos (scrolleable, clip).
fn threads_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let header_text = model
        .selected_mailbox
        .as_deref()
        .map(|m| format!("{m}  ·  {} hilos", model.threads.len()))
        .unwrap_or_else(|| "—".to_string());
    let header = panel_header(theme, &header_text);

    let mut rows: Vec<View<Msg>> = Vec::new();
    for (idx, thread) in model.threads.iter().enumerate().skip(model.list_scroll) {
        let newest: Option<&Message> = thread.message_ids.last().and_then(|id| model.store_ref().message(id));
        let selected = model.selected_thread == Some(idx);
        rows.push(thread_row(theme, thread, newest, selected, idx));
    }
    if rows.is_empty() {
        rows.push(empty_note(theme, &rimay_localize::t("paloma-empty-threads")));
    }

    let list = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        ..Default::default()
    })
    .clip(true)
    .children(rows);

    View::new(col(length(THREADS_W)))
        .fill(theme.bg_panel)
        .children(vec![header, list])
}

fn thread_row(
    theme: &Theme,
    thread: &paloma_core::Thread,
    newest: Option<&Message>,
    selected: bool,
    idx: usize,
) -> View<Msg> {
    let unread = thread.unread > 0;
    let bg = if selected { theme.bg_selected } else { theme.bg_panel };
    let title_color = if unread || selected { theme.fg_text } else { theme.fg_muted };
    let flagged = newest.map(|m| m.flags.flagged).unwrap_or(false);

    let sender = newest.map(|m| m.from.display_name().to_string()).unwrap_or_default();
    let sender_email = newest.map(|m| m.from.email.clone()).unwrap_or_default();
    let snippet = newest.map(|m| m.snippet(60)).unwrap_or_default();
    let date = newest.map(|m| fmt_date_short(m.date)).unwrap_or_default();
    let subject = if thread.subject.is_empty() { rimay_localize::t("paloma-no-subject") } else { thread.subject.clone() };
    let newest_id = newest.map(|m| m.id.clone());

    // Línea 1: remitente (negrita si no leído) · estrella · fecha.
    let mut top: Vec<View<Msg>> = Vec::new();
    top.push(
        View::new(Style {
            size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(sender, 13.5, title_color, Alignment::Start),
    );
    top.push(star_toggle(theme, flagged, newest_id));
    top.push(
        View::new(Style {
            size: Size { width: length(54.0_f32), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(date, 11.0, theme.fg_muted, Alignment::End),
    );
    let line_top = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(top);

    let count = if thread.message_ids.len() > 1 {
        format!("{}  ·  {}", subject, thread.message_ids.len())
    } else {
        subject
    };
    let line_subject = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        ..Default::default()
    })
    .text_aligned(count, 13.0, title_color, Alignment::Start);

    let line_snippet = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text_aligned(snippet, 11.0, theme.fg_muted, Alignment::Start);

    let texts = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![line_top, line_subject, line_snippet]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(ROW_H) },
        gap: Size { width: length(10.0_f32), height: length(0.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: pad_xy(12.0, 8.0),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::SelectThread(idx))
    .children(vec![
        accent_bar(if selected { theme.accent } else { bg }),
        avatar(newest.map(|m| m.from.display_name()).unwrap_or("?"), &sender_email),
        texts,
    ])
}

/// Estrella clicable: ★ si destacado, ☆ si no. Click alterna el flag sin
/// abrir el hilo (el handler del icono gana sobre el de la fila).
fn star_toggle(theme: &Theme, flagged: bool, id: Option<paloma_core::MessageId>) -> View<Msg> {
    let (glyph, color) = if flagged { ("★", theme.accent) } else { ("☆", theme.fg_muted) };
    let mut v = View::new(Style {
        size: Size { width: length(20.0_f32), height: length(18.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(glyph, 14.0, color);
    if let Some(id) = id {
        v = v.on_click(Msg::ToggleStar(id));
    }
    v
}

/// Panel central en **modo búsqueda**: lista plana de mensajes que matchean la
/// consulta, en todos los buzones. Click abre el mensaje en su hilo.
fn search_results_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let query = model.search.text();

    // En modo semántico (con motor inyectado) los resultados llegan async y se
    // leen de `semantic_hits`; en exacto, se calculan en el acto sobre la caché.
    let semantic = model.semantic_active();
    let (header_txt, rows): (String, Vec<View<Msg>>) = if semantic {
        if model.semantic_busy() {
            (
                format!("🧠  {}", rimay_localize::t("paloma-search-semantic-running")),
                vec![empty_note(theme, &rimay_localize::t("paloma-search-semantic-running"))],
            )
        } else if let Some(hits) = model.semantic_hits() {
            let n = hits.len();
            let mut rows: Vec<View<Msg>> =
                hits.into_iter().skip(model.list_scroll).map(|m| result_row(theme, m)).collect();
            if rows.is_empty() {
                rows.push(empty_note(theme, &rimay_localize::t("paloma-empty-search")));
            }
            (format!("🧠  {n} · «{}»", query.trim()), rows)
        } else {
            (
                format!("🧠  {}", rimay_localize::t("paloma-search-semantic")),
                vec![empty_note(theme, &rimay_localize::t("paloma-search-semantic-hint"))],
            )
        }
    } else {
        let hits = model.store_ref().search(&query);
        let n = hits.len();
        let mut rows: Vec<View<Msg>> =
            hits.into_iter().skip(model.list_scroll).map(|m| result_row(theme, m)).collect();
        if rows.is_empty() {
            rows.push(empty_note(theme, &rimay_localize::t("paloma-empty-search")));
        }
        (format!("🔍  {n} resultado(s) · «{}»", query.trim()), rows)
    };

    let header = panel_header(theme, &header_txt);

    let list = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        ..Default::default()
    })
    .clip(true)
    .children(rows);

    View::new(col(length(THREADS_W)))
        .fill(theme.bg_panel)
        .children(vec![header, mode_toggle(theme, model.search_semantic), list])
}

/// Control segmentado Exacta | Semántica para el modo de búsqueda.
fn mode_toggle(theme: &Theme, semantic: bool) -> View<Msg> {
    let seg = |label: &str, active: bool, msg: Msg| {
        let (bg, fg) = if active { (theme.accent, theme.bg_app) } else { (theme.bg_button, theme.fg_muted) };
        View::new(Style {
            size: Size { width: Dimension::auto(), height: length(24.0_f32) },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(bg)
        .radius(5.0)
        .text(label, 12.0, fg)
        .on_click(msg)
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: pad_xy(14.0, 5.0),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![
        seg(&format!("🔤  {}", rimay_localize::t("paloma-search-exact")), !semantic, Msg::SearchMode(false)),
        seg(&format!("🧠  {}", rimay_localize::t("paloma-search-semantic")), semantic, Msg::SearchMode(true)),
    ])
}

/// Fila de un resultado de búsqueda: avatar + remitente · fecha · buzón + asunto.
fn result_row(theme: &Theme, m: &Message) -> View<Msg> {
    let top = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(m.from.display_name().to_string(), 13.0, theme.fg_text, Alignment::Start),
        View::new(Style {
            size: Size { width: length(54.0_f32), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(fmt_date_short(m.date), 11.0, theme.fg_muted, Alignment::End),
    ]);

    let subject = if m.subject.trim().is_empty() { rimay_localize::t("paloma-no-subject") } else { m.subject.clone() };
    let subj = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        ..Default::default()
    })
    .text_aligned(format!("{}  ·  {}", m.mailbox, subject), 13.0, theme.fg_text, Alignment::Start);

    let snip = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text_aligned(m.snippet(60), 11.0, theme.fg_muted, Alignment::Start);

    let texts = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![top, subj, snip]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(ROW_H) },
        gap: Size { width: length(10.0_f32), height: length(0.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: pad_xy(12.0, 8.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::OpenMessage(m.id.clone()))
    .children(vec![avatar(m.from.display_name(), &m.from.email), texts])
}

/// Panel derecho: el hilo abierto, mensaje por mensaje, con barra de acciones.
fn reading_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let Some(thread) = model.threads.get(model.selected_thread.unwrap_or(usize::MAX)) else {
        let placeholder = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_direction: FlexDirection::Column,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .children(vec![
            View::new(Style { size: Size { width: percent(1.0_f32), height: length(40.0_f32) }, ..Default::default() })
                .text_aligned("🕊", 34.0, theme.fg_placeholder, Alignment::Center),
            View::new(Style { size: Size { width: percent(1.0_f32), height: length(20.0_f32) }, ..Default::default() })
                .text_aligned(rimay_localize::t("paloma-placeholder-read"), 14.0, theme.fg_placeholder, Alignment::Center),
        ]);
        return View::new(col(Dimension::auto()).grow()).fill(theme.bg_app).children(vec![placeholder]);
    };

    let newest = thread.message_ids.last().and_then(|id| model.store_ref().message(id));
    let newest_id = thread.message_ids.last().cloned();
    let flagged = newest.map(|m| m.flags.flagged).unwrap_or(false);
    let seen = newest.map(|m| m.flags.seen).unwrap_or(true);

    let subject = if thread.subject.is_empty() { rimay_localize::t("paloma-no-subject") } else { thread.subject.clone() };
    let subject_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(subject, 18.0, theme.fg_text, Alignment::Start);
    let meta = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        format!("{} mensaje(s) en el hilo", thread.message_ids.len()),
        11.0,
        theme.fg_muted,
        Alignment::Start,
    );
    let header = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(58.0_f32) },
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        padding: pad_xy(20.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![subject_view, meta]);

    // Barra de acciones.
    let t = rimay_localize::t;
    let star_lbl = if flagged {
        format!("★ {}", t("paloma-btn-starred"))
    } else {
        format!("☆ {}", t("paloma-btn-star"))
    };
    let seen_lbl = if seen {
        format!("✉ {}", t("paloma-btn-mark-unread"))
    } else {
        format!("✓ {}", t("paloma-btn-mark-read"))
    };
    let mut actions: Vec<View<Msg>> = vec![
        button(&format!("↩  {}", t("paloma-btn-reply")), theme.accent, theme.bg_app, Msg::ComposeReply),
        button(&format!("↪  {}", t("paloma-btn-forward")), theme.bg_button, theme.fg_text, Msg::ComposeForward),
    ];
    // Acciones LLM (sólo si hay asistente inyectado).
    if model.llm_available() {
        let sum_lbl = if model.summary_busy() {
            format!("✨ {}", t("paloma-btn-summarizing"))
        } else {
            format!("✨ {}", t("paloma-btn-summarize"))
        };
        let draft_lbl = if model.draft_busy() {
            format!("✨ {}", t("paloma-btn-drafting"))
        } else {
            format!("✨ {}", t("paloma-btn-ai-draft"))
        };
        actions.push(button(&sum_lbl, theme.bg_button, theme.fg_text, Msg::Summarize));
        actions.push(button(&draft_lbl, theme.bg_button, theme.fg_text, Msg::DraftReply));
    }
    if let Some(id) = &newest_id {
        actions.push(button(
            &star_lbl,
            theme.bg_button,
            if flagged { theme.accent } else { theme.fg_text },
            Msg::ToggleStar(id.clone()),
        ));
        actions.push(button(&seen_lbl, theme.bg_button, theme.fg_text, Msg::ToggleSeen(id.clone())));
    }
    // Guardar el remitente en la libreta de contactos.
    actions.push(button(
        &format!("＋ {}", t("paloma-btn-add-contact")),
        theme.bg_button,
        theme.fg_text,
        Msg::SaveSenderContact,
    ));
    actions.push(spacer());
    actions.push(button(&format!("🗑  {}", t("delete")), theme.bg_button, theme.fg_destructive, Msg::DeleteThread));
    let action_bar = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(42.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        padding: pad_xy(16.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(actions);

    let lang = model.effective_view_lang().to_string();
    let mut cards: Vec<View<Msg>> = Vec::new();
    // Banner de resumen LLM (si se pidió o está en curso), arriba del hilo.
    if model.summary_busy() {
        cards.push(summary_banner(theme, &rimay_localize::t("paloma-llm-summarizing"), false));
    } else if let Some(s) = model.summary() {
        cards.push(summary_banner(theme, s, true));
    }
    // Multilienzo: si algún mensaje del hilo trae lienzos, ofrecer el selector
    // de idioma (Original + cada lienzo disponible).
    let mut langs: Vec<String> = Vec::new();
    for id in &thread.message_ids {
        if let Some(m) = model.store_ref().message(id) {
            for l in m.cuerpo_langs() {
                if !langs.iter().any(|x| x.eq_ignore_ascii_case(&l)) {
                    langs.push(l);
                }
            }
        }
    }
    if !langs.is_empty() {
        cards.push(lang_selector(theme, &langs, model.view_lang()));
    }
    for id in &thread.message_ids {
        if let Some(m) = model.store_ref().message(id) {
            let sender = model.sender_contact(m);
            cards.push(message_card(theme, m, &lang, sender));
        }
    }
    let content = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(12.0_f32) },
        padding: pad(16.0),
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(-model.read_scroll),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(cards);

    let viewport = View::new(Style {
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        ..Default::default()
    })
    .clip(true)
    .children(vec![content]);

    View::new(col(Dimension::auto()).grow())
        .fill(theme.bg_app)
        .children(vec![header, action_bar, viewport])
}

/// Alto aproximado de la tarjeta-cuerpo según sus líneas (sin tope chico: en
/// lectura queremos el alto real para que el scroll revele todo).
fn card_body_height(text: &str) -> f32 {
    let lines = text.lines().count().max(1);
    (lines as f32 * 18.0).clamp(18.0, 6000.0)
}

/// Alto total estimado del contenido del panel de lectura para el hilo abierto.
/// Lo usa `update` para acotar el scroll. Espeja la geometría de `message_card`:
/// padding 14·2 + cabecera 40 + gap 8 + cuerpo.
pub(crate) fn reading_content_height(model: &Model) -> f32 {
    let Some(thread) = model.threads.get(model.selected_thread.unwrap_or(usize::MAX)) else {
        return 0.0;
    };
    let mut total = 32.0; // padding del contenedor (16·2)
    let n = thread.message_ids.len();
    for (i, id) in thread.message_ids.iter().enumerate() {
        if let Some(m) = model.store_ref().message(id) {
            total += 76.0 + card_body_height(&m.display_body());
            if i + 1 < n {
                total += 12.0; // gap entre tarjetas
            }
        }
    }
    total
}

/// Tarjeta de un mensaje: avatar + (de · fecha) + para + cuerpo. `sender` es el
/// nombre del contacto si el remitente está en la libreta (confianza).
fn message_card(theme: &Theme, m: &Message, lang: &str, sender: Option<&str>) -> View<Msg> {
    let from = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(m.from.to_string(), 13.5, theme.fg_text, Alignment::Start);
    let date = View::new(Style {
        size: Size { width: length(120.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(fmt_date(m.date), 11.0, theme.fg_muted, Alignment::End);
    let mut head_children = vec![from];
    if let Some(badge) = signature_badge(theme, m.signature) {
        head_children.push(badge);
    }
    // Chip de confianza de identidad (pubkey↔persona).
    if let Some(chip) = identity_chip(theme, m.signature, sender) {
        head_children.push(chip);
    }
    head_children.push(date);
    let head = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: Dimension::auto(), height: length(20.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(head_children);

    let to = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(16.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        format!("{}: {}", rimay_localize::t("paloma-msg-to-label"), m.to.iter().map(|a| a.display_name()).collect::<Vec<_>>().join(", ")),
        11.0,
        theme.fg_muted,
        Alignment::Start,
    );
    let head_col = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: Dimension::auto(), height: length(40.0_f32) },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![head, to]);

    let header_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        gap: Size { width: length(10.0_f32), height: length(0.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![avatar(m.from.display_name(), &m.from.email), head_col]);

    // Multilienzo: si hay un lienzo en el idioma del lector, mostralo; si no, el
    // cuerpo principal (display_body cae a texto-desde-HTML cuando hace falta).
    let body_text = if m.has_lang(lang) {
        m.body_for(lang).to_string()
    } else {
        m.display_body()
    };
    let body_h = card_body_height(&body_text);
    let body = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(body_h) },
        ..Default::default()
    })
    .text_aligned(body_text, 13.0, theme.fg_text, Alignment::Start);

    let mut children = vec![header_row, body];
    // Si el mensaje trae HTML, ofrecer el render enriquecido (gancho de puriy).
    if m.body_html.is_some() {
        let rich = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![button(
            &format!("⌹  {}", rimay_localize::t("paloma-btn-view-rich")),
            theme.bg_button,
            theme.fg_muted,
            Msg::ViewRich(m.id.clone()),
        )]);
        children.push(rich);
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        padding: pad(14.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(8.0)
    .children(children)
}

/// Barra inferior de estado.
fn status_bar(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(STATUS_H) },
        align_items: Some(AlignItems::Center),
        padding: pad_xy(14.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(model.status.clone(), 12.0, theme.fg_muted, Alignment::Start)
}

/// El modal de redacción: scrim + tarjeta con To/Cc/Asunto/Cuerpo + acciones.
pub fn compose_modal(model: &Model, c: &Compose) -> View<Msg> {
    let theme = &model.theme;
    let pal = TextInputPalette::from_theme(theme);
    let t = rimay_localize::t;

    let title = if c.in_reply_to.is_some() { t("paloma-compose-reply-title") } else { t("paloma-compose-new") };
    let title_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        ..Default::default()
    })
    .text_aligned(title, 16.0, theme.fg_text, Alignment::Start);

    let to = field(&c.to, &t("paloma-compose-placeholder-to"), c.focus == ComposeField::To, &pal, ComposeField::To);
    let cc = field(&c.cc, &t("paloma-compose-placeholder-cc"), c.focus == ComposeField::Cc, &pal, ComposeField::Cc);
    let subject = field(&c.subject, &t("paloma-compose-placeholder-subject"), c.focus == ComposeField::Subject, &pal, ComposeField::Subject);
    let body = body_field(&c.body, c.focus == ComposeField::Body, theme);

    // Fila inferior: firmar (gancho agora) a la izquierda, acciones a la derecha.
    let actions = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        sign_checkbox(theme, c.sign),
        spacer(),
        button(&t("cancel"), theme.bg_button, theme.fg_text, Msg::ComposeClose),
        button(&format!("{}  ⏎", t("paloma-compose-send")), theme.accent, theme.bg_app, Msg::ComposeSend),
    ]);

    // Multilienzo (Eje 4): derivar el cuerpo a otro idioma con el LLM. Sólo si
    // hay asistente. Un chip por idioma (✓ si ya hay lienzo, + si falta).
    let mut card_children = vec![title_view, to, cc, subject, body];
    if model.llm_available() {
        let mut kids = vec![View::new(Style {
            size: Size { width: Dimension::auto(), height: length(24.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(format!("✨ {}:", t("paloma-btn-lienzo")), 12.0, theme.fg_muted, Alignment::Start)];
        for lang in ["es", "en", "qu"] {
            let has = c.cuerpos.iter().any(|x| x.lang.eq_ignore_ascii_case(lang));
            let lbl = if has { format!("✓ {}", lang.to_uppercase()) } else { format!("+ {}", lang.to_uppercase()) };
            let fg = if has { theme.accent } else { theme.fg_text };
            kids.push(button(&lbl, theme.bg_button, fg, Msg::DeriveCuerpo(lang.to_string())));
        }
        card_children.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
                align_items: Some(AlignItems::Center),
                gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
                ..Default::default()
            })
            .children(kids),
        );
    }
    card_children.push(actions);

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(580.0_f32), height: length(520.0_f32) },
        gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
        padding: pad(20.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .on_click(Msg::ComposeFocus(c.focus))
    .children(card_children);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 150))
    .on_click(Msg::ComposeClose)
    .children(vec![card])
}

/// Checkbox "Firmar (Ed25519)" — preferencia de UI; la firma real con la
/// identidad de agora llega al integrar el keystore (ver LEEME · Pendiente).
fn sign_checkbox(theme: &Theme, on: bool) -> View<Msg> {
    let (box_glyph, color) = if on { ("☑", theme.accent) } else { ("☐", theme.fg_muted) };
    let mark = View::new(Style {
        size: Size { width: length(20.0_f32), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(box_glyph, 16.0, color);
    let label = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(rimay_localize::t("paloma-compose-sign"), 12.0, theme.fg_muted, Alignment::Start);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: length(170.0_f32), height: length(28.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .on_click(Msg::ComposeToggleSign)
    .children(vec![mark, label])
}

fn field(
    state: &TextInputState,
    placeholder: &str,
    focused: bool,
    pal: &TextInputPalette,
    which: ComposeField,
) -> View<Msg> {
    text_input_view(state, placeholder, focused, pal, Msg::ComposeFocus(which))
}

/// Campo de cuerpo: alto, multilínea. Reusa el render del text-input pero con
/// un rect grande; el contenido se ancla arriba-izquierda.
fn body_field(state: &TextInputState, focused: bool, theme: &Theme) -> View<Msg> {
    let (bg, border) = if focused {
        (theme.bg_input_focus, theme.border_focus)
    } else {
        (theme.bg_input, theme.border)
    };
    let text = state.text();
    let (shown, color) = if text.is_empty() {
        (rimay_localize::t("paloma-compose-placeholder-body"), theme.fg_placeholder)
    } else {
        (text, theme.fg_text)
    };
    let inner = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: pad(10.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .text_aligned(shown, 13.0, color, Alignment::Start);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        padding: pad(1.0),
        ..Default::default()
    })
    .fill(border)
    .radius(4.0)
    .on_click(Msg::ComposeFocus(ComposeField::Body))
    .children(vec![inner])
}

// ── primitivas chicas ────────────────────────────────────────────────────

fn button(label: &str, bg: Color, fg: Color, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size { width: Dimension::auto(), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: pad_xy(12.0, 0.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(6.0)
    .hover_fill(bg)
    .text(label, 12.5, fg)
    .on_click(msg)
}

/// Un hueco flexible que empuja lo que sigue al extremo opuesto.
fn spacer() -> View<Msg> {
    View::new(Style {
        size: Size { width: Dimension::auto(), height: length(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
}

/// Cabecera estándar de un panel central.
fn panel_header(theme: &Theme, text: &str) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: pad_xy(14.0, 0.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(text.to_string(), 13.0, theme.fg_muted, Alignment::Start)
}

/// Nota centrada para estados vacíos.
fn empty_note(theme: &Theme, text: &str) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(text, 13.0, theme.fg_placeholder, Alignment::Center)
}

/// Banner del resumen LLM sobre el hilo: título ✨ + (✕ descartar si hay texto)
/// y el cuerpo del resumen (o el aviso "resumiendo…"). `dismissable` agrega la
/// cruz para cerrarlo.
fn summary_banner(theme: &Theme, text: &str, dismissable: bool) -> View<Msg> {
    let title = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(18.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(rimay_localize::t("paloma-llm-summary-title"), 12.0, theme.accent, Alignment::Start);

    let mut head_children = vec![title];
    if dismissable {
        head_children.push(button("✕", theme.bg_button, theme.fg_muted, Msg::DismissSummary));
    }
    let head = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(head_children);

    let body_h = card_body_height(text);
    let body = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(body_h) },
        ..Default::default()
    })
    .text_aligned(text.to_string(), 13.0, theme.fg_text, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        padding: pad(14.0),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(8.0)
    .children(vec![head, body])
}

/// Selector de idioma de lectura (multilienzo): "Original" + un chip por lienzo.
/// `current` es el idioma elegido a mano (`None` = auto/Original).
fn lang_selector(theme: &Theme, langs: &[String], current: Option<&str>) -> View<Msg> {
    let chip = |label: &str, active: bool, msg: Msg| {
        let (bg, fg) = if active { (theme.accent, theme.bg_app) } else { (theme.bg_button, theme.fg_muted) };
        View::new(Style {
            size: Size { width: Dimension::auto(), height: length(24.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            padding: pad_xy(10.0, 0.0),
            ..Default::default()
        })
        .fill(bg)
        .radius(12.0)
        .text(label, 11.5, fg)
        .on_click(msg)
    };
    let mut chips = vec![chip(
        &rimay_localize::t("paloma-read-original"),
        current.is_none(),
        Msg::SetViewLang(None),
    )];
    for l in langs {
        let active = current.map(|c| c.eq_ignore_ascii_case(l)).unwrap_or(false);
        chips.push(chip(&format!("🌐 {}", l.to_uppercase()), active, Msg::SetViewLang(Some(l.clone()))));
    }
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(chips)
}

/// Barra de acento vertical de 3 px (marca selección a la izquierda de una fila).
fn accent_bar(color: Color) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(3.0_f32), height: percent(0.7_f32) },
        ..Default::default()
    })
    .fill(color)
    .radius(2.0)
}

/// Burbuja con un contador (no-leídos).
fn badge(theme: &Theme, n: usize) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(22.0_f32), height: length(18.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.accent)
    .radius(9.0)
    .text(n.to_string(), 11.0, theme.bg_app)
}

/// Badge de estado de firma Ed25519: verde si verificada, rojo si inválida,
/// nada si sin firma (para no meter ruido en el caso normal). El dato lo
/// poblará la integración con `agora`; el render ya está listo.
fn signature_badge(theme: &Theme, status: SignatureStatus) -> Option<View<Msg>> {
    let (label, fg) = match status {
        SignatureStatus::Unsigned => return None,
        SignatureStatus::Verified => (format!("✓ {}", rimay_localize::t("paloma-sig-verified")), Color::from_rgba8(90, 180, 120, 255)),
        SignatureStatus::Invalid => (format!("⚠ {}", rimay_localize::t("paloma-sig-invalid")), theme.fg_destructive),
    };
    Some(
        View::new(Style {
            size: Size { width: Dimension::auto(), height: length(18.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            padding: pad_xy(8.0, 0.0),
            ..Default::default()
        })
        .fill(theme.bg_panel_alt)
        .radius(9.0)
        .text(label, 10.5, fg),
    )
}

/// Chip de **confianza de identidad** (pubkey↔persona). Sobre un mensaje con
/// firma `Verified`: si el remitente está en la libreta → "✓ <nombre>" (verde,
/// identidad conocida); si no → "remitente no guardado" (ámbar, TOFU). Sin firma
/// válida no aplica (no hay identidad criptográfica que atar).
fn identity_chip(theme: &Theme, status: SignatureStatus, sender: Option<&str>) -> Option<View<Msg>> {
    if status != SignatureStatus::Verified {
        return None;
    }
    let (label, fg) = match sender {
        Some(name) => (
            format!("👤 {}", rimay_localize::t_args("paloma-trust-known", &[("name", name.to_string().into())])),
            Color::from_rgba8(90, 180, 120, 255),
        ),
        None => (
            format!("? {}", rimay_localize::t("paloma-trust-unknown")),
            Color::from_rgba8(200, 160, 70, 255),
        ),
    };
    Some(
        View::new(Style {
            size: Size { width: Dimension::auto(), height: length(18.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            padding: pad_xy(8.0, 0.0),
            ..Default::default()
        })
        .fill(theme.bg_panel_alt)
        .radius(9.0)
        .text(label, 10.5, fg),
    )
}

/// Avatar circular con iniciales, coloreado de forma estable por el correo.
fn avatar(name: &str, email: &str) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(AVATAR), height: length(AVATAR) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(avatar_color(email))
    .radius((AVATAR / 2.0) as f64)
    .text(initials(name), 15.0, Color::from_rgba8(255, 255, 255, 235))
}

/// Iniciales (1–2) de un nombre para mostrar en el avatar.
fn initials(name: &str) -> String {
    let mut words = name.split_whitespace().filter(|w| !w.is_empty());
    let first = words.next().and_then(|w| w.chars().next());
    let second = words.next().and_then(|w| w.chars().next());
    match (first, second) {
        (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_uppercase()),
        (Some(a), None) => a.to_uppercase().to_string(),
        _ => "?".to_string(),
    }
}

/// Glifo por rol de buzón.
fn role_glyph(role: MailboxRole) -> &'static str {
    match role {
        MailboxRole::Inbox => "📥",
        MailboxRole::Sent => "📤",
        MailboxRole::Drafts => "📝",
        MailboxRole::Trash => "🗑",
        MailboxRole::Junk => "⚠",
        MailboxRole::Archive => "📦",
        MailboxRole::Custom => "📁",
    }
}

/// Formatea un timestamp Unix (segundos UTC) como `YYYY-MM-DD HH:MM`. Sin
/// dependencias de tiempo: algoritmo civil-from-days de Howard Hinnant.
pub(crate) fn fmt_date(ts: i64) -> String {
    if ts <= 0 {
        return "—".to_string();
    }
    let (y, m, d, h, min) = civil(ts);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}")
}

/// Fecha corta para listas: `MM-DD HH:MM`.
fn fmt_date_short(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let (_, m, d, h, min) = civil(ts);
    format!("{m:02}-{d:02} {h:02}:{min:02}")
}

/// Descompone un timestamp Unix (s UTC) en `(año, mes, día, hora, minuto)`.
fn civil(ts: i64) -> (i64, i64, i64, i64, i64) {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let (h, min) = (secs / 3600, (secs % 3600) / 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d, h, min)
}

// ── helpers de estilo ─────────────────────────────────────────────────────

trait StyleGrow {
    fn grow(self) -> Self;
}
impl StyleGrow for Style {
    fn grow(mut self) -> Self {
        self.flex_grow = 1.0;
        self
    }
}

impl Model {
    /// Acceso de sólo-lectura al store para la vista.
    pub(crate) fn store_ref(&self) -> &MailStore {
        &self.store
    }
}
