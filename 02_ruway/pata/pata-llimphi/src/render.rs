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

use llimphi_theme::{Color, Theme};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{
        auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style,
    },
    Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
use llimphi_ui::View;
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};

use app_bus::AppEntry;
use pata_core::config::{FloatingCard, Surface, SurfaceKind};
use pata_core::layout::Rect;
use pata_core::widget::{Widget, WidgetView};

use crate::shuma::{self, ShumaState};
use crate::toplevel::WindowEntry;
use crate::tray::{TrayIcon, TrayItem};
use crate::{Model, Msg, SlotWidget, SurfaceWidgets};

mod sidebar;
pub use sidebar::{nav_panel_view, sidebar_rail_view, sidebar_surface_view};

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

/// El texto de tooltip de un widget de core, derivado de su view-model: la
/// lectura completa (medidor con su etiqueta + leyenda, texto tal cual). `None`
/// para los vacíos. Lo muestra el tooltip flotante al posar el cursor.
pub fn widget_tooltip(v: &WidgetView) -> Option<String> {
    match v {
        WidgetView::Empty => None,
        WidgetView::Text(t) if t.trim().is_empty() => None,
        WidgetView::Text(t) => Some(t.clone()),
        WidgetView::Meter { label, caption, .. } => {
            let l = label.as_deref().unwrap_or("").trim();
            let c = caption.trim();
            let s = format!("{l} {c}");
            let s = s.trim().to_string();
            (!s.is_empty()).then_some(s)
        }
        WidgetView::Placeholder(kind) => Some(kind.clone()),
    }
}

/// El cuerpo del **tooltip flotante**: una cajita opaca con el texto, rellenando
/// su contenedor (en layer-shell, la propia surface popup). Opaca a propósito —
/// así no depende de transparencia de la surface (que en algún compositor podría
/// fallar y ennegrecer todo).
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

/// Traduce el view-model de un widget al `View<Msg>` que lo pinta (sin teñido
/// por `kind` — los medidores caen al gradiente del acento). Lo usan las
/// tarjetas flotantes, que no rastrean el `kind`.
pub fn widget_view(v: &WidgetView, theme: &Theme) -> View<Msg> {
    widget_view_kinded(v, None, theme)
}

/// Como [`widget_view`] pero con el `kind` del widget, para que el medidor use
/// su gradiente propio (verde→rojo teñido por widget, [`meter_stops`]).
pub fn widget_view_kinded(v: &WidgetView, kind: Option<&str>, theme: &Theme) -> View<Msg> {
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
        } => {
            let stops = match kind {
                Some(k) => meter_stops(k),
                None => (theme.accent, aclarar(theme.accent, 0.5)),
            };
            meter_view(label.as_deref(), *fraction, caption, theme, stops)
        }
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

/// Aclara un color hacia el blanco en `amount` (`0.0` = igual, `1.0` = blanco).
/// Para el extremo claro del gradiente de los medidores.
fn aclarar(c: Color, amount: f32) -> Color {
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    let [r, g, b, a] = c.components;
    let m = amount.clamp(0.0, 1.0);
    AlphaColor::new([r + (1.0 - r) * m, g + (1.0 - g) * m, b + (1.0 - b) * m, a])
}

/// El mismo color con su alfa multiplicado por `op` (`0..1`). Para barras
/// translúcidas (`Surface::opacity`) sin teñir los widgets de adentro.
fn con_opacidad(c: Color, op: f32) -> Color {
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    let [r, g, b, a] = c.components;
    AlphaColor::new([r, g, b, a * op.clamp(0.0, 1.0)])
}

/// Parsea un color hex `#rrggbb` o `#rrggbbaa` (el `#` es opcional). `None` si no
/// cuadra. Lo usa el acento configurable (`general.accent`).
pub fn parse_hex(s: &str) -> Option<Color> {
    let h = s.trim().trim_start_matches('#');
    let par = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some(Color::from_rgba8(par(0)?, par(2)?, par(4)?, 255)),
        8 => Some(Color::from_rgba8(par(0)?, par(2)?, par(4)?, par(6)?)),
        _ => None,
    }
}

