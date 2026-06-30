//! Ventanitas de interacción de los medidores y el reloj (CPU / RAM / volumen /
//! brillo / reloj). Cada una es un panel flotante estilo applet de KDE.

use llimphi_theme::{Color, Theme};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style},
    Rect as TaffyRect,
};
use llimphi_ui::View;

use pata_core::widget::{MeterOrient, WidgetCtx};

use crate::{Msg, SurfaceWidgets};
use pata_core::config::Surface;
use crate::shuma::ShumaState;
use super::BarData;

use super::widgets::{barrita, meter_stops};

// ============================================================
// Constantes compartidas
// ============================================================

/// Ancho común de las ventanitas de medidor (px).
const METER_PANEL_W: f32 = 320.0;
/// Alto del slider vertical en las ventanitas de volumen/brillo (px).
const SLIDER_H: f32 = 140.0;
/// Ancho de la pista del slider (px).
const SLIDER_W: f32 = 18.0;

/// Ancho del panel del reloj (px). Da para una grilla de calendario de 7 columnas.
const CLOCK_PANEL_W: f32 = 280.0;

/// Días del mes `m` (1..=12) del año `y`, contemplando el bisiesto de febrero.
pub(crate) fn dias_del_mes(y: i32, m: i32) -> i32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 30,
    }
}

/// Columna (0 = lunes … 6 = domingo) en la que cae el día `d` del mes `m`/`y`.
/// Algoritmo de Sakamoto (devuelve 0 = domingo) reordenado a lunes-primero.
pub(crate) fn columna_lunes(y: i32, m: i32, d: i32) -> i32 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let yy = if m < 3 { y - 1 } else { y };
    let dow = (yy + yy / 4 - yy / 100 + yy / 400 + T[(m - 1) as usize] + d).rem_euclid(7); // 0=dom
    (dow + 6).rem_euclid(7) // 0=lun
}

/// Los cinco campos editables del reloj: índice + rótulo.
const CLOCK_FIELDS: [(u8, &str); 5] = [
    (0, "Año"),
    (1, "Mes"),
    (2, "Día"),
    (3, "Hora"),
    (4, "Min"),
];

// ============================================================
// Utilidades internas de paneles
// ============================================================

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

/// Como [`panel_box`] pero **en flujo** (sin `Position::Absolute` ni ancho fijo):
/// una tarjeta apilable para componer varios paneles en una columna (el monitor
/// del sidebar). Fondo `bg_panel_alt` para que se lea como sección.
pub(super) fn panel_box_flow(hijos: Vec<View<Msg>>, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: auto() },
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
    .fill(theme.bg_panel_alt)
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

