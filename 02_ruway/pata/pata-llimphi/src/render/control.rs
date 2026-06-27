//! Control panel (quick settings): un flyout con volumen, brillo, batería y
//! switches de Wi-Fi/Bluetooth. Unifica en un solo overlay lo que antes estaba
//! disperso en widgets sueltos de la barra (volumen/brillo) y lo que faltaba
//! del todo (batería, radios). Se abre desde un botón de la barra
//! ([`Msg::ControlToggle`]); el scrim cierra al click afuera.
//!
//! Volumen/brillo reusan los mismos mensajes que las ventanitas existentes
//! ([`Msg::VolumeSet`]/[`Msg::BrightnessSet`], fracción absoluta `0..1`); las
//! radios emiten [`Msg::ControlWifi`]/[`Msg::ControlBt`]. Las lecturas del
//! sistema (batería, estado de las radios) viven en [`ControlExtras`].

use llimphi_theme::{elevation, radius, Theme};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Shadow, View};
use llimphi_widget_switch::{switch_view, SwitchPalette};

use crate::Msg;

/// Ancho del panel (px).
pub(super) const PANEL_W: f32 = 300.0;
/// Alto de una fila de slider.
const ROW_H: f32 = 30.0;
/// Largo de la pista del slider horizontal.
const TRACK_W: f32 = 150.0;
const TRACK_H: f32 = 8.0;

/// Lecturas del sistema que no provee el `WidgetCtx` del sampler: estado de la
/// batería y de las radios. Se refrescan al abrir el panel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControlExtras {
    /// `(porcentaje 0..=100, cargando)`, o `None` si no hay batería (desktop).
    pub battery: Option<(u8, bool)>,
    pub wifi: bool,
    pub bt: bool,
    /// Perfil de energía activo (`power-saver`/`balanced`/`performance`), o `None`
    /// si no hay `powerprofilesctl` (power-profiles-daemon).
    pub power_profile: Option<String>,
    /// `true` si la luz nocturna (`wlsunset`) está corriendo.
    pub night: bool,
}

impl ControlExtras {
    /// Lee batería de `/sys/class/power_supply`, las radios de `rfkill`, el perfil
    /// de energía y la luz nocturna. Tolerante: lo que no se puede leer queda en
    /// su default.
    pub fn read() -> Self {
        Self {
            battery: read_battery(),
            wifi: rfkill_on("wlan"),
            bt: rfkill_on("bluetooth"),
            power_profile: read_power_profile(),
            night: night_on(),
        }
    }
}

/// El perfil de energía activo, vía `powerprofilesctl get`. `None` si el binario
/// no está (no hay power-profiles-daemon).
fn read_power_profile() -> Option<String> {
    let out = std::process::Command::new("powerprofilesctl")
        .arg("get")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then_some(p)
}

/// Los perfiles que ofrecemos y su rótulo, en orden ahorro→rendimiento.
pub(super) const PERFILES: [(&str, &str); 3] = [
    ("power-saver", "Ahorro"),
    ("balanced", "Equilibrado"),
    ("performance", "Rendimiento"),
];

/// Fija el perfil de energía (`powerprofilesctl set`). No bloquea.
pub fn set_power_profile(name: &str) {
    let _ = std::process::Command::new("powerprofilesctl")
        .args(["set", name])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// `true` si `wlsunset` está corriendo (luz nocturna activa).
fn night_on() -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", "wlsunset"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Enciende/apaga la luz nocturna. On = arranca `wlsunset` (matando una instancia
/// previa); off = la mata. Desacoplado (no espera). `wlsunset` necesita
/// argumentos —sin ellos aborta—: le damos horarios fijos de amanecer/atardecer
/// y temperaturas (cálido 4000K de noche, neutro 6500K de día), que no dependen
/// de geolocalización. Usa el `wlr-gamma-control` del compositor (mirada lo
/// implementa) para aplicar la rampa.
pub fn set_night(on: bool) {
    if on {
        crate::spawn_cmd(
            "pkill -x wlsunset 2>/dev/null; exec wlsunset -S 07:00 -s 19:00 -t 4000 -T 6500",
        );
    } else {
        crate::spawn_cmd("pkill -x wlsunset");
    }
}

/// Primer `BAT*` con `capacity` + `status`. `None` si no hay (máquina de escritorio).
fn read_battery() -> Option<(u8, bool)> {
    let base = std::path::Path::new("/sys/class/power_supply");
    let rd = std::fs::read_dir(base).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        if !name.to_string_lossy().starts_with("BAT") {
            continue;
        }
        let cap = std::fs::read_to_string(p.join("capacity")).ok()?;
        let pct: u8 = cap.trim().parse().ok()?;
        let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
        let charging = status.trim().eq_ignore_ascii_case("Charging");
        return Some((pct.min(100), charging));
    }
    None
}

