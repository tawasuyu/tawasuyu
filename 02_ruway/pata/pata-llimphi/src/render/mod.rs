//! El pincel: traduce el modelo resuelto de `pata-core` a `View<Msg>` de
//! Llimphi.
//!
//! Dos niveles:
//! - [`widget_view`] traduce un [`WidgetView`] —el view-model agnóstico que un
//!   widget emite— a un `View<Msg>` concreto (texto, medidor con barra,
//!   placeholder tenue).
//! - [`root`] coloca cada superficie en el rect que [`pata_core::layout`]
//!   resolvió (posición absoluta, en píxeles de pantalla) y reparte sus widgets
//!   en los slots start/center/end según el eje del anclaje.
//!
//! Estructura interna (submódulos):
//! - `widgets`      — primitivas de pintura: chips, medidores, barras, colores.
//! - `task_manager` — task manager, workspaces, tray, portapapeles, inicio.
//! - `panels`       — ventanitas de medidores (CPU/RAM/vol/bri) y reloj.
//! - `weather_cava` — widget de clima y visualizador de audio.
//! - `sidebar`      — rail de dientes (Sidebar) y su panel.
//! - `start_menus`  — menú de inicio estilo GNOME y XP.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{
        auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style,
    },
    Rect as TaffyRect,
};
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};
use llimphi_ui::View;

use app_bus::AppEntry;
use pata_core::config::{FloatingCard, Surface, SurfaceKind};
use pata_core::layout::Rect;
use pata_core::widget::{MeterOrient, MeterSize, Widget, WidgetCtx, WidgetView};

use crate::shuma::{self, ShumaState};
use crate::toplevel::WindowEntry;
use crate::tray::TrayItem;
use crate::{Model, Msg, SlotWidget, SurfaceWidgets};

// Submódulos internos
mod panels;
mod sidebar;
mod start_menus;
mod task_manager;
mod weather_cava;
mod widgets;

// Re-exportaciones del subconjunto que el resto del crate necesita.
pub use panels::{
    brightness_overlay, brightness_panel, clock_menu_view, clock_overlay, clock_panel,
    cpu_overlay, cpu_panel, ram_overlay, ram_panel, volume_overlay, volume_panel,
};
pub use sidebar::{nav_panel_view, sidebar_rail_view, sidebar_surface_view};
pub use start_menus::{start_menu_gnome_overlay, start_menu_xp_overlay};
pub use task_manager::{clipboard_overlay, clipboard_panel};
pub use widgets::parse_hex;

// Constantes internas re-usadas en submódulos vía `super::`.
const WINDOW_LABEL_MAX: usize = 22; // usado en task_manager → recortar
const CLIPBOARD_PREVIEW_MAX: usize = 28;
const TRAY_LABEL_MAX: usize = 14;

/// Los datos del host que el render necesita además del view-model de los
/// widgets de core: lo dinámico que vive en el backend (ventanas abiertas,
/// portapapeles) y se pasa aparte.
#[derive(Default)]
pub struct BarData<'a> {
    /// Las ventanas abiertas, para el `window_list`.
    pub windows: &'a [WindowEntry],
    /// El texto del portapapeles (ya en una línea), para el `clipboard`.
    pub clipboard: Option<&'a str>,
    /// Los items de la bandeja del sistema, para el `tray`.
    pub tray: &'a [TrayItem],
    /// La última lectura del clima, para el `weather`.
    pub weather: Option<&'a crate::weather::Weather>,
    /// El último cuadro del visualizador de audio, para el `cava`.
    pub cava: &'a [f32],
}

// ============================================================
// API pública de widget
// ============================================================

/// El texto de tooltip de un widget de core, derivado de su view-model.
pub fn widget_tooltip(v: &WidgetView) -> Option<String> {
    match v {
        WidgetView::Empty => None,
        WidgetView::Text(t) if t.trim().is_empty() => None,
        WidgetView::Text(t) => Some(t.clone()),
        WidgetView::TextRich { tooltip, .. } if tooltip.trim().is_empty() => None,
        WidgetView::TextRich { tooltip, .. } => Some(tooltip.clone()),
        WidgetView::Meter { label, caption, .. } => {
            let l = label.as_deref().unwrap_or("").trim();
            let c = caption.trim();
            let s = format!("{l} {c}");
            let s = s.trim().to_string();
            (!s.is_empty()).then_some(s)
        }
        WidgetView::Cores { label, caption, fractions, .. } => {
            let l = label.as_deref().unwrap_or("CPU").trim();
            let n = fractions.len();
            Some(format!("{l} {caption} · {n} cores"))
        }
        WidgetView::Workspaces { active, count, .. } => Some(format!("Escritorio {active}/{count}")),
        WidgetView::Moon { name, .. } => Some(name.clone()),
        WidgetView::Placeholder(kind) => Some(kind.clone()),
    }
}

