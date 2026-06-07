//! `llimphi-widget-transport` — botones de **transporte** para reproductores
//! de medios (play/pause/prev/next/seek/volume/mute/repeat/shuffle/speed/
//! snapshot/record/eq).
//!
//! Pattern análogo a `llimphi-widget-timeline`/`-waveform`: el widget **no
//! mantiene estado** del reproductor. El caller arma un
//! [`TransportButton`] por cada botón visible (con el flag que define su
//! estado activo, como `playing` o `muted`), lo pasa a
//! [`transport_button_view`], y recibe el [`TransportAction`] semántico en
//! un closure cuando el usuario clickea. Quien mapea esas acciones al
//! `MediaCommand` propio del dominio es la app.
//!
//! ```text
//!   [⏮ ] [⏯ ] [⏭ ] [⏪ ] [⏩ ] [🔊]
//! ```
//!
//! Uso típico (media-app):
//!
//! ```ignore
//! use llimphi_widget_transport::{transport_button_view, TransportAction as Ta,
//!     TransportButton as Tb, TransportPalette};
//! let pal = TransportPalette::from_theme(&theme);
//! transport_button_view(
//!     Tb::PlayPause { playing: !pause().is_paused() },
//!     &pal,
//!     |action| match action {
//!         Ta::TogglePlay => Msg::Command(MediaCommand::TogglePause),
//!         Ta::SeekBy(secs) => Msg::Command(MediaCommand::SeekBy { secs }),
//!         /* … */
//!     },
//! )
//! ```
//!
//! Para una fila completa el caller mapea su `Vec<TransportButton>` con
//! `.into_iter().map(|b| transport_button_view(b, &pal, on_action.clone()))`
//! y lo pone como `children` de una `View` con `flex_direction: Row` y
//! `gap: TransportPalette::gap`.

#![forbid(unsafe_code)]

use llimphi_icons::{icon_view, Icon};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;

/// Acción semántica que el widget reporta cuando el usuario clickea un
/// botón. El caller la traduce al comando propio de su dominio
/// (ej. `MediaCommand::TogglePause`, `MediaCommand::SeekBy { secs }`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransportAction {
    /// Alterna play/pause.
    TogglePlay,
    /// Detener — usualmente "seek a 0 + pause", pero el widget no opina:
    /// el caller decide.
    Stop,
    /// Pista previa.
    Prev,
    /// Pista siguiente.
    Next,
    /// Salto relativo en segundos (signed: negativo = atrás). Entero
    /// porque la mayoría de keymaps de transport usan pasos enteros
    /// (5/10/30/60 s, paridad VLC); si una app necesita fracciones,
    /// puede pasar `secs * k` y dividir al recibir.
    SeekBy(i64),
    /// Cambio relativo de volumen (signed; unidades de fracción 0..1
    /// típicamente).
    VolumeBy(f32),
    /// Mute on/off.
    ToggleMute,
    /// Ciclar modo de repetición (Off → One → All → Off…).
    CycleRepeat,
    /// Shuffle on/off.
    ToggleShuffle,
    /// Paso de velocidad (±1) — el caller decide la escala.
    SpeedStep(i32),
    /// Restablecer velocidad a 1.0×.
    SpeedReset,
    /// Capturar snapshot (frame del video, p.ej.).
    Snapshot,
    /// Toggle de grabación.
    ToggleRecord,
    /// Toggle del ecualizador.
    ToggleEqualizer,
}

/// Botón de transporte concreto + su estado de pintura. El widget elige
/// el icono y la `TransportAction` a partir de esto.
///
/// Para los pares simétricos (SeekBack/SeekForward, VolumeDown/VolumeUp)
/// el caller pasa el **valor absoluto** del paso; el widget se ocupa del
/// signo cuando arma la acción (`SeekBy(-secs)` / `VolumeBy(-step)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransportButton {
    /// Play/Pause — el icono refleja `playing`. `active` cuando NO está
    /// pausado (paridad VLC: el botón "iluminado" indica reproducción).
    PlayPause { playing: bool },
    /// Detener (seek a 0). Nunca active.
    Stop,
    /// Pista previa.
    Prev,
    /// Pista siguiente.
    Next,
    /// Saltar atrás `secs` segundos (positivo; el widget niega para
    /// armar la acción).
    SeekBack { secs: i64 },
    /// Saltar adelante `secs` segundos.
    SeekForward { secs: i64 },
    /// Bajar volumen en `step` (positivo).
    VolumeDown { step: f32 },
    /// Subir volumen en `step`.
    VolumeUp { step: f32 },
    /// Toggle mute — `active` cuando está muteado.
    Mute { muted: bool },
    /// Ciclar repeat — `active` cuando no es "Off".
    Repeat { active: bool },
    /// Toggle shuffle — `active` cuando está prendido.
    Shuffle { active: bool },
    /// Bajar velocidad (chevron ↓). Nunca active.
    SpeedDown,
    /// Subir velocidad (chevron ↑). Nunca active.
    SpeedUp,
    /// Restablecer velocidad — `active` cuando ya está en 1.0×
    /// (paridad VLC: indica "estado nominal").
    SpeedReset { is_default: bool },
    /// Snapshot (cámara).
    Snapshot,
    /// Grabar — `active` cuando está grabando. El widget colorea el
    /// icono con `palette.fg_record` para señalizar "rec on".
    Record { recording: bool },
    /// Toggle del EQ — `active` cuando está prendido.
    Equalizer { enabled: bool },
}

