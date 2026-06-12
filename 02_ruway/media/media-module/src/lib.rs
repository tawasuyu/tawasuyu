//! `media-module` — el reproductor de media como **módulo embebible**.
//!
//! Patrón `nahual-module`: `State` + `Msg` + `update` + `view(state, theme,
//! lift)`; el host (canvas de nahual, pata, shuma…) lo monta mapeando los
//! `Msg` con `lift` y le pasa su reloj (`tick` desde el `Tick` del host).
//!
//! **Controles en estilo dientes por default**: un rail de dientes
//! (`llimphi-widget-dock-rail`, el patrón canónico de cosmos) pegado al
//! borde interno izquierdo del player; cada diente es un panel — ▶
//! **controles** (transport + barra de progreso) e ℹ **info** (formato,
//! dimensiones, posición). Click en el diente activo lo colapsa y queda el
//! video limpio.
//!
//! v1 envuelve los players embebibles existentes (`nahual-{video,audio}-
//! viewer-llimphi`, construidos sobre `media-source-*`): AV1/WebM/MKV/GIF y
//! WAV/MP3/FLAC/Opus/Vorbis con espectro. **Subtítulos**: sidecar
//! `.srt/.vtt/.ass` autodetectado y pintado bajo el video (la carga, el
//! delay y el cue activo viven en `media-core::SubtitleTrack`). El seek
//! clickeable llega cuando las fuentes embebibles implementen
//! `media_core::Seekable` (la matemática ya está en `media-core::seek`).

#![forbid(unsafe_code)]

use std::path::Path;
use std::time::Duration;

use media_core::SubtitleTrack;

use llimphi_icons::{icon_view, Icon};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    style::Position,
    AlignItems, Rect,
};
use llimphi_ui::View;
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_toolbar::{toolbar_view, ToolbarGroup, ToolbarItem, ToolbarPalette};
use nahual_audio_viewer_llimphi::{audio_viewer_view, AudioViewerPalette, AudioViewerState};
use nahual_video_viewer_llimphi::{video_viewer_view, VideoViewerPalette, VideoViewerState};

/// Ancho del rail de dientes (px) — mismo que cosmos/nahual.
const RAIL_W: f32 = 40.0;

/// Qué reproduce el módulo.
pub enum Player {
    Video(VideoViewerState),
    Audio(AudioViewerState),
}

/// Paneles del rail de dientes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Diente {
    Controles,
    Info,
}

impl Diente {
    fn to_u64(self) -> u64 {
        match self {
            Diente::Controles => 0,
            Diente::Info => 1,
        }
    }
    fn from_u64(v: u64) -> Diente {
        match v {
            1 => Diente::Info,
            _ => Diente::Controles,
        }
    }
}

pub struct State {
    pub player: Player,
    pub nombre: String,
    /// Diente activo (`None` = paneles colapsados, sólo el media).
    pub diente: Option<Diente>,
    /// Subtítulos cargados (sidecar), si hay.
    pub subs: Option<SubtitleTrack>,
    /// Delay de subtítulos en ms (positivo retrasa). Clampea media-core.
    pub sub_delay_ms: i64,
}

#[derive(Clone)]
pub enum Msg {
    TogglePlay,
    /// Click en un diente del rail: activa su panel (o colapsa si ya estaba).
    Diente(u64),
}

impl State {
    /// Player de video ya abierto (el host suele tenerlo del discernimiento).
    pub fn desde_video(state: VideoViewerState, nombre: impl Into<String>) -> Self {
        Self {
            player: Player::Video(state),
            nombre: nombre.into(),
            diente: Some(Diente::Controles),
            subs: None,
            sub_delay_ms: 0,
        }
    }

    /// Busca y carga el subtítulo sidecar del video (`.srt/.vtt/.ass/.ssa`
    /// con el mismo nombre base) vía `media-core`. Silencioso si no hay.
    pub fn con_subtitulos_sidecar(mut self, video: &Path) -> Self {
        if let Some(cand) = SubtitleTrack::find_sidecar(video) {
            match SubtitleTrack::load(&cand) {
                Ok(t) if !t.is_empty() => self.subs = Some(t),
                _ => {}
            }
        }
        self
    }