/// El cuerpo del **tooltip flotante**: una cajita opaca con el texto.
pub fn tooltip_view(text: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(6.0)
    .text(text.to_string(), 12.0, theme.fg_text)
}

/// Traduce el view-model de un widget al `View<Msg>` que lo pinta.
pub fn widget_view(v: &WidgetView, theme: &Theme) -> View<Msg> {
    widget_view_kinded(v, None, theme)
}

/// Como [`widget_view`] pero con el `kind` del widget, para que el medidor use
/// su gradiente propio.
pub fn widget_view_kinded(v: &WidgetView, kind: Option<&str>, theme: &Theme) -> View<Msg> {
    use widgets::*;
    match v {
        WidgetView::Empty => View::new(Style {
            size: Size {
                width: length(0.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        }),
        WidgetView::Text(t) => chip(theme).text(t.clone(), 13.0, theme.fg_text),
        WidgetView::TextRich { text, .. } => {
            let color = match kind {
                Some("astro") => astro_color(text, theme.fg_text),
                _ => theme.fg_text,
            };
            let body_px = if matches!(kind, Some("astro")) { 19.0 } else { 16.0 };
            chip(theme).text(text.clone(), body_px, color)
        }
        WidgetView::Meter { label, fraction, caption, size, orient } => {
            let stops = match kind {
                Some(k) => meter_stops(k),
                None => (theme.accent, aclarar(theme.accent, 0.5)),
            };
            let m = meter_view(label.as_deref(), *fraction, caption, *size, *orient, theme, stops);
            con_icono_de_kind(m, kind, theme)
        }
        WidgetView::Cores { label, fractions, caption, size, orient } => {
            let stops = match kind {
                Some(k) => meter_stops(k),
                None => meter_stops("cpu_cores"),
            };
            cores_view(label.as_deref(), fractions, caption, *size, *orient, theme, stops)
        }
        WidgetView::Workspaces { active, count, occupied } => {
            task_manager::workspaces_view(*active, *count, *occupied, 4.0, FlexDirection::Row, theme)
        }
        WidgetView::Moon { phase, .. } => moon_view(*phase),
        WidgetView::Placeholder(kind) => widgets::chip(theme)
            .fill(theme.bg_panel)
            .radius(6.0)
            .text(kind.clone(), 12.0, theme.fg_muted),
    }
}

// ============================================================
// Tarjetas flotantes
// ============================================================

/// El **interior** de una tarjeta flotante (estilo conky).
pub fn card_view(card: &FloatingCard, widgets_list: &[Box<dyn Widget>], theme: &Theme) -> View<Msg> {
    let mut hijos: Vec<View<Msg>> = Vec::new();
    if let Some(t) = &card.title {
        hijos.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(20.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::FlexStart),
                ..Default::default()
            })
            .text(t.clone(), 12.0, theme.fg_muted),
        );
    }
    for w in widgets_list {
        hijos.push(widget_view(&w.view(), theme));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: TaffyRect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .children(hijos)
}

/// Una tarjeta flotante posicionada en absoluto en (x, y) con tamaño (w, h).
fn card_view_absolute(card: &FloatingCard, widgets_list: &[Box<dyn Widget>], theme: &Theme) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(card.x),
            top: length(card.y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(card.w),
            height: length(card.h),
        },
        ..Default::default()
    })
    .children(vec![card_view(card, widgets_list, theme)])
}

// ============================================================
// Vista raíz y layout de superficie
// ============================================================