impl TransportButton {
    fn icon(&self) -> Icon {
        match self {
            Self::PlayPause { playing } => {
                if *playing {
                    Icon::Pause
                } else {
                    Icon::Play
                }
            }
            Self::Stop => Icon::Stop,
            Self::Prev => Icon::SkipBack,
            Self::Next => Icon::SkipForward,
            Self::SeekBack { .. } => Icon::Rewind,
            Self::SeekForward { .. } => Icon::FastForward,
            Self::VolumeDown { .. } => Icon::Minus,
            Self::VolumeUp { .. } => Icon::Plus,
            Self::Mute { .. } => Icon::VolumeMute,
            Self::Repeat { .. } => Icon::Repeat,
            Self::Shuffle { .. } => Icon::Shuffle,
            Self::SpeedDown => Icon::ChevronDown,
            Self::SpeedUp => Icon::ChevronUp,
            Self::SpeedReset { .. } => Icon::Gauge,
            Self::Snapshot => Icon::Camera,
            Self::Record { .. } => Icon::Record,
            Self::Equalizer { .. } => Icon::Equalizer,
        }
    }

    fn action(&self) -> TransportAction {
        match self {
            Self::PlayPause { .. } => TransportAction::TogglePlay,
            Self::Stop => TransportAction::Stop,
            Self::Prev => TransportAction::Prev,
            Self::Next => TransportAction::Next,
            Self::SeekBack { secs } => TransportAction::SeekBy(-*secs),
            Self::SeekForward { secs } => TransportAction::SeekBy(*secs),
            Self::VolumeDown { step } => TransportAction::VolumeBy(-step),
            Self::VolumeUp { step } => TransportAction::VolumeBy(*step),
            Self::Mute { .. } => TransportAction::ToggleMute,
            Self::Repeat { .. } => TransportAction::CycleRepeat,
            Self::Shuffle { .. } => TransportAction::ToggleShuffle,
            Self::SpeedDown => TransportAction::SpeedStep(-1),
            Self::SpeedUp => TransportAction::SpeedStep(1),
            Self::SpeedReset { .. } => TransportAction::SpeedReset,
            Self::Snapshot => TransportAction::Snapshot,
            Self::Record { .. } => TransportAction::ToggleRecord,
            Self::Equalizer { .. } => TransportAction::ToggleEqualizer,
        }
    }

    fn is_active(&self) -> bool {
        match self {
            Self::PlayPause { playing } => *playing,
            Self::Mute { muted } => *muted,
            Self::Repeat { active } | Self::Shuffle { active } => *active,
            Self::SpeedReset { is_default } => *is_default,
            Self::Record { recording } => *recording,
            Self::Equalizer { enabled } => *enabled,
            _ => false,
        }
    }

    fn is_record(&self) -> bool {
        matches!(self, Self::Record { .. })
    }
}

/// Paleta + dimensiones de los botones del transport. Las medidas viven
/// acá porque definen la silueta de la barra; el caller no toca el
/// `Style` directamente.
#[derive(Debug, Clone, Copy)]
pub struct TransportPalette {
    /// Fondo del botón inactivo.
    pub bg: Color,
    /// Fondo del botón activo (PlayPause cuando reproduce, Mute cuando
    /// muteado, etc.).
    pub bg_active: Color,
    /// Fondo en hover.
    pub bg_hover: Color,
    /// Color del icono inactivo.
    pub fg: Color,
    /// Color del icono activo.
    pub fg_active: Color,
    /// Color especial del icono Record (siempre rojo, prendido o no).
    pub fg_record: Color,
    /// Ancho del botón (px).
    pub btn_w: f32,
    /// Alto del botón (px).
    pub btn_h: f32,
    /// Radio de las esquinas.
    pub radius: f64,
    /// Grosor del stroke del icono (unidades llimphi-icons; típico 1.6–2.0).
    pub icon_stroke: f32,
    /// Separación recomendada entre botones cuando el caller los pone
    /// en una fila (el widget no la aplica — sólo expone el sugerido).
    pub gap: f32,
}

impl Default for TransportPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TransportPalette {
    /// Construye la paleta desde un `Theme` semántico.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_button,
            bg_active: t.bg_selected,
            bg_hover: t.bg_button_hover,
            fg: t.fg_text,
            fg_active: t.accent,
            // Rojo "REC" universal — no viaja por el theme.
            fg_record: Color::from_rgba8(232, 86, 86, 255),
            btn_w: 40.0,
            btn_h: 34.0,
            radius: 8.0,
            icon_stroke: 2.0,
            gap: 6.0,
        }
    }
}