/// Envuelve un panel en un scrim a pantalla completa, posicionado bajo la barra.
fn overlay_con_scrim(panel: View<Msg>, click_msg: Msg, bar_h: f32, _theme: &Theme) -> View<Msg> {
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

/// Un botón chico genérico para los paneles.
pub(super) fn boton_panel(label: &str, msg: Msg, theme: &Theme, fondo: Option<Color>) -> View<Msg> {
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

/// Slider vertical clickeable: pista + relleno desde abajo.
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

// ============================================================
// Paneles de medidores
// ============================================================

/// La ventanita de CPU: agregado + una fila por core, cada una con su mini-barra.
pub fn cpu_panel(ctx: &WidgetCtx, theme: &Theme) -> View<Msg> {
    panel_box(cpu_panel_body(ctx, theme), theme)
}

/// El contenido del panel de CPU (sin la tarjeta flotante absoluta), para
/// componerlo en flujo (p.ej. el monitor del sidebar).
pub(super) fn cpu_panel_body(ctx: &WidgetCtx, theme: &Theme) -> Vec<View<Msg>> {
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

    vec![header, total, lista]
}

/// Overlay (winit) de la ventanita de CPU.
pub fn cpu_overlay(ctx: &WidgetCtx, bar_h: f32, theme: &Theme) -> View<Msg> {
    overlay_con_scrim(cpu_panel(ctx, theme), Msg::CpuPanel, bar_h, theme)
}

/// La ventanita de RAM: total + usado + libre.
pub fn ram_panel(ctx: &WidgetCtx, theme: &Theme) -> View<Msg> {
    panel_box(ram_panel_body(ctx, theme), theme)
}

/// El contenido del panel de RAM (sin la tarjeta flotante absoluta).
pub(super) fn ram_panel_body(ctx: &WidgetCtx, theme: &Theme) -> Vec<View<Msg>> {
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
    hijos
}

/// Overlay (winit) de la ventanita de RAM.
pub fn ram_overlay(ctx: &WidgetCtx, bar_h: f32, theme: &Theme) -> View<Msg> {
    overlay_con_scrim(ram_panel(ctx, theme), Msg::RamPanel, bar_h, theme)
}

/// La ventanita de volumen: slider vertical + botón mute + porcentaje, el
/// selector de dispositivo de salida y el mezclador por aplicación.
pub fn volume_panel(
    ctx: &WidgetCtx,
    sinks: &[crate::sampler::Sink],
    sink_inputs: &[crate::sampler::SinkInput],
    theme: &Theme,
) -> View<Msg> {
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

    let mut hijos = vec![header, row];
    // Selector de salida: a qué dispositivo sale el audio. Sólo si hay más de
    // uno donde elegir (con un único sink no hay decisión que tomar).
    if sinks.len() > 1 {
        hijos.push(header_panel("Salida", theme));
        for s in sinks {
            hijos.push(sink_row(s, theme));
        }
    }
    // Mezclador por aplicación (el "vapucontrol" nativo): una fila por corriente.
    if !sink_inputs.is_empty() {
        hijos.push(header_panel("Aplicaciones", theme));
        for si in sink_inputs {
            hijos.push(mixer_row(si, theme));
        }
    }
    panel_box(hijos, theme)
}

/// Una fila del selector de salida: marcador (● activo / ○ inactivo) + la
/// descripción del dispositivo, resaltada en acento si es el sink por defecto.
/// Al click manda `Msg::SinkSelect(name)` (→ `set-default-sink` + mueve las
/// corrientes activas).
fn sink_row(sink: &crate::sampler::Sink, theme: &Theme) -> View<Msg> {
    let activo = sink.is_default;
    let marca = if activo { "● " } else { "○ " };
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(5.0)
    .hover_fill(theme.bg_button_hover)
    .on_click(Msg::SinkSelect(sink.name.clone()))
    .text(
        format!("{marca}{}", recortar(&sink.description, 34)),
        12.0,
        if activo { theme.accent } else { theme.fg_text },
    )
}

/// Una fila del mezclador: nombre de la app + slider horizontal + botón mute.
fn mixer_row(si: &crate::sampler::SinkInput, theme: &Theme) -> View<Msg> {
    let index = si.index;
    let nombre = View::new(Style {
        size: Size { width: length(96.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(recortar(&si.app, 14), 12.0, theme.fg_text);

    // Slider horizontal clickeable (x → fracción).
    let frac = if si.muted { 0.0 } else { si.volume.clamp(0.0, 1.0) };
    let relleno = View::new(Style {
        size: Size { width: percent(frac), height: length(8.0_f32) },
        ..Default::default()
    })
    .fill(theme.accent)
    .radius(4.0);
    let pista = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: auto(), height: length(8.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_button)
    .radius(4.0)
    .on_click_at(move |x, _y, w, _h| {
        (w > 0.0).then(|| Msg::SinkInputVolume(index, (x / w).clamp(0.0, 1.0)))
    })
    .children(vec![relleno]);
    let pista_wrap = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: auto(), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![pista]);

    let mute = View::new(Style {
        size: Size { width: length(26.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .radius(5.0)
    .hover_fill(theme.bg_button_hover)
    .on_click(Msg::SinkInputMute(index))
    .text(
        if si.muted { "✕".to_string() } else { "♪".to_string() },
        13.0,
        if si.muted { theme.fg_muted } else { theme.fg_text },
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![nombre, pista_wrap, mute])
}

/// Recorta `s` a `max` caracteres con elipsis.
fn recortar(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('…');
    t
}

/// Overlay (winit) de la ventanita de volumen (con selector de salida y
/// mezclador por app).
pub fn volume_overlay(
    ctx: &WidgetCtx,
    sinks: &[crate::sampler::Sink],
    sink_inputs: &[crate::sampler::SinkInput],
    bar_h: f32,
    theme: &Theme,
) -> View<Msg> {
    overlay_con_scrim(
        volume_panel(ctx, sinks, sink_inputs, theme),
        Msg::VolumePanel,
        bar_h,
        theme,
    )
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

// ============================================================
// Panel del reloj
// ============================================================

/// Un selector ▲/valor/▼ para un campo de fecha/hora.
fn spinner(label: &str, field: u8, valor: &str, theme: &Theme) -> View<Msg> {
    let flecha = |glifo: &str, delta: i32| {
        View::new(Style {
            size: Size {
                width: length(26.0_f32),
                height: length(18.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .radius(5.0)
        .hover_fill(theme.bg_button_hover)
        .on_click(Msg::ClockAdjust(field, delta))
        .text(glifo.to_string(), 11.0, theme.accent)
    };
    let val = View::new(Style {
        size: Size {
            width: length(34.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .radius(5.0)
    .text(valor.to_string(), 13.0, theme.fg_text);
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

/// El **panel del reloj**: spinners de fecha/hora + Aplicar/NTP.
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
            width: length(3.0_f32),
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
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(8.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(7.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .children(vec![calendario(draft.year, draft.month, draft.day, theme), header, fila, botones])
}

/// Nombres de los meses (para el rótulo del calendario).
const MESES: [&str; 12] = [
    "Enero", "Febrero", "Marzo", "Abril", "Mayo", "Junio", "Julio", "Agosto",
    "Septiembre", "Octubre", "Noviembre", "Diciembre",
];

/// El calendario del mes `m`/`y`, con el día `hoy` resaltado en acento. Sólo
/// muestra (no cambia la fecha); el setter de abajo edita el reloj del sistema.
fn calendario(y: i32, m: i32, hoy: i32, theme: &Theme) -> View<Msg> {
    let mes = MESES.get((m - 1).clamp(0, 11) as usize).copied().unwrap_or("");
    let titulo = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(format!("{mes} {y}"), 13.0, theme.fg_text);

    // Encabezado de días (lunes primero).
    let cab = fila_cal(
        ["L", "M", "X", "J", "V", "S", "D"]
            .iter()
            .map(|d| celda_cal(d, theme.fg_muted, false, theme))
            .collect(),
    );

    let inicio = columna_lunes(y, m, 1);
    let total = dias_del_mes(y, m);
    let mut filas = vec![titulo, cab];
    let mut celdas: Vec<View<Msg>> = Vec::new();
    // Huecos previos al día 1.
    for _ in 0..inicio {
        celdas.push(celda_cal("", theme.fg_muted, false, theme));
    }
    for d in 1..=total {
        let hoy_cell = d == hoy;
        let color = if hoy_cell { theme.bg_panel } else { theme.fg_text };
        celdas.push(celda_cal(&d.to_string(), color, hoy_cell, theme));
    }
    // Repartir en filas de 7 (drain mueve, sin clonar: View no es Clone).
    while !celdas.is_empty() {
        let n = celdas.len().min(7);
        let semana: Vec<View<Msg>> = celdas.drain(0..n).collect();
        filas.push(fila_cal(semana));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: auto() },
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .children(filas)
}

/// Una fila de 7 celdas del calendario.
fn fila_cal(celdas: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        justify_content: Some(JustifyContent::SpaceBetween),
        ..Default::default()
    })
    .children(celdas)
}

/// Una celda del calendario; `hoy` la pinta con fondo de acento.
fn celda_cal(txt: &str, color: Color, hoy: bool, theme: &Theme) -> View<Msg> {
    let v = View::new(Style {
        size: Size { width: length(34.0_f32), height: length(24.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(txt.to_string(), 12.0, color);
    if hoy {
        v.fill(theme.accent).radius(12.0)
    } else {
        v
    }
}

/// El panel del reloj como **overlay** para winit.
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

/// El panel del reloj para **layer-shell**: barra arriba + panel llenando
/// lo que la surface creció.
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
    .children(vec![super::bar_view(surface, widgets, shuma_state, data, theme)]);
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

#[cfg(test)]
mod tests {
    use super::{columna_lunes, dias_del_mes};

    #[test]
    fn dias_del_mes_y_bisiesto() {
        assert_eq!(dias_del_mes(2026, 1), 31);
        assert_eq!(dias_del_mes(2026, 4), 30);
        assert_eq!(dias_del_mes(2026, 2), 28); // 2026 no bisiesto
        assert_eq!(dias_del_mes(2024, 2), 29); // div 4
        assert_eq!(dias_del_mes(2000, 2), 29); // div 400
        assert_eq!(dias_del_mes(1900, 2), 28); // div 100 no 400
    }

    #[test]
    fn columna_lunes_primero() {
        // Junio 2026 arranca un lunes; el 26 cae viernes (columna 4).
        assert_eq!(columna_lunes(2026, 6, 1), 0);
        assert_eq!(columna_lunes(2026, 6, 26), 4);
    }
}