/// El `View` raíz: cubre la pantalla y coloca cada superficie en su rect.
pub fn root(model: &Model) -> View<Msg> {
    let (sw, sh) = model.screen;
    let mut superficies: Vec<View<Msg>> = Vec::new();

    let tray_items = model.tray.as_ref().map(|t| t.items()).unwrap_or_default();
    let data = BarData {
        windows: &model.windows,
        clipboard: model.clipboard.as_deref(),
        tray: &tray_items,
        weather: model.weather_now.as_ref(),
        cava: &model.cava_frame,
    };

    for placed in &model.frame.surfaces {
        let surface = &model.cfg.surfaces[placed.index];
        let widgets = &model.surfaces[placed.index];
        if !placed.rect.es_visible() {
            continue;
        }
        if surface.kind == SurfaceKind::Sidebar {
            superficies.push(sidebar_rail_view(
                surface,
                placed.index,
                placed.rect,
                &model.nav,
                &model.shuma,
                &model.theme,
            ));
            continue;
        }
        superficies.push(surface_view(
            surface,
            placed.rect,
            widgets,
            &model.shuma,
            &data,
            &model.theme,
        ));
    }

    // El panel del diente desplegado flota sobre el área de trabajo.
    if let Some((si, ti)) = model.nav.open {
        if let Some(placed) = model.frame.surfaces.iter().find(|p| p.index == si) {
            if let Some(surface) = model.cfg.surfaces.get(si) {
                if surface.kind == SurfaceKind::Sidebar {
                    superficies.push(nav_panel_view(
                        surface,
                        ti,
                        placed.rect,
                        (sw, sh),
                        &model.nav,
                        &model.theme,
                    ));
                }
            }
        }
    }

    // Tarjetas flotantes (estilo conky).
    for (card, ws) in &model.cards {
        superficies.push(card_view_absolute(card, ws, &model.theme));
    }

    View::new(Style {
        size: Size {
            width: length(sw as f32),
            height: length(sh as f32),
        },
        ..Default::default()
    })
    .children(superficies)
}

/// Aplica la **apariencia configurable** al cuerpo de la barra `v`.
fn aplicar_apariencia(v: View<Msg>, surface: &Surface, theme: &Theme) -> View<Msg> {
    let bg = widgets::con_opacidad(theme.bg_panel_alt, surface.opacity);
    let v = if surface.gradient {
        use llimphi_ui::llimphi_raster::kurbo::Point;
        use llimphi_ui::llimphi_raster::peniko::Gradient;
        let top = widgets::con_opacidad(widgets::aclarar(theme.bg_panel_alt, 0.10), surface.opacity);
        let g = Gradient::new_linear(Point::new(0.0, 0.0), Point::new(0.0, 1.0))
            .with_stops([top, bg].as_slice());
        v.fill_gradient(g)
    } else {
        v.fill(bg)
    };
    if surface.radius > 0.0 {
        v.radius(surface.radius as f64)
    } else {
        v
    }
}

/// Si la barra tiene `margin > 0`, la separa del borde con un contenedor
/// transparente de padding `margin`.
fn envolver_margen(inner: View<Msg>, surface: &Surface) -> View<Msg> {
    if surface.margin <= 0.0 {
        return inner;
    }
    let m = length(surface.margin);
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: TaffyRect {
            left: m,
            right: m,
            top: m,
            bottom: m,
        },
        ..Default::default()
    })
    .children(vec![inner])
}

/// El cuerpo de una barra (100%×100% de su contenedor): los tres slots.
fn bar_body(
    surface: &Surface,
    surface_widgets: &SurfaceWidgets,
    shuma_state: &ShumaState,
    data: &BarData,
    theme: &Theme,
    dir: FlexDirection,
) -> View<Msg> {
    let cuerpo = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_direction: dir,
        padding: TaffyRect {
            left: length(surface.padding),
            right: length(surface.padding),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        ..Default::default()
    })
    .children(slots_de(surface, surface_widgets, shuma_state, data, theme, dir));
    envolver_margen(aplicar_apariencia(cuerpo, surface, theme), surface)
}

/// Una superficie colocada: rectángulo absoluto que aloja el cuerpo de la barra.
fn surface_view(
    surface: &Surface,
    rect: Rect,
    surface_widgets: &SurfaceWidgets,
    shuma_state: &ShumaState,
    data: &BarData,
    theme: &Theme,
) -> View<Msg> {
    let dir = if surface.anchor.es_horizontal() {
        FlexDirection::Row
    } else {
        FlexDirection::Column
    };

    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(rect.x as f32),
            top: length(rect.y as f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(rect.w as f32),
            height: length(rect.h as f32),
        },
        ..Default::default()
    })
    .children(vec![bar_body(surface, surface_widgets, shuma_state, data, theme, dir)])
}

