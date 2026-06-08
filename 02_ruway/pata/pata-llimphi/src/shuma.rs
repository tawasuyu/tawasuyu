//! El cabezal `shuma_input` y su drawer **Quake** — hospeda el **shell real** de
//! shuma.
//!
//! La frontera del SDD §5: el marco (`pata`) provee el borde; `shuma` provee el
//! contenido. `shuma_input` es el cabezal que vive en una barra; al activarlo
//! (click o hotkey) el frontend **despliega un drawer** estilo Quake sobre el
//! escritorio que **monta el módulo [`shuma_module_shell`]** —exactamente el
//! mismo shell de `shuma-shell-llimphi`: cards por comando, etapas de pipe
//! clickeables, cuerpo IDE-text read-only, barra de scroll arrastrable y
//! detección PTY/TUI (vim/htop a pantalla completa)—.
//!
//! pata **no reimplementa** nada del shell (Regla 2: la lógica de dominio no sabe
//! quién la pinta): instancia el [`shuma_module_shell::State`], le rutea las
//! teclas (`Msg::Key`), el latido que drena la salida (`Msg::Tick`) y los clicks
//! —que el `view` ya emite envueltos por el `lift` [`Msg::ShumaShell`]— y pinta
//! su `view`. Esto reemplaza de un saque las dos viejas reimplementaciones: las
//! cards propias del path winit y el terminal PTY aparte del path layer-shell.

use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style},
    Rect as TaffyRect,
};
use llimphi_ui::View;

use pata_core::WidgetSpec;
use shuma_module::Source;

use crate::Msg;

/// Alto máximo del drawer (path winit), como fracción de la pantalla.
const DRAWER_FRAC: f32 = 0.45;

/// El estado del cabezal del shell y su drawer. Vive en el `Model` del frontend
/// —es interacción, no modelo de dominio—, no en `pata-core`. El **contenido**
/// del drawer es el shell real, hospedado en [`ShumaState::inner`].
pub struct ShumaState {
    /// `true` cuando el drawer está desplegado.
    pub open: bool,
    /// El **shell real**, hospedado como módulo. Fuente de verdad del contenido
    /// (input, runs, historial, cwd, PTY/TUI). pata sólo le rutea eventos y lo
    /// pinta; nunca toca sus campos directamente.
    pub inner: shuma_module_shell::State,
    /// Hotkey que abre/cierra el drawer (de la prop `hotkey`), o `None`.
    pub hotkey: Option<String>,
    /// Prompt al frente del cabezal (`›`, `$`, …).
    pub prompt: String,
    /// Texto del cabezal cuando el drawer está plegado.
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
            inner: shuma_module_shell::State::new(Source::Local),
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

/// El cabezal de la barra: **el input vivo del shell**. No es un placeholder ni
/// un cabezal-rótulo — es el mismísimo `shell_input_view` del shell hospedado,
/// llevado a la barra. Tipeás acá, las teclas las recibe el shell, Enter ejecuta.
/// Click en el chip → despliega el drawer (para ver la salida); el shell además
/// recibe `FocusInput` por su propio `on_click` interno.
pub fn headline_view(state: &ShumaState, theme: &Theme) -> View<Msg> {
    let input = shuma_module_shell::input_view(&state.inner, theme, Msg::ShumaShell);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: length(380.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        flex_shrink: 1.0,
        padding: TaffyRect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        gap: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .on_click(Msg::ShumaToggle)
    .children(vec![input])
}

/// El drawer desplegado (path **winit**): scrim que cierra al click + panel
/// inferior con el shell real hospedado. `None` si no hay nada que mostrar.
pub fn drawer_overlay(state: &ShumaState, screen: (i32, i32), theme: &Theme) -> Option<View<Msg>> {
    if !state.visible() {
        return None;
    }
    let t = state.anim.value().clamp(0.0, 1.0);
    let (_sw, sh) = screen;
    let alto = (sh as f32 * DRAWER_FRAC * t).max(1.0);

    // El cuerpo es el shell real: su `view` ya trae cards/input/scroll/PTY y
    // pinta su propio fondo (`bg_app`). Los clicks de sus widgets vuelven como
    // `Msg::ShumaShell(..)` gracias al `lift`.
    let body = shuma_module_shell::view(&state.inner, theme, Msg::ShumaShell);

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
        ..Default::default()
    })
    // Absorbe los clicks sobre el borde del panel (padding) para que no se
    // filtren al scrim y cierren el drawer; `ShumaAnim` es un no-op de re-render.
    .on_click(Msg::ShumaAnim)
    .children(vec![body]);

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

/// El **cuerpo** del drawer (sin scrim ni posición absoluta), para el backend
/// `wlr-layer-shell`: ahí la propia layer surface ya *es* el panel del Quake (la
/// barra crece hacia arriba), así que no hace falta scrim ni animación. Es el
/// shell real hospedado, **sin el input** — el input ya vive en la barra (ver
/// [`headline_view`]). Llena el contenedor que le da el caller.
pub fn drawer_body_view(state: &ShumaState, theme: &Theme) -> View<Msg> {
    shuma_module_shell::body_view(&state.inner, theme, Msg::ShumaShell)
}

