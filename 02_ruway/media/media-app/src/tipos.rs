use llimphi_module_command_palette::PaletteMsg;
use media_core::control::MediaCommand;
use media_core::toolbar::BarItem;

#[derive(Clone)]
pub(crate) enum Msg {
    Tick,
    Command(MediaCommand),
    SwapTile { from: usize, to: usize },
    HostActivate(u32),
    ToggleHelp,
    ReloadConfig,
    Palette(PaletteMsg),
    MenuOpen(Option<usize>),
    MenuCommand(String),
    MenuNav(i32),
    MenuActivate,
    MenuTick,
    CloseMenus,
    ContextMenuOpen(f32, f32),
    ToggleSettings,
    SettingsClosed,
    TogglePlaylist,
    PlaylistClosed,
    JumpTrack(usize),
    WaveformReady,
    SettingsTab(SettingsTab),
    SettingsScroll(f32),
    ConfigEdit(ConfigEdit),
    BarEdit(BarEdit),
}

/// Pestañas de la ventana de configuración.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    Audio,
    Video,
    Playback,
    Bars,
    Controls,
}

impl SettingsTab {
    pub(crate) const ALL: &'static [SettingsTab] = &[
        SettingsTab::Audio,
        SettingsTab::Video,
        SettingsTab::Playback,
        SettingsTab::Bars,
        SettingsTab::Controls,
    ];
    pub(crate) fn label(self) -> String {
        let t = rimay_localize::t;
        match self {
            SettingsTab::Audio => t("media-settings-tab-audio"),
            SettingsTab::Video => t("media-settings-tab-video"),
            SettingsTab::Playback => t("media-settings-tab-playback"),
            SettingsTab::Bars => t("media-settings-tab-bars"),
            SettingsTab::Controls => t("media-settings-tab-controls"),
        }
    }
}

/// Edición de las barras de controles (pestaña "Barras").
#[derive(Debug, Clone)]
pub(crate) enum BarEdit {
    AddItem(usize, BarItem),
    RemoveItem(usize, usize),
    Nudge(usize, usize, i32),
    AddBar,
    RemoveBar(usize),
    SetTarget(usize),
    TogglePosition(usize),
}

/// Edición concreta sobre [`MediaConfig`] disparada por la ventana de
/// configuración. Cada variante toca una pref; el handler la aplica y
/// guarda `config.ron`.
#[derive(Debug, Clone)]
pub(crate) enum ConfigEdit {
    // Audio.
    VolumeDelta(f32),
    ToggleEq,
    ToggleNormalization,
    NormTargetDelta(f32),
    ToggleDownmix,
    // Video.
    ToggleColor,
    ColorReset,
    BrightnessDelta(f32),
    ContrastDelta(f32),
    GammaDelta(f32),
    SaturationDelta(f32),
    HueDelta(f32),
    RotateCw,
    FlipH,
    FlipV,
    // Playlist.
    ToggleResumeOnOpen,
    CycleRepeatDefault,
    ToggleShuffleDefault,
    // Subtítulos.
    ToggleAutoloadSidecar,
    SubDelayDelta(i64),
    SubFontDelta(f32),
    // Comportamiento.
    CrossfadeDelta(f32),
}

/// Config del arranque (tipo de fuente y etiqueta).
pub(crate) struct Config {
    pub(crate) label: String,
    pub(crate) kind: VideoKind,
}

#[derive(Clone, Copy)]
pub(crate) enum VideoKind {
    Testcard,
    Gif,
    Image,
    /// Video file decodificado por ffmpeg (mp4/webm/mkv/mov/avi/flv).
    Ffmpeg,
    /// Video AV1 sobre IVF decodificado NATIVO (puro-Rust, rav1d) —
    /// el formato de video nativo de tawasuyu, sin pasar por ffmpeg.
    Av1,
}