/// Color desde HSV (`h` en grados `0..360`, `s`/`v` en `0..1`). Base del
/// gradiente verde→rojo de los medidores, que rota el matiz por widget.
fn hsv(h: f32, s: f32, v: f32) -> Color {
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    let h = h.rem_euclid(360.0);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    AlphaColor::new([r + m, g + m, b + m, 1.0])
}

/// Los dos extremos del gradiente de un medidor según su `kind`: **verde (bajo)
/// → rojo (alto)**, pero con una **tonalidad propia** por widget (un corrimiento
/// de matiz) para que el racimo de indicadores no sea monocromo. El corrimiento
/// del extremo rojo va atenuado para que siga leyéndose como rojo.
fn meter_stops(kind: &str) -> (Color, Color) {
    let shift = match kind {
        "cpu_meter" => 0.0,
        "ram_meter" => 18.0,
        "volume" => -22.0,
        "brightness" => 36.0,
        _ => 8.0,
    };
    let verde = hsv(135.0 + shift, 0.60, 0.80);
    let rojo = hsv(4.0 + shift * 0.30, 0.78, 0.92);
    (verde, rojo)
}

/// Celdas de ancho que un `kind` reserva por defecto en la grilla (`cell`), si
/// el spec no fija `cells`. Los medidores ocupan más; los chips de texto, una.
fn default_cells(kind: &str) -> u32 {
    match kind {
        "cpu_meter" | "ram_meter" | "volume" | "brightness" => 3,
        "clock" => 2,
        "astro" => 4,
        "weather" => 3,
        "cava" => 3,
        _ => 1,
    }
}

