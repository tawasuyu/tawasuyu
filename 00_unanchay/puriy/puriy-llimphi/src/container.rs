//! El **contenedor** del navegador renovado: el chrome que rodea al viewport
//! cuando se integra a la Llimphi nueva. Dos piezas grandes:
//!
//! 1. **Sidebar vertical de dientes** (Block B) — reusa
//!    `llimphi-widget-dock-rail` (el patrón de cosmos): un rail con un diente
//!    por [`Space`] + el panel de pestañas verticales del space activo. En modo
//!    horizontal el chrome sigue usando `tabs_bar` (un nivel).
//! 2. **Input de URL repotenciado** (Block C) — indicador de seguridad/esquema,
//!    resaltado del dominio, "buscar-o-navegar" y autocompletar desde el
//!    historial + marcadores.
//!
//! Todo pinta con `model.theme` (Llimphi `Theme`), no colores hardcodeados.
//! Comparte los tipos del crate vía `use super::*` (regla #1).

use super::*;

use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};

/// Ancho del rail de dientes (igual que el dock de cosmos).
const RAIL_W: f32 = 44.0;
/// Ancho del panel de pestañas verticales del space activo.
const TAB_PANEL_W: f32 = 212.0;
/// Alto de una fila de pestaña en el panel vertical.
const VTAB_H: f32 = 34.0;

// =====================================================================
// Block C — omnibox: buscar-o-navegar + autocompletar
// =====================================================================

/// Normaliza lo tipeado en el address bar: si parece URL/dominio navega; si no,
/// lo manda a un buscador. Replica la heurística de un omnibox real.
pub(crate) fn normalize_omnibox_input(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    // Ya trae esquema explícito (http://, https://, file://, ftp://…).
    if s.contains("://") {
        return s.to_string();
    }
    // Esquemas sin "//" que igual son URLs válidas.
    if s.starts_with("about:") || s.starts_with("data:") || s.starts_with("mailto:") {
        return s.to_string();
    }
    // ¿Parece un host? Sin espacios y (localhost | con un punto interior).
    let no_space = !s.contains(char::is_whitespace);
    let looks_host = no_space
        && (s == "localhost"
            || s.starts_with("localhost:")
            || s.starts_with("localhost/")
            || (s.contains('.') && !s.starts_with('.') && !s.contains("..")));
    if looks_host {
        return format!("https://{s}");
    }
    // Si no, es una búsqueda. DuckDuckGo, codificando la query.
    let q: String = url::form_urlencoded::byte_serialize(s.as_bytes()).collect();
    format!("https://duckduckgo.com/?q={q}")
}

/// Sugerencias de autocompletar (historial + marcadores) que matchean `query`
/// por substring case-insensitive en url o título. Dedup por url, tope 6.
/// Vacío si no hay Profile cableado o la query está vacía.
pub(crate) fn compute_addr_suggestions(query: &str) -> Vec<(String, String)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let Some(handle) = profile_handle() else { return Vec::new() };
    let Ok(p) = handle.lock() else { return Vec::new() };
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let matches = |url: &str, title: &str| {
        url.to_lowercase().contains(&q) || title.to_lowercase().contains(&q)
    };
    // Marcadores primero (intención más fuerte), luego historial reciente.
    for b in p.bookmarks.items() {
        if matches(&b.url, &b.title) && seen.insert(b.url.clone()) {
            out.push((b.url.clone(), b.title.clone()));
        }
    }
    for e in p.history.recent(200) {
        if out.len() >= 6 {
            break;
        }
        if matches(&e.url, &e.title) && seen.insert(e.url.clone()) {
            out.push((e.url.clone(), e.title.clone()));
        }
    }
    out.truncate(6);
    out
}

/// Parte una URL en `(esquema, host, resto)` para resaltar el dominio. Maneja
/// tanto `https://host/path` como esquemas sin `//` (`about:blank`).
fn split_url(u: &str) -> (String, String, String) {
    if let Some(i) = u.find("://") {
        let scheme = &u[..i + 3];
        let after = &u[i + 3..];
        let (host, rest) = match after.find(['/', '?', '#']) {
            Some(j) => (&after[..j], &after[j..]),
            None => (after, ""),
        };
        return (scheme.to_string(), host.to_string(), rest.to_string());
    }
    if let Some(i) = u.find(':') {
        return (u[..i + 1].to_string(), String::new(), u[i + 1..].to_string());
    }
    (String::new(), u.to_string(), String::new())
}

