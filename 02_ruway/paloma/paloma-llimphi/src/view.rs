//! El árbol de vistas de paloma: tres paneles + barra de estado + el modal de
//! redacción. Todo se reconstruye en cada frame desde `&Model` (Elm puro).

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, LengthPercentage, Rect, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

use paloma_core::{MailboxRole, Message};

use crate::{Compose, ComposeField, Model, Msg};

const MAILBOX_W: f32 = 200.0;
const THREADS_W: f32 = 340.0;
const TOOLBAR_H: f32 = 44.0;
const STATUS_H: f32 = 26.0;
const ROW_H: f32 = 60.0;

/// Estilo de columna que ocupa todo el alto disponible.
fn col(width: Dimension) -> Style {
    Style {
        flex_direction: FlexDirection::Column,
        size: Size { width, height: percent(1.0_f32) },
        ..Default::default()
    }
}

fn pad(all: f32) -> Rect<LengthPercentage> {
    Rect {
        left: length(all),
        right: length(all),
        top: length(all),
        bottom: length(all),
    }
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

/// Barra superior: marca + botón Redactar + Refrescar.
fn toolbar(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let brand = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        padding: Rect { left: length(16.0_f32), right: length(0.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .text_aligned("🕊  paloma", 18.0, theme.fg_text, Alignment::Start);

    // Caja de búsqueda (ancho fijo). El `text_input_view` ya pinta foco/borde.
    let pal = TextInputPalette::from_theme(theme);
    let search = View::new(Style {
        size: Size { width: length(280.0_f32), height: Dimension::auto() },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![text_input_view(
        &model.search,
        "🔍  Buscar…  ( / )",
        model.search_focused,
        &pal,
        Msg::SearchFocus(true),
    )]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(TOOLBAR_H) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        padding: Rect { left: length(0.0_f32), right: length(12.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![
        brand,
        search,
        button("✎  Redactar", theme.accent, theme.bg_app, Msg::ComposeOpen),
        button("⟳", theme.bg_button, theme.fg_text, Msg::Refresh),
    ])
}

/// Panel central en **modo búsqueda**: lista plana de mensajes que matchean la
/// consulta, en todos los buzones. Click abre el mensaje en su hilo.
fn search_results_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let query = model.search.text();
    let hits = model.store_ref().search(&query);

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: Rect { left: length(14.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(
        format!("🔍  {} resultado(s) · «{}»", hits.len(), query.trim()),
        13.0,
        theme.fg_muted,
        Alignment::Start,
    );

    let mut rows: Vec<View<Msg>> = Vec::new();
    for m in hits.into_iter().skip(model.list_scroll) {
        rows.push(result_row(theme, m));
    }
    if rows.is_empty() {
        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(ROW_H) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text_aligned("sin coincidencias", 13.0, theme.fg_placeholder, Alignment::Center),
        );
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

/// Fila de un resultado de búsqueda: remitente · buzón · fecha + asunto + extracto.
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
            size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(crate::view::fmt_date(m.date), 11.0, theme.fg_muted, Alignment::End),
    ]);

    let subject = if m.subject.trim().is_empty() { "(sin asunto)".to_string() } else { m.subject.clone() };
    let subj = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        ..Default::default()
    })
    .text_aligned(format!("{}  ·  {}", m.mailbox, subject), 13.0, theme.fg_text, Alignment::Start);

    let snip = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text_aligned(m.snippet(64), 11.0, theme.fg_muted, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(ROW_H) },
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        padding: Rect { left: length(14.0_f32), right: length(12.0_f32), top: length(8.0_f32), bottom: length(8.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::OpenMessage(m.id.clone()))
    .children(vec![top, subj, snip])
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

    View::new(Style {
        padding: Rect { left: length(8.0_f32), right: length(8.0_f32), top: length(8.0_f32), bottom: length(8.0_f32) },
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
    let fg = if unread > 0 { theme.fg_text } else { theme.fg_muted };

    let label = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(format!("{glyph}  {name}"), 14.0, fg, Alignment::Start);

    let mut children = vec![label];
    if unread > 0 {
        children.push(badge(theme, unread));
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(32.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: Rect { left: length(10.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .fill(bg)
    .radius(4.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::SelectMailbox(key))
    .children(children)
}

/// Panel central: cabecera del buzón + lista de hilos (scrolleable, clip).
fn threads_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let header_text = model
        .selected_mailbox
        .as_deref()
        .map(|m| format!("{m}  ·  {} hilos", model.threads.len()))
        .unwrap_or_else(|| "—".to_string());
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: Rect { left: length(14.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(header_text, 13.0, theme.fg_muted, Alignment::Start);

    let mut rows: Vec<View<Msg>> = Vec::new();
    for (idx, thread) in model.threads.iter().enumerate().skip(model.list_scroll) {
        let newest: Option<&Message> = thread.message_ids.last().and_then(|id| model.store_ref().message(id));
        let selected = model.selected_thread == Some(idx);
        rows.push(thread_row(theme, thread, newest, selected, idx));
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
    let title_color = if unread { theme.fg_text } else { theme.fg_muted };

    let sender = newest.map(|m| m.from.display_name().to_string()).unwrap_or_default();
    let snippet = newest.map(|m| m.snippet(64)).unwrap_or_default();
    let date = newest.map(|m| crate::view::fmt_date(m.date)).unwrap_or_default();
    let subject = if thread.subject.is_empty() { "(sin asunto)".to_string() } else { thread.subject.clone() };

    // Línea 1: remitente · fecha (+ punto de no-leído).
    let mut top: Vec<View<Msg>> = Vec::new();
    if unread {
        top.push(dot(theme.accent));
    }
    top.push(
        View::new(Style {
            size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(sender, 13.0, title_color, Alignment::Start),
    );
    top.push(
        View::new(Style {
            size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
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

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(ROW_H) },
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        padding: Rect { left: length(14.0_f32), right: length(12.0_f32), top: length(8.0_f32), bottom: length(8.0_f32) },
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::SelectThread(idx))
    .children(vec![line_top, line_subject, line_snippet])
}

/// Panel derecho: el hilo abierto, mensaje por mensaje.
fn reading_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let Some(thread) = model.threads.get(model.selected_thread.unwrap_or(usize::MAX)) else {
        let placeholder = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned("Elegí un hilo para leerlo", 14.0, theme.fg_placeholder, Alignment::Center);
        return View::new(col(Dimension::auto()).grow()).fill(theme.bg_app).children(vec![placeholder]);
    };

    let subject = if thread.subject.is_empty() { "(sin asunto)".to_string() } else { thread.subject.clone() };
    let subject_view = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(subject, 17.0, theme.fg_text, Alignment::Start);
    let header = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(44.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        padding: Rect { left: length(20.0_f32), right: length(12.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![
        subject_view,
        button("↩  Responder", theme.bg_button, theme.fg_text, Msg::ComposeReply),
    ]);

    let mut cards: Vec<View<Msg>> = Vec::new();
    for id in &thread.message_ids {
        if let Some(m) = model.store_ref().message(id) {
            cards.push(message_card(theme, m));
        }
    }

    let scroll = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(12.0_f32) },
        padding: pad(16.0),
        ..Default::default()
    })
    .clip(true)
    .children(cards);

    View::new(col(Dimension::auto()).grow())
        .fill(theme.bg_app)
        .children(vec![header, scroll])
}

/// Tarjeta de un mensaje: cabecera (de · para · fecha) + cuerpo de texto.
fn message_card(theme: &Theme, m: &Message) -> View<Msg> {
    let from = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(m.from.to_string(), 13.0, theme.fg_text, Alignment::Start);
    let date = View::new(Style {
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(crate::view::fmt_date(m.date), 11.0, theme.fg_muted, Alignment::End);
    let head = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![from, date]);

    let to = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        format!("para: {}", m.to.iter().map(|a| a.display_name()).collect::<Vec<_>>().join(", ")),
        11.0,
        theme.fg_muted,
        Alignment::Start,
    );

    // Cuerpo: alto aproximado por cantidad de líneas (el panel recorta).
    let lines = m.body_text.lines().count().max(1);
    let body_h = (lines as f32 * 18.0).min(420.0).max(18.0);
    let body = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(body_h) },
        ..Default::default()
    })
    .text_aligned(m.body_text.clone(), 13.0, theme.fg_text, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        padding: pad(14.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(6.0)
    .children(vec![head, to, body])
}

/// Barra inferior de estado.
fn status_bar(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(STATUS_H) },
        align_items: Some(AlignItems::Center),
        padding: Rect { left: length(14.0_f32), right: length(0.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(model.status.clone(), 12.0, theme.fg_muted, Alignment::Start)
}

/// El modal de redacción: scrim + tarjeta con To/Asunto/Cuerpo + acciones.
pub fn compose_modal(model: &Model, c: &Compose) -> View<Msg> {
    let theme = &model.theme;
    let pal = TextInputPalette::from_theme(theme);

    let title = if c.in_reply_to.is_some() { "Responder" } else { "Mensaje nuevo" };
    let title_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        ..Default::default()
    })
    .text_aligned(title, 16.0, theme.fg_text, Alignment::Start);

    let to = field(&c.to, "Para: nombre <correo@dominio>", c.focus == ComposeField::To, &pal, ComposeField::To);
    let subject = field(&c.subject, "Asunto", c.focus == ComposeField::Subject, &pal, ComposeField::Subject);
    let body = body_field(&c.body, c.focus == ComposeField::Body, theme);

    let actions = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::End),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        button("Cancelar", theme.bg_button, theme.fg_text, Msg::ComposeClose),
        button("Enviar  ⏎", theme.accent, theme.bg_app, Msg::ComposeSend),
    ]);

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(560.0_f32), height: length(440.0_f32) },
        gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
        padding: pad(20.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    // Click sobre el fondo de la tarjeta no la cierra (reenfoca el campo
    // actual); sólo el scrim de afuera cierra.
    .on_click(Msg::ComposeFocus(c.focus))
    .children(vec![title_view, to, subject, body, actions]);

    // Scrim a pantalla completa: oscurece y captura clicks-afuera.
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
        ("Escribí tu mensaje…".to_string(), theme.fg_placeholder)
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
        padding: Rect { left: length(14.0_f32), right: length(14.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
        ..Default::default()
    })
    .fill(bg)
    .radius(6.0)
    .text(label, 13.0, fg)
    .on_click(msg)
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

/// Punto de acento (hilo con no-leídos).
fn dot(color: Color) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(8.0_f32), height: length(8.0_f32) },
        ..Default::default()
    })
    .fill(color)
    .radius(4.0)
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
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let (h, min) = (secs / 3600, (secs % 3600) / 60);

    // days since 1970-01-01 → (year, month, day) — civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };

    format!("{year:04}-{m:02}-{d:02} {h:02}:{min:02}")
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

use paloma_core::MailStore;
