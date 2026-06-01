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

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{
        auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style,
    },
    Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
use llimphi_ui::View;

use pata_core::config::Surface;
use pata_core::layout::Rect;
use pata_core::widget::WidgetView;

use crate::shuma::{self, ShumaState};
use crate::toplevel::WindowEntry;
use crate::tray::{TrayIcon, TrayItem};
use crate::{Model, Msg, SlotWidget, SurfaceWidgets};

/// Largo máximo de la etiqueta de una ventana en el `window_list` antes de
/// recortar con `…`. Evita que un título largo empuje el resto de la barra.
const WINDOW_LABEL_MAX: usize = 22;

/// Largo máximo del preview del portapapeles antes de recortar con `…`.
const CLIPBOARD_PREVIEW_MAX: usize = 28;

/// Largo máximo de la etiqueta de un item del tray antes de recortar con `…`.
const TRAY_LABEL_MAX: usize = 14;

/// Los datos del host que el render necesita además del view-model de los
/// widgets de core: lo dinámico que vive en el backend (ventanas abiertas,
/// portapapeles) y se pasa aparte. Agrupado para no inflar cada firma a medida
/// que se suman widgets de este tipo (mañana, el tray).
#[derive(Default)]
pub struct BarData<'a> {
    /// Las ventanas abiertas, para el `window_list`.
    pub windows: &'a [WindowEntry],
    /// El texto del portapapeles (ya en una línea), para el `clipboard`.
    pub clipboard: Option<&'a str>,
    /// Los items de la bandeja del sistema, para el `tray`.
    pub tray: &'a [TrayItem],
}

/// Ancho de la barrita de un medidor, en píxeles.
const BARRA_W: f32 = 48.0;

/// Ancho fijo de la leyenda de un medidor (px). Cabe `"10.5/15.5G"` (RAM), la
/// más ancha; evita que el cambio de dígitos reacomode la barra.
const CAPTION_W: f32 = 72.0;