/// La barra de shuma **desplegada**: cuerpo del drawer + barra abajo.
pub fn shuma_open_view(
    surface: &Surface,
    surface_widgets: &SurfaceWidgets,
    shuma_state: &ShumaState,
    data: &BarData,
    theme: &Theme,
    bar_px: f32,
    drawer_h: f32,
) -> View<Msg> {
    // Scrim transparente arriba del drawer — captura el click "fuera".
    let scrim = {
        let mut style = Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        };
        style.flex_grow = 1.0;
        View::new(style).on_click(Msg::ShumaToggle)
    };

    let body = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(drawer_h),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![shuma::drawer_body_view(shuma_state, theme)]);

    let bar = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(bar_px),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![bar_view(surface, surface_widgets, shuma_state, data, theme)]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![scrim, body, bar])
}

/// Construye los tres slots (start/center/end) de una superficie.
fn slots_de(
    surface: &Surface,
    surface_widgets: &SurfaceWidgets,
    shuma_state: &ShumaState,
    data: &BarData,
    theme: &Theme,
    dir: FlexDirection,
) -> Vec<View<Msg>> {
    let slot = |ws: &[SlotWidget], justify: JustifyContent| -> View<Msg> {
        let items: Vec<View<Msg>> = ws
            .iter()
            .map(|sw| match sw {
                SlotWidget::Core { kind, widget, exec, cells } => {
                    let wv = widget.view();
                    if let WidgetView::Workspaces { active, count, occupied } = wv {
                        task_manager::workspaces_view(active, count, occupied, surface.gap, dir, theme)
                    } else {
                        let mut v = widget_view_kinded(&wv, Some(kind), theme)
                            .radius(6.0)
                            .hover_fill(theme.bg_button_hover);
                        if let Some(tip) = widget_tooltip(&wv) {
                            v = v.tooltip(tip);
                        }
                        v = widgets::interaccion_widget(v, kind, exec.as_deref());
                        widgets::cuantizar(v, surface.cell, *cells, kind, dir)
                    }
                }
                SlotWidget::Start { label, exec } => task_manager::start_button_view(label, exec.as_deref(), theme),
                SlotWidget::Shuma => shuma::headline_view(shuma_state, theme),
                SlotWidget::WindowList => task_manager::window_list_view(data.windows, surface.gap, dir, theme),
                SlotWidget::Clipboard { exec } => {
                    task_manager::clipboard_view(data.clipboard, exec.as_deref(), theme)
                }
                SlotWidget::Tray => task_manager::tray_view(data.tray, surface.gap, dir, theme),
                SlotWidget::Weather { exec } => {
                    widgets::cuantizar(weather_cava::weather_view(data.weather, exec.as_deref(), theme), surface.cell, 0, "weather", dir)
                }
                SlotWidget::Cava => {
                    widgets::cuantizar(weather_cava::cava_view(data.cava, theme), surface.cell, 0, "cava", dir)
                }
            })
            .collect();
        let mut style = Style {
            flex_direction: dir,
            align_items: Some(AlignItems::Center),
            justify_content: Some(justify),
            gap: Size {
                width: length(surface.gap),
                height: length(surface.gap),
            },
            ..Default::default()
        };
        style.flex_grow = 1.0;
        View::new(style).children(items)
    };
    vec![
        slot(&surface_widgets.start, JustifyContent::FlexStart),
        slot(&surface_widgets.center, JustifyContent::Center),
        slot(&surface_widgets.end, JustifyContent::FlexEnd),
    ]
}

// ============================================================
// Menú de inicio (path winit)
// ============================================================

/// Alto de cada fila del menú (px).
const MENU_ROW_H: f32 = 28.0;
/// Gap vertical entre filas (px).
const MENU_ROW_GAP: f32 = 2.0;
/// Alto del campo de búsqueda (px).
const MENU_SEARCH_H: f32 = 34.0;
/// Ancho del menú de inicio desplegado, en px.
const START_MENU_W: f32 = 280.0;