/// `true` si la radio `kind` (`wlan`/`bluetooth`) está habilitada (no bloqueada).
/// Lee `rfkill -rn` y mira la columna `soft`. Sin `rfkill` → asume encendida.
fn rfkill_on(kind: &str) -> bool {
    let out = std::process::Command::new("rfkill")
        .args(["-rno", "TYPE,SOFT"])
        .output();
    let Ok(out) = out else {
        return true;
    };
    String::from_utf8_lossy(&out.stdout).lines().any(|l| {
        let l = l.trim();
        l.starts_with(kind) && !l.contains("blocked")
    })
}

/// Conmuta una radio vía `rfkill` (no espera). `wlan`/`bluetooth`.
pub fn set_radio(kind: &str, on: bool) {
    let action = if on { "unblock" } else { "block" };
    let _ = std::process::Command::new("rfkill").args([action, kind]).spawn();
}

/// El botón de la barra que abre el control panel (un engranaje clickeable).
pub fn control_button_view(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(28.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .radius(6.0)
    .hover_fill(theme.bg_button_hover)
    .tooltip("Configuración rápida".to_string())
    .on_click(Msg::ControlToggle)
    .text("⚙".to_string(), 16.0, theme.fg_text)
}

/// El overlay completo: scrim (cierra al click) + el panel anclado arriba a la
/// derecha, bajo la barra.
pub fn control_overlay(
    volume: f32,
    muted: bool,
    brightness: f32,
    extras: &ControlExtras,
    bar_h: f32,
    screen: (f32, f32),
    theme: &Theme,
) -> View<Msg> {
    let _ = screen;
    let panel = control_panel(volume, muted, brightness, extras, theme);
    // Fila que empuja el panel a la derecha.
    let fila = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: auto() },
        justify_content: Some(JustifyContent::FlexEnd),
        padding: TaffyRect {
            left: length(0.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
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
    .on_click(Msg::ControlToggle)
    .children(vec![fila])
}

/// Las filas del control (volumen, brillo, batería, Wi-Fi, Bluetooth, perfil de
/// energía, luz nocturna), **sin** título ni chrome de tarjeta. Las comparten el
/// flyout flotante ([`control_panel`]) y el control center del sidebar
/// ([`control_center_view`]).
pub(super) fn control_sections(
    volume: f32,
    muted: bool,
    brightness: f32,
    extras: &ControlExtras,
    theme: &Theme,
) -> Vec<View<Msg>> {
    let mut hijos: Vec<View<Msg>> = Vec::new();
    // Glifos DejaVu-safe (el sistema no trae emoji a color → tofu): ♪ volumen,
    // ☀ brillo. El mute se marca tachando con ✕.
    let vol_glifo = if muted { "✕" } else { "♪" };
    hijos.push(slider_row(vol_glifo, volume, theme, Msg::VolumeSet));
    hijos.push(slider_row("☀", brightness, theme, Msg::BrightnessSet));

    if let Some((pct, charging)) = extras.battery {
        let valor = if charging {
            format!("{pct}% ⚡")
        } else {
            format!("{pct}%")
        };
        hijos.push(kv_row("Batería", &valor, theme));
    }

    hijos.push(switch_row("Wi-Fi", extras.wifi, theme, Msg::ControlWifi));
    hijos.push(switch_row("Bluetooth", extras.bt, theme, Msg::ControlBt));

    // Perfil de energía (sólo si hay power-profiles-daemon).
    if let Some(actual) = &extras.power_profile {
        hijos.push(perfil_row(actual, theme));
    }
    hijos.push(switch_row("Luz nocturna", extras.night, theme, Msg::ControlNight));
    hijos
}

/// Construye un [`ControlExtras`] con los datos **vivos** del modelo (batería,
/// radios) en vez de la lectura cacheada al abrir el flyout — para el control
/// center del sidebar, que es persistente. `power_profile`/`night` salen de la
/// base cacheada (se leen sólo al togglear; estar levemente atrás es tolerable).
pub fn extras_vivos(
    bat_now: Option<(f32, bool)>,
    wifi: bool,
    bt: bool,
    base: &ControlExtras,
) -> ControlExtras {
    ControlExtras {
        battery: bat_now.map(|(f, c)| ((f * 100.0).round() as u8, c)),
        wifi,
        bt,
        power_profile: base.power_profile.clone(),
        night: base.night,
    }
}

/// El **control center** del sidebar: reloj grande + las filas de control, en un
/// panel de alto completo (sin la tarjeta flotante del flyout). Reusa las mismas
/// filas y los mismos `Msg` que el quick-settings de la barra.
pub fn control_center_view(
    panel_h: f32,
    clock: &pata_core::widget::ClockReading,
    volume: f32,
    muted: bool,
    brightness: f32,
    extras: &ControlExtras,
    theme: &Theme,
) -> View<Msg> {
    let mut hijos = vec![reloj_grande(clock, theme)];
    hijos.extend(control_sections(volume, muted, brightness, extras, theme));
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(panel_h) },
        padding: TaffyRect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(hijos)
}

/// Reloj grande (HH:MM) + fecha, como cabezal del control center.
fn reloj_grande(clock: &pata_core::widget::ClockReading, theme: &Theme) -> View<Msg> {
    let hora = format!("{:02}:{:02}", clock.hour, clock.minute);
    const DIAS: [&str; 7] = [
        "domingo", "lunes", "martes", "miércoles", "jueves", "viernes", "sábado",
    ];
    let dia = DIAS.get(clock.weekday as usize).copied().unwrap_or("");
    let fecha = format!("{} {}/{:02}/{}", dia, clock.day, clock.month, clock.year);
    let hora_v = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(hora, 26.0, theme.fg_text);
    let fecha_v = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(fecha, 12.0, theme.fg_muted);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: auto() },
        ..Default::default()
    })
    .children(vec![hora_v, fecha_v])
}