/// Envuelve `v` en un contenedor con **ancho (o alto) mínimo cuantizado** a la
/// grilla de la barra (`cell`): el widget reserva al menos `cell * n` px sobre el
/// eje principal, así el racimo de indicadores queda alineado en vez de bailar
/// con cada cambio de dígitos. `cell <= 0` desactiva la grilla (ancho
/// automático). `n` sale de la prop `cells` o, si es 0, de [`default_cells`].
fn cuantizar(v: View<Msg>, cell: f32, cells: u32, kind: &str, dir: FlexDirection) -> View<Msg> {
    if cell <= 0.0 {
        return v;
    }
    let n = if cells > 0 { cells } else { default_cells(kind) };
    let q = length(cell * n as f32);
    let min_size = if matches!(dir, FlexDirection::Row) {
        Size { width: q, height: auto() }
    } else {
        Size { width: auto(), height: q }
    };
    View::new(Style {
        min_size,
        flex_direction: dir,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![v])
}

/// Cablea la interacción de un widget de core según su `kind`. Los kinds con
/// interacción propia (volumen, brillo, reloj) la traen acá; el resto cae al
/// `exec` configurable (click → lanzar comando), si lo hay.
fn interaccion_widget(v: View<Msg>, kind: &str, exec: Option<&str>) -> View<Msg> {
    match kind {
        "volume" => volume_interactivo(v, exec),
        "brightness" => brightness_interactivo(v),
        "clock" => clock_interactivo(v),
        _ => match exec {
            Some(cmd) => v.on_click(Msg::Spawn(cmd.to_string())),
            None => v,
        },
    }
}

/// Volumen interactivo: rueda sube/baja, click togglea mute, click derecho abre
/// el mezclador (`exec`) o el panel. (Cableado completo en el bloque de
/// interacción.)
fn volume_interactivo(v: View<Msg>, exec: Option<&str>) -> View<Msg> {
    match exec {
        Some(cmd) => v.on_click(Msg::Spawn(cmd.to_string())),
        None => v,
    }
}

/// Brillo interactivo: la rueda ajusta la luminosidad. (Cableado completo en el
/// bloque de interacción.)
fn brightness_interactivo(v: View<Msg>) -> View<Msg> {
    v
}

/// Reloj interactivo: el click abre el panel para fijar fecha/hora. (Cableado
/// completo en el bloque de interacción.)
fn clock_interactivo(v: View<Msg>) -> View<Msg> {
    v
}

/// Un medidor: etiqueta opcional + barrita proporcional + leyenda. La barra de
/// relleno lleva un **gradiente** horizontal del acento (izquierda) a un acento
/// aclarado (derecha), pintado a mano con `paint_with` (Llimphi no tiene fill de
/// brush, sólo color sólido).
fn meter_view(
    label: Option<&str>,
    fraction: f32,
    caption: &str,
    theme: &Theme,
    stops: (Color, Color),
) -> View<Msg> {
    let frac = fraction.clamp(0.0, 1.0);
    let (c0, c1) = stops;
    let relleno = View::new(Style {
        size: Size {
            width: length(BARRA_W * frac),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, RoundedRect};
        use llimphi_ui::llimphi_raster::peniko::{Fill, Gradient};
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let (x0, y0) = (rect.x as f64, rect.y as f64);
        let (x1, y1) = ((rect.x + rect.w) as f64, (rect.y + rect.h) as f64);
        let rr = RoundedRect::new(x0, y0, x1, y1, 2.0);
        // El gradiente abarca **toda** la barra (no sólo el relleno): así un
        // valor bajo muestra el tramo verde y uno alto llega al rojo —el color
        // indica el nivel, no sólo el largo—. El relleno recorta su porción.
        let x_full = x0 + BARRA_W as f64;
        let g = Gradient::new_linear(Point::new(x0, y0), Point::new(x_full, y0))
            .with_stops([c0, c1].as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rr);
    });
    let barra = View::new(Style {
        size: Size {
            width: length(BARRA_W),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(2.0)
    .children(vec![relleno]);

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

/// El **interior** de una tarjeta flotante (estilo conky): título opcional +
/// widgets apilados, rellenando su contenedor (100%×100%). Lo usa el backend
/// layer-shell, donde la propia layer surface ya tiene el tamaño de la tarjeta.
/// Para el path winit, [`card_view_absolute`] lo posiciona en (x, y).
pub fn card_view(card: &FloatingCard, widgets: &[Box<dyn Widget>], theme: &Theme) -> View<Msg> {
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
    for w in widgets {
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

/// Una tarjeta flotante posicionada en **absoluto** en (x, y) con tamaño (w, h)
/// — para el path winit, donde todas las superficies viven en una sola ventana.
fn card_view_absolute(card: &FloatingCard, widgets: &[Box<dyn Widget>], theme: &Theme) -> View<Msg> {
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
    .children(vec![card_view(card, widgets, theme)])
}

/// El `View` raíz: cubre la pantalla y coloca cada superficie en su rect.
pub fn root(model: &Model) -> View<Msg> {
    let (sw, sh) = model.screen;
    let mut superficies: Vec<View<Msg>> = Vec::new();

    // Datos del host muestreados por el Model: portapapeles y tray ya funcionan en
    // este path winit. El `window_list` queda vacío hasta que el compositor mirada
    // exponga sus toplevels por IPC (en layer-shell sí se llena).
    let tray_items = model.tray.as_ref().map(|t| t.items()).unwrap_or_default();
    let data = BarData {
        windows: &[],
        clipboard: model.clipboard.as_deref(),
        tray: &tray_items,
    };

    for placed in &model.frame.surfaces {
        let surface = &model.cfg.surfaces[placed.index];
        let widgets = &model.surfaces[placed.index];
        if !placed.rect.es_visible() {
            continue;
        }
        // Un Sidebar no tiene slots: pinta el rail de dientes a partir de
        // `surface.tabs` (su panel flota aparte, después, para quedar encima).
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

    // El panel del diente desplegado flota sobre el área de trabajo, junto al
    // rail (no entra en el layout — lo maneja el frontend, como un drawer).
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

    // Tarjetas flotantes (estilo conky), posicionadas en absoluto sobre la
    // pantalla. En layer-shell cada una es su propia surface; acá (winit) viven
    // en la ventana única.
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

/// Aplica la **apariencia configurable** al cuerpo de la barra `v`: fondo
/// translúcido (`opacity`) o degradé vertical sutil (`gradient`) y esquinas
/// redondeadas (`radius`). Compartido por el path winit y el layer-shell.
fn aplicar_apariencia(v: View<Msg>, surface: &Surface, theme: &Theme) -> View<Msg> {
    let bg = con_opacidad(theme.bg_panel_alt, surface.opacity);
    let v = if surface.gradient {
        use llimphi_ui::llimphi_raster::kurbo::Point;
        use llimphi_ui::llimphi_raster::peniko::Gradient;
        let top = con_opacidad(aclarar(theme.bg_panel_alt, 0.10), surface.opacity);
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
/// transparente de padding `margin` (el look de barra **flotante**). La reserva
/// de franja no cambia: el margen es sólo pincel.
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

/// El cuerpo de una barra (100%×100% de su contenedor): los tres slots a lo
/// largo de su eje, con la apariencia configurable aplicada. Lo comparten
/// [`surface_view`] (winit, dentro de un rect absoluto) y [`bar_view`]
/// (layer-shell, llenando su layer surface).
fn bar_body(
    surface: &Surface,
    widgets: &SurfaceWidgets,
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
    .children(slots_de(surface, widgets, shuma_state, data, theme, dir));
    envolver_margen(aplicar_apariencia(cuerpo, surface, theme), surface)
}

/// Una superficie colocada: rectángulo absoluto que aloja el cuerpo de la barra
/// (con su apariencia + slots repartidos por su eje).
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
        ..Default::default()
    })
    .children(vec![bar_body(surface, widgets, shuma_state, data, theme, dir)])
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
    viewport_h: f32,
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
    let body =
        View::new(body_style).children(vec![shuma::drawer_body_view(shuma_state, theme, viewport_h)]);

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

/// Como [`shuma_open_view`] pero con el cuerpo ya construido por el caller (el
/// terminal PTY del drawer Quake). Mantiene la barra-cabezal abajo con su grosor
/// original. `body` ya viene dimensionado a `surface - bar_px`.
pub fn shuma_open_with_body(
    surface: &Surface,
    widgets: &SurfaceWidgets,
    shuma_state: &ShumaState,
    data: &BarData,
    theme: &Theme,
    bar_px: f32,
    body: View<Msg>,
) -> View<Msg> {
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
                SlotWidget::Core {
                    kind,
                    widget,
                    exec,
                    cells,
                } => {
                    // Realce al hover en todos los widgets (feedback de "estoy
                    // encima") + tooltip con su lectura completa; los que tienen
                    // `exec` además lanzan su comando. El medidor se tiñe con su
                    // gradiente propio (verde→rojo por widget).
                    let wv = widget.view();
                    let mut v = widget_view_kinded(&wv, Some(kind), theme)
                        .radius(6.0)
                        .hover_fill(theme.bg_button_hover);
                    if let Some(tip) = widget_tooltip(&wv) {
                        v = v.tooltip(tip);
                    }
                    v = interaccion_widget(v, kind, exec.as_deref());
                    cuantizar(v, surface.cell, *cells, kind, dir)
                }
                SlotWidget::Start { label, exec } => start_button_view(label, exec.as_deref(), theme),
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

/// Lado del ícono-badge (cuadrado) de una ventana en el task manager, en px.
const WIN_BADGE_PX: f32 = 18.0;

/// El **task manager** (estilo KDE): un botón por ventana abierta con un
/// ícono-badge (la inicial del `app_id`) + el título. La activa va resaltada con
/// fondo de panel y badge en acento; las minimizadas, atenuadas. Clic izquierdo
/// → [`Msg::ActivateWindow`] (activa, o minimiza si ya estaba activa); clic
/// derecho → [`Msg::CloseWindow`]. Los botones siguen el eje de la barra.
fn window_list_view(
    windows: &[WindowEntry],
    gap: f32,
    dir: FlexDirection,
    theme: &Theme,
) -> View<Msg> {
    let chips: Vec<View<Msg>> = windows.iter().map(|w| window_button(w, theme)).collect();

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

/// Un botón de ventana del task manager: badge (inicial) + título recortado.
fn window_button(w: &WindowEntry, theme: &Theme) -> View<Msg> {
    // Activa: fondo de panel, texto pleno, badge en acento. Inactiva: tenue.
    // Minimizada: aún más atenuada (texto y badge en muted).
    let (fg, fill, badge_bg, badge_fg) = if w.active {
        (theme.fg_text, theme.bg_panel, theme.accent, theme.bg_panel)
    } else if w.minimized {
        (theme.fg_muted, theme.bg_panel_alt, theme.bg_panel, theme.fg_muted)
    } else {
        (theme.fg_text, theme.bg_panel_alt, theme.bg_panel, theme.fg_muted)
    };

    let badge = View::new(Style {
        size: Size {
            width: length(WIN_BADGE_PX),
            height: length(WIN_BADGE_PX),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(badge_bg)
    .radius(4.0)
    .text(w.inicial(), 11.0, badge_fg);

    let titulo = View::new(Style {
        size: Size {
            width: auto(),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(recortar(&w.label, WINDOW_LABEL_MAX), 12.0, fg);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: length(24.0_f32),
        },
        padding: TaffyRect {
            left: length(6.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(fill)
    .radius(6.0)
    .hover_fill(theme.bg_button_hover)
    .tooltip(w.label.clone())
    .on_click(Msg::ActivateWindow(w.id))
    .on_right_click(Msg::CloseWindow(w.id))
    .children(vec![badge, titulo])
}

/// El **botón de inicio**: un chip con su label/ícono. Clic → despliega el menú
/// nativo de apps ([`Msg::StartToggle`]), salvo que la config fije `exec` (en
/// cuyo caso lanza ese comando, override estilo waybar).
fn start_button_view(label: &str, exec: Option<&str>, theme: &Theme) -> View<Msg> {
    let click = match exec {
        Some(cmd) => Msg::Spawn(cmd.to_string()),
        None => Msg::StartToggle,
    };
    chip(theme)
        .fill(theme.bg_panel)
        .radius(6.0)
        .hover_fill(theme.bg_button_hover)
        .tooltip(if exec.is_some() { "Lanzar" } else { "Menú de inicio" })
        .on_click(click)
        .text(label.to_string(), 14.0, theme.accent)
}

/// Ancho del menú de inicio desplegado, en px.
const START_MENU_W: f32 = 280.0;

/// El **menú de inicio** desplegado bajo la barra superior: un scrim que cierra
/// al click + un panel a la izquierda con una fila por app del registro. Pensado
/// para llenar el área que la barra superior libera al crecer hacia abajo (mismo
/// truco que el drawer Quake, pero hacia abajo). Cada fila lanza su app
/// ([`Msg::LaunchApp`]); si el registro está vacío, una pista.
/// Alto de cada fila del menú (px) — debe seguir a [`app_row`].
const MENU_ROW_H: f32 = 28.0;
/// Gap vertical entre filas (px) — debe seguir al `gap` del panel.
const MENU_ROW_GAP: f32 = 2.0;
/// Alto del campo de búsqueda (px).
const MENU_SEARCH_H: f32 = 34.0;

/// Filtra el registro por `query` (substring, sin distinguir mayúsculas) sobre
/// el label. El registro ya viene ordenado alfabéticamente por label.
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

    // Campo de búsqueda: glyph de lupa + lo tecleado (o placeholder) + conteo.
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

    // Filas filtradas, dentro de un área scrolleable (las apps del sistema son
    // muchas; el buscador estrecha y la rueda recorre el resto).
    let filas: Vec<View<Msg>> = if matches.is_empty() {
        vec![View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(MENU_ROW_H) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            if query.is_empty() {
                "sin apps (¿XDG_DATA_DIRS? ¿~/.config/gioser/apps/?)".to_string()
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
    .fill(theme.bg_app)
    .alpha(0.45)
    .on_click(Msg::StartToggle)
    .children(vec![panel])
}

/// Una fila del menú de inicio: ícono (glyph) + label, clickeable.
fn app_row(a: &AppEntry, theme: &Theme) -> View<Msg> {
    // El ícono de una app gioser es un glyph (1 char); el de un `.desktop` es un
    // nombre freedesktop (palabra) que no sabemos resolver a imagen acá → cae a
    // un glyph genérico para que la fila quede prolija.
    let icono = a
        .icon
        .as_deref()
        .filter(|s| s.chars().count() <= 2)
        .unwrap_or("▸")
        .to_string();
    let badge = View::new(Style {
        size: Size { width: length(22.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(icono, 14.0, theme.accent);
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

/// El menú de inicio como **overlay** para el path winit: un contenedor a
/// pantalla completa desplazado `bar_h` px hacia abajo (para que el panel caiga
/// bajo la barra superior) que aloja [`start_menu_body`]. El scrim del body
/// cierra al click.
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

/// La barra superior con el menú de inicio **desplegado** hacia abajo: la barra
/// arriba (su grosor original) y el menú llenando lo que queda. Espeja
/// [`shuma_open_view`] pero hacia abajo (anclaje superior). El compositor ya
/// creció la layer surface a [`crate::layer`]'s alto de menú.
pub fn start_menu_view(
    surface: &Surface,
    widgets: &SurfaceWidgets,
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
    .children(vec![bar_view(surface, widgets, shuma_state, data, theme)]);

    // El cuerpo (menú desplegado) mide el alto de surface menos la barra; el
    // área scrolleable de la lista descuenta el campo de búsqueda y los paddings.
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

/// El `clipboard`: un chip con el ícono 📋 y un preview del texto copiado
/// (recortado). Si `exec` está, clickearlo lanza ese comando —típicamente un
/// selector de historial (cliphist)— con realce al hover. Sin texto copiado
/// muestra sólo el ícono tenue.
fn clipboard_view(text: Option<&str>, exec: Option<&str>, theme: &Theme) -> View<Msg> {
    let (etiqueta, fg) = match text {
        Some(t) if !t.is_empty() => (format!("📋 {}", recortar(t, CLIPBOARD_PREVIEW_MAX)), theme.fg_text),
        _ => ("📋".to_string(), theme.fg_muted),
    };
    // Tooltip: el texto copiado completo (sin recortar), útil cuando el preview
    // de la barra lo trunca.
    let v = chip(theme)
        .hover_fill(theme.bg_button_hover)
        .radius(6.0)
        .text(etiqueta, 12.0, fg);
    let v = match text {
        Some(t) if !t.is_empty() => v.tooltip(t.to_string()),
        _ => v,
    };
    match exec {
        Some(cmd) => v.on_click(Msg::Spawn(cmd.to_string())),
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
            let tip = if it.label.trim().is_empty() {
                it.key.clone()
            } else {
                it.label.clone()
            };
            let base = chip(theme)
                .fill(theme.bg_panel_alt)
                .radius(6.0)
                .hover_fill(theme.bg_button_hover)
                .tooltip(tip)
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
    bar_body(surface, widgets, shuma_state, data, theme, dir)
}

#[cfg(test)]
mod tests {
    use super::widget_tooltip;
    use pata_core::widget::WidgetView;

    #[test]
    fn tooltip_de_un_medidor_junta_etiqueta_y_leyenda() {
        let v = WidgetView::Meter {
            label: Some("CPU".into()),
            fraction: 0.42,
            caption: "42%".into(),
        };
        assert_eq!(widget_tooltip(&v).as_deref(), Some("CPU 42%"));
    }

    #[test]
    fn tooltip_de_texto_y_vacio() {
        assert_eq!(widget_tooltip(&WidgetView::Text("14:05".into())).as_deref(), Some("14:05"));
        assert_eq!(widget_tooltip(&WidgetView::Text("  ".into())), None);
        assert_eq!(widget_tooltip(&WidgetView::Empty), None);
    }
}