/// Filtra el registro por `query` (substring, sin distinguir mayúsculas).
pub fn menu_filtered<'a>(apps: &'a [AppEntry], query: &str) -> Vec<&'a AppEntry> {
    let needle = query.to_lowercase();
    apps.iter()
        .filter(|a| needle.is_empty() || a.label.to_lowercase().contains(&needle))
        .collect()
}

pub fn start_menu_body(
    apps: &[AppEntry],
    query: &str,
    offset: f32,
    viewport_h: f32,
    theme: &Theme,
) -> View<Msg> {
    let matches = menu_filtered(apps, query);

    let texto_busqueda = if query.is_empty() {
        "Buscar aplicaciones…".to_string()
    } else {
        query.to_string()
    };
    let conteo = format!("{}", matches.len());
    let search = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(MENU_SEARCH_H) },
        align_items: Some(AlignItems::Center),
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(6.0)
    .children(vec![
        View::new(Style {
            size: Size { width: length(16.0_f32), height: length(MENU_SEARCH_H) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text("⌕".to_string(), 14.0, theme.accent),
        View::new(Style {
            flex_grow: 1.0,
            size: Size { width: auto(), height: length(MENU_SEARCH_H) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            texto_busqueda,
            13.0,
            if query.is_empty() { theme.fg_muted } else { theme.fg_text },
        ),
        View::new(Style {
            size: Size { width: auto(), height: length(MENU_SEARCH_H) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(conteo, 11.0, theme.fg_muted),
    ]);

    let filas: Vec<View<Msg>> = if matches.is_empty() {
        vec![View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(MENU_ROW_H) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            if query.is_empty() {
                "sin apps (¿XDG_DATA_DIRS? ¿~/.config/tawasuyu/apps/?)".to_string()
            } else {
                format!("sin resultados para «{query}»")
            },
            12.0,
            theme.fg_muted,
        )]
    } else {
        matches.iter().map(|a| app_row(a, theme)).collect()
    };

    let content_len = matches.len() as f32 * (MENU_ROW_H + MENU_ROW_GAP);
    let lista_inner = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: auto() },
        gap: Size { width: length(0.0_f32), height: length(MENU_ROW_GAP) },
        ..Default::default()
    })
    .children(filas);
    let lista = scroll_y(
        clamp_offset(offset, content_len, viewport_h),
        content_len,
        viewport_h,
        lista_inner,
        Msg::StartScroll,
        &ScrollPalette::from_theme(theme),
    );
    let lista_wrap = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(viewport_h.max(MENU_ROW_H)) },
        ..Default::default()
    })
    .children(vec![lista]);

    let panel = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(START_MENU_W),
            height: auto(),
        },
        flex_direction: FlexDirection::Column,
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .children(vec![search, lista_wrap]);

    // Scrim a ancho completo del área: cierra al click fuera del panel.
    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .on_click(Msg::StartToggle)
    .children(vec![panel])
}