    /// Player de audio ya abierto.
    pub fn desde_audio(state: AudioViewerState, nombre: impl Into<String>) -> Self {
        Self {
            player: Player::Audio(state),
            nombre: nombre.into(),
            diente: Some(Diente::Controles),
            subs: None,
            sub_delay_ms: 0,
        }
    }

    /// Abre `path` por extensión (video AV1/WebM/MKV/GIF; audio
    /// WAV/MP3/FLAC/Opus/OGG). Para despacho por contenido, el host usa
    /// `desde_video`/`desde_audio` con su propio discernimiento.
    pub fn abrir(path: &Path) -> Self {
        let nombre = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        let player = match ext.as_str() {
            "wav" | "mp3" | "flac" | "opus" | "ogg" | "oga" => {
                Player::Audio(AudioViewerState::open(path))
            }
            "webm" | "mkv" => Player::Video(VideoViewerState::open_webm(path)),
            "gif" => Player::Video(VideoViewerState::open_gif(path)),
            _ => Player::Video(VideoViewerState::open_av1(path)),
        };
        let st = Self {
            player,
            nombre,
            diente: Some(Diente::Controles),
            subs: None,
            sub_delay_ms: 0,
        };
        st.con_subtitulos_sidecar(path)
    }

    pub fn is_playing(&self) -> bool {
        match &self.player {
            Player::Video(v) => v.is_playing(),
            Player::Audio(a) => a.is_playing(),
        }
    }

    pub fn position(&self) -> Duration {
        match &self.player {
            Player::Video(v) => v.position(),
            Player::Audio(a) => a.position(),
        }
    }

    pub fn duration(&self) -> Option<Duration> {
        match &self.player {
            Player::Video(v) => v.duration(),
            Player::Audio(a) => a.duration(),
        }
    }
}

/// Avanza el reloj del player — el host lo llama desde su `Tick`.
pub fn tick(st: &mut State, dt: Duration) {
    match &mut st.player {
        Player::Video(v) => {
            v.tick(dt);
        }
        Player::Audio(a) => a.tick(dt),
    }
}

pub fn update(st: &mut State, msg: Msg) {
    match msg {
        Msg::TogglePlay => match &mut st.player {
            Player::Video(v) => v.toggle_play(),
            Player::Audio(a) => a.toggle_play(),
        },
        Msg::Diente(id) => {
            let d = Diente::from_u64(id);
            st.diente = if st.diente == Some(d) { None } else { Some(d) };
        }
    }
}

fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// Vista del módulo: el media llenando el área + el rail de dientes al
/// borde interno + el panel del diente activo abajo.
pub fn view<H: Clone + Send + Sync + 'static>(
    st: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> H + Clone + Send + Sync + 'static,
) -> View<H> {
    let media: View<H> = match &st.player {
        Player::Video(v) => video_viewer_view(v, &VideoViewerPalette::from_theme(theme)),
        Player::Audio(a) => audio_viewer_view(a, &AudioViewerPalette::from_theme(theme)),
    };
    let media_wrap = View::new(Style {
        flex_grow: 1.0,
        min_size: Size { width: length(0.0), height: length(0.0) },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![media]);

    let mut col: Vec<View<H>> = vec![media_wrap];
    if let Some(strip) = subtitulo_strip(st, theme) {
        col.push(strip);
    }
    match st.diente {
        Some(Diente::Controles) => col.push(controles(st, theme, lift.clone())),
        Some(Diente::Info) => col.push(info(st, theme)),
        None => {}
    }
    let cuerpo = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(col)]);

    // Rail de dientes del player, pegado a su borde interno izquierdo.
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![cuerpo, dientes(st, theme, lift)])
}

