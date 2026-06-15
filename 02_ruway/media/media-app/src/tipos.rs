use llimphi_module_command_palette::PaletteMsg;
use llimphi_ui::KeyEvent;
use media_core::control::MediaCommand;
use media_core::toolbar::BarItem;

/// Qué se está tipeando en el input compartido del panel de Perfiles.
#[derive(Clone, Debug)]
pub(crate) enum InputTarget {
    /// Nombre de un perfil nuevo.
    NewProfile,
    /// Contraseña para desbloquear el perfil nombrado.
    Unlock(String),
    /// Contraseña a fijar en el perfil activo (vacío = quitar candado).
    SetPass,
    /// Ruta de un directorio a escanear recursivamente como playlist.
    AddDir,
}

impl InputTarget {
    /// Si el campo debe enmascararse (contraseñas).
    pub(crate) fn masked(&self) -> bool {
        matches!(self, InputTarget::Unlock(_) | InputTarget::SetPass)
    }
}

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
    /// Alterna la revelación manual de las barras con autohide.
    ToggleRevealBars,
    /// Activa el diente `id` del rail de sidebars in-app (None = colapsar).
    DockActivate(u64),
    /// Suelta un diente arrastrado sobre el rail (reservado para reordenar).
    DockDrop(u64),
    // --- Perfiles / playlists ---
    /// Enfoca el input compartido del panel de perfiles para `target`.
    ProfileFocus(InputTarget),
    /// Tecla al input enfocado.
    ProfileKey(KeyEvent),
    /// Enter: confirma el input enfocado según su `InputTarget`.
    ProfileSubmit,
    /// Esc: cancela el input enfocado.
    ProfileCancel,
    /// Selecciona/activa un perfil (si tiene candado, pide la clave).
    ProfileSelect(String),
    /// Borra un perfil por nombre.
    ProfileDelete(String),
    /// Quita el candado del perfil activo.
    ProfileClearPass,
    /// Carga la playlist `idx` del perfil activo en el motor vivo.
    PlaylistLoad(usize),
    /// Borra la playlist `idx` del perfil activo.
    PlaylistDelete(usize),
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
    /// Apaga/prende el pintado de la barra (sin borrarla).
    ToggleEnabled(usize),
    /// Apaga/prende el autohide de la barra.
    ToggleAutohide(usize),
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
