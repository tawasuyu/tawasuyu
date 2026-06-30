//! El **lanzador** de la vista espacial (estilo «Actividades» de GNOME): un
//! buscador de apps sobre el registro de `app-bus`, pintado como overlay encima
//! del mapa de escritorios del Prezi.
//!
//! Vive en la librería —separado del binario— para reusarlo y, sobre todo,
//! **verificarlo headless** (`examples/dump_launcher` lo pinta a PNG sin
//! levantar el compositor). Es agnóstico del `Msg` de la app: el llamante pasa
//! `on_pick(i)` (lanzar la app `i` de los resultados), `on_dismiss` (click en el
//! scrim) y `swallow` (click dentro del panel, que NO debe cerrarlo).

use app_bus::{AppEntry, AppRegistry};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, FlexDirection, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Cuántos resultados muestra el panel como máximo (el buscador es para llegar
/// rápido por nombre, no para listar el catálogo entero).
pub const MAX_ROWS: usize = 8;

/// Filtra el registro por `query`: subcadena case-insensitive sobre `label` (o
/// `id`); query vacía = todas. Orden = el del registro (alfabético por label).
/// Pura sobre `(reg, query)` para testearla sin estado de app.
pub fn filtrar_apps<'a>(reg: &'a AppRegistry, query: &str) -> Vec<&'a AppEntry> {
    let q = query.trim().to_lowercase();
    reg.all()
        .iter()
        .filter(|e| {
            q.is_empty()
                || e.label.to_lowercase().contains(&q)
                || e.id.to_lowercase().contains(&q)
        })
        .collect()
}

/// Pinta el panel del lanzador: un scrim tenue a pantalla completa, un campo de
/// búsqueda con la `query` y un cursor, y hasta [`MAX_ROWS`] filas de resultados
/// (cada una clicable → `on_pick(i)`). La fila `sel` va resaltada con el acento.
/// Click en el scrim → `on_dismiss`; click en el panel → `swallow` (no cierra).
pub fn launcher_overlay<M, F>(
    theme: &Theme,
    hits: &[&AppEntry],
    query: &str,
    sel: usize,
    on_pick: F,
    on_dismiss: M,
    swallow: M,
) -> View<M>
where
    M: Clone + Send + Sync + 'static,
    F: Fn(usize) -> M,
{
    let on_accent = Color::from_rgba8(12, 16, 24, 255);

    // Campo de búsqueda con un cursor de texto al final.
    let field = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_input)
    .radius(8.0)
    .text_aligned(
        // Sin emoji (cae a tofu sin fuente de emoji): placeholder cuando está
        // vacío, la query con cursor cuando hay texto.
        if query.is_empty() {
            "Buscar apps…".to_string()
        } else {
            format!("{query}▏")
        },
        15.0,
        if query.is_empty() {
            theme.fg_placeholder
        } else {
            theme.fg_text
        },
        Alignment::Start,
    );

    let mut items: Vec<View<M>> = vec![field];

    if hits.is_empty() {
        items.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(34.0_f32),
                },
                align_items: Some(AlignItems::Center),
                padding: Rect {
                    left: length(14.0_f32),
                    right: length(0.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(
                "sin resultados".to_string(),
                13.0,
                theme.fg_placeholder,
                Alignment::Start,
            ),
        );
    }

    for (i, e) in hits.iter().take(MAX_ROWS).enumerate() {
        let active = i == sel;
        let fg = if active { on_accent } else { theme.fg_text };
        let icon = View::new(Style {
            size: Size {
                width: length(26.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned(
            e.icon.clone().unwrap_or_else(|| "▢".to_string()),
            15.0,
            fg,
            Alignment::Center,
        );
        let label = View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: auto(),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(e.label.clone(), 14.0, fg, Alignment::Start);
        items.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(34.0_f32),
                },
                align_items: Some(AlignItems::Center),
                padding: Rect {
                    left: length(10.0_f32),
                    right: length(10.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(if active { theme.accent } else { theme.bg_panel })
            .radius(6.0)
            .on_click(on_pick(i))
            .children(vec![icon, label]),
        );
    }

    let panel = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(560.0_f32),
            height: auto(),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(12.0)
    // Tragar el click dentro del panel para no cerrarlo (sólo el scrim cierra).
    .on_click(swallow)
    .children(items);

    // Scrim a pantalla completa: oscurece el mapa y, al clickear fuera del panel,
    // cierra el buscador.
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(90.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 120))
    .on_click(on_dismiss)
    .children(vec![panel])
}
