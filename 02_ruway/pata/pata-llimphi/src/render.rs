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
use llimphi_ui::llimphi_raster::peniko::{
    Blob, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
};
use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::{PaintRect, View};
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};

use app_bus::AppEntry;
use pata_core::config::{FloatingCard, Surface, SurfaceKind};
use pata_core::layout::Rect;
use pata_core::widget::{MeterOrient, MeterSize, Widget, WidgetCtx, WidgetView};

use crate::shuma::{self, ShumaState};
use crate::toplevel::WindowEntry;
use crate::tray::{TrayIcon, TrayItem};
use crate::{Model, Msg, SlotWidget, SurfaceWidgets};

mod sidebar;
mod start_menus;
pub use start_menus::{start_menu_gnome_overlay, start_menu_xp_overlay};
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
    /// La última lectura del clima, para el `weather`.
    pub weather: Option<&'a crate::weather::Weather>,
    /// El último cuadro del visualizador de audio, para el `cava`: una barra por
    /// banda, fracción `0..1`.
    pub cava: &'a [f32],
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
        WidgetView::TextRich { text, .. } => chip(theme).text(text.clone(), 16.0, theme.fg_text),
        WidgetView::Meter {
            label,
            fraction,
            caption,
            size,
            orient,
        } => {
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
            // Para tarjetas flotantes (que no rastrean kind/dir): fila horizontal
            // con gap chico. En la barra, el slot lo pinta con su gap real.
            workspaces_view(*active, *count, *occupied, 4.0, FlexDirection::Row, theme)
        }
        WidgetView::Placeholder(kind) => chip(theme)
            .fill(theme.bg_panel)
            .radius(6.0)
            .text(kind.clone(), 12.0, theme.fg_muted),
    }
}

/// Glifo de cabecera con color propio por widget — el "iconito colorido" que
/// el usuario reconoce de un saque (parlante naranja para volumen, sol amarillo
/// para brillo, etc.). Devuelve `None` para kinds que no llevan icono.
fn kind_icon(kind: &str) -> Option<(&'static str, Color)> {
    match kind {
        "volume" => Some(("♪", Color::from_rgba8(255, 152, 64, 255))),
        "brightness" => Some(("☀", Color::from_rgba8(255, 214, 90, 255))),
        "ram_meter" => Some(("▦", Color::from_rgba8(178, 132, 240, 255))),
        "cpu_meter" => Some(("◉", Color::from_rgba8(96, 200, 232, 255))),
        "cpu_cores" | "cpu_cores_meter" => Some(("◉", Color::from_rgba8(96, 200, 232, 255))),
        _ => None,
    }
}