/// Una fila del menú de inicio: ícono + label, clickeable.
fn app_row(a: &AppEntry, theme: &Theme) -> View<Msg> {
    let icon_raw = a.icon.as_deref();
    let glyph_or_default: String = icon_raw
        .filter(|s| s.chars().count() <= 2)
        .unwrap_or("▸")
        .to_string();
    let svg_asset = icon_raw
        .filter(|s| s.chars().count() > 2)
        .and_then(crate::app_icons::get_or_load);
    let badge_base = View::new(Style {
        size: Size { width: length(22.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    });
    let badge = match svg_asset {
        Some(asset) => badge_base.children(vec![asset.view::<Msg>()]),
        None => badge_base.text(glyph_or_default, 14.0, theme.accent),
    };
    let nombre = View::new(Style {
        size: Size { width: auto(), height: length(28.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(a.label.clone(), 13.0, theme.fg_text);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: TaffyRect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .radius(6.0)
    .hover_fill(theme.bg_button_hover)
    .on_click(Msg::LaunchApp(a.id.clone()))
    .children(vec![badge, nombre])
}

/// El menú de inicio como **overlay** para el path winit.
pub fn start_menu_overlay(
    apps: &[AppEntry],
    query: &str,
    offset: f32,
    bar_h: f32,
    screen_h: f32,
    theme: &Theme,
) -> View<Msg> {
    let viewport = (screen_h - bar_h - MENU_SEARCH_H - 28.0).max(MENU_ROW_H);
    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(bar_h),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![start_menu_body(apps, query, offset, viewport, theme)])
}

/// La barra superior con el menú de inicio **desplegado** hacia abajo.
pub fn start_menu_view(
    surface: &Surface,
    surface_widgets: &SurfaceWidgets,
    shuma_state: &ShumaState,
    data: &BarData,
    theme: &Theme,
    bar_px: f32,
    apps: &[AppEntry],
    query: &str,
    offset: f32,
    menu_h: f32,
) -> View<Msg> {
    let bar = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(bar_px),
        },
        ..Default::default()
    })
    .children(vec![bar_view(surface, surface_widgets, shuma_state, data, theme)]);

    let viewport = (menu_h - bar_px - MENU_SEARCH_H - 28.0).max(MENU_ROW_H);
    let mut body_style = Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    };
    body_style.flex_grow = 1.0;
    let body = View::new(body_style)
        .children(vec![start_menu_body(apps, query, offset, viewport, theme)]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![bar, body])
}

/// El historial de portapapeles para el **layer-shell**.
#[allow(clippy::too_many_arguments)]
pub fn clipboard_menu_view(
    surface: &Surface,
    surface_widgets: &SurfaceWidgets,
    shuma_state: &ShumaState,
    data: &BarData,
    theme: &Theme,
    bar_px: f32,
    history: &[String],
) -> View<Msg> {
    let bar = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(bar_px),
        },
        ..Default::default()
    })
    .children(vec![bar_view(surface, surface_widgets, shuma_state, data, theme)]);

    let mut body_style = Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    };
    body_style.flex_grow = 1.0;
    let body = View::new(body_style)
        .on_click(Msg::ClipboardMenu)
        .children(vec![clipboard_panel(history, theme)]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![bar, body])
}

// ============================================================
// Barra layer-shell
// ============================================================

/// La barra de **una** superficie llenando su contenedor (100%×100%).
pub fn bar_view(
    surface: &Surface,
    surface_widgets: &SurfaceWidgets,
    shuma_state: &ShumaState,
    data: &BarData,
    theme: &Theme,
) -> View<Msg> {
    let dir = if surface.anchor.es_horizontal() {
        FlexDirection::Row
    } else {
        FlexDirection::Column
    };
    bar_body(surface, surface_widgets, shuma_state, data, theme, dir)
}

/// **Dock estilo macOS**: una fila centrada, pegada al borde, con un ícono por
/// ventana abierta, **magnificados** según la cercanía del puntero. `cursor_x`
/// es la coord X local del panel (o `None` si el puntero no está encima). Cada
/// ícono activa su ventana al click. La magnificación es analítica (centros
/// sobre una grilla de tamaño base), independiente del layout — así no necesita
/// las posiciones ya calculadas por taffy.
pub fn dock_view(
    surface: &Surface,
    pins: &[AppEntry],
    windows: &[WindowEntry],
    theme: &Theme,
    panel_w: f32,
    cursor_x: Option<f32>,
) -> View<Msg> {
    const BASE: f32 = 40.0; // lado del ícono en reposo
    const MAX_SCALE: f32 = 1.9; // magnificación máxima bajo el cursor
    const RADIUS: f32 = 110.0; // alcance px de la lupa
    let gap = surface.gap.max(10.0);

    let fondo = if surface.gradient {
        theme.bg_panel_alt
    } else {
        theme.bg_panel
    };
    let contenedor = |hijos: Vec<View<Msg>>| {
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::FlexEnd), // íconos pegados al borde
            justify_content: Some(JustifyContent::Center),
            gap: Size {
                width: length(gap),
                height: length(0.0_f32),
            },
            padding: TaffyRect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(4.0_f32),
                bottom: length(4.0_f32),
            },
            ..Default::default()
        })
        .fill(fondo)
        .children(hijos)
    };

    // El dock es: apps fijadas (lanzadores) + ventanas abiertas (activadores),
    // en una sola fila magnificada por el puntero.
    let n = pins.len() + windows.len();
    if n == 0 {
        return contenedor(Vec::new());
    }
    let total = n as f32 * BASE + (n.saturating_sub(1)) as f32 * gap;
    let start_x = ((panel_w - total) / 2.0).max(10.0);
    let scale_de = |i: usize| -> f32 {
        let center = start_x + i as f32 * (BASE + gap) + BASE / 2.0;
        match cursor_x {
            Some(cx) => {
                let d = (center - cx).abs();
                if d >= RADIUS {
                    1.0
                } else {
                    1.0 + (MAX_SCALE - 1.0) * (1.0 - d / RADIUS)
                }
            }
            None => 1.0,
        }
    };
    let mut tiles: Vec<View<Msg>> = Vec::with_capacity(n);
    for (k, p) in pins.iter().enumerate() {
        tiles.push(dock_pin_tile(p, theme, BASE * scale_de(k)));
    }
    for (k, w) in windows.iter().enumerate() {
        tiles.push(dock_win_tile(w, theme, BASE * scale_de(pins.len() + k)));
    }
    contenedor(tiles)
}