/// Indicador de seguridad: un punto de color según el esquema (verde = https,
/// ámbar = http inseguro, atenuado = about/file/data).
fn security_dot(scheme: &str, theme: &Theme) -> View<Msg> {
    let color = if scheme.starts_with("https") {
        Color::from_rgb8(70, 190, 110)
    } else if scheme.starts_with("http") {
        Color::from_rgb8(220, 165, 70)
    } else {
        theme.fg_muted
    };
    View::new(Style {
        size: Size { width: length(20.0_f32), height: length(28.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned("●", 9.0, color, Alignment::Center)
}

/// URL "linda" (no enfocada): esquema y resto atenuados, **dominio resaltado**.
/// Clic enfoca el address bar (vuelve al input editable).
fn pretty_url_view(url: &str, theme: &Theme) -> View<Msg> {
    let (scheme, host, rest) = split_url(url);
    let muted = theme.fg_muted;
    let strong = theme.fg_text;
    let span = |t: String, color: Color, sz: f32| {
        View::new(Style {
            size: Size { width: auto(), height: length(20.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(t, sz, color, Alignment::Start)
    };
    let mut spans: Vec<View<Msg>> = Vec::new();
    if !scheme.is_empty() {
        spans.push(span(scheme, muted, 13.0));
    }
    if !host.is_empty() {
        spans.push(span(host, strong, 13.0));
    }
    if !rest.is_empty() {
        spans.push(span(truncate(&rest, 80), muted, 13.0));
    }
    if spans.is_empty() {
        spans.push(span(rimay_localize::t("puriy-addr-empty"), theme.fg_placeholder, 13.0));
    }
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: Rect {
            left: length(4.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_input)
    .radius(6.0)
    .on_click(Msg::FocusAddr)
    .children(spans)
}

/// Dropdown de autocompletar bajo el address bar. `None` si no hay sugerencias.
fn addr_suggestions_view(model: &Model) -> Option<View<Msg>> {
    if !model.active().addr_focused || model.addr_suggest.is_empty() {
        return None;
    }
    let theme = &model.theme;
    let rows: Vec<View<Msg>> = model
        .addr_suggest
        .iter()
        .map(|(url, title)| {
            let label = if title.is_empty() { url.clone() } else { title.clone() };
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
                padding: Rect {
                    left: length(10.0_f32),
                    right: length(10.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::Center),
                gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
                ..Default::default()
            })
            .hover_fill(theme.bg_row_hover)
            .on_click(Msg::AddrSuggestPick(url.clone()))
            .children(vec![
                View::new(Style {
                    size: Size { width: length(220.0_f32), height: length(16.0_f32) },
                    ..Default::default()
                })
                .text_aligned(truncate(&label, 38), 12.0, theme.fg_text, Alignment::Start),
                View::new(Style {
                    flex_grow: 1.0,
                    size: Size { width: percent(0.0_f32), height: length(16.0_f32) },
                    ..Default::default()
                })
                .text_aligned(truncate(url, 48), 11.0, theme.fg_muted, Alignment::Start),
            ])
        })
        .collect();
    Some(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: auto() },
            margin: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(4.0_f32),
                bottom: length(0.0_f32),
            },
            padding: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(4.0_f32),
                bottom: length(4.0_f32),
            },
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(8.0)
        .children(rows),
    )
}

/// Header renovado (theme-driven): botones nav + indicador de seguridad +
/// address bar (editable o URL linda) + status, con el dropdown de
/// autocompletar debajo. Reemplaza al `header_bar` viejo de colores fijos.
pub(crate) fn nav_header_bar(model: &Model) -> View<Msg> {
    let t = model.active();
    let theme = &model.theme;

    let nav_btn = |label: &str, enabled: bool, msg: Msg| {
        let color = if enabled { theme.fg_text } else { theme.fg_muted };
        View::new(Style {
            size: Size { width: length(28.0_f32), height: length(28.0_f32) },
            margin: Rect {
                left: length(0.0_f32),
                right: length(4.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(theme.bg_button)
        .hover_fill(theme.bg_button_hover)
        .radius(6.0)
        .text_aligned(label.to_string(), 14.0, color, Alignment::Center)
        .on_click(msg)
    };

    // Campo de dirección: editable con foco, URL "linda" (dominio resaltado) sin.
    let (scheme, _, _) = split_url(&t.url);
    let addr_field: View<Msg> = if t.addr_focused {
        let palette = TextInputPalette::from_theme(theme);
        let addr_ph = rimay_localize::t("puriy-addr-placeholder");
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .children(vec![text_input_view(
            &t.addr,
            &addr_ph,
            true,
            &palette,
            Msg::FocusAddr,
        )])
    } else {
        pretty_url_view(&t.url, theme)
    };

    let addr_row = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![
        nav_btn("◀", t.can_back(), Msg::Back),
        nav_btn("▶", t.can_fwd(), Msg::Forward),
        nav_btn("↻", true, Msg::Reload),
        security_dot(&scheme, theme),
        View::new(Style {
            flex_grow: 1.0,
            size: Size { width: percent(0.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .children(vec![addr_field]),
        nav_btn("⚙", true, Msg::OpenSettings),
    ]);

    // Status / preview de link.
    let title_line = if t.title.is_empty() { t.url.as_str() } else { t.title.as_str() };
    let status_line = if let Some(href) = model.hover_link.as_deref() {
        format!("→ {}", truncate(href, 220))
    } else {
        format!("{}    ·    {}", title_line, t.status)
    };

    let mut kids: Vec<View<Msg>> = vec![addr_row];
    if let Some(sugg) = addr_suggestions_view(model) {
        kids.push(sugg);
    }
    kids.push(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(14.0_f32) },
            margin: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(3.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(status_line, 10.0, theme.fg_muted, Alignment::Start),
    );

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: auto() },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(kids)
}

// =====================================================================
// Block B — sidebar vertical de dientes (spaces)
// =====================================================================

/// Rail de dientes: un diente por space, vía `llimphi-widget-dock-rail` (el
/// patrón de cosmos). Clic activa el space; debajo, un "+" agrega uno nuevo.
fn space_rail(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let items: Vec<DockRailItem> = (0..model.spaces.len())
        .map(|i| DockRailItem { id: i as u64, active: i == model.active_space })
        .collect();
    let spaces = model.spaces.clone();
    let rail = dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| {
            // La inicial del nombre del space — siempre renderiza (a diferencia
            // de glifos decorativos que la fuente puede no tener) y es legible.
            let glyph = spaces
                .get(id as usize)
                .and_then(|s| s.name.chars().next())
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "·".to_string());
            View::new(Style {
                size: Size { width: length(size), height: length(size + 4.0) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            // Ícono del diente: vector si el glifo está en el catálogo, texto
            // si no (las iniciales de nombre caen a texto). El nodo ya tiene
            // size fijo, así que el View absoluto 100% del helper se dimensiona bien.
            .children(vec![llimphi_icons::glyph_or_text_view(&glyph, size * 0.9, color, 1.8)])
        },
        |id| Msg::SelectSpace(id as usize),
        // El rail no recibe drops de tabs (no sabe sobre qué diente cayó).
        |_payload| None,
    );

    let add = View::new(Style {
        size: Size { width: length(RAIL_W), height: length(34.0_f32) },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(5.0)
    .hover_fill(theme.bg_row_hover)
    .text_aligned("+", 18.0, theme.fg_muted, Alignment::Center)
    .on_click(Msg::NewSpace);

    View::new(Style {
        size: Size { width: length(RAIL_W), height: percent(1.0_f32) },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![rail, add])
}

/// Panel de pestañas verticales del space activo: el nombre del space arriba,
/// la lista de sus pestañas (activa resaltada, con ✕), y "+ pestaña" abajo.
fn vertical_tab_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let space_name = model
        .spaces
        .get(model.active_space)
        .map(|s| s.name.clone())
        .unwrap_or_default();

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(truncate(&space_name, 24), 12.0, theme.fg_muted, Alignment::Start);

    let mut kids: Vec<View<Msg>> = vec![header];
    for idx in model.active_space_tabs() {
        let t = &model.tabs[idx];
        let active = idx == model.active;
        let label = if t.title.is_empty() { t.url.as_str() } else { t.title.as_str() };
        let new_label = rimay_localize::t("puriy-new-tab-label");
        let label = if label.is_empty() { new_label.as_str() } else { label };
        let fg = if active { theme.fg_text } else { theme.fg_muted };

        let close = View::new(Style {
            size: Size { width: length(18.0_f32), height: length(18.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned("×", 14.0, fg, Alignment::Center)
        .on_click(Msg::CloseTab(idx));

        let mut row = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(VTAB_H) },
            padding: Rect {
                left: length(12.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            margin: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(2.0_f32),
            },
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .radius(6.0)
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::SelectTab(idx))
        .children(vec![
            View::new(Style {
                flex_grow: 1.0,
                size: Size { width: percent(0.0_f32), height: length(18.0_f32) },
                ..Default::default()
            })
            .text_aligned(truncate(label, 26), 12.0, fg, Alignment::Start),
            close,
        ]);
        if active {
            row = row.fill(theme.bg_selected);
        }
        kids.push(row);
    }

    let add_tab = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .radius(6.0)
    .hover_fill(theme.bg_row_hover)
    .text_aligned(rimay_localize::t("puriy-add-tab"), 12.0, theme.fg_muted, Alignment::Start)
    .on_click(Msg::NewTab);
    kids.push(add_tab);

    View::new(Style {
        size: Size { width: length(TAB_PANEL_W), height: percent(1.0_f32) },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(kids)])
}

/// Sidebar completo (modo vertical) = rail de dientes + panel de pestañas del
/// space activo, lado a lado (exactamente el patrón rail+panel de cosmos).
pub(crate) fn sidebar_view(model: &Model) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(RAIL_W + TAB_PANEL_W), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Row,
        ..Default::default()
    })
    .children(vec![space_rail(model), vertical_tab_panel(model)])
}
