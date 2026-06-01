//! El `shuma_input` y su despliegue **Quake**.
//!
//! La frontera del SDD §5: el marco (`pata`) provee el borde; `shuma` provee el
//! contenido. `shuma_input` es el cabezal que vive en una barra; al activarlo
//! (click o hotkey) el frontend **despliega un drawer** estilo Quake sobre el
//! escritorio, con un input que captura el teclado. Repliega al cerrar.
//!
//! La ejecución del comando es, estrictamente, trabajo de `shuma` (no de
//! `pata`). Mientras no exista el puente a `shuma`, [`ejecutar_stand_in`] corre
//! el comando por `sh -c` como **sustituto temporal** —el mismo patrón que usa
//! `mirada-launcher`—; cuando el shell de `shuma` sea embebible, reemplaza a
//! esta función sin tocar el mecanismo del drawer.

use std::process::Command;

use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style},
    Rect as TaffyRect,
};
use llimphi_ui::View;

use pata_core::WidgetSpec;

use crate::Msg;

/// Alto máximo del drawer, como fracción de la pantalla.
const DRAWER_FRAC: f32 = 0.45;

/// El estado del cabezal del shell y su drawer. Vive en el `Model` del frontend
/// —es interacción, no modelo de dominio—, no en `pata-core`.
pub struct ShumaState {
    /// `true` cuando el drawer está desplegado.
    pub open: bool,
    /// El comando que se está escribiendo.
    pub buffer: String,
    /// Salida del último comando (stdout) o el error formateado.
    pub output: Option<Result<String, String>>,
    /// `true` mientras el comando corre en segundo plano.
    pub pending: bool,
    /// Hotkey que abre/cierra el drawer (de la prop `hotkey`), o `None`.
    pub hotkey: Option<String>,
    /// Prompt al frente del input (`›`, `$`, …).
    pub prompt: String,
    /// Placeholder cuando el buffer está vacío.
    pub placeholder: String,
    /// Animación de despliegue `0..1` (0 = replegado, 1 = desplegado).
    pub anim: Tween<f32>,
    /// `true` si el config declaró algún `shuma_input` (si no, no hay cabezal
    /// ni drawer).
    pub present: bool,
}

impl Default for ShumaState {
    fn default() -> Self {
        Self {
            open: false,
            buffer: String::new(),
            output: None,
            pending: false,
            hotkey: None,
            prompt: "›".into(),
            placeholder: "shuma".into(),
            anim: Tween::idle(0.0),
            present: false,
        }
    }
}

impl ShumaState {
    /// Construye el estado desde la spec del `shuma_input` (prompt/placeholder/
    /// hotkey). Marca `present = true`.
    pub fn from_spec(spec: &WidgetSpec) -> Self {
        let hotkey = spec.str_prop("hotkey", "");
        Self {
            prompt: spec.str_prop("prompt", "›").to_string(),
            placeholder: spec.str_prop("placeholder", "shuma").to_string(),
            hotkey: if hotkey.is_empty() {
                None
            } else {
                Some(hotkey.to_string())
            },
            present: true,
            ..Self::default()
        }
    }

    /// `true` si el drawer debe pintarse (abierto o aún animando el cierre).
    pub fn visible(&self) -> bool {
        self.open || self.anim.value() > 0.01
    }
}

/// El cabezal clicable que va en la barra: prompt + buffer/placeholder. Un click
/// despliega el drawer.
pub fn headline_view(state: &ShumaState, theme: &Theme) -> View<Msg> {
    let texto = if state.buffer.is_empty() {
        state.placeholder.clone()
    } else {
        state.buffer.clone()
    };
    let color = if state.buffer.is_empty() {
        theme.fg_muted
    } else {
        theme.fg_text
    };

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: length(220.0_f32),
            height: length(24.0_f32),
        },
        padding: TaffyRect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(6.0)
    .hover_fill(theme.bg_button_hover)
    .on_click(Msg::ShumaToggle)
    .children(vec![
        chip_text(&state.prompt, 13.0, theme.accent),
        chip_text(&texto, 13.0, color),
    ])
}