/// Compone **un** botón de transporte. El handler `on_action` recibe la
/// [`TransportAction`] semántica cuando el usuario clickea — el caller
/// la traduce al `MediaCommand` (o equivalente) de su dominio.
///
/// El widget no mantiene estado: pasale el `TransportButton` con su
/// estado vigente cada frame y el icono / color / bg-activo se ajusta
/// solo. Para una **fila** de botones, mapeá tu `Vec<TransportButton>`
/// con `.iter().map(|b| transport_button_view(*b, &pal, on_action.clone()))`
/// como children de una `View` con `flex_direction: Row` y
/// `gap: palette.gap`.
pub fn transport_button_view<Msg, F>(
    button: TransportButton,
    palette: &TransportPalette,
    on_action: F,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(TransportAction) -> Msg + Send + Sync + 'static,
{
    let active = button.is_active();
    let bg = if active { palette.bg_active } else { palette.bg };
    let fg = if button.is_record() {
        palette.fg_record
    } else if active {
        palette.fg_active
    } else {
        palette.fg
    };
    let action = button.action();
    let icon = button.icon();
    View::new(Style {
        size: Size {
            width: length(palette.btn_w),
            height: length(palette.btn_h),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(palette.bg_hover)
    .radius(palette.radius)
    .on_click(on_action(action))
    .children(vec![icon_view::<Msg>(icon, fg, palette.icon_stroke)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Cmd(TransportAction);

    #[test]
    fn from_theme_usa_colores_semanticos() {
        let t = llimphi_theme::Theme::dark();
        let p = TransportPalette::from_theme(&t);
        assert_eq!(p.bg, t.bg_button);
        assert_eq!(p.bg_active, t.bg_selected);
        assert_eq!(p.bg_hover, t.bg_button_hover);
        assert_eq!(p.fg, t.fg_text);
        assert_eq!(p.fg_active, t.accent);
        // El rojo de REC no debe venir del theme.
        let [r, _, _, _] = p.fg_record.components;
        assert!(r > 0.8, "fg_record debe ser rojo dominante");
    }

    #[test]
    fn play_pause_alterna_icono_y_activa_al_reproducir() {
        let on = TransportButton::PlayPause { playing: true };
        let off = TransportButton::PlayPause { playing: false };
        assert!(matches!(on.icon(), Icon::Pause));
        assert!(matches!(off.icon(), Icon::Play));
        assert!(on.is_active());
        assert!(!off.is_active());
        assert_eq!(on.action(), TransportAction::TogglePlay);
        assert_eq!(off.action(), TransportAction::TogglePlay);
    }

    #[test]
    fn seek_simetrico_niega_signo_atras() {
        let back = TransportButton::SeekBack { secs: 5 };
        let fwd = TransportButton::SeekForward { secs: 5 };
        assert_eq!(back.action(), TransportAction::SeekBy(-5));
        assert_eq!(fwd.action(), TransportAction::SeekBy(5));
    }

    #[test]
    fn volumen_simetrico_niega_signo_abajo() {
        let down = TransportButton::VolumeDown { step: 0.1 };
        let up = TransportButton::VolumeUp { step: 0.1 };
        assert_eq!(down.action(), TransportAction::VolumeBy(-0.1));
        assert_eq!(up.action(), TransportAction::VolumeBy(0.1));
    }

    #[test]
    fn record_activo_y_record_es_caso_especial() {
        let on = TransportButton::Record { recording: true };
        let off = TransportButton::Record { recording: false };
        assert!(on.is_active());
        assert!(!off.is_active());
        // Pero ambos son "record" → ambos usan fg_record.
        assert!(on.is_record());
        assert!(off.is_record());
    }

    #[test]
    fn speed_reset_active_cuando_default() {
        let nominal = TransportButton::SpeedReset { is_default: true };
        let off = TransportButton::SpeedReset { is_default: false };
        assert!(nominal.is_active());
        assert!(!off.is_active());
    }

    #[test]
    fn construye_sin_panic_todos_los_botones() {
        let pal = TransportPalette::default();
        let buttons = [
            TransportButton::PlayPause { playing: false },
            TransportButton::Stop,
            TransportButton::Prev,
            TransportButton::Next,
            TransportButton::SeekBack { secs: 5 },
            TransportButton::SeekForward { secs: 5 },
            TransportButton::VolumeDown { step: 0.1 },
            TransportButton::VolumeUp { step: 0.1 },
            TransportButton::Mute { muted: false },
            TransportButton::Repeat { active: false },
            TransportButton::Shuffle { active: false },
            TransportButton::SpeedDown,
            TransportButton::SpeedUp,
            TransportButton::SpeedReset { is_default: true },
            TransportButton::Snapshot,
            TransportButton::Record { recording: false },
            TransportButton::Equalizer { enabled: false },
        ];
        for b in buttons {
            let _ = transport_button_view::<Cmd, _>(b, &pal, Cmd);
        }
    }
}