pub(super) fn control_panel(
    volume: f32,
    muted: bool,
    brightness: f32,
    extras: &ControlExtras,
    theme: &Theme,
) -> View<Msg> {
    let mut hijos: Vec<View<Msg>> = vec![titulo("Control", theme)];
    hijos.extend(control_sections(volume, muted, brightness, extras, theme));

    let (a, blur, dy) = elevation::E4;
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(PANEL_W), height: auto() },
        padding: TaffyRect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(radius::LG)
    .shadow(Shadow {
        color: Color::from_rgba8(0, 0, 0, a),
        blur,
        dx: 0.0,
        dy,
        spread: 0.0,
    })
    .children(hijos)
}

fn titulo(t: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(t.to_string(), 13.0, theme.fg_muted)
}

/// Fila glifo + slider horizontal clickeable (mapea x → fracción → `on_set`).
fn slider_row(glifo: &str, frac: f32, theme: &Theme, on_set: fn(f32) -> Msg) -> View<Msg> {
    let icono = View::new(Style {
        size: Size { width: length(26.0_f32), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(glifo.to_string(), 15.0, theme.fg_text);

    // Pista: fondo + relleno proporcional.
    let frac = frac.clamp(0.0, 1.0);
    let relleno = View::new(Style {
        size: Size { width: percent(frac), height: length(TRACK_H) },
        ..Default::default()
    })
    .fill(theme.accent)
    .radius((TRACK_H / 2.0) as f64);
    let pista = View::new(Style {
        size: Size { width: length(TRACK_W), height: length(TRACK_H) },
        ..Default::default()
    })
    .fill(theme.bg_button)
    .radius((TRACK_H / 2.0) as f64)
    .on_click_at(move |x, _y, w, _h| {
        if w <= 0.0 {
            return None;
        }
        Some(on_set((x / w).clamp(0.0, 1.0)))
    })
    .children(vec![relleno]);
    let pista_wrap = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: auto(), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![pista]);

    let valor = View::new(Style {
        size: Size { width: length(38.0_f32), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexEnd),
        ..Default::default()
    })
    .text_aligned(
        format!("{:.0}%", frac * 100.0),
        12.0,
        theme.fg_muted,
        Alignment::End,
    );

    fila_base(vec![icono, pista_wrap, valor])
}

/// Fila etiqueta (izquierda) + valor (derecha): batería.
fn kv_row(label: &str, valor: &str, theme: &Theme) -> View<Msg> {
    let etiqueta = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: auto(), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(label.to_string(), 12.5, theme.fg_text);
    let v = View::new(Style {
        size: Size { width: length(90.0_f32), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexEnd),
        ..Default::default()
    })
    .text_aligned(valor.to_string(), 12.5, theme.fg_muted, Alignment::End);
    fila_base(vec![etiqueta, v])
}

/// Fila etiqueta + switch (radios).
fn switch_row(label: &str, on: bool, theme: &Theme, make: fn(bool) -> Msg) -> View<Msg> {
    let etiqueta = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: auto(), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(label.to_string(), 12.5, theme.fg_text);
    let sw = View::new(Style {
        size: Size { width: length(44.0_f32), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexEnd),
        ..Default::default()
    })
    .children(vec![switch_view(
        if on { 1.0 } else { 0.0 },
        make(!on),
        &SwitchPalette::from_theme(theme),
    )]);
    fila_base(vec![etiqueta, sw])
}

fn fila_base(hijos: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(hijos)
}

/// Fila «Energía» + selector segmentado de perfiles (ahorro/equilibrado/
/// rendimiento). El activo va en acento.
fn perfil_row(actual: &str, theme: &Theme) -> View<Msg> {
    let etiqueta = View::new(Style {
        size: Size { width: length(60.0_f32), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text("Energía".to_string(), 12.5, theme.fg_text);

    let botones: Vec<View<Msg>> = PERFILES
        .iter()
        .map(|(id, rotulo)| {
            let activo = *id == actual;
            let v = View::new(Style {
                flex_grow: 1.0,
                size: Size { width: auto(), height: length(24.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .radius(5.0)
            .hover_fill(theme.bg_button_hover)
            .on_click(Msg::ControlPowerProfile(id.to_string()))
            .text(
                rotulo.to_string(),
                11.0,
                if activo { theme.bg_panel } else { theme.fg_muted },
            );
            if activo {
                v.fill(theme.accent)
            } else {
                v.fill(theme.bg_button)
            }
        })
        .collect();

    let seg = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size { width: auto(), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(botones);

    fila_base(vec![etiqueta, seg])
}