/// Cascarón de un ícono del dock: tamaño `size` (ya magnificado), centrado.
fn dock_icon_shell(size: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(size),
            height: length(size),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
}

/// Pinta el ícono SVG de `name` (tema XDG) o, si no hay, una pastilla con la
/// `inicial`. Reutilizado por pins (apps) y ventanas.
fn dock_icon_inner(shell: View<Msg>, name: &str, inicial: &str, theme: &Theme, size: f32) -> View<Msg> {
    match crate::app_icons::get_or_load(name) {
        Some(asset) => shell.children(vec![asset.view::<Msg>()]),
        None => shell
            .fill(theme.bg_button)
            .radius(8.0)
            .text(inicial.to_string(), size * 0.42, theme.accent),
    }
}

/// Ícono de una **app fijada**: lanza la app al click.
fn dock_pin_tile(a: &AppEntry, theme: &Theme, size: f32) -> View<Msg> {
    let icon = a.icon.as_deref().unwrap_or(&a.id);
    let inicial = a
        .label
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "•".to_string());
    dock_icon_inner(dock_icon_shell(size), icon, &inicial, theme, size)
        .on_click(Msg::LaunchApp(a.id.clone()))
        .hover_fill(theme.bg_button_hover)
}

/// Ícono de una **ventana abierta**: la activa al click.
fn dock_win_tile(w: &WindowEntry, theme: &Theme, size: f32) -> View<Msg> {
    let inicial = w
        .label
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "•".to_string());
    dock_icon_inner(dock_icon_shell(size), &w.app_id, &inicial, theme, size)
        .on_click(Msg::ActivateWindow(w.id))
        .hover_fill(theme.bg_button_hover)
}

// ============================================================
// Utilidades internas compartidas por submódulos
// ============================================================

/// Recorta una cadena a `max` caracteres, agregando `…` si sobró.
pub(super) fn recortar(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::widget_tooltip;
    use pata_core::widget::{MeterOrient, MeterSize, WidgetView};

    #[test]
    fn tooltip_de_un_medidor_junta_etiqueta_y_leyenda() {
        let v = WidgetView::Meter {
            label: Some("CPU".into()),
            fraction: 0.42,
            caption: "42%".into(),
            size: MeterSize::Medium,
            orient: MeterOrient::Horizontal,
        };
        assert_eq!(widget_tooltip(&v).as_deref(), Some("CPU 42%"));
    }

    #[test]
    fn tooltip_de_texto_y_vacio() {
        assert_eq!(widget_tooltip(&WidgetView::Text("14:05".into())).as_deref(), Some("14:05"));
        assert_eq!(widget_tooltip(&WidgetView::Text("  ".into())), None);
        assert_eq!(widget_tooltip(&WidgetView::Empty), None);
    }

    #[test]
    fn tooltip_de_cores_incluye_la_cantidad() {
        let v = WidgetView::Cores {
            label: Some("CPU".into()),
            fractions: vec![0.1, 0.2, 0.3, 0.4],
            caption: "25% (4)".into(),
            size: MeterSize::Medium,
            orient: MeterOrient::Horizontal,
        };
        assert_eq!(widget_tooltip(&v).as_deref(), Some("CPU 25% (4) · 4 cores"));
    }
}