/// Antepone al chip un glifo coloreado por kind, si corresponde, manteniendo
/// el medidor como cuerpo del chip. Para medidores verticales el icono va
/// arriba; para horizontales, a la izquierda.
fn con_icono_de_kind(meter: View<Msg>, kind: Option<&str>, _theme: &Theme) -> View<Msg> {
    let Some(k) = kind else { return meter };
    let Some((glifo, color)) = kind_icon(k) else { return meter };
    let icono = View::new(Style {
        size: Size {
            width: length(16.0_f32),
            height: length(16.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(glifo.to_string(), 13.0, color);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: auto(), height: auto() },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(vec![icono, meter])
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
        "cpu_cores" | "cpu_cores_meter" => 5,
        "clock" => 2,
        "astro" => 1,
        "moon" => 1,
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
/// interacción propia (volumen, brillo, reloj, cpu, ram, cores) la traen acá;
/// el resto cae al `exec` configurable (click → lanzar comando), si lo hay.
/// Para los medidores de sistema, el click izquierdo abre su ventanita (estilo
/// applet de KDE); la rueda/derecho mantienen el atajo waybar.
fn interaccion_widget(v: View<Msg>, kind: &str, exec: Option<&str>) -> View<Msg> {
    match kind {
        "volume" => volume_interactivo(v, exec),
        "brightness" => brightness_interactivo(v, exec),
        "clock" => clock_interactivo(v),
        "cpu_meter" | "cpu_cores" | "cpu_cores_meter" => cpu_interactivo(v, exec),
        "ram_meter" => ram_interactivo(v, exec),
        _ => match exec {
            Some(cmd) => v.on_click(Msg::Spawn(cmd.to_string())),
            None => v,
        },
    }
}

/// Volumen interactivo (idioma waybar): la **rueda** sube/baja el volumen del
/// sink, el **click** abre la ventanita (slider + mute), el **click derecho**
/// togglea el mute. Si la config trae `exec`, el click lanza ese comando en
/// vez de abrir la ventanita (override estilo waybar). El medidor refleja el
/// cambio en el próximo tick.
fn volume_interactivo(v: View<Msg>, exec: Option<&str>) -> View<Msg> {
    let v = v
        .on_scroll(|_dx, dy| (dy != 0.0).then_some(Msg::VolumeWheel(dy)))
        .on_right_click(Msg::VolumeMute);
    match exec {
        Some(cmd) => v.on_click(Msg::Spawn(cmd.to_string())),
        None => v.on_click(Msg::VolumePanel),
    }
}

/// Brillo interactivo: la **rueda** sube/baja la luminosidad de la pantalla;
/// el **click** abre la ventanita (slider). Si la config trae `exec`, el
/// click lanza ese comando en vez de la ventanita.
fn brightness_interactivo(v: View<Msg>, exec: Option<&str>) -> View<Msg> {
    let v = v.on_scroll(|_dx, dy| (dy != 0.0).then_some(Msg::BrightnessWheel(dy)));
    match exec {
        Some(cmd) => v.on_click(Msg::Spawn(cmd.to_string())),
        None => v.on_click(Msg::BrightnessPanel),
    }
}

/// CPU interactivo: el **click** abre la ventanita (lista de cores + agregado).
/// `exec` override estilo waybar (p. ej. lanzar `htop`).
fn cpu_interactivo(v: View<Msg>, exec: Option<&str>) -> View<Msg> {
    match exec {
        Some(cmd) => v.on_click(Msg::Spawn(cmd.to_string())),
        None => v.on_click(Msg::CpuPanel),
    }
}

/// RAM interactiva: el **click** abre la ventanita (usado/total + swap si hay).
/// `exec` override estilo waybar (p. ej. `gnome-system-monitor`).
fn ram_interactivo(v: View<Msg>, exec: Option<&str>) -> View<Msg> {
    match exec {
        Some(cmd) => v.on_click(Msg::Spawn(cmd.to_string())),
        None => v.on_click(Msg::RamPanel),
    }
}

/// Reloj interactivo: el click abre el panel para fijar la fecha/hora del
/// sistema.
fn clock_interactivo(v: View<Msg>) -> View<Msg> {
    v.on_click(Msg::ClockPanel)
}

/// Dimensiones de barra para cada combinación `(size, orient)`. Para barras
/// horizontales: `(ancho_barra, grosor)`; para verticales: `(grosor, alto)`.
/// El frontend traduce; la regla del repo (modelo agnóstico) no se rompe — el
/// `WidgetView` lleva las intenciones (size/orient), acá las convertimos a px.
fn barra_dims(size: MeterSize, orient: MeterOrient) -> (f32, f32) {
    match (size, orient) {
        (MeterSize::Small, MeterOrient::Horizontal) => (28.0, 4.0),
        (MeterSize::Medium, MeterOrient::Horizontal) => (BARRA_W, 6.0),
        (MeterSize::Large, MeterOrient::Horizontal) => (78.0, 8.0),
        (MeterSize::Small, MeterOrient::Vertical) => (4.0, 18.0),
        (MeterSize::Medium, MeterOrient::Vertical) => (6.0, 28.0),
        (MeterSize::Large, MeterOrient::Vertical) => (8.0, 48.0),
    }
}

/// Cuerpo de fuente para la leyenda según el tamaño del medidor. `Small` no
/// pinta leyenda al lado (no le entra); cae a 0 y el llamador la omite.
fn caption_px(size: MeterSize) -> f32 {
    match size {
        MeterSize::Small => 0.0,
        MeterSize::Medium => 12.0,
        MeterSize::Large => 14.0,
    }
}

/// Cuerpo de fuente para la etiqueta corta.
fn label_px(size: MeterSize) -> f32 {
    match size {
        MeterSize::Small => 10.0,
        MeterSize::Medium => 12.0,
        MeterSize::Large => 13.0,
    }
}

/// Una barrita proporcional con gradiente, en `orient` y tamaño dados.
/// Compartida por `meter_view` y `cores_view`.
fn barrita(frac: f32, ancho: f32, grosor: f32, orient: MeterOrient, theme: &Theme, stops: (Color, Color)) -> View<Msg> {
    let frac = frac.clamp(0.0, 1.0);
    let (c0, c1) = stops;
    let (size_pista, size_relleno) = match orient {
        MeterOrient::Horizontal => (
            Size { width: length(ancho), height: length(grosor) },
            Size { width: length(ancho * frac), height: length(grosor) },
        ),
        MeterOrient::Vertical => (
            // En vertical, "ancho" es el alto total y "grosor" es la columna.
            Size { width: length(grosor), height: length(ancho) },
            Size { width: length(grosor), height: length(ancho * frac) },
        ),
    };
    let relleno = View::new(Style {
        size: size_relleno,
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
        // El gradiente abarca toda la barra (no sólo el relleno) en el eje
        // mayor: valor bajo = tramo verde; valor alto = llega al rojo.
        let (p_ini, p_fin) = match orient {
            MeterOrient::Horizontal => {
                let x_full = x0 + ancho as f64;
                (Point::new(x0, y0), Point::new(x_full, y0))
            }
            MeterOrient::Vertical => {
                // El extremo bajo (verde) abajo, el alto (rojo) arriba.
                let y_top = y1 - ancho as f64;
                (Point::new(x0, y1), Point::new(x0, y_top))
            }
        };
        let g = Gradient::new_linear(p_ini, p_fin).with_stops([c0, c1].as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rr);
    });
    // El relleno vertical sale desde abajo: la pista es columna; el hijo va
    // anclado abajo gracias a `JustifyContent::FlexEnd`.
    let pista_style = Style {
        size: size_pista,
        flex_direction: match orient {
            MeterOrient::Horizontal => FlexDirection::Row,
            MeterOrient::Vertical => FlexDirection::Column,
        },
        align_items: Some(AlignItems::FlexStart),
        justify_content: Some(match orient {
            MeterOrient::Horizontal => JustifyContent::FlexStart,
            MeterOrient::Vertical => JustifyContent::FlexEnd,
        }),
        ..Default::default()
    };
    View::new(pista_style)
        .fill(theme.bg_panel)
        .radius(2.0)
        .children(vec![relleno])
}

/// Un medidor: etiqueta opcional + barrita proporcional + leyenda. La barra de
/// relleno lleva un **gradiente** del acento (extremo verde) al acento
/// aclarado (extremo rojo), pintado a mano con `paint_with`. `size` y `orient`
/// vienen del view-model (config) y modulan dimensiones y eje.
fn meter_view(
    label: Option<&str>,
    fraction: f32,
    caption: &str,
    size: MeterSize,
    orient: MeterOrient,
    theme: &Theme,
    stops: (Color, Color),
) -> View<Msg> {
    let (ancho, grosor) = barra_dims(size, orient);
    let barra = barrita(fraction, ancho, grosor, orient, theme, stops);
    let cap_px = caption_px(size);
    let lab_px = label_px(size);

    // En `Small` no entra leyenda al lado de la barra: la barra sola es la
    // lectura (tooltip en hover trae la leyenda completa). En vertical, los
    // hijos van en columna y la barra ocupa el centro.
    let dir = match orient {
        MeterOrient::Horizontal => FlexDirection::Row,
        MeterOrient::Vertical => FlexDirection::Column,
    };

    let mut hijos: Vec<View<Msg>> = Vec::new();
    if let Some(l) = label {
        if lab_px > 0.0 && size != MeterSize::Small {
            hijos.push(etiqueta_dim(l, lab_px, orient, theme));
        }
    }
    hijos.push(barra);
    if cap_px > 0.0 && !caption.is_empty() {
        hijos.push(caption_dim(caption, cap_px, size, orient, theme));
    }

    let (h_chip, gap_main) = match orient {
        MeterOrient::Horizontal => (22.0_f32, 8.0_f32),
        MeterOrient::Vertical => (auto_h(size), 3.0_f32),
    };
    let pad_main = match size {
        MeterSize::Small => 2.0_f32,
        MeterSize::Medium => 8.0_f32,
        MeterSize::Large => 10.0_f32,
    };

    let size_outer = match orient {
        MeterOrient::Horizontal => Size { width: auto(), height: length(h_chip) },
        MeterOrient::Vertical => Size { width: auto(), height: length(h_chip) },
    };

    View::new(Style {
        flex_direction: dir,
        size: size_outer,
        padding: TaffyRect {
            left: length(pad_main),
            right: length(pad_main),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(gap_main),
            height: length(gap_main),
        },
        ..Default::default()
    })
    .children(hijos)
}

/// Alto del chip de un medidor vertical, según tamaño (cubre el alto de la
/// barra + padding + caption si la hay).
fn auto_h(size: MeterSize) -> f32 {
    match size {
        MeterSize::Small => 24.0,
        MeterSize::Medium => 44.0,
        MeterSize::Large => 70.0,
    }
}

/// Un racimo de mini-medidores (typically uno por core). En horizontal pinta
/// columnas verticales (estilo systemmonitor de KDE) — cada core como una
/// columnita que sube con la carga. En vertical, gira a filas horizontales que
/// se apilan. La leyenda agregada va al lado/abajo.
fn cores_view(
    label: Option<&str>,
    fractions: &[f32],
    caption: &str,
    size: MeterSize,
    orient: MeterOrient,
    theme: &Theme,
    stops: (Color, Color),
) -> View<Msg> {
    // Para el racimo, el "orient" del view-model decide cómo se acomoda el
    // contenedor exterior; las mini-barras van en el eje perpendicular para
    // formar el grid. Default: contenedor horizontal → mini-barras verticales.
    let (mini_w, mini_h) = match size {
        MeterSize::Small => (3.0, 14.0),
        MeterSize::Medium => (5.0, 22.0),
        MeterSize::Large => (7.0, 32.0),
    };
    let mini_orient_relleno = if orient == MeterOrient::Horizontal {
        MeterOrient::Vertical
    } else {
        MeterOrient::Horizontal
    };
    let (mini_largo, mini_grosor) = if mini_orient_relleno == MeterOrient::Vertical {
        (mini_h, mini_w)
    } else {
        (mini_h, mini_w) // mismo perfil, sólo cambia el eje del flex
    };
    let mini_gap = if size == MeterSize::Small { 1.0_f32 } else { 2.0_f32 };

    let minis: Vec<View<Msg>> = fractions
        .iter()
        .map(|f| barrita(*f, mini_largo, mini_grosor, mini_orient_relleno, theme, stops))
        .collect();

    let racimo_dir = if orient == MeterOrient::Horizontal {
        FlexDirection::Row
    } else {
        FlexDirection::Column
    };
    let racimo = View::new(Style {
        flex_direction: racimo_dir,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(mini_gap),
            height: length(mini_gap),
        },
        ..Default::default()
    })
    .children(minis);

    let lab_px = label_px(size);
    let cap_px = caption_px(size);
    let mut hijos: Vec<View<Msg>> = Vec::new();
    if let Some(l) = label {
        if size != MeterSize::Small {
            hijos.push(etiqueta_dim(l, lab_px, orient, theme));
        }
    }
    hijos.push(racimo);
    if cap_px > 0.0 && !caption.is_empty() && size != MeterSize::Small {
        hijos.push(caption_dim(caption, cap_px, size, orient, theme));
    }

    let dir_outer = if orient == MeterOrient::Horizontal {
        FlexDirection::Row
    } else {
        FlexDirection::Column
    };
    let h_outer = match (size, orient) {
        (MeterSize::Small, _) => 22.0_f32,
        (MeterSize::Medium, MeterOrient::Horizontal) => 26.0,
        (MeterSize::Medium, MeterOrient::Vertical) => 44.0,
        (MeterSize::Large, MeterOrient::Horizontal) => 36.0,
        (MeterSize::Large, MeterOrient::Vertical) => 70.0,
    };
    View::new(Style {
        flex_direction: dir_outer,
        size: Size { width: auto(), height: length(h_outer) },
        padding: TaffyRect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .children(hijos)
}

/// La leyenda de un medidor en una caja de **tamaño fijo**: como el texto
/// cambia de dígitos a cada tick, una caja fija evita que el medidor (y con él
/// toda la barra) se reacomode. El ancho/alto sale del `size` + `orient` —
/// la `Medium horizontal` cabe `"10.5/15.5G"` de la RAM; la vertical se reduce.
fn caption_dim(t: &str, font_px: f32, size: MeterSize, orient: MeterOrient, theme: &Theme) -> View<Msg> {
    let (w, h) = match (size, orient) {
        (_, MeterOrient::Horizontal) => match size {
            MeterSize::Small => (length(36.0_f32), length(22.0_f32)),
            MeterSize::Medium => (length(CAPTION_W), length(22.0_f32)),
            MeterSize::Large => (length(86.0_f32), length(26.0_f32)),
        },
        (_, MeterOrient::Vertical) => match size {
            MeterSize::Small => (auto(), length(12.0_f32)),
            MeterSize::Medium => (auto(), length(14.0_f32)),
            MeterSize::Large => (auto(), length(16.0_f32)),
        },
    };
    View::new(Style {
        size: Size { width: w, height: h },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(t.to_string(), font_px, theme.fg_muted)
}

/// Un texto corto en color tenue (etiqueta de un medidor). Default 12 px; se
/// puede pedir otro cuerpo desde la versión `_dim`.
#[allow(dead_code)]
fn etiqueta(t: &str, theme: &Theme) -> View<Msg> {
    etiqueta_dim(t, 12.0, MeterOrient::Horizontal, theme)
}

fn etiqueta_dim(t: &str, font_px: f32, orient: MeterOrient, theme: &Theme) -> View<Msg> {
    let (w, h) = match orient {
        MeterOrient::Horizontal => (auto(), length(22.0_f32)),
        MeterOrient::Vertical => (auto(), length(14.0_f32)),
    };
    View::new(Style {
        size: Size { width: w, height: h },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(t.to_string(), font_px, theme.fg_muted)
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
        weather: model.weather_now.as_ref(),
        cava: &model.cava_frame,
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
/// arriba, así que pintamos el cuerpo del drawer —el **shell real** hospedado
/// ([`shuma::drawer_body_view`])— llenando lo alto y la barra (su cabezal) abajo,
/// con su grosor original `bar_px`. Asume anclaje inferior (el caso del preset).
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
                    // El workspace switcher es una fila de celdas con click por
                    // escritorio: no pasa por el wrapping de chip/hover/tooltip/
                    // cuantizar de un widget simple — cada celda trae su propia
                    // interacción, y respeta el gap y la dirección del slot.
                    if let WidgetView::Workspaces { active, count, occupied } = wv {
                        workspaces_view(active, count, occupied, surface.gap, dir, theme)
                    } else {
                        let mut v = widget_view_kinded(&wv, Some(kind), theme)
                            .radius(6.0)
                            .hover_fill(theme.bg_button_hover);
                        if let Some(tip) = widget_tooltip(&wv) {
                            v = v.tooltip(tip);
                        }
                        v = interaccion_widget(v, kind, exec.as_deref());
                        cuantizar(v, surface.cell, *cells, kind, dir)
                    }
                }
                SlotWidget::Start { label, exec } => start_button_view(label, exec.as_deref(), theme),
                SlotWidget::Shuma => shuma::headline_view(shuma_state, theme),
                SlotWidget::WindowList => window_list_view(data.windows, surface.gap, dir, theme),
                SlotWidget::Clipboard { exec } => {
                    clipboard_view(data.clipboard, exec.as_deref(), theme)
                }
                SlotWidget::Tray => tray_view(data.tray, surface.gap, dir, theme),
                SlotWidget::Weather { exec } => {
                    cuantizar(weather_view(data.weather, exec.as_deref(), theme), surface.cell, 0, "weather", dir)
                }
                SlotWidget::Cava => {
                    cuantizar(cava_view(data.cava, theme), surface.cell, 0, "cava", dir)
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

/// El **workspace switcher**: una celda por escritorio virtual, en fila. La
/// activa va en acento; las ocupadas (con ventanas) llevan el fondo de panel; las
/// vacías quedan tenues. Cada celda salta a su escritorio al clickearla
/// ([`Msg::SwitchWorkspace`] → `mirada-ctl workspace N`). Es el análogo del
/// `window_list` —items clickeables en un row—, pero indexado por escritorio.
fn workspaces_view(
    active: u8,
    count: u8,
    occupied: u16,
    gap: f32,
    dir: FlexDirection,
    theme: &Theme,
) -> View<Msg> {
    let celdas: Vec<View<Msg>> = (1..=count)
        .map(|n| {
            let ocupado = occupied & (1u16 << (n as u16 - 1)) != 0;
            workspace_cell(n, n == active, ocupado, theme)
        })
        .collect();
    let g = gap.max(2.0);
    View::new(Style {
        flex_direction: dir,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(g),
            height: length(g),
        },
        ..Default::default()
    })
    .children(celdas)
}

/// Una celda del switcher: un cuadradito con el número del escritorio. El color
/// codifica su estado —activo (acento) / ocupado (panel) / vacío (tenue)— y el
/// click pide saltar a él.
fn workspace_cell(n: u8, active: bool, occupied: bool, theme: &Theme) -> View<Msg> {
    let (fill, fg) = if active {
        (theme.accent, theme.bg_panel)
    } else if occupied {
        (theme.bg_panel, theme.fg_text)
    } else {
        (theme.bg_panel_alt, theme.fg_muted)
    };
    View::new(Style {
        size: Size {
            width: length(22.0_f32),
            height: length(20.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(fill)
    .radius(5.0)
    .hover_fill(theme.bg_button_hover)
    .tooltip(format!("Escritorio {n}"))
    .on_click(Msg::SwitchWorkspace(n))
    .text(n.to_string(), 12.0, fg)
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
        .tooltip(if exec.is_some() {
            "Lanzar"
        } else {
            "Menú de inicio (clic-der: cambiar estilo)"
        })
        .on_click(click)
        .on_right_click(Msg::StartStyleCycle)
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
    .fill(theme.bg_app)
    .alpha(0.0)
    .on_click(Msg::StartToggle)
    .children(vec![panel])
}

/// Una fila del menú de inicio: ícono (glyph o SVG real) + label, clickeable.
fn app_row(a: &AppEntry, theme: &Theme) -> View<Msg> {
    // Tres caminos según lo que diga `a.icon`:
    //  - `≤2 chars` → glyph/emoji (apps tawasuyu), texto centrado.
    //  - nombre freedesktop o path → intentamos resolver a un `.svg` (themes
    //    XDG → SvgAsset cacheado). Si encaja, pintamos el SVG real.
    //  - nada/ninguno encaja → fallback al glyph genérico `▸`.
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

/// El historial de portapapeles para el **layer-shell**: la barra arriba (su
/// grosor) y el panel del historial llenando lo que la surface creció hacia
/// abajo. Espeja [`start_menu_view`].
#[allow(clippy::too_many_arguments)]
pub fn clipboard_menu_view(
    surface: &Surface,
    widgets: &SurfaceWidgets,
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
    .children(vec![bar_view(surface, widgets, shuma_state, data, theme)]);

    let mut body_style = Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    };
    body_style.flex_grow = 1.0;
    // Scrim que cierra al click + el panel del historial a la izquierda.
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
    // Click izquierdo: el popup con el historial (in-app). Click derecho: el
    // selector externo (`exec`, p. ej. cliphist) si la config lo fija.
    let v = v.on_click(Msg::ClipboardMenu);
    match exec {
        Some(cmd) => v.on_right_click(Msg::Spawn(cmd.to_string())),
        None => v,
    }
}

/// Ancho del popup del historial de portapapeles (px).
const CLIP_MENU_W: f32 = 360.0;
/// Largo máximo de cada fila del historial antes de recortar.
const CLIP_ROW_MAX: usize = 48;

/// El **panel** del historial de portapapeles: cabecera + una fila por copia,
/// cada una re-copia al clickearse ([`Msg::ClipboardPick`]). Posicionado en
/// absoluto arriba-izquierda; lo enmarca el scrim del overlay (winit) o el área
/// que crece de la barra (layer-shell).
pub fn clipboard_panel(history: &[String], theme: &Theme) -> View<Msg> {
    let mut hijos: Vec<View<Msg>> = Vec::new();
    hijos.push(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(22.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        })
        .text("Portapapeles", 12.0, theme.fg_muted),
    );
    if history.is_empty() {
        hijos.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(26.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text("(sin copias todavía)", 12.0, theme.fg_muted),
        );
    } else {
        for entry in history {
            let fila = View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(26.0_f32),
                },
                padding: TaffyRect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::FlexStart),
                ..Default::default()
            })
            .radius(6.0)
            .hover_fill(theme.bg_button_hover)
            .tooltip(entry.clone())
            .on_click(Msg::ClipboardPick(entry.clone()))
            .text(recortar(entry, CLIP_ROW_MAX), 12.0, theme.fg_text);
            hijos.push(fila);
        }
    }

    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(CLIP_MENU_W),
            height: auto(),
        },
        flex_direction: FlexDirection::Column,
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .children(vec![View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(hijos)])
}

/// El historial de portapapeles como **overlay** para el path winit: un scrim a
/// pantalla completa (cierra al click) bajo la barra, con el panel a la
/// izquierda. Espeja [`start_menu_overlay`].
pub fn clipboard_overlay(history: &[String], bar_h: f32, theme: &Theme) -> View<Msg> {
    let scrim = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .alpha(0.0)
    .on_click(Msg::ClipboardMenu)
    .children(vec![clipboard_panel(history, theme)]);
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
    .children(vec![scrim])
}

/// Ancho del panel del reloj (px).
const CLOCK_PANEL_W: f32 = 360.0;

/// Los cinco campos editables del reloj: índice + rótulo.
const CLOCK_FIELDS: [(u8, &str); 5] = [
    (0, "Año"),
    (1, "Mes"),
    (2, "Día"),
    (3, "Hora"),
    (4, "Min"),
];

/// Un botón chico genérico para los paneles (texto centrado, hover, click).
fn boton_panel(label: &str, msg: Msg, theme: &Theme, fondo: Option<Color>) -> View<Msg> {
    let mut v = View::new(Style {
        size: Size {
            width: auto(),
            height: length(28.0_f32),
        },
        padding: TaffyRect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .radius(6.0)
    .hover_fill(theme.bg_button_hover)
    .on_click(msg);
    if let Some(bg) = fondo {
        v = v.fill(bg);
    }
    let fg = if fondo.is_some() { theme.bg_panel } else { theme.fg_text };
    v.text(label.to_string(), 12.0, fg)
}

/// Un selector ▲/valor/▼ para un campo de fecha/hora.
fn spinner(label: &str, field: u8, valor: &str, theme: &Theme) -> View<Msg> {
    let flecha = |glifo: &str, delta: i32| {
        View::new(Style {
            size: Size {
                width: length(34.0_f32),
                height: length(22.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .radius(5.0)
        .hover_fill(theme.bg_button_hover)
        .on_click(Msg::ClockAdjust(field, delta))
        .text(glifo.to_string(), 12.0, theme.accent)
    };
    let val = View::new(Style {
        size: Size {
            width: length(40.0_f32),
            height: length(26.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(5.0)
    .text(valor.to_string(), 15.0, theme.fg_text);
    let rotulo = View::new(Style {
        size: Size {
            width: auto(),
            height: length(14.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(label.to_string(), 10.0, theme.fg_muted);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(0.0_f32),
            height: length(3.0_f32),
        },
        ..Default::default()
    })
    .children(vec![flecha("▲", 1), val, flecha("▼", -1), rotulo])
}

/// El **panel del reloj**: spinners de fecha/hora + Aplicar/NTP. Posicionado en
/// absoluto arriba-izquierda; lo enmarca el scrim/overlay como el de
/// portapapeles. Cambiar la hora escribe al sistema (`timedatectl` vía pkexec).
pub fn clock_panel(draft: &crate::ClockDraft, theme: &Theme) -> View<Msg> {
    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text("Fecha y hora del sistema", 12.0, theme.fg_muted);

    let spinners: Vec<View<Msg>> = CLOCK_FIELDS
        .iter()
        .map(|(f, l)| spinner(l, *f, &draft.campo(*f), theme))
        .collect();
    let fila = View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(spinners);

    let botones = View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        boton_panel("Aplicar", Msg::ClockApply, theme, Some(theme.accent)),
        boton_panel("Sincronizar NTP", Msg::ClockSyncNtp, theme, None),
    ]);

    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(CLOCK_PANEL_W),
            height: auto(),
        },
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        padding: TaffyRect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(12.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .children(vec![header, fila, botones])
}

/// El panel del reloj como **overlay** para winit (scrim que cierra + panel bajo
/// la barra). Espeja [`clipboard_overlay`].
pub fn clock_overlay(draft: &crate::ClockDraft, bar_h: f32, theme: &Theme) -> View<Msg> {
    let scrim = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .alpha(0.0)
    .on_click(Msg::ClockPanel)
    .children(vec![clock_panel(draft, theme)]);
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
    .children(vec![scrim])
}

// =============================================================================
// Ventanitas de interacción de los medidores (CPU / RAM / volumen / brillo).
// Cada una es una "applet window" estilo KDE: un panel chico con la lectura
// detallada (lista de cores, slider arrastrable, leyenda) que se despliega al
// click en el medidor de la barra y se cierra al click fuera o con Esc.
// =============================================================================

/// Ancho común de las ventanitas de medidor (px).
const METER_PANEL_W: f32 = 320.0;
/// Alto del slider vertical en las ventanitas de volumen/brillo (px).
const SLIDER_H: f32 = 140.0;
/// Ancho de la pista del slider (px).
const SLIDER_W: f32 = 18.0;

/// Header común: una etiqueta tenue arriba de la ventanita.
fn header_panel(t: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        ..Default::default()
    })
    .text(t.to_string(), 12.0, theme.fg_muted)
}

/// Envuelve un panel como caja redondeada con el `bg_panel` del tema.
fn panel_box(hijos: Vec<View<Msg>>, theme: &Theme) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(METER_PANEL_W), height: auto() },
        flex_direction: FlexDirection::Column,
        padding: TaffyRect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(12.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .children(hijos)
}

/// Una fila "etiqueta · valor" en una ventanita (estilo "key: value").
fn fila_kv(k: &str, v: &str, theme: &Theme) -> View<Msg> {
    let key = View::new(Style {
        size: Size { width: auto(), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(k.to_string(), 12.0, theme.fg_muted);
    let mut val_style = Style {
        size: Size { width: auto(), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexEnd),
        ..Default::default()
    };
    val_style.flex_grow = 1.0;
    let val = View::new(val_style).text(v.to_string(), 12.0, theme.fg_text);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![key, val])
}

/// Envuelve un panel en un scrim a pantalla completa, posicionado bajo la barra
/// (`bar_h` desde arriba). `click_msg` es el toggle del panel (cierra al click
/// fuera). Compartido por las 4 ventanitas y por el reloj/portapapeles.
fn overlay_con_scrim(panel: View<Msg>, click_msg: Msg, bar_h: f32, theme: &Theme) -> View<Msg> {
    let scrim = View::new(Style {
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
    .alpha(0.0)
    .on_click(click_msg)
    .children(vec![panel]);
    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(bar_h),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size { width: percent(1.0_f32), height: auto() },
        ..Default::default()
    })
    .children(vec![scrim])
}

/// La ventanita de CPU: agregado + una fila por core, cada una con su mini-barra.
/// La leyenda muestra el promedio. Estilo "System Load Viewer" de KDE.
pub fn cpu_panel(ctx: &WidgetCtx, theme: &Theme) -> View<Msg> {
    let n = (ctx.cpu_cores_n as usize).min(pata_core::widget::MAX_CORES);
    let header = header_panel("CPU — uso por núcleo", theme);
    let total = fila_kv("Promedio", &format!("{:.0}%", ctx.cpu * 100.0), theme);
    let stops = meter_stops("cpu_meter");

    let mut filas: Vec<View<Msg>> = Vec::with_capacity(n + 2);
    if n == 0 {
        filas.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text("(sin datos por núcleo — el sampler aún no respondió)".to_string(), 12.0, theme.fg_muted),
        );
    } else {
        for i in 0..n {
            let f = ctx.cpu_cores[i].clamp(0.0, 1.0);
            let etq = View::new(Style {
                size: Size { width: length(36.0_f32), height: length(20.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text(format!("#{i}"), 11.0, theme.fg_muted);
            let mut barra_style = Style {
                size: Size { width: auto(), height: length(20.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            };
            barra_style.flex_grow = 1.0;
            let barra = View::new(barra_style)
                .children(vec![barrita(f, 220.0, 6.0, MeterOrient::Horizontal, theme, stops)]);
            let pct = View::new(Style {
                size: Size { width: length(40.0_f32), height: length(20.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::FlexEnd),
                ..Default::default()
            })
            .text(format!("{:.0}%", f * 100.0), 11.0, theme.fg_text);
            filas.push(
                View::new(Style {
                    flex_direction: FlexDirection::Row,
                    size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
                    align_items: Some(AlignItems::Center),
                    gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
                    ..Default::default()
                })
                .children(vec![etq, barra, pct]),
            );
        }
    }

    let lista = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: auto() },
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .children(filas);

    panel_box(vec![header, total, lista], theme)
}

/// Overlay (winit) de la ventanita de CPU: scrim que cierra + panel arriba-izq.
pub fn cpu_overlay(ctx: &WidgetCtx, bar_h: f32, theme: &Theme) -> View<Msg> {
    overlay_con_scrim(cpu_panel(ctx, theme), Msg::CpuPanel, bar_h, theme)
}

/// La ventanita de RAM: total + usado + libre, con la barrita arriba.
pub fn ram_panel(ctx: &WidgetCtx, theme: &Theme) -> View<Msg> {
    let header = header_panel("Memoria — uso del sistema", theme);
    let stops = meter_stops("ram_meter");
    let barra_grande = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(14.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        ..Default::default()
    })
    .children(vec![barrita(ctx.ram, 280.0, 10.0, MeterOrient::Horizontal, theme, stops)]);

    let total_g = ctx.ram_total_mb as f32 / 1024.0;
    let usado_g = ctx.ram_used_mb as f32 / 1024.0;
    let libre_g = (total_g - usado_g).max(0.0);
    let pct = (ctx.ram * 100.0 + 0.5) as i32;

    let kv = vec![
        fila_kv("Total", &format!("{total_g:.1} GiB"), theme),
        fila_kv("Usado", &format!("{usado_g:.1} GiB · {pct}%"), theme),
        fila_kv("Libre", &format!("{libre_g:.1} GiB"), theme),
    ];
    let mut hijos = vec![header, barra_grande];
    hijos.extend(kv);
    panel_box(hijos, theme)
}

/// Overlay (winit) de la ventanita de RAM.
pub fn ram_overlay(ctx: &WidgetCtx, bar_h: f32, theme: &Theme) -> View<Msg> {
    overlay_con_scrim(ram_panel(ctx, theme), Msg::RamPanel, bar_h, theme)
}

/// Slider vertical clickeable: pista + relleno desde abajo. Al click sobre la
/// pista, dispara `on_set(frac)` con la fracción `0..1` derivada de `y`. Lo
/// usan las ventanitas de volumen y brillo. La `y` viene local al nodo, y `h`
/// es el alto actual del nodo (el `on_click_at` lo entrega).
fn slider_vertical(
    frac: f32,
    theme: &Theme,
    stops: (Color, Color),
    on_set: fn(f32) -> Msg,
) -> View<Msg> {
    let h = SLIDER_H;
    let pista = barrita(frac, h, SLIDER_W, MeterOrient::Vertical, theme, stops);
    View::new(Style {
        size: Size { width: length(SLIDER_W), height: length(h) },
        ..Default::default()
    })
    .on_click_at(move |_x, y, _w, h| {
        if h <= 0.0 {
            return None;
        }
        let f = ((h - y) / h).clamp(0.0, 1.0);
        Some(on_set(f))
    })
    .children(vec![pista])
}

/// La ventanita de volumen: slider vertical + botón mute + porcentaje.
pub fn volume_panel(ctx: &WidgetCtx, theme: &Theme) -> View<Msg> {
    let header = header_panel("Volumen — sink por defecto", theme);
    let stops = meter_stops("volume");
    let slider = slider_vertical(ctx.volume, theme, stops, Msg::VolumeSet);
    let pct = if ctx.muted {
        "Silenciado".to_string()
    } else {
        format!("{:.0}%", ctx.volume * 100.0)
    };
    let valor = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: auto(), height: auto() },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size { width: auto(), height: length(20.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(pct, 14.0, theme.fg_text),
        boton_panel(
            if ctx.muted { "Activar" } else { "Silenciar" },
            Msg::VolumeMute,
            theme,
            None,
        ),
    ]);

    let row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: auto() },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        padding: TaffyRect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        gap: Size { width: length(16.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![slider, valor]);
    panel_box(vec![header, row], theme)
}

/// Overlay (winit) de la ventanita de volumen.
pub fn volume_overlay(ctx: &WidgetCtx, bar_h: f32, theme: &Theme) -> View<Msg> {
    overlay_con_scrim(volume_panel(ctx, theme), Msg::VolumePanel, bar_h, theme)
}

/// La ventanita de brillo: slider vertical + porcentaje.
pub fn brightness_panel(ctx: &WidgetCtx, theme: &Theme) -> View<Msg> {
    let header = header_panel("Brillo — pantalla", theme);
    let stops = meter_stops("brightness");
    let slider = slider_vertical(ctx.brightness, theme, stops, Msg::BrightnessSet);
    let pct = format!("{:.0}%", ctx.brightness * 100.0);
    let valor = View::new(Style {
        size: Size { width: auto(), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(pct, 14.0, theme.fg_text);
    let row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: auto() },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        padding: TaffyRect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        gap: Size { width: length(16.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![slider, valor]);
    panel_box(vec![header, row], theme)
}

/// Overlay (winit) de la ventanita de brillo.
pub fn brightness_overlay(ctx: &WidgetCtx, bar_h: f32, theme: &Theme) -> View<Msg> {
    overlay_con_scrim(brightness_panel(ctx, theme), Msg::BrightnessPanel, bar_h, theme)
}

/// El panel del reloj para **layer-shell**: barra arriba + panel llenando lo que
/// la surface creció. Espeja [`clipboard_menu_view`].
#[allow(clippy::too_many_arguments)]
pub fn clock_menu_view(
    surface: &Surface,
    widgets: &SurfaceWidgets,
    shuma_state: &ShumaState,
    data: &BarData,
    theme: &Theme,
    bar_px: f32,
    draft: &crate::ClockDraft,
) -> View<Msg> {
    let bar = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(bar_px),
        },
        ..Default::default()
    })
    .children(vec![bar_view(surface, widgets, shuma_state, data, theme)]);
    let mut body_style = Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    };
    body_style.flex_grow = 1.0;
    let body = View::new(body_style)
        .on_click(Msg::ClockPanel)
        .children(vec![clock_panel(draft, theme)]);
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
    let img = Image::new(ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: icon.width, height: icon.height });
    View::new(Style {
        size: Size {
            width: length(TRAY_ICON_PX),
            height: length(TRAY_ICON_PX),
        },
        ..Default::default()
    })
    .image(img)
}

/// Ancho del dibujo del clima (px).
const WEATHER_ICON_W: f32 = 24.0;
/// Ancho del visualizador de audio (px) y su alto útil.
const CAVA_W: f32 = 56.0;
const CAVA_H: f32 = 18.0;

/// El widget `weather`: un **dibujo colorido del cielo** + la temperatura. El
/// dibujo lo pinta [`dibujar_cielo`] a mano (sol/nube/lluvia/nieve/tormenta).
/// Tooltip = la descripción; `exec` (opcional) abre el pronóstico al click.
fn weather_view(w: Option<&crate::weather::Weather>, exec: Option<&str>, theme: &Theme) -> View<Msg> {
    use crate::weather::Sky;
    let (sky, temp, desc) = match w {
        Some(w) => (w.sky, Some(w.temp_c), w.desc.clone()),
        None => (Sky::Unknown, None, "clima…".to_string()),
    };
    let icono = View::new(Style {
        size: Size {
            width: length(WEATHER_ICON_W),
            height: length(22.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| dibujar_cielo(scene, rect, sky));

    let texto = match temp {
        Some(t) => format!("{}°", t.round() as i32),
        None => "—".to_string(),
    };
    let etiqueta = View::new(Style {
        size: Size {
            width: auto(),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(texto, 13.0, theme.fg_text);

    let v = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: length(22.0_f32),
        },
        padding: TaffyRect {
            left: length(6.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(4.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(6.0)
    .hover_fill(theme.bg_button_hover)
    .tooltip(desc)
    .children(vec![icono, etiqueta]);
    match exec {
        Some(cmd) => v.on_click(Msg::Spawn(cmd.to_string())),
        None => v,
    }
}

/// El widget `cava`: las barras del visualizador de audio, pintadas a mano con un
/// gradiente verde (bajo) → rojo (pico) por barra ([`dibujar_cava`]).
fn cava_view(frame: &[f32], theme: &Theme) -> View<Msg> {
    let bars = frame.to_vec();
    View::new(Style {
        size: Size {
            width: length(CAVA_W),
            height: length(CAVA_H),
        },
        ..Default::default()
    })
    .tooltip("Audio")
    .paint_with(move |scene, _ts, rect| dibujar_cava(scene, rect, &bars))
}

// --- Colores del dibujo del clima ---
fn rgba(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

/// Pinta el cielo de [`weather_view`] dentro de `rect` según la categoría `sky`.
fn dibujar_cielo(scene: &mut Scene, rect: PaintRect, sky: crate::weather::Sky) {
    use crate::weather::Sky;
    let (x, y, w, h) = (rect.x as f64, rect.y as f64, rect.w as f64, rect.h as f64);
    let sol = rgba(0xFD, 0xB8, 0x13);
    let nube_c = rgba(0xB8, 0xC2, 0xCC);
    let nube_osc = rgba(0x6E, 0x76, 0x81);
    let lluvia = rgba(0x58, 0xA6, 0xFF);
    let nieve = rgba(0xFF, 0xFF, 0xFF);
    let rayo = rgba(0xFF, 0xD3, 0x3D);
    match sky {
        Sky::Clear => sol_dibujo(scene, x + w * 0.5, y + h * 0.5, h * 0.26, sol),
        Sky::PartlyCloudy => {
            sol_dibujo(scene, x + w * 0.38, y + h * 0.40, h * 0.20, sol);
            nube_dibujo(scene, x + w * 0.20, y + h * 0.32, w * 0.70, h * 0.55, nube_c);
        }
        Sky::Cloudy | Sky::Unknown => {
            nube_dibujo(scene, x + w * 0.10, y + h * 0.24, w * 0.78, h * 0.58, nube_c)
        }
        Sky::Fog => {
            nube_dibujo(scene, x + w * 0.10, y + h * 0.14, w * 0.78, h * 0.50, nube_c);
            lineas_h(scene, x, y, w, h, nube_osc);
        }
        Sky::Rain => {
            nube_dibujo(scene, x + w * 0.10, y + h * 0.12, w * 0.78, h * 0.52, nube_c);
            gotas(scene, x, y, w, h, lluvia);
        }
        Sky::Snow => {
            nube_dibujo(scene, x + w * 0.10, y + h * 0.12, w * 0.78, h * 0.52, nube_c);
            copos(scene, x, y, w, h, nieve);
        }
        Sky::Storm => {
            nube_dibujo(scene, x + w * 0.10, y + h * 0.10, w * 0.78, h * 0.50, nube_osc);
            rayo_dibujo(scene, x + w * 0.5, y, h, rayo);
        }
    }
}

/// Un sol: disco + ocho rayos.
fn sol_dibujo(scene: &mut Scene, cx: f64, cy: f64, r: f64, color: Color) {
    use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle, Line, Point, Stroke};
    use llimphi_ui::llimphi_raster::peniko::Fill;
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &Circle::new(Point::new(cx, cy), r));
    let stroke = Stroke::new(1.4);
    for i in 0..8 {
        let a = std::f64::consts::PI * 2.0 * i as f64 / 8.0;
        let (c, s) = (a.cos(), a.sin());
        let p0 = Point::new(cx + c * r * 1.25, cy + s * r * 1.25);
        let p1 = Point::new(cx + c * r * 1.7, cy + s * r * 1.7);
        scene.stroke(&stroke, Affine::IDENTITY, color, None, &Line::new(p0, p1));
    }
}

/// Una nube: base redondeada + tres bultos.
fn nube_dibujo(scene: &mut Scene, x: f64, y: f64, w: f64, h: f64, color: Color) {
    use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle, Point, RoundedRect};
    use llimphi_ui::llimphi_raster::peniko::Fill;
    let base = RoundedRect::new(x, y + h * 0.5, x + w, y + h, h * 0.28);
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &base);
    for (fx, fy, fr) in [(0.32, 0.55, 0.30), (0.58, 0.42, 0.36), (0.80, 0.58, 0.24)] {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            color,
            None,
            &Circle::new(Point::new(x + w * fx, y + h * fy), h * fr),
        );
    }
}

/// Dos líneas horizontales tenues (niebla) bajo la nube.
fn lineas_h(scene: &mut Scene, x: f64, y: f64, w: f64, h: f64, color: Color) {
    use llimphi_ui::llimphi_raster::kurbo::{Affine, Line, Point, Stroke};
    let stroke = Stroke::new(1.4);
    for fy in [0.76, 0.92] {
        let p0 = Point::new(x + w * 0.18, y + h * fy);
        let p1 = Point::new(x + w * 0.82, y + h * fy);
        scene.stroke(&stroke, Affine::IDENTITY, color, None, &Line::new(p0, p1));
    }
}

/// Tres gotas diagonales (lluvia) bajo la nube.
fn gotas(scene: &mut Scene, x: f64, y: f64, w: f64, h: f64, color: Color) {
    use llimphi_ui::llimphi_raster::kurbo::{Affine, Line, Point, Stroke};
    let stroke = Stroke::new(1.6);
    for fx in [0.30, 0.50, 0.70] {
        let p0 = Point::new(x + w * fx, y + h * 0.72);
        let p1 = Point::new(x + w * (fx - 0.06), y + h * 0.96);
        scene.stroke(&stroke, Affine::IDENTITY, color, None, &Line::new(p0, p1));
    }
}

/// Tres copos (nieve) bajo la nube.
fn copos(scene: &mut Scene, x: f64, y: f64, w: f64, h: f64, color: Color) {
    use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle, Point};
    use llimphi_ui::llimphi_raster::peniko::Fill;
    for fx in [0.30, 0.50, 0.70] {
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            color,
            None,
            &Circle::new(Point::new(x + w * fx, y + h * 0.86), h * 0.07),
        );
    }
}

/// Un rayo (tormenta): zigzag amarillo relleno.
fn rayo_dibujo(scene: &mut Scene, cx: f64, y: f64, h: f64, color: Color) {
    use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Point};
    use llimphi_ui::llimphi_raster::peniko::Fill;
    let mut p = BezPath::new();
    p.move_to(Point::new(cx + 2.0, y + h * 0.52));
    p.line_to(Point::new(cx - 4.0, y + h * 0.80));
    p.line_to(Point::new(cx, y + h * 0.80));
    p.line_to(Point::new(cx - 3.0, y + h * 1.0));
    p.line_to(Point::new(cx + 5.0, y + h * 0.70));
    p.line_to(Point::new(cx + 1.0, y + h * 0.70));
    p.close_path();
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &p);
}

/// Pinta las barras del visualizador `cava` con un gradiente vertical
/// verde→rojo por barra (la altura define cuánto sube al rojo).
fn dibujar_cava(scene: &mut Scene, rect: PaintRect, bars: &[f32]) {
    use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, RoundedRect};
    use llimphi_ui::llimphi_raster::peniko::{Fill, Gradient};
    let n = bars.len();
    if n == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let (x, y, w, h) = (rect.x as f64, rect.y as f64, rect.w as f64, rect.h as f64);
    let gap = 1.5_f64;
    let bw = ((w - gap * (n as f64 - 1.0)) / n as f64).max(1.0);
    for (i, &v) in bars.iter().enumerate() {
        let v = v.clamp(0.0, 1.0);
        let bh = (v as f64 * h).max(1.5);
        let bx = x + i as f64 * (bw + gap);
        let by = y + h - bh;
        let rr = RoundedRect::new(bx, by, bx + bw, y + h, 1.0);
        let lo = hsv(140.0, 0.55, 0.45);
        let hi = hsv(140.0 * (1.0 - v), 0.80, 0.95);
        let g = Gradient::new_linear(Point::new(bx, y + h), Point::new(bx, by))
            .with_stops([lo, hi].as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rr);
    }
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