/// Traduce el view-model de un widget al `View<Msg>` que lo pinta.
pub fn widget_view(v: &WidgetView, theme: &Theme) -> View<Msg> {
    match v {
        WidgetView::Empty => View::new(Style {
            size: Size {
                width: length(0.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        }),
        WidgetView::Text(t) => chip(theme).text(t.clone(), 13.0, theme.fg_text),
        WidgetView::Meter {
            label,
            fraction,
            caption,
        } => meter_view(label.as_deref(), *fraction, caption, theme),
        WidgetView::Placeholder(kind) => chip(theme)
            .fill(theme.bg_panel)
            .radius(6.0)
            .text(kind.clone(), 12.0, theme.fg_muted),
    }
}

/// Un contenedor compacto, centrado, con padding horizontal — la base de
/// cualquier widget de barra.
fn chip(_theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: auto(),
            height: length(22.0_f32),
        },
        padding: TaffyRect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
}

/// Un medidor: etiqueta opcional + barrita proporcional + leyenda.
fn meter_view(label: Option<&str>, fraction: f32, caption: &str, theme: &Theme) -> View<Msg> {
    let frac = fraction.clamp(0.0, 1.0);
    let barra = View::new(Style {
        size: Size {
            width: length(BARRA_W),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(2.0)
    .children(vec![View::new(Style {
        size: Size {
            width: length(BARRA_W * frac),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.accent)
    .radius(2.0)]);

    let mut hijos: Vec<View<Msg>> = Vec::new();
    if let Some(l) = label {
        hijos.push(etiqueta(l, theme));
    }
    hijos.push(barra);
    if !caption.is_empty() {
        // Ancho FIJO: la leyenda cambia de dígitos cada tick ("7%"→"42%"→
        // "100%") y, con ancho automático, eso reflota toda la barra. Una caja
        // fija mantiene el layout quieto.
        hijos.push(caption_fija(caption, theme));
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: length(22.0_f32),
        },
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(hijos)
}

/// La leyenda de un medidor en una caja de **ancho fijo**: como el texto cambia
/// de dígitos a cada tick, una caja fija evita que el medidor (y con él toda la
/// barra) se reacomode. Cabe la más ancha (`"10.5/15.5G"` de la RAM).
fn caption_fija(t: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(CAPTION_W),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        ..Default::default()
    })
    .text(t.to_string(), 12.0, theme.fg_muted)
}

/// Un texto corto en color tenue (etiqueta o leyenda de un medidor).
fn etiqueta(t: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: auto(),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(t.to_string(), 12.0, theme.fg_muted)
}

/// El `View` raíz: cubre la pantalla y coloca cada superficie en su rect.
pub fn root(model: &Model) -> View<Msg> {
    let (sw, sh) = model.screen;
    let mut superficies: Vec<View<Msg>> = Vec::new();

    for placed in &model.frame.surfaces {
        let surface = &model.cfg.surfaces[placed.index];
        let widgets = &model.surfaces[placed.index];
        if !placed.rect.es_visible() {
            continue;
        }
        superficies.push(surface_view(
            surface,
            placed.rect,
            widgets,
            &model.shuma,
            // Bajo el compositor mirada (este path winit) los datos del host
            // (ventanas, portapapeles) aún no se muestrean; llegarán por su IPC.
            &BarData::default(),
            &model.theme,
        ));
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

/// Una superficie colocada: rectángulo absoluto con los tres slots repartidos
/// a lo largo de su eje (fila si el anclaje es horizontal, columna si vertical).
fn surface_view(
    surface: &Surface,
    rect: Rect,
    widgets: &SurfaceWidgets,
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
    .fill(theme.bg_panel_alt)
    .children(slots_de(surface, widgets, shuma_state, data, theme, dir))
}

/// La barra de shuma **desplegada**: la propia layer surface creció hacia
/// arriba, así que pintamos el cuerpo del drawer (input + salida) llenando lo
/// alto y la barra (su cabezal) abajo, con su grosor original `bar_px`. Asume
/// anclaje inferior (el caso del preset).
pub fn shuma_open_view(
    surface: &Surface,
    widgets: &SurfaceWidgets,
    shuma_state: &ShumaState,
    data: &BarData,
    theme: &Theme,
    bar_px: f32,
) -> View<Msg> {
    // El cuerpo del drawer ocupa todo lo que sobra por encima de la barra.
    let mut body_style = Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    };
    body_style.flex_grow = 1.0;
    let body = View::new(body_style).children(vec![shuma::drawer_body_view(shuma_state, theme)]);

    let bar = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(bar_px),
        },
        ..Default::default()
    })
    .children(vec![bar_view(surface, widgets, shuma_state, data, theme)]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![body, bar])
}

/// Construye los tres slots (start/center/end) de una superficie a lo largo de
/// su eje. Compartido por [`surface_view`] (una superficie colocada en su rect
/// dentro de una ventana grande) y [`bar_view`] (la barra llenando su propia
/// layer surface de Wayland).
fn slots_de(
    surface: &Surface,
    widgets: &SurfaceWidgets,
    shuma_state: &ShumaState,
    data: &BarData,
    theme: &Theme,
    dir: FlexDirection,
) -> Vec<View<Msg>> {
    let slot = |ws: &[SlotWidget], justify: JustifyContent| -> View<Msg> {
        let items: Vec<View<Msg>> = ws
            .iter()
            .map(|sw| match sw {
                SlotWidget::Core { widget, exec } => {
                    let v = widget_view(&widget.view(), theme);
                    match exec {
                        // Con `exec`: clickeable (lanza el comando) + realce al hover.
                        Some(cmd) => v
                            .radius(6.0)
                            .hover_fill(theme.bg_button_hover)
                            .on_click(Msg::Spawn(cmd.clone())),
                        None => v,
                    }
                }
                SlotWidget::Shuma => shuma::headline_view(shuma_state, theme),
                SlotWidget::WindowList => window_list_view(data.windows, surface.gap, dir, theme),
                SlotWidget::Clipboard { exec } => {
                    clipboard_view(data.clipboard, exec.as_deref(), theme)
                }
                SlotWidget::Tray => tray_view(data.tray, surface.gap, dir, theme),
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
        slot(&widgets.start, JustifyContent::FlexStart),
        slot(&widgets.center, JustifyContent::Center),
        slot(&widgets.end, JustifyContent::FlexEnd),
    ]
}

/// El `window_list`: un chip clickeable por ventana abierta, resaltando la
/// activa. Clickear envía [`Msg::ActivateWindow`] con su `id`; el backend
/// layer-shell la trae al frente. Los chips siguen el eje de la barra (fila u
/// columna) con el mismo `gap` que el resto de los slots.
fn window_list_view(
    windows: &[WindowEntry],
    gap: f32,
    dir: FlexDirection,
    theme: &Theme,
) -> View<Msg> {
    let chips: Vec<View<Msg>> = windows
        .iter()
        .map(|w| {
            let texto = recortar(&w.label, WINDOW_LABEL_MAX);
            // La activa va con texto pleno + fondo de panel; el resto, tenue.
            let (fg, fill) = if w.active {
                (theme.fg_text, theme.bg_panel)
            } else {
                (theme.fg_muted, theme.bg_panel_alt)
            };
            chip(theme)
                .fill(fill)
                .radius(6.0)
                .hover_fill(theme.bg_button_hover)
                .on_click(Msg::ActivateWindow(w.id))
                .text(texto, 12.0, fg)
        })
        .collect();

    View::new(Style {
        flex_direction: dir,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(gap),
            height: length(gap),
        },
        ..Default::default()
    })
    .children(chips)
}

/// El `clipboard`: un chip con el ícono 📋 y un preview del texto copiado
/// (recortado). Si `exec` está, clickearlo lanza ese comando —típicamente un
/// selector de historial (cliphist)— con realce al hover. Sin texto copiado
/// muestra sólo el ícono tenue.
fn clipboard_view(text: Option<&str>, exec: Option<&str>, theme: &Theme) -> View<Msg> {
    let (etiqueta, fg) = match text {
        Some(t) if !t.is_empty() => (format!("📋 {}", recortar(t, CLIPBOARD_PREVIEW_MAX)), theme.fg_text),
        _ => ("📋".to_string(), theme.fg_muted),
    };
    let v = chip(theme).text(etiqueta, 12.0, fg);
    match exec {
        Some(cmd) => v
            .radius(6.0)
            .hover_fill(theme.bg_button_hover)
            .on_click(Msg::Spawn(cmd.to_string())),
        None => v,
    }
}

/// Tamaño del ícono del tray en la barra (px).
const TRAY_ICON_PX: f32 = 18.0;

/// El `tray`: un chip clickeable por item de la bandeja, resaltando los que
/// piden atención (`NeedsAttention`). Click → [`Msg::TrayActivate`] con su `key`;
/// el backend activa el item por D-Bus. Pinta el ícono si la app lo proveyó (pixmap
/// o PNG por nombre); si no, cae a la etiqueta de texto. Los chips siguen el eje.
fn tray_view(items: &[TrayItem], gap: f32, dir: FlexDirection, theme: &Theme) -> View<Msg> {
    let chips: Vec<View<Msg>> = items
        .iter()
        .map(|it| {
            let base = chip(theme)
                .fill(theme.bg_panel_alt)
                .radius(6.0)
                .hover_fill(theme.bg_button_hover)
                .on_click(Msg::TrayActivate(it.key.clone()));
            match &it.icon {
                Some(icon) => base.children(vec![tray_icon_node(icon)]),
                None => {
                    // NeedsAttention: acento; el resto, normal.
                    let fg = if it.status == "NeedsAttention" {
                        theme.accent
                    } else {
                        theme.fg_text
                    };
                    base.text(recortar(&it.label, TRAY_LABEL_MAX), 12.0, fg)
                }
            }
        })
        .collect();

    View::new(Style {
        flex_direction: dir,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(gap),
            height: length(gap),
        },
        ..Default::default()
    })
    .children(chips)
}

/// Un nodo cuadrado de [`TRAY_ICON_PX`] con el ícono del item (aspect-fit). Arma la
/// `peniko::Image` desde los bytes RGBA que el hilo del tray ya decodificó.
fn tray_icon_node(icon: &TrayIcon) -> View<Msg> {
    let blob = Blob::from(icon.rgba.clone());
    let img = Image::new(blob, ImageFormat::Rgba8, icon.width, icon.height);
    View::new(Style {
        size: Size {
            width: length(TRAY_ICON_PX),
            height: length(TRAY_ICON_PX),
        },
        ..Default::default()
    })
    .image(img)
}

/// Recorta una cadena a `max` caracteres, agregando `…` si sobró.
fn recortar(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// La barra de **una** superficie llenando su contenedor (100%×100%): la raíz
/// que pinta el backend `wlr-layer-shell`, donde el compositor ya dimensionó y
/// ancló la layer surface al borde — no hace falta posicionarla en absoluto.
pub fn bar_view(
    surface: &Surface,
    widgets: &SurfaceWidgets,
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
    .fill(theme.bg_panel_alt)
    .children(slots_de(surface, widgets, shuma_state, data, theme, dir))
}