/// El drawer desplegado: scrim que cierra al click + panel inferior con el input
/// y la salida. `None` si no hay nada que mostrar.
pub fn drawer_overlay(state: &ShumaState, screen: (i32, i32), theme: &Theme) -> Option<View<Msg>> {
    if !state.visible() {
        return None;
    }
    let t = state.anim.value().clamp(0.0, 1.0);
    let (_sw, sh) = screen;
    let alto = (sh as f32 * DRAWER_FRAC * t).max(1.0);

    // Línea del input: prompt + buffer + cursor.
    let linea = {
        let mut s = state.buffer.clone();
        s.push('▏'); // cursor
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(8.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![
            chip_text(&state.prompt, 16.0, theme.accent),
            chip_text(&s, 16.0, theme.fg_text),
        ])
    };

    // Cuerpo: salida del comando o pista de integración con shuma.
    let cuerpo_texto;
    let cuerpo_color;
    if state.pending {
        cuerpo_texto = "…".to_string();
        cuerpo_color = theme.fg_muted;
    } else {
        match &state.output {
            Some(Ok(out)) => {
                cuerpo_texto = out.clone();
                cuerpo_color = theme.fg_text;
            }
            Some(Err(err)) => {
                cuerpo_texto = err.clone();
                cuerpo_color = theme.fg_destructive;
            }
            None => {
                cuerpo_texto =
                    "shuma se despliega aquí — escribí un comando y Enter (Esc cierra)".to_string();
                cuerpo_color = theme.fg_muted;
            }
        }
    }
    let cuerpo = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .text(cuerpo_texto, 13.0, cuerpo_color);

    let panel = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: auto(),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: length(alto),
        },
        flex_direction: FlexDirection::Column,
        padding: TaffyRect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(16.0_f32),
            bottom: length(16.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(12.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![linea, cuerpo]);

    // Scrim a pantalla completa: oscurece el fondo y cierra al click.
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
    .alpha(0.55 * t)
    .on_click(Msg::ShumaToggle)
    .children(vec![panel]);

    Some(scrim)
}

/// El **cuerpo** del drawer (sin scrim ni posición absoluta), pensado para
/// llenar el contenedor que le da el backend `wlr-layer-shell`: ahí la propia
/// layer surface ya *es* el panel del Quake (la barra crece hacia arriba), así
/// que no hace falta scrim ni animación. Línea de input (prompt + buffer +
/// cursor) arriba, salida del último comando debajo.
pub fn drawer_body_view(state: &ShumaState, theme: &Theme) -> View<Msg> {
    let mut buf = state.buffer.clone();
    buf.push('▏'); // cursor
    let linea = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        chip_text(&state.prompt, 16.0, theme.accent),
        chip_text(&buf, 16.0, theme.fg_text),
    ]);

    let (texto, color) = if state.pending {
        ("…".to_string(), theme.fg_muted)
    } else {
        match &state.output {
            Some(Ok(out)) => (out.clone(), theme.fg_text),
            Some(Err(err)) => (err.clone(), theme.fg_destructive),
            None => (
                "shuma se despliega aquí — escribí un comando y Enter (Esc cierra)".to_string(),
                theme.fg_muted,
            ),
        }
    };
    let cuerpo = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .text(texto, 13.0, color);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: TaffyRect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(16.0_f32),
            bottom: length(16.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(12.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![linea, cuerpo])
}

/// Un texto suelto, centrado verticalmente.
fn chip_text(t: &str, size: f32, color: llimphi_theme::Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: auto(),
            height: length(size + 6.0),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(t.to_string(), size, color)
}

/// **Sustituto temporal de shuma**: corre el comando por `sh -c` y devuelve su
/// stdout, o el stderr/explicación como error. Bloqueante: se llama desde un
/// `Handle::spawn`. Reemplazar por el puente a `shuma` cuando exista.
pub fn ejecutar_stand_in(cmd: &str) -> Result<String, String> {
    let salida = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map_err(|e| format!("no pude lanzar sh: {e}"))?;
    if salida.status.success() {
        Ok(String::from_utf8_lossy(&salida.stdout).trim_end().to_string())
    } else {
        let err = String::from_utf8_lossy(&salida.stderr);
        Err(if err.trim().is_empty() {
            format!("salió con código {}", salida.status)
        } else {
            err.trim_end().to_string()
        })
    }
}