fn dientes<H: Clone + Send + Sync + 'static>(
    st: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> H + Clone + Send + Sync + 'static,
) -> View<H> {
    let items = [
        DockRailItem { id: Diente::Controles.to_u64(), active: st.diente == Some(Diente::Controles) },
        DockRailItem { id: Diente::Info.to_u64(), active: st.diente == Some(Diente::Info) },
    ];
    let l = lift.clone();
    let rail = dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        |id, size, color| {
            let ic = match Diente::from_u64(id) {
                Diente::Controles => Icon::Play,
                Diente::Info => Icon::Info,
            };
            View::new(Style {
                size: Size { width: length(size), height: length(size) },
                ..Default::default()
            })
            .children(vec![icon_view(ic, color, 1.7)])
        },
        move |id| l(Msg::Diente(id)),
        |_payload| -> Option<H> { None },
    );
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(6.0_f32),
            left: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(RAIL_W), height: auto() },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![rail])
}

/// Panel "controles": transport (toolbar) + barra de progreso.
fn controles<H: Clone + Send + Sync + 'static>(
    st: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> H + Clone + Send + Sync + 'static,
) -> View<H> {
    let play_icon = if st.is_playing() { Icon::Pause } else { Icon::Play };
    let barra = toolbar_view(
        vec![ToolbarGroup::new(vec![ToolbarItem::new(
            move |_s, c| icon_view(play_icon, c, 1.7),
            lift(Msg::TogglePlay),
        )])],
        32.0,
        &ToolbarPalette::from_theme(theme),
    );
    // Progreso: posición / duración + barra proporcional (informativa; el
    // seek real llega cuando media-core exponga Seekable en estas fuentes).
    let pos = st.position();
    let dur = st.duration();
    let frac = dur
        .filter(|d| !d.is_zero())
        .map(|d| (pos.as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let tiempo = match dur {
        Some(d) => format!("{} / {}", fmt_dur(pos), fmt_dur(d)),
        None => fmt_dur(pos),
    };
    let track = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: length(5.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(2.5)
    .children(vec![View::new(Style {
        size: Size { width: percent(frac), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.accent)
    .radius(2.5)]);
    let progreso = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size { width: length(10.0_f32), height: length(0.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![
        track,
        View::new(Style {
            size: Size { width: auto(), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            flex_shrink: 0.0,
            ..Default::default()
        })
        .text(tiempo, 11.5, theme.fg_muted),
    ]);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: auto() },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![progreso, barra])
}

/// Panel "info": nombre + datos del stream.
fn info<H: Clone + Send + Sync + 'static>(st: &State, theme: &Theme) -> View<H> {
    let mut detalle = match &st.player {
        Player::Video(v) => {
            let (w, h) = v.dimensions();
            format!("video · {w}×{h}")
        }
        Player::Audio(_) => "audio".to_string(),
    };
    if let Some(t) = &st.subs {
        detalle.push_str(&format!(" · subtítulos: {} cues", t.len()));
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(52.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(st.nombre.clone(), 13.0, theme.fg_text),
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(detalle, 12.0, theme.fg_muted),
    ])
}

/// Franja de subtítulos bajo el media: el cue activo en la posición actual
/// (delay aplicado por `media-core`). Altura fija mientras haya pista
/// cargada — el layout no salta entre cues. Estilo plano v1 (blanco,
/// centrado); el estilo ASS completo queda en media-app.
fn subtitulo_strip<H: Clone + Send + Sync + 'static>(st: &State, theme: &Theme) -> Option<View<H>> {
    let track = st.subs.as_ref()?;
    let texto = track
        .at_with_delay(st.position(), st.sub_delay_ms)
        .map(|c| c.text.clone())
        .unwrap_or_default();
    Some(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(36.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(llimphi_ui::llimphi_layout::taffy::JustifyContent::Center),
            flex_shrink: 0.0,
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .text(texto, 14.0, llimphi_ui::llimphi_raster::peniko::Color::from_rgba8(240, 240, 240, 255)),
    )
}
