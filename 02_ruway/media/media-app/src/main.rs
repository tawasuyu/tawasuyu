//! media-app — primer reproductor del dominio.
//!
//! Pipeline video: una fuente [`FrameSource`] genera RGBA, lo empuja
//! a un [`llimphi_surface::ExternalSurface`], y la UI Llimphi lo
//! expone en un canvas central vía `View::gpu_paint_with`. Con
//! argumento es un GIF en disco (loop infinito); sin argumento cae
//! al [`TestCard`] sintético.
//!
//! Pipeline audio: junto al video se abre un sink cpal sobre el
//! default output device, alimentado por un [`ToneSource`] (A4 a
//! -12 dB). Si el sink no puede abrir el device, se loguea y se
//! sigue sólo con video — la falta de audio no aborta la app.
//!
//! Visor de audio: la fuente sale envuelta en [`ProbedAudioSource`],
//! que duplica cada bloque a un ring buffer compartido. Debajo del
//! canvas de video se pinta una franja con la forma de onda del
//! último tramo del stream (vía `paint_with`). Cuando el audio está
//! muteado, la franja queda en silencio (línea recta) — el visor no
//! depende del sink.
//!
//! Captura: dos botones en el row del título toman fotos del estado
//! actual. `rec` arma/cierra una grabación WAV (PCM 16) del stream
//! audio en el cwd; `snap` escribe un PNG con el frame de video
//! pendiente. Pausa silencia/congela ambos taps a la vez.
//!
//! Corre con:
//!   `cargo run -p media-app --release`
//!   `cargo run -p media-app --release -- /ruta/al/anim.gif`
//!   `cargo run -p media-app --release -- /ruta/foto.png`
//!   `cargo run -p media-app --release -- https://host/stream.m3u8`
//!   `cargo run -p media-app --release -- https://youtu.be/<id>` (yt-dlp)
//!   `MEDIA_WAV=/ruta/clip.wav cargo run -p media-app --release`
//!   `MEDIA_MP3=/ruta/cancion.mp3 cargo run -p media-app --release`
//!   `MEDIA_MUTE=1 cargo run -p media-app --release`
//!
//! El primer argumento posicional es el video; la extensión decide
//! la fuente (`.gif` → anim, `.png/.jpg/.webp/.bmp/.tiff/.jpeg` →
//! imagen fija, `.mp4/.webm/.mkv/.mov/.avi/.flv/.m4v/.ogv` → video
//! real vía ffmpeg subprocess). Si el argumento es una **URL de red**
//! (`http(s)://`, `rtsp://`, `rtmp://`, `hls://`, `udp://`…) se deriva
//! al decoder ffmpeg sin mirar la extensión — libavformat resuelve el
//! protocolo (R1 de PARIDAD.md). Cuando es video file (o stream), audio
//! y video salen del MISMO ffmpeg via pipes dup'eados a fd 3/4 — un
//! proceso por fuente, no dos. La pista de audio cuando NO hay video file
//! se elige con `MEDIA_WAV` o `MEDIA_MP3` — sin ninguna,
//! suena un tono A4 sintético.
//!
//! `MEDIA_MIX_TONE=0.25` (rango 0..1) superpone un tono A4 a esa
//! ganancia sobre la fuente principal vía MixerAudio — demo del
//! mezclador con cualquier fuente.
//!
//! `MEDIA_PLAYLIST=lista.m3u` carga una lista (formato m3u
//! simple: una línea por archivo `.wav`/`.mp3`, `#` = comentario,
//! paths relativos al .m3u). Los botones `⟨trk` / `trk⟩` ciclan
//! manualmente y `speed` cicla velocidades 0.5×..2×. `MEDIA_SRT=
//! subs.srt` (o `MEDIA_VTT` / `MEDIA_ASS`) carga subtítulos —
//! SRT/WebVTT/ASS-SSA, autodetectados— sincronizados a la posición
//! actual del track. Sin esa env, si junto al video hay un archivo con
//! su mismo nombre base (`peli.mp4` → `peli.srt`/`.vtt`/`.ass`/`.ssa`)
//! se carga solo (S5, auto-carga sidecar).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use llimphi_surface::ExternalSurface;
use llimphi_ui::llimphi_hal::wgpu;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, FlexWrap, JustifyContent, Rect as TaffyRect,
};
use llimphi_icons::{icon_view, Icon};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{self, TextBlock};
use llimphi_ui::{
    App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta,
};
use media_audio_cpal::AudioSink;
use media_core::{
    AudioProbe, AudioSource, FrameSource, Levels, MixerAudio, Pause, PausableAudio,
    ProbedAudioSource, Seekable, SubtitleTrack, TestCard, ToneSource, Volume,
    VolumeAudio, Waterfall,
};
use media_core::color::{ColorControl, ColorParams, ColorVideo};
use media_core::config::MediaConfig;
use media_core::control::{ColorParam, ControlSettings, KeyChord, MediaCommand};
use media_core::dynamics::{DynamicsAudio, DynamicsControl};
use media_core::library::History;
use media_core::loudness::{LoudnessProbe, LoudnessTap, REPLAYGAIN_TARGET_LUFS};
use media_core::chapters::Chapters;
use media_core::metadata::{self, Metadata};
use media_core::toolbar::{BarItem, BarPosition};
use media_core::transform::{Rotation, Transform, TransformControl, TransformVideo};
use media_core::eq::{EqControl, EqualizerAudio, ISO_10_BANDS_HZ};
use media_core::layout::{LayoutSettings, PanelId as TileId};
use media_core::sync::{AvSync, FramePlan};
use llimphi_widget_shortcuts_help::{
    shortcuts_help_view, ShortcutEntry, ShortcutGroup, ShortcutsHelpPalette, ShortcutsHelpSpec,
};
use llimphi_widget_timeline::{timeline_view, TimelinePalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_tabs::{tabs_view, TabsPalette, TabsSpec};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_motion::{animate, motion, Tween};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_module_command_palette::{
    self as palette, Command as PaletteCommand, PaletteAction, PaletteMsg, PalettePalette,
    PaletteState,
};
use media_recorder_wav::{default_recording_path, RecordedAudioSource, WavRecorder};
use foreign_av::{FfmpegAudioSource, FfmpegVideoSource, MediaSession};
use media_source_gif::GifSource;
use media_source_image::ImageSource;
use media_source_mp3::Mp3Source;
use media_source_opus::OpusSource;
use media_source_wav::WavSource;
use parking_lot::Mutex;

const TESTCARD_W: u32 = 480;
const TESTCARD_H: u32 = 270;
const TESTCARD_FPS: f32 = 30.0;
/// Key de la ventana OS secundaria de configuración (multiventana llimphi-ui).
const CONFIG_WIN: u64 = 1;
/// Key de la ventana OS secundaria de lista de reproducción / cola.
const PLAYLIST_WIN: u64 = 2;
const TICK_MS: u64 = 33;
/// Capacidad del ring del probe. ~85 ms a 48 kHz · 2 ch — suficiente
/// para una franja de visor responsiva sin meter latencia ni RAM.
const PROBE_CAPACITY: usize = 8192;

#[derive(Clone)]
enum Msg {
    Tick,
    /// Acción de reproducción resuelta desde un botón o una tecla. Único
    /// punto de despacho — los pasos (volumen/seek/velocidad) los hornea
    /// quien construye el comando, leyendo de [`settings`].
    Command(MediaCommand),
    /// Swap dos tiles del grid reorderable. `from`/`to` son índices
    /// sobre `Model::tile_order`.
    SwapTile { from: usize, to: usize },
    /// Abre/cierra el overlay de ayuda de atajos (`?`).
    ToggleHelp,
    /// Relee `controles.ron` desde disco en caliente (`F5`).
    ReloadConfig,
    /// Mensajes del módulo command palette (Ctrl+Shift+P). El palette es
    /// agnóstico: emite `Invoke(id)` y la app mapea el id a un
    /// [`MediaCommand`] vía el índice de su catálogo.
    Palette(PaletteMsg),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cierra).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg`/efecto
    /// real (un `MediaCommand`, toggle de ayuda/tema, recarga, salir).
    MenuCommand(String),
    /// Navegación por teclado en el dropdown del menú principal (↑/↓).
    MenuNav(i32),
    /// Ejecuta el comando de la fila activa del menú principal (Enter).
    MenuActivate,
    /// Tick de la animación de aparición/swap del menú principal (re-render).
    MenuTick,
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click en la raíz → abre el menú contextual anclado en
    /// `(x, y)` de ventana. Origen de la raíz es 0,0 ⇒ local == ventana.
    ContextMenuOpen(f32, f32),
    /// Abre/cierra la ventana de configuración (`F2`). Abre/cierra una
    /// **ventana OS secundaria** (multiventana de llimphi-ui).
    ToggleSettings,
    /// El usuario cerró la ventana de config con el botón del SO: sólo
    /// sincroniza el modelo (la ventana ya la destruyó el runtime; no hay que
    /// volver a pedir cerrarla).
    SettingsClosed,
    /// Abre/cierra la ventana de lista de reproducción (cola).
    TogglePlaylist,
    /// El SO cerró la ventana de cola: sincroniza el modelo.
    PlaylistClosed,
    /// Salta a la pista `idx` de la cola (clic en la lista).
    JumpTrack(usize),
    /// El escaneo de la onda de pista completa terminó (dispara repintado).
    WaveformReady,
    /// Cambia la pestaña activa de la ventana de configuración.
    SettingsTab(SettingsTab),
    /// Desplaza el contenido de la ventana de config (rueda del mouse).
    SettingsScroll(f32),
    /// Edita un campo de la config desde la ventana de ajustes. Muta
    /// `Model::config`, aplica a los handles vivos y persiste.
    ConfigEdit(ConfigEdit),
    /// Edita las barras de controles desde la pestaña "Barras".
    BarEdit(BarEdit),
}

/// Pestañas de la ventana de configuración.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Audio,
    Video,
    Playback,
    Bars,
    Controls,
}

impl SettingsTab {
    const ALL: &'static [SettingsTab] = &[
        SettingsTab::Audio,
        SettingsTab::Video,
        SettingsTab::Playback,
        SettingsTab::Bars,
        SettingsTab::Controls,
    ];
    fn label(self) -> &'static str {
        match self {
            SettingsTab::Audio => "Audio",
            SettingsTab::Video => "Video",
            SettingsTab::Playback => "Reproducción",
            SettingsTab::Bars => "Barras",
            SettingsTab::Controls => "Controles",
        }
    }
}

/// Edición de las barras de controles (pestaña "Barras").
#[derive(Debug, Clone)]
enum BarEdit {
    /// Agrega un item al final de la barra `bar`.
    AddItem(usize, BarItem),
    /// Quita el item `(bar, pos)`.
    RemoveItem(usize, usize),
    /// Reordena el item `(bar, pos)` un lugar (`dir` = -1/+1).
    Nudge(usize, usize, i32),
    /// Agrega una barra vacía.
    AddBar,
    /// Quita la barra `idx`.
    RemoveBar(usize),
    /// Fija a qué barra agrega el selector de items.
    SetTarget(usize),
    /// Alterna la barra `idx` entre arriba/abajo del video.
    TogglePosition(usize),
}

/// Edición concreta sobre [`MediaConfig`] disparada por la ventana de
/// configuración. Cada variante toca una pref; el handler la aplica y
/// guarda `config.ron`.
#[derive(Debug, Clone)]
enum ConfigEdit {
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

// Los tiles del grid reorderable son los [`TileId`] (= `PanelId` del
// core): el vocabulario de paneles y su orden por defecto viven en
// `media-core::layout`, agnósticos de cómo los pinta esta app. Acá sólo
// los mapeamos a vistas concretas (ver `tile_content`).

/// Settings de control (pasos + keymap) cargados al arrancar desde RON
/// en XDG, o el default tipo VLC si no hay archivo. Ver `CONTROLES.md`.
/// Settings vivos tras un `RwLock` — a diferencia de los demás slots
/// (inmutables tras el arranque), éste se reemplaza en caliente cuando
/// el usuario edita `controles.ron` y aprieta F5.
fn settings_slot() -> &'static std::sync::RwLock<ControlSettings> {
    static SLOT: OnceLock<std::sync::RwLock<ControlSettings>> = OnceLock::new();
    SLOT.get_or_init(|| std::sync::RwLock::new(ControlSettings::default()))
}

/// Accessor de conveniencia: devuelve un clon del snapshot actual. El
/// struct es chico, así que clonar por frame es despreciable y evita
/// repartir guards del lock por todo el render.
fn settings() -> ControlSettings {
    settings_slot().read().expect("settings lock").clone()
}

/// Recarga `controles.ron` en caliente. Reemplaza el contenido del lock
/// con lo que haya en disco (o el default si no se puede leer).
fn reload_settings() {
    let nuevo = load_settings();
    *settings_slot().write().expect("settings lock") = nuevo;
    eprintln!("media-app: controles recargados");
}

/// Vigila `controles.ron` en un hilo aparte: cada segundo compara el mtime
/// y, si cambió, dispatcha `ReloadConfig` — recarga **automática** sin
/// tener que apretar F5 (que sigue valiendo como recarga manual). Un poll
/// liviano sobre un archivo diminuto, sin dependencias de FS-watch ni
/// debounce. El hilo es daemon: muere con el proceso.
fn spawn_controles_watcher(handle: &Handle<Msg>) {
    let Some(path) = controles_path() else { return };
    let handle = handle.clone();
    std::thread::spawn(move || {
        let mtime = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let mut last = mtime(&path);
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            let now = mtime(&path);
            if now != last {
                last = now;
                handle.dispatch(Msg::ReloadConfig);
            }
        }
    });
}

/// Resuelve el path de un archivo de config de media bajo
/// `$XDG_CONFIG_HOME/gioser/media/<name>` (o `~/.config/...` si XDG no
/// está set). Lo comparten `controles.ron` (mapeo de entrada) y
/// `layout.ron` (orden de paneles) — dos ejes, dos archivos.
fn config_file(name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("gioser").join("media").join(name))
}

/// Path del archivo de controles (mapeo de entrada).
fn controles_path() -> Option<PathBuf> {
    config_file("controles.ron")
}

/// Path del archivo de layout (orden de los paneles del grid).
fn layout_path() -> Option<PathBuf> {
    config_file("layout.ron")
}

/// Carga el orden de paneles desde `layout.ron`. Si no existe o no
/// parsea, cae al default. El resultado pasa por
/// [`LayoutSettings::sanitized`] para tolerar archivos viejos (paneles
/// nuevos se anexan, entradas desconocidas/duplicadas se descartan) — no
/// sembramos el default en disco como con los controles: el layout sólo
/// se escribe cuando el usuario reordena algo.
fn load_layout() -> Vec<TileId> {
    let Some(path) = layout_path() else {
        return LayoutSettings::default().panels;
    };
    match std::fs::read_to_string(&path) {
        Ok(body) => match ron::from_str::<LayoutSettings>(&body) {
            Ok(l) => {
                let s = l.sanitized();
                eprintln!("media-app: layout cargado de {}", path.display());
                s.panels
            }
            Err(e) => {
                eprintln!("media-app: layout.ron inválido ({e}) — uso default");
                LayoutSettings::default().panels
            }
        },
        Err(_) => LayoutSettings::default().panels,
    }
}

/// Persiste el orden actual de paneles a `layout.ron`. Falla silenciosa
/// con log — no reordenar nunca debe abortar la app.
fn save_layout(order: &[TileId]) {
    let Some(path) = layout_path() else { return };
    let settings = LayoutSettings {
        panels: order.to_vec(),
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match ron::ser::to_string_pretty(&settings, ron::ser::PrettyConfig::default()) {
        Ok(txt) => {
            if let Err(e) = std::fs::write(&path, txt) {
                eprintln!("media-app: no pude escribir layout: {e}");
            }
        }
        Err(e) => eprintln!("media-app: no pude serializar layout: {e}"),
    }
}

/// Carga los settings de control. Si el archivo no existe, escribe el
/// default para que el usuario lo edite (estilo VLC: config descubrible
/// en disco). Cualquier fallo cae al default sin abortar.
fn load_settings() -> ControlSettings {
    let Some(path) = controles_path() else {
        return ControlSettings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(body) => match ron::from_str::<ControlSettings>(&body) {
            Ok(s) => {
                eprintln!("media-app: controles cargados de {}", path.display());
                s
            }
            Err(e) => {
                eprintln!(
                    "media-app: controles.ron inválido ({e}) — uso default"
                );
                ControlSettings::default()
            }
        },
        Err(_) => {
            // No existe: sembramos el default para que sea editable.
            let def = ControlSettings::default();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match ron::ser::to_string_pretty(&def, ron::ser::PrettyConfig::default()) {
                Ok(txt) => match std::fs::write(&path, txt) {
                    Ok(()) => eprintln!(
                        "media-app: sembré controles default en {}",
                        path.display()
                    ),
                    Err(e) => eprintln!("media-app: no pude escribir controles: {e}"),
                },
                Err(e) => eprintln!("media-app: no pude serializar controles: {e}"),
            }
            def
        }
    }
}

// ============================================================
// MediaConfig — config unificada que edita la ventana de ajustes
// ============================================================

/// Path del `config.ron` unificado (prefs de playlist/audio/video/
/// subtítulos/comportamiento, además de controles+layout que también
/// viven en sus archivos legacy). Mismo directorio XDG que el resto.
fn media_config_path() -> Option<PathBuf> {
    config_file("config.ron")
}

/// Config cargada al arrancar, guardada para que `init` la lea sin volver
/// a tocar disco ni el `Playlist`. La pone [`apply_startup_config`].
fn media_config_slot() -> &'static OnceLock<MediaConfig> {
    static SLOT: OnceLock<MediaConfig> = OnceLock::new();
    &SLOT
}

/// Aplica TODO el arranque que depende del `Playlist` (defaults de cola,
/// resume, metadata). **Debe llamarse en `main` ANTES de abrir el sink
/// cpal**: una vez que el callback de audio corre, retiene el lock del
/// `Playlist` mientras bloquea en el pipe de ffmpeg —que no avanza hasta
/// que el paint drene el video, que aún no arrancó—, así que un lock
/// bloqueante acá colgaría la app antes de mostrar la ventana (el deadlock
/// de `project_media_render_thread_deadlock`). Acá cpal todavía no abrió,
/// el lock está libre, y todo es fiable.
fn apply_startup_config() {
    let config = load_media_config();
    // Handles vivos (volumen/EQ/color/normalización/orientación) — no
    // tocan el Playlist, pero los dejamos acá para tenerlo todo junto.
    apply_media_config(&config);
    // Defaults de la cola.
    if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
        let mut pl = h.lock();
        pl.set_repeat(repeat_mode_from(config.playlist.repeat));
        if config.playlist.shuffle && !pl.shuffle_on() {
            pl.toggle_shuffle();
        }
    }
    // Resume (U2): salta a la posición guardada del track actual.
    if config.playlist.resume_on_open {
        let key = playlist_slot()
            .get()
            .and_then(|o| o.as_ref())
            .map(|h| h.lock().track_path().to_string_lossy().into_owned());
        if let Some(key) = key {
            let resume = history()
                .lock()
                .resume_position(&key, Duration::from_secs(5));
            if let Some(pos) = resume {
                seek_audio_to_pos(pos);
            }
            history().lock().note_play(&key, now_secs());
        }
    }
    // Metadata (U5) + capítulos (V7): sólo de archivos locales reales — en
    // una URL de red, leer/spawnear ffmpeg al arranque colgaría o bajaría
    // datos. `current_media_path` puede devolver una URL-como-path.
    if let Some(p) = current_media_path().filter(|p| p.is_file()) {
        let _ = media_metadata_slot().set(load_media_metadata(&p));
        let _ = chapters_slot().set(load_chapters(&p));
    }
    let _ = media_config_slot().set(config);
}

/// Carga el `config.ron` o el default, saneado. Si no existe, siembra uno
/// por defecto para que sea descubrible y editable a mano.
fn load_media_config() -> MediaConfig {
    let Some(path) = media_config_path() else {
        return MediaConfig::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(body) => match ron::from_str::<MediaConfig>(&body) {
            Ok(c) => {
                eprintln!("media-app: config cargada de {}", path.display());
                c.sanitized()
            }
            Err(e) => {
                eprintln!("media-app: config.ron inválido ({e}) — uso default");
                MediaConfig::default()
            }
        },
        Err(_) => {
            let def = MediaConfig::default();
            save_media_config(&def);
            def
        }
    }
}

/// Persiste la config a `config.ron`. Falla silenciosa (sólo log).
fn save_media_config(cfg: &MediaConfig) {
    let Some(path) = media_config_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match ron::ser::to_string_pretty(cfg, ron::ser::PrettyConfig::default()) {
        Ok(txt) => {
            if let Err(e) = std::fs::write(&path, txt) {
                eprintln!("media-app: no pude escribir config.ron: {e}");
            }
        }
        Err(e) => eprintln!("media-app: no pude serializar config: {e}"),
    }
}

/// Empuja la config a los handles vivos de la cadena (lo que se aplica en
/// caliente: volumen, EQ, color, normalización). Las prefs que sólo valen
/// al cargar una pista (resume, autoload, repeat/shuffle por defecto,
/// crossfade) se consultan en su punto de uso, no acá.
fn apply_media_config(cfg: &MediaConfig) {
    // Audio.
    volume().set(cfg.audio.volume);
    eq().set_enabled(cfg.audio.eq_enabled);
    eq().set_all_gains(&cfg.audio.eq_bands_db);
    dynamics().set_enabled(cfg.audio.normalization_enabled);
    // Video (color).
    color().set_enabled(cfg.video.color_enabled);
    color().set_params(ColorParams {
        brightness: cfg.video.brightness,
        contrast: cfg.video.contrast,
        gamma: cfg.video.gamma,
        saturation: cfg.video.saturation,
        hue: cfg.video.hue,
    });
    // Video (orientación) — V3.
    transform().set_transform(Transform {
        rotation: Rotation::from_degrees(cfg.video.rotation),
        flip_h: cfg.video.flip_h,
        flip_v: cfg.video.flip_v,
    });
}

/// Aplica una [`ConfigEdit`] sobre la config (sin sanear — el caller lo
/// hace después). Sólo muta el modelo; los handles vivos se actualizan en
/// el handler de `Msg::ConfigEdit` vía [`apply_media_config`].
fn apply_config_edit(cfg: &mut MediaConfig, edit: ConfigEdit) {
    match edit {
        ConfigEdit::VolumeDelta(d) => cfg.audio.volume += d,
        ConfigEdit::ToggleEq => cfg.audio.eq_enabled = !cfg.audio.eq_enabled,
        ConfigEdit::ToggleNormalization => {
            cfg.audio.normalization_enabled = !cfg.audio.normalization_enabled
        }
        ConfigEdit::NormTargetDelta(d) => cfg.audio.normalization_target_lufs += d,
        ConfigEdit::ToggleDownmix => cfg.audio.downmix_to_stereo = !cfg.audio.downmix_to_stereo,
        ConfigEdit::ToggleColor => cfg.video.color_enabled = !cfg.video.color_enabled,
        ConfigEdit::ColorReset => {
            cfg.video.brightness = 0.0;
            cfg.video.contrast = 1.0;
            cfg.video.gamma = 1.0;
            cfg.video.saturation = 1.0;
            cfg.video.hue = 0.0;
        }
        ConfigEdit::BrightnessDelta(d) => cfg.video.brightness += d,
        ConfigEdit::ContrastDelta(d) => cfg.video.contrast += d,
        ConfigEdit::GammaDelta(d) => cfg.video.gamma += d,
        ConfigEdit::SaturationDelta(d) => cfg.video.saturation += d,
        ConfigEdit::HueDelta(d) => cfg.video.hue += d,
        ConfigEdit::RotateCw => cfg.video.rotation = (cfg.video.rotation + 90) % 360,
        ConfigEdit::FlipH => cfg.video.flip_h = !cfg.video.flip_h,
        ConfigEdit::FlipV => cfg.video.flip_v = !cfg.video.flip_v,
        ConfigEdit::ToggleResumeOnOpen => {
            cfg.playlist.resume_on_open = !cfg.playlist.resume_on_open
        }
        ConfigEdit::CycleRepeatDefault => cfg.playlist.repeat = cfg.playlist.repeat.cycle(),
        ConfigEdit::ToggleShuffleDefault => cfg.playlist.shuffle = !cfg.playlist.shuffle,
        ConfigEdit::ToggleAutoloadSidecar => {
            cfg.subtitles.autoload_sidecar = !cfg.subtitles.autoload_sidecar
        }
        ConfigEdit::SubDelayDelta(d) => cfg.subtitles.delay_ms += d,
        ConfigEdit::SubFontDelta(d) => cfg.subtitles.font_scale += d,
        ConfigEdit::CrossfadeDelta(d) => cfg.behavior.crossfade_secs += d,
    }
}

// ============================================================
// Metadata (U5) — tags del archivo en reproducción
// ============================================================

/// Tags del medio actual (título/artista/álbum/carátula), leídos una vez
/// al arrancar. `Metadata::default()` (todo `None`) si el archivo no trae
/// tags o no es local.
fn media_metadata_slot() -> &'static OnceLock<Metadata> {
    static SLOT: OnceLock<Metadata> = OnceLock::new();
    &SLOT
}

/// Ruta del medio local en reproducción (track de la cola, o el archivo de
/// video/imagen). `None` para tono/testcard o streams de red.
fn current_media_path() -> Option<PathBuf> {
    if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
        return Some(h.lock().track_path().to_path_buf());
    }
    video_path_slot()
        .get()
        .filter(|p| !p.as_os_str().is_empty())
        .cloned()
}

/// Lee los primeros ~2 MB del archivo y parsea sus tags (ID3v2/FLAC). El
/// tope cubre tags y la carátula típica sin cargar el archivo entero.
fn load_media_metadata(path: &Path) -> Metadata {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return Metadata::default();
    };
    let mut buf = Vec::new();
    let _ = file.take(2 * 1024 * 1024).read_to_end(&mut buf);
    metadata::parse(&buf)
}

/// Capítulos del medio actual (V7), extraídos una vez al arrancar.
fn chapters_slot() -> &'static OnceLock<Chapters> {
    static SLOT: OnceLock<Chapters> = OnceLock::new();
    &SLOT
}

/// Extrae los capítulos del archivo vía ffmpeg (ffmetadata) y los parsea.
/// `Chapters` vacío si el archivo no trae o ffmpeg falla.
fn load_chapters(path: &Path) -> Chapters {
    match foreign_av::ffmetadata(path) {
        Ok(text) => Chapters::parse_ffmetadata(&text),
        Err(_) => Chapters::default(),
    }
}

// ============================================================
// Historial / resume (U2)
// ============================================================

/// Historial de reproducción global (resume por medio). Se carga de
/// `history.ron` al primer acceso y se persiste throttle/al salir.
fn history() -> &'static Mutex<History> {
    static SLOT: OnceLock<Mutex<History>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(load_history()))
}

fn history_path() -> Option<PathBuf> {
    config_file("history.ron")
}

fn load_history() -> History {
    let Some(p) = history_path() else {
        return History::default();
    };
    match std::fs::read_to_string(&p) {
        Ok(body) => ron::from_str::<History>(&body)
            .map(History::sanitized)
            .unwrap_or_default(),
        Err(_) => History::default(),
    }
}

/// Persiste el historial a `history.ron` (best-effort, sólo log).
fn save_history() {
    let Some(p) = history_path() else {
        return;
    };
    let snapshot = history().lock().clone();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(txt) = ron::ser::to_string_pretty(&snapshot, ron::ser::PrettyConfig::default()) {
        let _ = std::fs::write(&p, txt);
    }
}

/// Época Unix en segundos (para la recencia del historial).
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Clave del medio en reproducción = ruta del track actual. `None` sin
/// playlist (tono/testcard) o si el lock está ocupado este frame.
fn current_track_key() -> Option<String> {
    let handle = playlist_slot().get().and_then(|o| o.as_ref())?;
    let pl = handle.try_lock()?;
    Some(pl.track_path().to_string_lossy().into_owned())
}

/// Salta a una posición **absoluta** (resume). No-op sin playlist.
fn seek_audio_to_pos(pos: Duration) {
    let Some(handle) = playlist_slot().get().and_then(|o| o.as_ref()) else {
        return;
    };
    let mut src = handle.lock();
    let dur = src.duration().unwrap_or(Duration::ZERO);
    let target = if dur.is_zero() { pos } else { pos.min(dur) };
    src.seek_to(target);
    drop(src);
    reset_av_sync_anchor();
}

/// Registra el avance de reproducción en el historial cada frame y guarda
/// throttle (~cada 5 s). Best-effort: si no hay clave/posición, no hace nada.
fn record_playback_progress(frame: u64) {
    let s = playback_snapshot();
    if !s.present {
        return;
    }
    if let Some(key) = current_track_key() {
        history()
            .lock()
            .update_position(&key, s.position, s.duration, now_secs());
    }
    // ~5 s a 30 fps (TICK_MS ≈ 33 ms).
    if frame % 150 == 0 {
        save_history();
    }
}

/// Mapea el modo de repetición de la config (`media-core`) al de la cola
/// viva de `media-app`.
fn repeat_mode_from(r: media_core::playlist::Repeat) -> RepeatMode {
    use media_core::playlist::Repeat;
    match r {
        Repeat::Off => RepeatMode::Off,
        Repeat::One => RepeatMode::One,
        Repeat::All => RepeatMode::All,
    }
}

/// Aplica una [`BarEdit`] sobre las barras (y el target). El saneo y el
/// guardado los hace el handler de `Msg::BarEdit`.
fn apply_bar_edit(model: &mut Model, edit: BarEdit) {
    let tb = &mut model.config.toolbar;
    match edit {
        BarEdit::AddItem(bar, item) => tb.add_item(bar, item),
        BarEdit::RemoveItem(bar, pos) => {
            tb.remove_item(bar, pos);
        }
        BarEdit::Nudge(bar, pos, dir) => {
            tb.nudge_item(bar, pos, dir);
        }
        BarEdit::AddBar => tb.add_bar(),
        BarEdit::RemoveBar(idx) => {
            tb.remove_bar(idx);
        }
        BarEdit::SetTarget(idx) => model.bar_target = idx,
        BarEdit::TogglePosition(idx) => {
            if let Some(bar) = tb.bars.get_mut(idx) {
                bar.position = bar.position.toggled();
            }
        }
    }
}

struct Model {
    frames: u64,
    started_at: Instant,
    /// Orden actual de los tiles del grid de controles. Drag-to-swap
    /// vía `Msg::SwapTile` lo permuta in-place.
    tile_order: Vec<TileId>,
    /// Si el overlay de ayuda de atajos está abierto (`?` lo alterna).
    help_open: bool,
    /// Command palette (Ctrl+Shift+P); `None` = cerrado. El módulo se
    /// lleva todas las teclas mientras está abierto.
    palette: Option<PaletteState>,
    /// Catálogo de acciones que muestra el palette. Se reconstruye con el
    /// keymap vivo (para anexar el hint del atajo) y queda alineado
    /// índice-a-índice con [`Model::palette_cmds`] — el `id` del palette
    /// es el índice, y `Invoke(id)` lo resuelve a un [`MediaCommand`].
    palette_commands: Vec<PaletteCommand>,
    /// Comandos paralelos al catálogo: `palette_cmds[i]` es la acción del
    /// `palette_commands[i]` cuyo `id` es `i`.
    palette_cmds: Vec<MediaCommand>,
    /// Tamaño aproximado del viewport para centrar overlays. Sin hook de
    /// resize en llimphi-ui, lo fijamos al `initial_size` — mismo
    /// compromiso que la galería.
    viewport: (f32, f32),
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado).
    menu_open: Option<usize>,
    /// Fila resaltada por teclado dentro del dropdown del menú principal
    /// (`usize::MAX` = ninguna). La mueven las flechas ↑/↓.
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal (0→1).
    menu_anim: Tween<f32>,
    /// Menú contextual del reproductor: ancla `(x, y)` en ventana sobre el
    /// área de video/controles. `None` cerrado. media-app no tiene campos
    /// de texto editables, así que el contextual mapea a comandos de
    /// transporte/captura reales — no a edición.
    context_menu: Option<(f32, f32)>,
    /// Config unificada editable (la fuente de verdad de la ventana de
    /// ajustes). Se persiste a `config.ron` en cada cambio.
    config: MediaConfig,
    /// Si la ventana de configuración está abierta (`F2` la alterna).
    settings_open: bool,
    /// Pestaña activa de la ventana de configuración.
    settings_tab: SettingsTab,
    /// Barra a la que el selector de items agrega (pestaña "Barras").
    bar_target: usize,
    /// Desplazamiento vertical (px) del contenido de la pestaña activa.
    settings_scroll: f32,
    /// Si el panel de visualizadores de audio (onda + waterfall + medidores)
    /// está visible. Default **oculto**: por defecto se ve sólo video + barras;
    /// se despliega desde el menú Ver. (Estado de sesión, no se persiste aún.)
    visualizers_open: bool,
    /// Si la ventana de lista de reproducción (cola) está abierta.
    playlist_open: bool,
}

struct Pipeline {
    surface: ExternalSurface,
    source: Mutex<Box<dyn FrameSource + Send>>,
    buf: Mutex<Vec<u8>>,
    /// Última dimensión `(w, h)` que emitió la fuente. `(0, 0)` hasta
    /// el primer tick exitoso. Lo lee el handler de Snapshot para
    /// armar el `ImageBuffer`.
    last_dim: Mutex<(u32, u32)>,
    last_tick: Mutex<Instant>,
    /// Política de sincronización A/V (M1 de `PARIDAD.md`). Decide por
    /// frame, contra el reloj de audio, si presentarlo o descartarlo;
    /// además lleva contadores de diagnóstico y el desfase manual (A4).
    sync: Mutex<AvSync>,
}

fn config_slot() -> &'static OnceLock<Config> {
    static SLOT: OnceLock<Config> = OnceLock::new();
    &SLOT
}

fn pipeline_slot() -> &'static OnceLock<Pipeline> {
    static SLOT: OnceLock<Pipeline> = OnceLock::new();
    &SLOT
}

struct Config {
    label: String,
    kind: VideoKind,
}

#[derive(Clone, Copy)]
enum VideoKind {
    Testcard,
    Gif,
    Image,
    /// Video file decodificado por ffmpeg (mp4/webm/mkv/mov/avi/flv).
    Ffmpeg,
    /// Video AV1 sobre IVF decodificado NATIVO (puro-Rust, rav1d) —
    /// el formato de video nativo de gioser, sin pasar por ffmpeg.
    Av1,
}

/// Path del archivo de video (GIF o imagen estática) cuando aplica.
/// Vacío para Testcard.
fn video_path_slot() -> &'static OnceLock<PathBuf> {
    static SLOT: OnceLock<PathBuf> = OnceLock::new();
    &SLOT
}

/// Nombres de las pistas de la playlist, cacheados al crearla (son estáticos
/// en media-app: la cola se arma una vez desde los argumentos). La ventana de
/// cola los lee de acá sin lockear el `Playlist` (evita el deadlock del paint);
/// el índice actual lo toma del `playback_snapshot`.
fn playlist_labels_slot() -> &'static OnceLock<Vec<String>> {
    static SLOT: OnceLock<Vec<String>> = OnceLock::new();
    &SLOT
}

/// Onda de pista completa (tipo Audacity) computada en background por
/// `foreign_av::decode_peaks`. `None` hasta que el escaneo termina (mientras
/// tanto el visor cae a la onda en vivo). Lo lee el paint del waveform panel.
fn waveform_slot() -> &'static Mutex<Option<media_core::waveform::Waveform>> {
    static SLOT: OnceLock<Mutex<Option<media_core::waveform::Waveform>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// URL de audio **separada** (DASH) cuando yt-dlp resolvió video y audio en
/// streams distintos (R2, YouTube > 720p). `None` ⇒ stream muxeado normal.
/// Si está, la sesión ffmpeg se abre con `probe_dash` (dos entradas).
fn dash_audio_slot() -> &'static OnceLock<PathBuf> {
    static SLOT: OnceLock<PathBuf> = OnceLock::new();
    &SLOT
}

/// Probe del stream de audio que `audio_source_from_env` instaló.
/// `None` cuando no hay audio (MEDIA_MUTE o el sink no abrió) —
/// el visor entonces pinta una franja en silencio.
fn audio_probe_slot() -> &'static OnceLock<Option<AudioProbe>> {
    static SLOT: OnceLock<Option<AudioProbe>> = OnceLock::new();
    &SLOT
}

/// Handle de pausa compartido por audio y video. Se materializa antes
/// de armar las fuentes para poder pasarlo a los wrappers Pausable*.
fn pause() -> &'static Pause {
    static SLOT: OnceLock<Pause> = OnceLock::new();
    SLOT.get_or_init(Pause::new)
}

/// Handle compartido del recorder WAV. Cuando `is_recording()` es
/// false, el wrapper `RecordedAudioSource` es transparente; al
/// armarlo desde la UI empieza a copiar cada bloque del stream a
/// disco.
fn recorder() -> &'static WavRecorder {
    static SLOT: OnceLock<WavRecorder> = OnceLock::new();
    SLOT.get_or_init(WavRecorder::new)
}

/// Ganancia lineal compartida con el wrapper [`VolumeAudio`]. 1.0 =
/// passthrough; los botones suben/bajan en pasos de 0.1.
fn volume() -> &'static Volume {
    static SLOT: OnceLock<Volume> = OnceLock::new();
    SLOT.get_or_init(|| Volume::new(1.0))
}

/// Volumen guardado mientras está silenciado (mute real). `Some` = mute
/// activo; al des-silenciar restaura este valor. Ver `ToggleMute`.
fn muted_volume() -> &'static Mutex<Option<f32>> {
    static SLOT: OnceLock<Mutex<Option<f32>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Ecualizador gráfico de 10 bandas compartido con el wrapper
/// [`EqualizerAudio`] en la cadena de audio. [`EqControl`] es clonable y
/// lock-free en el callback realtime (compara una versión atómica); la UI
/// (tile + palette + tecla `e`) lo ajusta desde otro hilo. Default plano.
fn eq() -> &'static EqControl {
    static SLOT: OnceLock<EqControl> = OnceLock::new();
    SLOT.get_or_init(EqControl::graphic_10band)
}

/// Control de ajustes de color del video (brillo/contraste/gamma/
/// saturación, V4). Compartido entre el wrapper `ColorVideo` de la cadena
/// de video y los comandos `Color*`. Arranca en identidad (bypass).
fn color() -> &'static ColorControl {
    static SLOT: OnceLock<ColorControl> = OnceLock::new();
    SLOT.get_or_init(ColorControl::default)
}

/// Control de orientación del video (rotación/flip, V3). Compartido entre
/// el wrapper `TransformVideo` de la cadena de video y los comandos de
/// orientación. Arranca en identidad (bypass).
fn transform() -> &'static TransformControl {
    static SLOT: OnceLock<TransformControl> = OnceLock::new();
    SLOT.get_or_init(TransformControl::default)
}

/// Control de normalización + limitador de audio (A5). Compartido entre el
/// wrapper `DynamicsAudio` de la cadena de audio y los comandos Norm*.
/// Arranca en 0 dB con el limitador activo (techo por defecto).
fn dynamics() -> &'static DynamicsControl {
    static SLOT: OnceLock<DynamicsControl> = OnceLock::new();
    SLOT.get_or_init(DynamicsControl::default)
}

/// Tap de medición de sonoridad (EBU R128) compartido entre el wrapper
/// `LoudnessProbe` de la cadena de audio (que mide pre-ganancia) y el comando
/// `NormAuto`, que lee la medida y fija la ganancia de `dynamics()`.
fn loudness() -> &'static LoudnessTap {
    static SLOT: OnceLock<LoudnessTap> = OnceLock::new();
    SLOT.get_or_init(LoudnessTap::new)
}

/// Handle al [`Playlist`] activo cuando hay tracks WAV/MP3. `None`
/// si la fuente es tono A4 — en ese caso los botones de seek /
/// playlist / speed quedan apagados.
fn playlist_slot() -> &'static OnceLock<Option<Arc<Mutex<Playlist>>>> {
    static SLOT: OnceLock<Option<Arc<Mutex<Playlist>>>> = OnceLock::new();
    &SLOT
}

/// Pista de subtítulos cargada (por env o auto-carga sidecar S5). Se
/// consulta por timestamp del seekable_handle activo.
fn subtitles_slot() -> &'static OnceLock<Option<SubtitleTrack>> {
    static SLOT: OnceLock<Option<SubtitleTrack>> = OnceLock::new();
    &SLOT
}

/// Delay de subtítulos en ms (S4). Positivo retrasa el subtítulo; se aplica
/// al consultar el cue activo (`subtitle_strip`). Tope ±60 s.
static SUB_DELAY_MS: AtomicI64 = AtomicI64::new(0);
const MAX_SUB_DELAY_MS: i64 = 60_000;

/// Lee y parsea un archivo de subtítulos (autodetect SRT/WebVTT/ASS por
/// cabecera). Log a stderr; `None` si no se puede leer o no hay cues.
fn load_subtitle_file(path: &Path) -> Option<SubtitleTrack> {
    match std::fs::read_to_string(path) {
        Ok(body) => match SubtitleTrack::parse_subtitles(&body) {
            Ok(t) => {
                eprintln!("media-app: subtitles {} · {} cues", path.display(), t.len());
                Some(t)
            }
            Err(e) => {
                eprintln!("media-app: subtítulos inválidos en {} ({e})", path.display());
                None
            }
        },
        Err(e) => {
            eprintln!("media-app: no pude leer subtítulos {}: {e}", path.display());
            None
        }
    }
}

/// Candidatos de subtítulo "sidecar" de un video: mismo nombre base con
/// extensión de subtítulo, en orden de preferencia. Puro y testeable.
fn subtitle_sidecar_candidates(video: &Path) -> Vec<PathBuf> {
    ["srt", "vtt", "ass", "ssa"]
        .iter()
        .map(|e| video.with_extension(e))
        .collect()
}

/// S5: busca junto al video un subtítulo con su mismo nombre base y lo
/// carga, sin necesidad de env. Sólo para archivos locales (un stream de
/// red no tiene hermano en disco). Silencioso si no hay ninguno.
fn auto_load_sidecar_subtitles() -> Option<SubtitleTrack> {
    let video = video_path_slot().get()?;
    if is_network_url(&video.to_string_lossy()) {
        return None;
    }
    for cand in subtitle_sidecar_candidates(video) {
        if cand.is_file() {
            eprintln!("media-app: subtítulo sidecar {}", cand.display());
            return load_subtitle_file(&cand);
        }
    }
    None
}

/// `MediaSession` compartida entre el FfmpegVideoSource del pipeline y
/// el FfmpegAudioSource del Playlist cuando la fuente es un archivo de
/// video. Un único proceso ffmpeg sirve ambos streams.
fn ffmpeg_session_slot() -> &'static OnceLock<Option<MediaSession>> {
    static SLOT: OnceLock<Option<MediaSession>> = OnceLock::new();
    &SLOT
}

/// Adapter que comparte una fuente vía `Arc<Mutex<T>>` sin moverla.
/// El cpal sink ve un `AudioSource` normal; otros consumidores (la UI
/// para seek/position) pueden seguir hablando con el inner por la
/// otra punta del Arc.
struct SharedAudio<T> {
    inner: Arc<Mutex<T>>,
}

impl<T: AudioSource> AudioSource for SharedAudio<T> {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.inner.lock().fill(buf, sample_rate, channels);
    }
}

/// Una pista cargada de la playlist — enum cerrado para evitar
/// dispatch dinámico y mantener clara la lista de formatos.
enum LoadedTrack {
    Wav(WavSource),
    Mp3(Mp3Source),
    /// Audio Opus NATIVO (puro-Rust, opus-wave) desde Ogg. Par del video
    /// AV1 nativo — sin pasar por ffmpeg.
    Opus(OpusSource),
    /// Audio extraído por ffmpeg desde un archivo de video. Comparte
    /// `MediaSession` con el FfmpegVideoSource del pipeline — un solo
    /// subprocess sirve ambos streams.
    FfmpegAudio(FfmpegAudioSource),
}

impl LoadedTrack {
    fn from_path(path: &std::path::Path) -> Result<Self, String> {
        match path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("wav") => WavSource::from_path(path)
                .map(LoadedTrack::Wav)
                .map_err(|e| format!("WAV {}: {e}", path.display())),
            Some("mp3") => Mp3Source::from_path(path)
                .map(LoadedTrack::Mp3)
                .map_err(|e| format!("MP3 {}: {e}", path.display())),
            Some("opus" | "ogg") => OpusSource::from_path(path)
                .map(LoadedTrack::Opus)
                .map_err(|e| format!("Opus {}: {e}", path.display())),
            other => Err(format!(
                "extensión {:?} no soportada en playlist (.wav | .mp3 | .opus)",
                other
            )),
        }
    }

    fn set_speed(&mut self, speed: f32) {
        match self {
            LoadedTrack::Wav(w) => w.set_speed(speed),
            LoadedTrack::Mp3(m) => m.set_speed(speed),
            LoadedTrack::Opus(o) => o.set_speed(speed),
            // FfmpegAudio: el binario ffmpeg no expone varispeed en
            // tiempo real sin re-encoding; respawnear con -af atempo
            // metería un salto cada vez. Por ahora no-op.
            LoadedTrack::FfmpegAudio(_) => {}
        }
    }

    fn set_loop(&mut self, looped: bool) {
        match self {
            LoadedTrack::Wav(w) => w.set_loop(looped),
            LoadedTrack::Mp3(m) => m.set_loop(looped),
            LoadedTrack::Opus(o) => o.set_loop(looped),
            // FfmpegAudio no loopea naturalmente (al EOF emite
            // silencio); el Playlist decide qué hacer con la pista.
            LoadedTrack::FfmpegAudio(_) => {}
        }
    }

    /// `true` cuando la pista llegó al final en modo no-loop. Para
    /// FfmpegAudio se compara position con duration porque el
    /// `exhausted` interno no es accesible.
    fn is_finished(&self) -> bool {
        match self {
            LoadedTrack::Wav(w) => w.is_finished(),
            LoadedTrack::Mp3(m) => m.is_finished(),
            LoadedTrack::Opus(o) => o.is_finished(),
            LoadedTrack::FfmpegAudio(a) => {
                let dur = a.duration().unwrap_or(Duration::ZERO);
                !dur.is_zero()
                    && a.position() + Duration::from_millis(80) >= dur
            }
        }
    }
}

impl AudioSource for LoadedTrack {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        match self {
            LoadedTrack::Wav(w) => w.fill(buf, sample_rate, channels),
            LoadedTrack::Mp3(m) => m.fill(buf, sample_rate, channels),
            LoadedTrack::Opus(o) => o.fill(buf, sample_rate, channels),
            LoadedTrack::FfmpegAudio(a) => a.fill(buf, sample_rate, channels),
        }
    }
}

impl Seekable for LoadedTrack {
    fn position(&self) -> Duration {
        match self {
            LoadedTrack::Wav(w) => w.position(),
            LoadedTrack::Mp3(m) => m.position(),
            LoadedTrack::Opus(o) => o.position(),
            LoadedTrack::FfmpegAudio(a) => a.position(),
        }
    }
    fn duration(&self) -> Option<Duration> {
        match self {
            LoadedTrack::Wav(w) => w.duration(),
            LoadedTrack::Mp3(m) => m.duration(),
            LoadedTrack::Opus(o) => o.duration(),
            LoadedTrack::FfmpegAudio(a) => a.duration(),
        }
    }
    fn seek_to(&mut self, pos: Duration) {
        match self {
            LoadedTrack::Wav(w) => w.seek_to(pos),
            LoadedTrack::Mp3(m) => m.seek_to(pos),
            LoadedTrack::Opus(o) => o.seek_to(pos),
            LoadedTrack::FfmpegAudio(a) => a.seek_to(pos),
        }
    }
}

/// Modo de loop del Playlist global.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RepeatMode {
    /// Reproduce las pistas en orden y al final del último deja
    /// silencio (las pistas individuales NO loopean).
    Off,
    /// La pista actual se repite hasta que el usuario cambie de
    /// pista manualmente. Se delega al `set_loop(true)` del track.
    One,
    /// Al terminar avanza a la próxima; al final del último vuelve
    /// al primero.
    All,
}

impl RepeatMode {
    fn next(self) -> Self {
        match self {
            Self::Off => Self::One,
            Self::One => Self::All,
            Self::All => Self::Off,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Off => "rep-",
            Self::One => "rep1",
            Self::All => "repA",
        }
    }
}

/// Playlist con prev/next manual + auto-advance al fin de cada pista
/// según [`RepeatMode`] y [`Shuffle`]. Mantiene una `current` cargada
/// y el resto de `tracks` como paths — al cambiar de pista se decodea
/// el archivo nuevo y se descarta el viejo. La velocidad se persiste
/// entre cambios.
struct Playlist {
    tracks: Vec<PathBuf>,
    idx: usize,
    current: LoadedTrack,
    speed: f32,
    repeat: RepeatMode,
    shuffle: Option<ShuffleOrder>,
    /// Estado RNG simple para `ShuffleOrder::reshuffle` — xorshift de
    /// 64 bits, suficiente para randomizar un orden de N pistas.
    rng_state: u64,
}

struct ShuffleOrder {
    order: Vec<usize>,
    pos: usize,
}

impl Playlist {
    fn new(tracks: Vec<PathBuf>) -> Result<Self, String> {
        if tracks.is_empty() {
            return Err("playlist vacía".into());
        }
        let mut current = LoadedTrack::from_path(&tracks[0])?;
        // Default: las pistas individuales no loopean — el Playlist
        // decide qué pasa al final según RepeatMode.
        current.set_loop(false);
        Ok(Self {
            tracks,
            idx: 0,
            current,
            speed: 1.0,
            repeat: RepeatMode::Off,
            shuffle: None,
            rng_state: 0x9E37_79B9_7F4A_7C15,
        })
    }

    /// Playlist de una sola pista construida desde un track ya
    /// cargado (caso video file con FfmpegAudio). `tracks` queda con
    /// el `path` correspondiente para etiquetado pero no se usa para
    /// reload — prev/next quedan inertes con un solo elemento.
    fn new_single(label_path: PathBuf, mut track: LoadedTrack) -> Self {
        track.set_loop(false);
        Self {
            tracks: vec![label_path],
            idx: 0,
            current: track,
            speed: 1.0,
            repeat: RepeatMode::Off,
            shuffle: None,
            rng_state: 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn repeat_mode(&self) -> RepeatMode {
        self.repeat
    }

    fn shuffle_on(&self) -> bool {
        self.shuffle.is_some()
    }

    fn cycle_repeat(&mut self) {
        self.repeat = self.repeat.next();
        // Si pasamos a `One`, activamos el loop interno del track —
        // así no hay glitch al reiniciar el sample 0.
        let want_loop = matches!(self.repeat, RepeatMode::One);
        self.current.set_loop(want_loop);
    }

    /// Fija el modo de repetición absoluto (para aplicar el default de la
    /// config al arrancar), con el mismo efecto de loop que `cycle_repeat`.
    fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
        self.current.set_loop(matches!(self.repeat, RepeatMode::One));
    }

    fn toggle_shuffle(&mut self) {
        if self.shuffle.is_some() {
            self.shuffle = None;
        } else if self.tracks.len() > 1 {
            self.shuffle = Some(self.build_shuffle_order());
        }
    }

    fn build_shuffle_order(&mut self) -> ShuffleOrder {
        let mut order: Vec<usize> = (0..self.tracks.len()).collect();
        // Fisher–Yates con xorshift64.
        for i in (1..order.len()).rev() {
            let j = (self.rand_u64() % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
        // Posiciona al inicio del shuffle en el track actual si está
        // adentro — UX más natural que arrancar de otra pista.
        let pos = order.iter().position(|&t| t == self.idx).unwrap_or(0);
        ShuffleOrder { order, pos }
    }

    fn rand_u64(&mut self) -> u64 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        x
    }

    fn track_path(&self) -> &std::path::Path {
        &self.tracks[self.idx]
    }

    fn len(&self) -> usize {
        self.tracks.len()
    }

    fn idx(&self) -> usize {
        self.idx
    }

    fn current_speed(&self) -> f32 {
        self.speed
    }

    fn step(&mut self, delta: i64) {
        if self.tracks.len() <= 1 {
            return;
        }
        let new = match self.shuffle.as_mut() {
            Some(sh) => {
                let n = sh.order.len() as i64;
                let new_pos = (sh.pos as i64 + delta).rem_euclid(n) as usize;
                sh.pos = new_pos;
                sh.order[new_pos]
            }
            None => {
                let n = self.tracks.len() as i64;
                ((self.idx as i64 + delta).rem_euclid(n)) as usize
            }
        };
        match LoadedTrack::from_path(&self.tracks[new]) {
            Ok(mut t) => {
                t.set_speed(self.speed);
                // Respeta el modo: One = loop interno, Off/All = no.
                t.set_loop(matches!(self.repeat, RepeatMode::One));
                self.idx = new;
                self.current = t;
                eprintln!(
                    "media-app: playlist [{}/{}] → {}",
                    self.idx + 1,
                    self.tracks.len(),
                    self.tracks[self.idx].display()
                );
            }
            Err(e) => eprintln!("media-app: cambio de pista falló: {e}"),
        }
    }

    fn next(&mut self) {
        self.step(1)
    }
    fn prev(&mut self) {
        self.step(-1)
    }

    /// Salta a la pista `target` (índice absoluto). No-op si está fuera de
    /// rango o ya es la actual. Sincroniza la posición de shuffle si aplica.
    fn jump_to(&mut self, target: usize) {
        if target >= self.tracks.len() || target == self.idx {
            return;
        }
        match LoadedTrack::from_path(&self.tracks[target]) {
            Ok(mut t) => {
                t.set_speed(self.speed);
                t.set_loop(matches!(self.repeat, RepeatMode::One));
                self.idx = target;
                self.current = t;
                if let Some(sh) = self.shuffle.as_mut() {
                    if let Some(p) = sh.order.iter().position(|&i| i == target) {
                        sh.pos = p;
                    }
                }
            }
            Err(e) => eprintln!("media-app: salto de pista falló: {e}"),
        }
    }

    /// Nombres (basename) de cada pista, para pintar la cola.
    fn track_labels(&self) -> Vec<String> {
        self.tracks
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.display().to_string())
            })
            .collect()
    }

    /// Verifica si la pista terminó (en modo no-loop) y avanza según
    /// `repeat`. Llamado desde [`AudioSource::fill`] del Playlist
    /// después de cada bloque para que el siguiente bloque ya salga
    /// del track nuevo.
    fn maybe_auto_advance(&mut self) {
        if !self.current.is_finished() {
            return;
        }
        match self.repeat {
            RepeatMode::One => {
                // Con loop interno encendido nunca debería pasar
                // (set_loop(true) en cycle_repeat / step), pero por
                // robustez reseteamos.
                self.current.seek_to(Duration::ZERO);
            }
            RepeatMode::All => {
                if self.tracks.len() > 1 {
                    self.next();
                } else {
                    // Single track con repeat All se comporta como
                    // repeat One — reinicia.
                    self.current.seek_to(Duration::ZERO);
                }
            }
            RepeatMode::Off => {
                // Avanzo si no es la última; si es la última, dejo
                // silencio (la pista se queda finished y fill emite
                // ceros indefinidamente).
                let last = match self.shuffle.as_ref() {
                    Some(sh) => sh.pos + 1 >= sh.order.len(),
                    None => self.idx + 1 >= self.tracks.len(),
                };
                if !last {
                    self.next();
                }
            }
        }
    }

    fn set_speed(&mut self, speed: f32) {
        let s = speed.clamp(0.1, 4.0);
        self.speed = s;
        self.current.set_speed(s);
    }
}

impl AudioSource for Playlist {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.current.fill(buf, sample_rate, channels);
        // Después de cada bloque, si la pista quedó "finished" el
        // próximo bloque arranca con la nueva pista (o queda en
        // silencio si Off + última).
        self.maybe_auto_advance();
    }
}

impl Seekable for Playlist {
    fn position(&self) -> Duration {
        self.current.position()
    }
    fn duration(&self) -> Option<Duration> {
        self.current.duration()
    }
    fn seek_to(&mut self, pos: Duration) {
        self.current.seek_to(pos);
    }
}

/// Posición del reloj de audio (sample-accurate, avanza al ritmo en que
/// el sink consume samples) del track en curso, o `None` cuando no hay
/// playlist (tono A4 / testcard). Es el reloj **maestro** de M1: el video
/// se acomoda a él; sin él, el video cae al reloj de pared.
/// Foto no-bloqueante del estado de reproducción para la vista.
#[derive(Clone)]
struct PlaybackSnapshot {
    /// `false` cuando no hay playlist (tono A4 / testcard).
    present: bool,
    position: Duration,
    duration: Option<Duration>,
    idx: usize,
    len: usize,
    speed: f32,
    repeat_label: &'static str,
    shuffle_on: bool,
}

impl Default for PlaybackSnapshot {
    fn default() -> Self {
        PlaybackSnapshot {
            present: false,
            position: Duration::ZERO,
            duration: None,
            idx: 0,
            len: 0,
            speed: 1.0,
            repeat_label: "rep-",
            shuffle_on: false,
        }
    }
}

/// Snapshot del estado de reproducción SIN bloquear el hilo de UI. Usa
/// `try_lock`: el hilo de audio (cpal) puede tener tomado el lock del
/// `Playlist` mientras hace el `read_exact` bloqueante del pipe de ffmpeg —
/// si el UI lo esperara, dejaría de drenar el pipe de video → ffmpeg se traba
/// → el audio no avanza → cpal no suelta el lock: deadlock ("avanza un
/// segundo y se detiene, ni se deja cerrar"). Por eso TODA la vista lee de
/// acá. Si el lock está ocupado, devuelve el último snapshot conocido (la UI
/// muestra valores de hace un frame, no se congela).
fn playback_snapshot() -> PlaybackSnapshot {
    static CACHE: OnceLock<Mutex<PlaybackSnapshot>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(PlaybackSnapshot::default()));
    let Some(handle) = playlist_slot().get().and_then(|o| o.as_ref()) else {
        return PlaybackSnapshot::default();
    };
    match handle.try_lock() {
        Some(pl) => {
            let snap = PlaybackSnapshot {
                present: true,
                position: pl.position(),
                duration: pl.duration(),
                idx: pl.idx(),
                len: pl.len(),
                speed: pl.current_speed(),
                repeat_label: pl.repeat_mode().label(),
                shuffle_on: pl.shuffle_on(),
            };
            drop(pl);
            // CACHE sólo lo toca el hilo de UI → este lock nunca contiende.
            *cache.lock() = snap.clone();
            snap
        }
        None => cache.lock().clone(),
    }
}

/// Posición del reloj de audio para el sync A/V del paint. Deriva del
/// snapshot no-bloqueante (ver [`playback_snapshot`]).
fn current_audio_position() -> Option<Duration> {
    let s = playback_snapshot();
    s.present.then_some(s.position)
}

/// Reinicia los contadores de sync A/V tras un seek o cambio de pista (los
/// frames viejos no deben contar contra el nuevo punto). No-op si el
/// pipeline todavía no se inicializó (el primer paint lo crea).
/// Pedido de "presentar un frame de video YA aunque esté en pausa": lo activa
/// cada seek (vía [`reset_av_sync_anchor`]). El render loop (`gpu_paint`) lo lee
/// y, mientras esté puesto, tickea el video aun pausado y presenta el frame sin
/// pasar por el drop de A/V; lo apaga recién cuando logró subir un frame (así un
/// seek en la ruta ffmpeg, que respawnea el proceso, no se pierde el primer
/// frame si tarda un par de paints en llegar).
static SEEK_FORCE: AtomicBool = AtomicBool::new(false);

fn reset_av_sync_anchor() {
    if let Some(pipe) = pipeline_slot().get() {
        pipe.sync.lock().reset();
    }
    // Tras un seek, queremos ver el destino aunque estemos en pausa.
    SEEK_FORCE.store(true, Ordering::Relaxed);
}

/// Mueve la posición del Playlist (= track actual) en `delta_secs`
/// (negativo = atrás) con wrap módulo duration. No-op si no hay
/// playlist (tono A4).
fn seek_audio_by(delta_secs: i64) {
    let Some(handle) = playlist_slot().get().and_then(|o| o.as_ref()) else {
        return;
    };
    let mut src = handle.lock();
    let dur = src.duration().unwrap_or(Duration::from_secs(1));
    let dur_s = dur.as_secs_f64().max(0.001);
    let cur_s = src.position().as_secs_f64();
    let new_s = (cur_s + delta_secs as f64).rem_euclid(dur_s);
    src.seek_to(Duration::from_secs_f64(new_s));
    drop(src);
    // El video no debe interpretar el salto como tiempo transcurrido.
    reset_av_sync_anchor();
}

/// Salta a la pista `idx` de la cola (clic en la ventana de playlist). No-op
/// sin playlist. Mismo patrón que `seek_audio_to`: lock breve + re-ancla A/V.
fn jump_playlist_to(idx: usize) {
    let Some(handle) = playlist_slot().get().and_then(|o| o.as_ref()) else {
        return;
    };
    handle.lock().jump_to(idx);
    reset_av_sync_anchor();
}

/// Salta a la posición **absoluta** `fraction` (0..1) de la duración del
/// track actual. Lo dispara el click en el timeline (`MediaCommand::SeekTo`).
/// No-op sin playlist (tono A4).
fn seek_audio_to(fraction: f32) {
    let Some(handle) = playlist_slot().get().and_then(|o| o.as_ref()) else {
        return;
    };
    let mut src = handle.lock();
    let dur_s = src.duration().unwrap_or(Duration::ZERO).as_secs_f64();
    let f = fraction.clamp(0.0, 1.0) as f64;
    src.seek_to(Duration::from_secs_f64(dur_s * f));
    drop(src);
    // El video no debe interpretar el salto como tiempo transcurrido.
    reset_av_sync_anchor();
}

/// Construye el catálogo de acciones para el command palette a partir de
/// los settings vivos. Cada acción del reproductor entra como un
/// [`PaletteCommand`] con su grupo y, si hay una tecla atada en el
/// keymap, el hint del atajo. El `id` es el índice — el vector paralelo
/// devuelto lo mapea de vuelta al [`MediaCommand`] a ejecutar. El título
/// sale de `MediaCommand::describe()`: una sola fuente, igual que la
/// ayuda.
fn build_command_catalog(s: &ControlSettings) -> (Vec<PaletteCommand>, Vec<MediaCommand>) {
    use MediaCommand::*;
    let step = s.seek_step_secs;
    let vstep = s.volume_step;
    let acciones: Vec<(MediaCommand, &str)> = vec![
        (TogglePause, "Transporte"),
        (SeekBy { secs: step }, "Transporte"),
        (SeekBy { secs: -step }, "Transporte"),
        (SeekTo { fraction: 0.0 }, "Transporte"),
        (PrevTrack, "Playlist"),
        (NextTrack, "Playlist"),
        (ChapterPrev, "Capítulos"),
        (ChapterNext, "Capítulos"),
        (CycleRepeat, "Playlist"),
        (ToggleShuffle, "Playlist"),
        (VolumeBy { delta: vstep }, "Volumen"),
        (VolumeBy { delta: -vstep }, "Volumen"),
        (SetVolume { level: 1.0 }, "Volumen"),
        (SetVolume { level: 0.5 }, "Volumen"),
        (SetVolume { level: 0.0 }, "Volumen"),
        (ToggleMute, "Volumen"),
        (SpeedStep { dir: 1 }, "Velocidad"),
        (SpeedStep { dir: -1 }, "Velocidad"),
        (SetSpeed { mult: 1.0 }, "Velocidad"),
        (EqToggle, "Ecualizador"),
        (EqReset, "Ecualizador"),
        (AvSyncBy { ms: -50 }, "Sync A/V"),
        (AvSyncBy { ms: 50 }, "Sync A/V"),
        (AvSyncReset, "Sync A/V"),
        (ColorToggle, "Color"),
        (ColorReset, "Color"),
        (ColorBy { param: ColorParam::Brightness, delta: 0.05 }, "Color"),
        (ColorBy { param: ColorParam::Brightness, delta: -0.05 }, "Color"),
        (ColorBy { param: ColorParam::Contrast, delta: 0.1 }, "Color"),
        (ColorBy { param: ColorParam::Contrast, delta: -0.1 }, "Color"),
        (ColorBy { param: ColorParam::Gamma, delta: 0.1 }, "Color"),
        (ColorBy { param: ColorParam::Gamma, delta: -0.1 }, "Color"),
        (ColorBy { param: ColorParam::Saturation, delta: 0.1 }, "Color"),
        (ColorBy { param: ColorParam::Saturation, delta: -0.1 }, "Color"),
        (ColorBy { param: ColorParam::Hue, delta: 10.0 }, "Color"),
        (ColorBy { param: ColorParam::Hue, delta: -10.0 }, "Color"),
        (RotateBy { dir: 1 }, "Orientación"),
        (RotateBy { dir: -1 }, "Orientación"),
        (FlipH, "Orientación"),
        (FlipV, "Orientación"),
        (OrientReset, "Orientación"),
        (SubDelayBy { ms: -100 }, "Subtítulos"),
        (SubDelayBy { ms: 100 }, "Subtítulos"),
        (SubDelayReset, "Subtítulos"),
        (NormToggle, "Normalización"),
        (NormAuto, "Normalización"),
        (NormGainBy { db: 3.0 }, "Normalización"),
        (NormGainBy { db: -3.0 }, "Normalización"),
        (NormReset, "Normalización"),
        (Snapshot, "Captura"),
        (ToggleRecord, "Captura"),
    ];
    let mut catalog = Vec::with_capacity(acciones.len());
    let mut cmds = Vec::with_capacity(acciones.len());
    for (i, (cmd, group)) in acciones.into_iter().enumerate() {
        let mut pc = PaletteCommand::new(i.to_string(), cmd.describe(), group);
        if let Some(sc) = shortcut_for(&s.keymap, &cmd) {
            pc = pc.with_shortcut(sc);
        }
        catalog.push(pc);
        cmds.push(cmd);
    }
    // Scripts Rhai de la biblioteca: descubribles y ejecutables desde el
    // palette igual que las acciones nativas, agrupados aparte. El hint del
    // atajo sale del mismo reverse-lookup sobre el keymap vivo.
    for ns in &s.scripts {
        let cmd = Script {
            name: ns.name.clone(),
        };
        let id = cmds.len();
        let mut pc = PaletteCommand::new(id.to_string(), cmd.describe(), "Scripts");
        if let Some(sc) = shortcut_for(&s.keymap, &cmd) {
            pc = pc.with_shortcut(sc);
        }
        catalog.push(pc);
        cmds.push(cmd);
    }
    // Ecualizador: cada banda con un realce y un corte de 3 dB,
    // descubribles y ejecutables desde el palette. El título sale de
    // `describe()` (frecuencia ISO + signo) — misma fuente que la ayuda.
    for idx in 0..ISO_10_BANDS_HZ.len() {
        for delta_db in [3.0_f32, -3.0] {
            let cmd = EqBandBy { idx, delta_db };
            let id = cmds.len();
            let mut pc = PaletteCommand::new(id.to_string(), cmd.describe(), "Ecualizador");
            if let Some(sc) = shortcut_for(&s.keymap, &cmd) {
                pc = pc.with_shortcut(sc);
            }
            catalog.push(pc);
            cmds.push(cmd);
        }
    }
    (catalog, cmds)
}

/// Reverse-lookup: el display del primer chord atado a `cmd` en el
/// keymap, si hay alguno. Es el hint que el palette muestra a la derecha
/// de la fila — refleja el binding vivo, no una constante.
fn shortcut_for(km: &media_core::control::Keymap, cmd: &MediaCommand) -> Option<String> {
    km.bindings
        .iter()
        .find(|b| &b.command == cmd)
        .map(|b| b.chord.display())
}

/// Routea un `PaletteMsg` al módulo command-palette. Lazy-init en `Open`.
/// En `Invoke(id)` cierra el palette y dispatcha el `MediaCommand` cuyo
/// índice es `id` — el comando se ejecuta en el siguiente turno del loop,
/// pasando por el mismo `apply_command` que botones y teclado.
fn apply_palette(model: Model, pm: PaletteMsg, handle: &Handle<Msg>) -> Model {
    let mut m = model;
    if matches!(pm, PaletteMsg::Open) && m.palette.is_none() {
        m.palette = Some(PaletteState::new(&m.palette_commands));
        return m;
    }
    let action = match m.palette.as_mut() {
        Some(state) => palette::apply(state, pm, &m.palette_commands),
        None => return m,
    };
    match action {
        PaletteAction::None => {}
        PaletteAction::Close => m.palette = None,
        PaletteAction::Invoke(id) => {
            m.palette = None;
            if let Some(cmd) = id.parse::<usize>().ok().and_then(|i| m.palette_cmds.get(i)) {
                handle.dispatch(Msg::Command(cmd.clone()));
            }
        }
    }
    m
}

/// Ejecuta un [`MediaCommand`] sobre el estado vivo del reproductor.
/// Único punto donde un comando se vuelve efecto — lo comparten botones
/// (vía `Msg::Command`) y teclado (vía `on_key`).
fn apply_command(cmd: MediaCommand) {
    use MediaCommand::*;
    match cmd {
        TogglePause => {
            pause().toggle();
        }
        SeekBy { secs } => seek_audio_by(secs),
        SeekTo { fraction } => seek_audio_to(fraction),
        VolumeBy { delta } => {
            volume().update(|v| v + delta);
        }
        SetVolume { level } => {
            volume().update(|_| level);
        }
        ToggleMute => {
            // Guarda el volumen al silenciar; lo restaura al des-silenciar.
            let slot = muted_volume();
            let mut g = slot.lock();
            match g.take() {
                Some(prev) => volume().update(|_| prev),
                None => {
                    *g = Some(volume().get());
                    volume().update(|_| 0.0);
                }
            }
        }
        PrevTrack => {
            if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
                h.lock().prev();
            }
        }
        NextTrack => {
            if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
                h.lock().next();
            }
        }
        ChapterNext => {
            if let Some(ch) = chapters_slot().get() {
                let pos = playback_snapshot().position;
                if let Some(c) = ch.next(pos) {
                    seek_audio_to_pos(c.start);
                }
            }
        }
        ChapterPrev => {
            if let Some(ch) = chapters_slot().get() {
                let pos = playback_snapshot().position;
                if let Some(c) = ch.prev(pos, Duration::from_secs(3)) {
                    seek_audio_to_pos(c.start);
                }
            }
        }
        SpeedStep { dir } => step_speed(dir),
        SetSpeed { mult } => set_speed_abs(mult),
        CycleRepeat => {
            if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
                let mut pl = h.lock();
                pl.cycle_repeat();
                eprintln!("media-app: repeat {}", pl.repeat_mode().label());
            }
        }
        ToggleShuffle => {
            if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
                let mut pl = h.lock();
                pl.toggle_shuffle();
                eprintln!(
                    "media-app: shuffle {}",
                    if pl.shuffle_on() { "on" } else { "off" }
                );
            }
        }
        Snapshot => do_snapshot(),
        ToggleRecord => toggle_record(),
        Script { name } => run_script(&name),
        EqToggle => {
            let e = eq();
            let on = !e.is_enabled();
            e.set_enabled(on);
            eprintln!("media-app: eq {}", if on { "on" } else { "off" });
        }
        EqBandBy { idx, delta_db } => {
            let e = eq();
            let cur = e.gains().get(idx).copied().unwrap_or(0.0);
            e.set_gain(idx, (cur + delta_db).clamp(-12.0, 12.0));
        }
        EqReset => {
            eq().set_all_gains(&[0.0; ISO_10_BANDS_HZ.len()]);
            eprintln!("media-app: eq plano");
        }
        AvSyncBy { ms } => {
            if let Some(pipe) = pipeline_slot().get() {
                let mut s = pipe.sync.lock();
                s.add_offset_ms(ms);
                eprintln!("media-app: sync A/V {:+}ms", s.offset_ms());
            }
        }
        AvSyncReset => {
            if let Some(pipe) = pipeline_slot().get() {
                pipe.sync.lock().set_offset_ms(0);
                eprintln!("media-app: sync A/V a cero");
            }
        }
        ColorToggle => {
            let c = color();
            let on = !c.is_enabled();
            c.set_enabled(on);
            eprintln!("media-app: color {}", if on { "on" } else { "off" });
        }
        ColorBy { param, delta } => {
            let c = color();
            match param {
                ColorParam::Brightness => c.add_brightness(delta),
                ColorParam::Contrast => c.add_contrast(delta),
                ColorParam::Gamma => c.add_gamma(delta),
                ColorParam::Saturation => c.add_saturation(delta),
                ColorParam::Hue => c.add_hue(delta),
            }
        }
        ColorReset => {
            color().reset();
            eprintln!("media-app: color original");
        }
        RotateBy { dir } => {
            transform().rotate(dir);
            eprintln!("media-app: rotación {}", transform().transform().rotation.label());
        }
        FlipH => transform().toggle_flip_h(),
        FlipV => transform().toggle_flip_v(),
        OrientReset => {
            transform().reset();
            eprintln!("media-app: orientación original");
        }
        SubDelayBy { ms } => {
            let new = (SUB_DELAY_MS.load(Ordering::Relaxed) + ms)
                .clamp(-MAX_SUB_DELAY_MS, MAX_SUB_DELAY_MS);
            SUB_DELAY_MS.store(new, Ordering::Relaxed);
            eprintln!("media-app: subtítulo {new:+}ms");
        }
        SubDelayReset => {
            SUB_DELAY_MS.store(0, Ordering::Relaxed);
            eprintln!("media-app: subtítulo sin delay");
        }
        NormToggle => {
            let d = dynamics();
            let on = !d.is_enabled();
            d.set_enabled(on);
            eprintln!("media-app: normalización {}", if on { "on" } else { "off" });
        }
        NormGainBy { db } => {
            let d = dynamics();
            d.add_gain_db(db);
            eprintln!("media-app: normalización {:+.0} dB", d.gain_db());
        }
        NormReset => {
            dynamics().reset();
            eprintln!("media-app: normalización a 0 dB");
        }
        NormAuto => {
            // Lee la sonoridad integrada medida hasta ahora y fija la ganancia
            // para llevarla al objetivo ReplayGain. La medición la activa el
            // limitador, así que aseguramos la etapa encendida.
            match loudness().gain_to_target_db(REPLAYGAIN_TARGET_LUFS) {
                Some(gain) => {
                    let d = dynamics();
                    d.set_enabled(true);
                    d.set_gain_db(gain);
                    eprintln!(
                        "media-app: normalización automática → {:+.1} dB (objetivo {:.0} LUFS)",
                        d.gain_db(),
                        REPLAYGAIN_TARGET_LUFS
                    );
                }
                None => eprintln!(
                    "media-app: normalización automática — aún sin medición \
                     (reproducí ≳ 1 s primero)"
                ),
            }
        }
    }
}

/// Ejecuta el script Rhai `name` de la biblioteca de `settings()` contra
/// la API del reproductor. Es el `MediaCommand::Script` hecho efecto: el
/// core sólo nombra el script (agnóstico de Rhai), acá se resuelve el
/// `source`, se compila y se corre sobre el runtime vivo. Falla silenciosa
/// con log — un script roto o inexistente nunca debe tumbar la app.
fn run_script(name: &str) {
    let Some(src) = settings().script(name).map(str::to_string) else {
        eprintln!("media-app: script «{name}» no existe en controles.ron");
        return;
    };
    let engine = script_engine();
    if let Err(e) = engine.run(&src) {
        eprintln!("media-app: script «{name}»: {e}");
    }
}

/// Velocidad de reproducción actual (1.0× si no hay playlist). Getter para
/// la API de scripts.
fn player_speed() -> f64 {
    playlist_slot()
        .get()
        .and_then(|o| o.as_ref())
        .map(|h| h.lock().current_speed() as f64)
        .unwrap_or(1.0)
}

/// Arma un motor Rhai con la API del reproductor bindeada. Cada función
/// reentra a los mismos primitivos que `apply_command` (los slots
/// globales), así un script compone acciones nativas: `set_volume(1.0);
/// set_speed(1.25);` o condicionales sobre `is_paused()`. El motor se
/// construye por ejecución (microsegundos, una tecla es un evento raro) y
/// lleva una cota de operaciones para que un script no cuelgue la UI.
fn script_engine() -> rhai::Engine {
    let mut engine = rhai::Engine::new();
    engine.set_max_operations(50_000);
    // Transporte / pausa.
    engine.register_fn("toggle_pause", || {
        pause().toggle();
    });
    engine.register_fn("pause", || pause().pause());
    engine.register_fn("resume", || pause().resume());
    engine.register_fn("is_paused", || pause().is_paused());
    engine.register_fn("seek", |secs: i64| seek_audio_by(secs));
    // Volumen.
    engine.register_fn("volume", || volume().get() as f64);
    engine.register_fn("set_volume", |level: f64| {
        volume().update(|_| level as f32);
    });
    engine.register_fn("add_volume", |delta: f64| {
        volume().update(|v| v + delta as f32);
    });
    // Velocidad.
    engine.register_fn("speed", player_speed);
    engine.register_fn("set_speed", |mult: f64| set_speed_abs(mult as f32));
    engine.register_fn("step_speed", |dir: i64| step_speed(dir as i32));
    // Playlist.
    engine.register_fn("next_track", || apply_command(MediaCommand::NextTrack));
    engine.register_fn("prev_track", || apply_command(MediaCommand::PrevTrack));
    engine.register_fn("cycle_repeat", || apply_command(MediaCommand::CycleRepeat));
    engine.register_fn("toggle_shuffle", || {
        apply_command(MediaCommand::ToggleShuffle)
    });
    // Captura.
    engine.register_fn("snapshot", do_snapshot);
    engine.register_fn("toggle_record", toggle_record);
    engine.register_fn("is_recording", || recorder().is_recording());
    engine
}

/// Cicla la velocidad `dir` pasos por `settings().speed_steps` (wrap en
/// ambos sentidos). No-op sin playlist activo o sin pasos configurados.
fn step_speed(dir: i32) {
    let Some(handle) = playlist_slot().get().and_then(|o| o.as_ref()) else {
        return;
    };
    let s = settings();
    let steps = &s.speed_steps;
    if steps.is_empty() {
        return;
    }
    let mut pl = handle.lock();
    let cur = pl.current_speed();
    // Índice actual (con tolerancia ε para evitar problemas de f32).
    let idx = steps
        .iter()
        .position(|&s| (s - cur).abs() < 1e-3)
        .unwrap_or(0) as i32;
    let n = steps.len() as i32;
    let next_idx = ((idx + dir) % n + n) % n;
    let next = steps[next_idx as usize];
    pl.set_speed(next);
    eprintln!("media-app: speed {:.2}×", next);
}

/// Fija una velocidad absoluta (p.ej. `=` → 1.0×). No-op sin playlist.
fn set_speed_abs(mult: f32) {
    if let Some(handle) = playlist_slot().get().and_then(|o| o.as_ref()) {
        handle.lock().set_speed(mult);
        eprintln!("media-app: speed {:.2}×", mult);
    }
}

/// Arma/cierra la grabación WAV del stream de audio en el cwd.
fn toggle_record() {
    let rec = recorder();
    if rec.is_recording() {
        match rec.stop() {
            Ok(p) => eprintln!("media-app: recording cerrada en {}", p.display()),
            Err(e) => eprintln!("media-app: stop recording: {e}"),
        }
    } else {
        let path = default_recording_path(".");
        match rec.start(&path) {
            Ok(p) => eprintln!("media-app: grabando en {}", p.display()),
            Err(e) => eprintln!("media-app: start recording: {e}"),
        }
    }
}

/// Escribe un PNG con el frame de video pendiente. No-op (con log) si la
/// pipeline aún no montó o no hay frame consistente.
fn do_snapshot() {
    let Some(pipe) = pipeline_slot().get() else {
        eprintln!("media-app: pipeline aún no montada");
        return;
    };
    let (w, h) = *pipe.last_dim.lock();
    let buf = pipe.buf.lock().clone();
    let expected = (w as usize) * (h as usize) * 4;
    if w == 0 || h == 0 || buf.len() != expected {
        eprintln!("media-app: no hay frame para snapshot todavía");
        return;
    }
    let path = default_snapshot_path();
    match image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(w, h, buf) {
        Some(img) => match img.save(&path) {
            Ok(()) => eprintln!(
                "media-app: snapshot {}×{} guardado en {}",
                w,
                h,
                path.display()
            ),
            Err(e) => eprintln!("media-app: save snapshot: {e}"),
        },
        None => eprintln!("media-app: buf inconsistente para snapshot"),
    }
}

/// Traduce un evento de teclado de Llimphi al [`KeyChord`] canónico y
/// agnóstico que entiende el keymap. Sólo dispara en `Pressed`; los
/// caracteres se normalizan a minúscula (el estado de Shift viaja en el
/// flag, no en el case). Teclas que no mapeamos devuelven `None`.
fn chord_from_event(ev: &KeyEvent) -> Option<KeyChord> {
    if ev.state != KeyState::Pressed {
        return None;
    }
    let key = match &ev.key {
        Key::Named(NamedKey::Space) => "Space".to_string(),
        Key::Named(NamedKey::ArrowLeft) => "ArrowLeft".to_string(),
        Key::Named(NamedKey::ArrowRight) => "ArrowRight".to_string(),
        Key::Named(NamedKey::ArrowUp) => "ArrowUp".to_string(),
        Key::Named(NamedKey::ArrowDown) => "ArrowDown".to_string(),
        Key::Named(NamedKey::Enter) => "Enter".to_string(),
        Key::Character(c) => c.to_lowercase(),
        _ => return None,
    };
    Some(KeyChord {
        key,
        ctrl: ev.modifiers.ctrl,
        shift: ev.modifiers.shift,
        alt: ev.modifiers.alt,
    })
}

/// Carga un .m3u simple: una línea por archivo, líneas vacías y `#`
/// se ignoran. Paths relativos se resuelven contra el directorio
/// del propio archivo.
fn load_playlist_file(path: &str) -> Result<Vec<PathBuf>, String> {
    let p = PathBuf::from(path);
    let body = std::fs::read_to_string(&p).map_err(|e| format!("io: {e}"))?;
    let base = p.parent().map(|d| d.to_path_buf());
    let mut out = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let item = PathBuf::from(line);
        let resolved = if item.is_absolute() {
            item
        } else if let Some(b) = &base {
            b.join(item)
        } else {
            item
        };
        out.push(resolved);
    }
    Ok(out)
}

/// Formatea una duración como `M:SS`. Para tracks de menos de una
/// hora — más allá rolls over y se ve raro, pero MVP.
fn fmt_secs(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

/// Path de snapshot único por segundo, en el cwd: `media-snap-N.png`.
fn default_snapshot_path() -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    PathBuf::from(format!("media-snap-{secs}.png"))
}

// El gate de pausa del VIDEO ya no vive en un `PausableVideo`: lo decide el
// render loop (`gpu_paint`), que es quien sabe si hay un seek pendiente. Así,
// estando en pausa, un seek puede tickear y presentar el frame destino (lo que
// `PausableVideo` impedía, porque devolvía `None` mientras estuviera pausado).
// El audio sigue con `PausableAudio` aparte.
fn new_testcard() -> Box<dyn FrameSource + Send> {
    Box::new(TestCard::new(TESTCARD_W, TESTCARD_H, TESTCARD_FPS))
}

fn build_video_source() -> Box<dyn FrameSource + Send> {
    let cfg = config_slot().get().expect("config set");
    match cfg.kind {
        VideoKind::Testcard => new_testcard(),
        VideoKind::Gif => {
            let path = video_path_slot().get().expect("video path set");
            match GifSource::from_path(path) {
                Ok(s) => Box::new(s),
                Err(e) => {
                    eprintln!(
                        "media-app: error abriendo GIF {path:?}: {e} — caigo a testcard"
                    );
                    new_testcard()
                }
            }
        }
        VideoKind::Image => {
            let path = video_path_slot().get().expect("video path set");
            match ImageSource::from_path(path) {
                Ok(s) => Box::new(s),
                Err(e) => {
                    eprintln!(
                        "media-app: error abriendo imagen {path:?}: {e} — caigo a testcard"
                    );
                    new_testcard()
                }
            }
        }
        VideoKind::Ffmpeg => {
            // El audio side ya consumió `audio_read` del session; el
            // video pipe sigue disponible para nosotros.
            match ffmpeg_session_slot()
                .get()
                .and_then(|o| o.as_ref())
                .ok_or_else(|| "ffmpeg session no disponible".to_string())
                .and_then(|s| {
                    FfmpegVideoSource::from_session(s.clone())
                        .map_err(|e| e.to_string())
                }) {
                Ok(s) => Box::new(s),
                Err(e) => {
                    eprintln!("media-app: ffmpeg video: {e} — caigo a testcard");
                    new_testcard()
                }
            }
        }
        VideoKind::Av1 => {
            let path = video_path_slot().get().expect("video path set");
            match media_source_av1::Av1VideoSource::open(path) {
                Ok(s) => Box::new(s),
                Err(e) => {
                    eprintln!("media-app: AV1 nativo {path:?}: {e} — caigo a testcard");
                    new_testcard()
                }
            }
        }
    }
}

fn pipeline_for(device: &wgpu::Device, queue: &wgpu::Queue) -> &'static Pipeline {
    pipeline_slot().get_or_init(|| Pipeline {
        surface: ExternalSurface::new(device, queue),
        // Cadena de video: <decoder> → ColorVideo (V4) → TransformVideo
        // (V3). Ambos hacen bypass en identidad, así que no cuestan nada
        // hasta que el usuario toca color u orientación.
        source: Mutex::new(Box::new(TransformVideo::new(
            ColorVideo::new(build_video_source(), color().clone()),
            transform().clone(),
        ))),
        buf: Mutex::new(Vec::new()),
        last_dim: Mutex::new((0, 0)),
        last_tick: Mutex::new(Instant::now()),
        sync: Mutex::new(AvSync::default()),
    })
}

struct MediaApp;

impl App for MediaApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "media · player"
    }

    /// Título dinámico de la ventana: el medio en reproducción aparece en la
    /// barra del SO (antes era un cartelón pintado encima del video). Cae al
    /// nombre genérico cuando no hay medio cargado.
    fn window_title(_model: &Self::Model) -> Option<String> {
        let t = media_title_string();
        let t = t.trim();
        Some(if t.is_empty() {
            "media · player".to_string()
        } else {
            format!("media — {t}")
        })
    }

    /// Contenido de las ventanas OS secundarias (multiventana): config y cola.
    fn secondary_view(model: &Self::Model, key: u64) -> Option<View<Self::Msg>> {
        match key {
            CONFIG_WIN if model.settings_open => Some(settings_content(model)),
            PLAYLIST_WIN if model.playlist_open => Some(playlist_content()),
            _ => None,
        }
    }

    fn secondary_title(_model: &Self::Model, key: u64) -> Option<String> {
        match key {
            CONFIG_WIN => Some("Configuración — media".to_string()),
            PLAYLIST_WIN => Some("Lista de reproducción — media".to_string()),
            _ => None,
        }
    }

    /// El usuario cerró una ventana secundaria con el botón del SO → sincroniza
    /// el modelo (sin volver a pedir cerrarla, que ya no existe).
    fn on_secondary_close(_model: &Self::Model, key: u64) -> Option<Self::Msg> {
        match key {
            CONFIG_WIN => Some(Msg::SettingsClosed),
            PLAYLIST_WIN => Some(Msg::PlaylistClosed),
            _ => None,
        }
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        handle.spawn_periodic(Duration::from_millis(TICK_MS), || Msg::Tick);
        spawn_controles_watcher(handle);
        // Escaneo de la onda de pista completa (tipo Audacity) en un hilo de
        // fondo: ffmpeg decodea todo el archivo a picos. Al terminar, el
        // resultado queda en `waveform_slot` y `Msg::WaveformReady` dispara un
        // repintado. Sólo para archivos locales (un stream de red no se escanea).
        if let Some(path) = current_media_path() {
            handle.spawn(move || {
                match foreign_av::decode_peaks(&path, 1600) {
                    Ok(w) => *waveform_slot().lock() = Some(w),
                    Err(e) => eprintln!("media-app: escaneo de onda: {e}"),
                }
                Msg::WaveformReady
            });
        }
        let (palette_commands, palette_cmds) = build_command_catalog(&settings());
        // La config y todo el arranque que toca el Playlist ya se aplicó en
        // `main` ANTES de abrir cpal (ver apply_startup_config — evita el
        // deadlock). Acá sólo clonamos la config para el Model; NUNCA
        // lockear el Playlist en init.
        let config = media_config_slot().get().cloned().unwrap_or_default();
        Model {
            frames: 0,
            started_at: Instant::now(),
            tile_order: load_layout(),
            help_open: false,
            palette: None,
            palette_commands,
            palette_cmds,
            viewport: (960.0, 540.0),
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            context_menu: None,
            config,
            settings_open: false,
            settings_tab: SettingsTab::Audio,
            bar_target: 0,
            settings_scroll: 0.0,
            visualizers_open: false,
            playlist_open: false,
        }
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => {
                // Registra el avance en el historial (resume / U2).
                record_playback_progress(model.frames);
                Model {
                    frames: model.frames.wrapping_add(1),
                    ..model
                }
            }
            Msg::SwapTile { from, to } => {
                let mut m = model;
                if from != to && from < m.tile_order.len() && to < m.tile_order.len() {
                    m.tile_order.swap(from, to);
                    // El nuevo orden se persiste en el acto: la próxima
                    // sesión arranca con el layout que dejó el usuario.
                    save_layout(&m.tile_order);
                }
                m
            }
            Msg::Command(cmd) => {
                apply_command(cmd);
                model
            }
            Msg::ToggleHelp => {
                let mut m = model;
                m.help_open = !m.help_open;
                m
            }
            Msg::ToggleSettings => {
                let mut m = model;
                m.settings_open = !m.settings_open;
                m.settings_scroll = 0.0;
                // La config es una ventana OS aparte (secundaria): abrir/cerrar
                // la ventana real, no un overlay.
                if m.settings_open {
                    handle.open_window(CONFIG_WIN, "Configuración — media", 760, 600);
                } else {
                    handle.close_window(CONFIG_WIN);
                }
                m
            }
            Msg::SettingsClosed => {
                let mut m = model;
                m.settings_open = false;
                m
            }
            Msg::TogglePlaylist => {
                let mut m = model;
                m.playlist_open = !m.playlist_open;
                if m.playlist_open {
                    handle.open_window(PLAYLIST_WIN, "Lista de reproducción — media", 420, 560);
                } else {
                    handle.close_window(PLAYLIST_WIN);
                }
                m
            }
            Msg::PlaylistClosed => {
                let mut m = model;
                m.playlist_open = false;
                m
            }
            Msg::JumpTrack(i) => {
                jump_playlist_to(i);
                model
            }
            Msg::WaveformReady => model, // sólo dispara el repintado
            Msg::ConfigEdit(edit) => {
                let mut m = model;
                apply_config_edit(&mut m.config, edit);
                m.config = std::mem::take(&mut m.config).sanitized();
                apply_media_config(&m.config);
                save_media_config(&m.config);
                m
            }
            Msg::SettingsTab(tab) => {
                let mut m = model;
                if m.settings_tab != tab {
                    m.settings_scroll = 0.0; // empieza arriba en cada pestaña
                }
                m.settings_tab = tab;
                m
            }
            Msg::SettingsScroll(dy) => {
                let mut m = model;
                // Rueda hacia abajo baja el contenido (sube el offset). Clamp
                // generoso; el sobre-scroll sólo muestra espacio en blanco.
                m.settings_scroll = (m.settings_scroll - dy * 28.0).clamp(0.0, 900.0);
                m
            }
            Msg::BarEdit(edit) => {
                let mut m = model;
                apply_bar_edit(&mut m, edit);
                m.config.toolbar = std::mem::take(&mut m.config.toolbar).sanitized();
                // Mantén el target dentro de rango tras agregar/quitar barras.
                m.bar_target = m.bar_target.min(m.config.toolbar.bars.len().saturating_sub(1));
                save_media_config(&m.config);
                m
            }
            Msg::ReloadConfig => {
                reload_settings();
                // Los pasos y atajos pueden haber cambiado: reconstruimos
                // el catálogo del palette para que refleje el keymap nuevo.
                let (palette_commands, palette_cmds) = build_command_catalog(&settings());
                Model {
                    palette_commands,
                    palette_cmds,
                    ..model
                }
            }
            Msg::Palette(pm) => apply_palette(model, pm, handle),
            Msg::MenuOpen(which) => {
                let mut m = model;
                m.menu_open = which;
                m.menu_active = usize::MAX;
                // Abrir un menú raíz cierra cualquier contextual.
                m.context_menu = None;
                if which.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
                m
            }
            Msg::MenuNav(dir) => {
                let mut m = model;
                if let Some(mi) = m.menu_open {
                    let menu = app_menu();
                    m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
                }
                m
            }
            Msg::MenuActivate => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu();
                    if let Some(cmd) = menubar_command_at(&menu, mi, model.menu_active) {
                        let mut m = model;
                        m.menu_open = None;
                        m.context_menu = None;
                        return handle_menu_command(m, &cmd, handle);
                    }
                }
                model
            }
            Msg::MenuTick => model,
            Msg::CloseMenus => {
                let mut m = model;
                m.menu_open = None;
                m.menu_active = usize::MAX;
                m.context_menu = None;
                m
            }
            Msg::MenuCommand(cmd) => {
                let mut m = model;
                m.menu_open = None;
                m.context_menu = None;
                handle_menu_command(m, &cmd, handle)
            }
            Msg::ContextMenuOpen(x, y) => {
                let mut m = model;
                m.menu_open = None;
                m.context_menu = Some((x, y));
                m
            }
        }
    }

    /// Atajos globales: `?` alterna la ayuda, `Esc` la cierra; el resto
    /// traduce la tecla a un [`KeyChord`] y la resuelve contra el keymap
    /// de [`settings`]. media-app no tiene text-input, así que no hace
    /// falta routing de foco.
    fn on_wheel(
        model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _modifiers: Modifiers,
    ) -> Option<Self::Msg> {
        // Con la ventana de config abierta, la rueda scrollea su contenido.
        if model.settings_open {
            return Some(Msg::SettingsScroll(delta.y));
        }
        None
    }

    fn on_key(model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        // Palette abierto: el módulo se lleva TODAS las teclas (filtro,
        // ↓↑, Enter, Esc). Mismo patrón que nada.
        if let Some(state) = model.palette.as_ref() {
            return palette::on_key(state, event).map(Msg::Palette);
        }
        if event.state != KeyState::Pressed {
            return None;
        }
        // Ctrl+Shift+P abre el palette (igual que VS Code).
        if palette::open_shortcut(event) {
            return Some(Msg::Palette(PaletteMsg::Open));
        }
        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
        // cierra. Tiene prioridad sobre todo lo demás.
        if let Some(mi) = model.menu_open {
            let n = app_menu().menus.len().max(1);
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        // Esc cierra cualquier menú abierto antes que nada.
        if matches!(event.key, Key::Named(NamedKey::Escape))
            && (model.menu_open.is_some() || model.context_menu.is_some())
        {
            return Some(Msg::CloseMenus);
        }
        match &event.key {
            Key::Character(c) if c == "?" => return Some(Msg::ToggleHelp),
            Key::Named(NamedKey::Escape) if model.help_open => return Some(Msg::ToggleHelp),
            Key::Named(NamedKey::Escape) if model.settings_open => return Some(Msg::ToggleSettings),
            Key::Named(NamedKey::F2) => return Some(Msg::ToggleSettings),
            Key::Named(NamedKey::F5) => return Some(Msg::ReloadConfig),
            _ => {}
        }
        let chord = chord_from_event(event)?;
        settings().keymap.resolve(&chord).cloned().map(Msg::Command)
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // Prioridad: menú contextual > dropdown del menú principal >
        // palette > ayuda.
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu(model, x, y));
        }
        let menu = app_menu();
        if let Some(v) = menubar_overlay_animated(
            &menubar_spec(&menu, model, &llimphi_theme::Theme::dark()),
            model.menu_active,
            model.menu_anim.value(),
        ) {
            return Some(v);
        }
        // El palette tiene prioridad sobre la ayuda (sólo uno visible).
        if let Some(state) = model.palette.as_ref() {
            return Some(palette_overlay(model, state));
        }
        // La config ya NO es overlay: vive en su ventana OS (secondary_view).
        if !model.help_open {
            return None;
        }
        let theme = llimphi_theme::Theme::dark();
        // Un entry por binding del keymap vivo — la ayuda refleja
        // exactamente lo que el usuario configuró en controles.ron.
        let acciones: Vec<ShortcutEntry> = settings()
            .keymap
            .bindings
            .iter()
            .map(|b| ShortcutEntry::new(b.chord.display(), b.command.describe()))
            .collect();
        Some(shortcuts_help_view(ShortcutsHelpSpec {
            title: "media · atajos".to_string(),
            groups: vec![
                ShortcutGroup::new("Reproducción", acciones),
                ShortcutGroup::new(
                    "Ayuda",
                    vec![
                        ShortcutEntry::new("?", "Mostrar/ocultar esta ayuda"),
                        ShortcutEntry::new("Esc", "Cerrar la ayuda"),
                        ShortcutEntry::new("F5", "Recargar controles.ron en caliente"),
                        ShortcutEntry::new("Ctrl+Shift+P", "Paleta de comandos (buscar acción)"),
                    ],
                ),
            ],
            viewport: model.viewport,
            on_dismiss: Msg::ToggleHelp,
            palette: ShortcutsHelpPalette::from_theme(&theme),
        }))
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let secs = model.started_at.elapsed().as_secs_f32().max(0.001);
        let fps = model.frames as f32 / secs;

        // Barra de menú principal — primer hijo del column raíz.
        let theme = llimphi_theme::Theme::dark();
        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));

        // --- Hero: canvas de video. El título del medio ya NO se pinta encima
        // (iba como un cartelón de 36px): ahora vive en la barra de título de
        // la ventana del SO vía `MediaApp::window_title`. Queda más limpio y el
        // item `Title` de la barra de controles sigue mostrándolo si se quiere.
        let canvas = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .fill(Color::from_rgba8(10, 12, 18, 255))
        .radius(10.0)
        .gpu_paint_with(move |device, queue, encoder, view, rect, viewport| {
            let pipe = pipeline_for(device, queue);
            let now = Instant::now();
            let wall_dt = {
                let mut last = pipe.last_tick.lock();
                let d = now - *last;
                *last = now;
                d
            };

            // M1 — sync A/V con el audio como reloj maestro, SIN acoplar el
            // ritmo de decode al audio. El video avanza por el reloj de pared
            // (`wall_dt`): el propio source respeta el fps del archivo vía su
            // acumulador, así que esto NO es el timer fijo de 30 fps de antes.
            // Crucial: tickear por wall_dt mantiene drenado el pipe de ffmpeg
            // (el video alimenta al audio); regular el decode con el reloj de
            // audio deadlockeaba el pipe al arranque. El reloj de audio se usa
            // sólo para DESCARTAR frames atrasados (drop), abajo.
            let dt = wall_dt;
            let audio_pos = current_audio_position();

            // Gate de pausa del video (antes lo hacía `PausableVideo`): si está
            // en pausa no avanzamos el frame… salvo que haya un seek pendiente
            // (`SEEK_FORCE`), en cuyo caso tickeamos igual para mostrar de
            // inmediato el destino del salto y seguir pausados.
            let force = SEEK_FORCE.load(Ordering::Relaxed);
            let do_tick = !pause().is_paused() || force;

            let mut buf = pipe.buf.lock();
            let mut src = pipe.source.lock();
            if do_tick {
                if let Some((w, h)) = src.tick(dt, &mut buf) {
                    let frame_pts = src.pts();
                    drop(src);
                    // Forzado por seek → presentar sí o sí (sin pasar por el
                    // drop de A/V). Si no: con reloj de audio + PTS, la política
                    // descarta el frame atrasado. Sin PTS o sin audio, siempre.
                    let present = force
                        || match (audio_pos, frame_pts) {
                            (Some(audio), Some(pts)) => {
                                !matches!(pipe.sync.lock().plan(audio, pts), FramePlan::Drop)
                            }
                            _ => true,
                        };
                    if present {
                        pipe.surface.upload(&buf, w, h);
                        *pipe.last_dim.lock() = (w, h);
                        // Recién ahora apagamos el pedido de seek: si el frame
                        // nuevo tardó en llegar (respawn de ffmpeg), seguimos
                        // forzando en los próximos paints hasta presentarlo.
                        if force {
                            SEEK_FORCE.store(false, Ordering::Relaxed);
                        }
                    }
                } else {
                    drop(src);
                }
            } else {
                drop(src);
            }
            drop(buf);
            pipe.surface.blit(queue, encoder, view, rect, viewport);
        });

        let subs_strip = subtitle_strip();
        // Barras de controles configurables (estilo VLC/eww). Cada barra se
        // ancla arriba o abajo del video según su `position`; acá las
        // separamos en dos grupos para colocarlas a ambos lados del canvas.
        let above_bars = toolbar_view_at(model, BarPosition::Above);
        let below_bars = toolbar_view_at(model, BarPosition::Below);

        let time_label = {
            let s = playback_snapshot();
            if s.present {
                let dur = s.duration.unwrap_or(Duration::ZERO);
                let track = if s.len > 1 {
                    format!(" · trk {}/{}", s.idx + 1, s.len)
                } else {
                    String::new()
                };
                format!(" · {} / {}{}", fmt_secs(s.position), fmt_secs(dur), track)
            } else {
                String::new()
            }
        };
        let footer = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            format!("ticks {} · ui ≈ {fps:.1} fps{time_label}", model.frames),
            13.0,
            Color::from_rgba8(150, 165, 185, 255),
        );

        // El contenido (todo menos la menubar) va en un Column interno con
        // el padding. La menubar queda flush en (0,0) del root, así se alinea
        // con el overlay del dropdown (que se dibuja en absoluto 0,0); si la
        // menubar estuviera padded, el overlay aparecería corrido y se vería
        // "duplicada" al abrir un menú. (Mismo criterio que `nada`.)
        let content = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            flex_grow: 1.0,
            gap: Size {
                width: length(0.0_f32),
                height: length(12.0_f32),
            },
            padding: TaffyRect {
                left: length(18.0_f32),
                right: length(18.0_f32),
                top: length(10.0_f32),
                bottom: length(14.0_f32),
            },
            ..Default::default()
        })
        .children({
            // Orden vertical: barras "arriba" → video → subtítulos → barras
            // "abajo" → (visualizadores) → pie. Cada barra elige su lado.
            let mut kids: Vec<View<Msg>> = Vec::new();
            if let Some(v) = above_bars {
                kids.push(v);
            }
            kids.push(canvas);
            kids.push(subs_strip);
            if let Some(v) = below_bars {
                kids.push(v);
            }
            // Visualizadores ocultos por default: sólo van si el usuario los
            // desplegó desde el menú Ver. Así por defecto se ve video + barras.
            // Son el "lienzo" del audio: forma de onda + waterfall + medidores.
            if model.visualizers_open {
                let visualizers = View::new(Style {
                    flex_direction: FlexDirection::Row,
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(200.0_f32),
                    },
                    gap: Size {
                        width: length(10.0_f32),
                        height: length(0.0_f32),
                    },
                    ..Default::default()
                })
                .children(vec![fulltrack_waveform_view(), waterfall_panel(), meters_panel()]);
                kids.push(visualizers);
            }
            kids.push(footer);
            kids
        });

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(22, 26, 34, 255))
        // Right-click en la raíz (origen 0,0 ⇒ local == ventana) abre el
        // menú contextual del reproductor.
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, content])
    }
}

/// Overlay del command palette: scrim a pantalla completa con la caja del
/// módulo centrada cerca del top. El scrim cierra al click; la caja
/// intercepta el click (con un `Open` inerte — el palette ya está
/// abierto) para no cerrarse al tipear en el input.
fn palette_overlay(model: &Model, state: &PaletteState) -> View<Msg> {
    let theme = llimphi_theme::Theme::dark();
    let pal = PalettePalette::from_theme(&theme);
    let inner = palette::view(state, &model.palette_commands, &pal, Msg::Palette);

    let (vw, vh) = model.viewport;
    let box_w = 560.0_f32.min(vw - 32.0);
    let x = ((vw - box_w) * 0.5).max(0.0);
    let y = (vh * 0.16).max(0.0);

    let panel = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(box_w),
            height: length(286.0_f32),
        },
        ..Default::default()
    })
    .on_click(Msg::Palette(PaletteMsg::Open))
    .children(vec![inner]);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 150))
    .on_click(Msg::Palette(PaletteMsg::Close))
    .children(vec![panel])
}

// ============================================================
// Ventana de configuración (F2)
// ============================================================

/// Chip de toggle booleano: verde "sí" / gris "no".
fn cfg_toggle(on: bool, edit: ConfigEdit) -> View<Msg> {
    let (label, bg) = if on {
        ("sí", Color::from_rgba8(56, 120, 84, 255))
    } else {
        ("no", Color::from_rgba8(74, 60, 70, 255))
    };
    chip_button(label, bg, Color::from_rgba8(235, 240, 248, 255), Msg::ConfigEdit(edit))
}

/// Chip de acción genérico de la ventana de config.
fn cfg_chip(label: &str, edit: ConfigEdit) -> View<Msg> {
    chip_button(
        label,
        Color::from_rgba8(55, 65, 80, 255),
        Color::from_rgba8(220, 230, 245, 255),
        Msg::ConfigEdit(edit),
    )
}

/// Una fila de ajuste: etiqueta · valor · controles.
fn settings_row(label: &str, value: &str, controls: Vec<View<Msg>>) -> View<Msg> {
    let lab = View::new(Style {
        size: Size {
            width: length(148.0_f32),
            height: length(38.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        ..Default::default()
    })
    .text(label.to_string(), 13.5, Color::from_rgba8(178, 193, 214, 255));
    let val = View::new(Style {
        size: Size {
            width: length(60.0_f32),
            height: length(38.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(value.to_string(), 13.5, Color::from_rgba8(232, 238, 248, 255));
    let mut kids = vec![lab, val];
    kids.extend(controls);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(42.0_f32),
        },
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(kids)
}

/// Cabecera de sección dentro de la ventana de config.
fn settings_header(title: &str) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(26.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(title.to_string(), 14.5, Color::from_rgba8(118, 182, 232, 255))
}

/// Columna de contenido (una pestaña) — ancho completo del panel.
/// Caja de contenido con scroll: recorta a `visible_h` y desplaza el
/// contenido `scroll` px hacia arriba (margen superior negativo). El clip
/// oculta lo que sobresale; la rueda mueve `scroll` (ver `on_wheel`).
fn scroll_box(children: Vec<View<Msg>>, visible_h: f32, scroll: f32) -> View<Msg> {
    let inner = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        margin: TaffyRect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(-scroll),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(children);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(visible_h),
        },
        ..Default::default()
    })
    .clip(true)
    .children(vec![inner])
}

/// Fila que envuelve sus hijos a varias líneas (paleta de items, selector).
fn wrap_row(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(8.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

/// Chip ancho (cabe una etiqueta larga). Para la paleta y el editor de barras.
fn wide_chip(label: &str, bg: Color, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(116.0_f32),
            height: length(30.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(Color::from_rgba8(80, 100, 130, 255))
    .radius(7.0)
    .text(label.to_string(), 12.5, Color::from_rgba8(225, 232, 245, 255))
    .on_click(msg)
}

/// Contenido de la pestaña Audio.
fn tab_audio(c: &MediaConfig) -> Vec<View<Msg>> {
    vec![
        settings_header("Audio"),
        settings_row(
            "Volumen",
            &format!("{:.0}%", (c.audio.volume * 100.0).round()),
            vec![
                cfg_chip("−", ConfigEdit::VolumeDelta(-0.05)),
                cfg_chip("+", ConfigEdit::VolumeDelta(0.05)),
            ],
        ),
        settings_row("Ecualizador", "", vec![cfg_toggle(c.audio.eq_enabled, ConfigEdit::ToggleEq)]),
        settings_row(
            "Normalización",
            "",
            vec![cfg_toggle(c.audio.normalization_enabled, ConfigEdit::ToggleNormalization)],
        ),
        settings_row(
            "Objetivo LUFS",
            &format!("{:.0}", c.audio.normalization_target_lufs),
            vec![
                cfg_chip("−", ConfigEdit::NormTargetDelta(-1.0)),
                cfg_chip("+", ConfigEdit::NormTargetDelta(1.0)),
            ],
        ),
        settings_row(
            "Downmix estéreo",
            "",
            vec![cfg_toggle(c.audio.downmix_to_stereo, ConfigEdit::ToggleDownmix)],
        ),
    ]
}

/// Una columna (mitad de ancho) que apila filas/cabeceras.
fn half_column(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(0.5_f32),
            height: length(352.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

/// Pone dos columnas lado a lado.
fn two_columns(left: Vec<View<Msg>>, right: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(356.0_f32),
        },
        gap: Size {
            width: length(18.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![half_column(left), half_column(right)])
}

/// Contenido de la pestaña Video — dos columnas: color | orientación.
fn tab_video(c: &MediaConfig) -> Vec<View<Msg>> {
    let v = &c.video;
    let color = vec![
        settings_header("Color"),
        settings_row("Activar", "", vec![cfg_toggle(v.color_enabled, ConfigEdit::ToggleColor)]),
        settings_row(
            "Brillo",
            &format!("{:+.2}", v.brightness),
            vec![cfg_chip("−", ConfigEdit::BrightnessDelta(-0.05)), cfg_chip("+", ConfigEdit::BrightnessDelta(0.05))],
        ),
        settings_row(
            "Contraste",
            &format!("{:.2}", v.contrast),
            vec![cfg_chip("−", ConfigEdit::ContrastDelta(-0.05)), cfg_chip("+", ConfigEdit::ContrastDelta(0.05))],
        ),
        settings_row(
            "Gamma",
            &format!("{:.2}", v.gamma),
            vec![cfg_chip("−", ConfigEdit::GammaDelta(-0.05)), cfg_chip("+", ConfigEdit::GammaDelta(0.05))],
        ),
        settings_row(
            "Saturación",
            &format!("{:.2}", v.saturation),
            vec![cfg_chip("−", ConfigEdit::SaturationDelta(-0.05)), cfg_chip("+", ConfigEdit::SaturationDelta(0.05))],
        ),
        settings_row(
            "Matiz",
            &format!("{:.0}°", v.hue),
            vec![cfg_chip("−", ConfigEdit::HueDelta(-10.0)), cfg_chip("+", ConfigEdit::HueDelta(10.0))],
        ),
        settings_row("", "", vec![cfg_chip("reset", ConfigEdit::ColorReset)]),
    ];
    let orient = vec![
        settings_header("Orientación"),
        settings_row(
            "Rotación",
            &format!("{}°", v.rotation),
            vec![cfg_chip("rotar 90°", ConfigEdit::RotateCw)],
        ),
        settings_row("Espejo H", "", vec![cfg_toggle(v.flip_h, ConfigEdit::FlipH)]),
        settings_row("Espejo V", "", vec![cfg_toggle(v.flip_v, ConfigEdit::FlipV)]),
    ];
    vec![two_columns(color, orient)]
}

/// Contenido de la pestaña Reproducción (playlist + subtítulos + comportamiento).
fn tab_playback(c: &MediaConfig) -> Vec<View<Msg>> {
    vec![
        settings_header("Playlist"),
        settings_row(
            "Reanudar al abrir",
            "",
            vec![cfg_toggle(c.playlist.resume_on_open, ConfigEdit::ToggleResumeOnOpen)],
        ),
        settings_row(
            "Repetición",
            c.playlist.repeat.slug(),
            vec![cfg_chip("ciclar", ConfigEdit::CycleRepeatDefault)],
        ),
        settings_row("Aleatorio", "", vec![cfg_toggle(c.playlist.shuffle, ConfigEdit::ToggleShuffleDefault)]),
        settings_header("Subtítulos"),
        settings_row(
            "Auto-cargar sidecar",
            "",
            vec![cfg_toggle(c.subtitles.autoload_sidecar, ConfigEdit::ToggleAutoloadSidecar)],
        ),
        settings_row(
            "Desfase (ms)",
            &format!("{}", c.subtitles.delay_ms),
            vec![
                cfg_chip("−", ConfigEdit::SubDelayDelta(-100)),
                cfg_chip("+", ConfigEdit::SubDelayDelta(100)),
            ],
        ),
        settings_row(
            "Tamaño de letra",
            &format!("{:.1}×", c.subtitles.font_scale),
            vec![
                cfg_chip("−", ConfigEdit::SubFontDelta(-0.1)),
                cfg_chip("+", ConfigEdit::SubFontDelta(0.1)),
            ],
        ),
        settings_header("Comportamiento"),
        settings_row(
            "Crossfade (s)",
            &format!("{:.1}", c.behavior.crossfade_secs),
            vec![
                cfg_chip("−", ConfigEdit::CrossfadeDelta(-0.5)),
                cfg_chip("+", ConfigEdit::CrossfadeDelta(0.5)),
            ],
        ),
    ]
}

/// Contenido de la pestaña Controles (keymap, sólo lectura por ahora).
fn tab_controls() -> Vec<View<Msg>> {
    let s = settings();
    // Las teclas atadas como chips compactos (sólo informativo).
    let keys: Vec<View<Msg>> = s
        .keymap
        .bindings
        .iter()
        .map(|b| {
            View::new(Style {
                size: Size {
                    width: length(112.0_f32),
                    height: length(28.0_f32),
                },
                justify_content: Some(JustifyContent::Center),
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(Color::from_rgba8(40, 46, 58, 255))
            .radius(6.0)
            .text(
                format!("{} · {}", b.chord.display(), short_action(&b.command)),
                11.5,
                Color::from_rgba8(200, 212, 228, 255),
            )
        })
        .collect();
    vec![
        settings_header("Controles (teclado)"),
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(40.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            "Editá controles.ron y apretá F5 para reasignar teclas. El editor visual de atajos llega después.".to_string(),
            12.5,
            Color::from_rgba8(150, 165, 185, 255),
        ),
        wrap_row(keys),
    ]
}

/// Etiqueta corta de un comando para el chip de controles.
fn short_action(cmd: &MediaCommand) -> &'static str {
    use MediaCommand::*;
    match cmd {
        TogglePause => "play",
        SeekBy { .. } => "seek",
        SeekTo { .. } => "ir a",
        VolumeBy { .. } | SetVolume { .. } => "vol",
        ToggleMute => "mute",
        NextTrack => "sig",
        PrevTrack => "ant",
        ChapterNext | ChapterPrev => "cap",
        SpeedStep { .. } | SetSpeed { .. } => "vel",
        CycleRepeat => "rep",
        ToggleShuffle => "shuf",
        Snapshot => "snap",
        ToggleRecord => "rec",
        Script { .. } => "script",
        EqToggle | EqBandBy { .. } | EqReset => "eq",
        AvSyncBy { .. } | AvSyncReset => "sync",
        ColorToggle | ColorBy { .. } | ColorReset => "color",
        _ => "acción",
    }
}

/// Editor de barras (pestaña "Barras"): cada barra con sus items
/// (clic = quitar) + reorden, un selector de barra destino y la paleta de
/// items disponibles que se agregan a esa barra. Estilo VLC/eww.
/// Ícono del set canónico para un [`BarItem`] de acción (los widgets
/// especiales —timeline/reloj/etiqueta/título/separador— no tienen).
fn bar_item_icon(item: BarItem) -> Option<Icon> {
    Some(match item {
        BarItem::PlayPause => Icon::Play,
        BarItem::Stop => Icon::Stop,
        BarItem::Prev => Icon::SkipBack,
        BarItem::Next => Icon::SkipForward,
        BarItem::SeekBack => Icon::Rewind,
        BarItem::SeekForward => Icon::FastForward,
        BarItem::VolumeDown => Icon::Minus,
        BarItem::VolumeUp => Icon::Plus,
        BarItem::Mute => Icon::VolumeMute,
        BarItem::Repeat => Icon::Repeat,
        BarItem::Shuffle => Icon::Shuffle,
        BarItem::SpeedDown => Icon::ChevronDown,
        BarItem::SpeedUp => Icon::ChevronUp,
        BarItem::SpeedReset => Icon::Gauge,
        BarItem::Snapshot => Icon::Camera,
        BarItem::Record => Icon::Record,
        BarItem::Equalizer => Icon::Equalizer,
        BarItem::Settings => Icon::Settings,
        _ => return None,
    })
}

/// Chip del editor de barras: ícono (si lo hay) + etiqueta. Mismo lenguaje
/// visual que los botones reales de abajo del video. `msg` se dispara al
/// click (quitar en las barras, agregar en la paleta).
fn editor_item_chip(item: BarItem, bg: Color, msg: Msg) -> View<Msg> {
    let fg = Color::from_rgba8(225, 232, 245, 255);
    let mut kids: Vec<View<Msg>> = Vec::new();
    if let Some(ic) = bar_item_icon(item) {
        kids.push(
            View::new(Style {
                size: Size {
                    width: length(20.0_f32),
                    height: length(22.0_f32),
                },
                ..Default::default()
            })
            .children(vec![icon_view::<Msg>(ic, fg, 1.8)]),
        );
    }
    kids.push(
        View::new(Style {
            size: Size {
                width: length(84.0_f32),
                height: length(22.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(item.label().to_string(), 11.5, fg),
    );
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: length(124.0_f32),
            height: length(30.0_f32),
        },
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(4.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(Color::from_rgba8(80, 100, 130, 255))
    .radius(7.0)
    .on_click(msg)
    .children(kids)
}

fn tab_bars(model: &Model) -> Vec<View<Msg>> {
    let tb = &model.config.toolbar;
    let mut out: Vec<View<Msg>> = vec![settings_header("Barras de controles — clic en un item lo quita")];

    for (bi, bar) in tb.bars.iter().enumerate() {
        let head = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            gap: Size {
                width: length(8.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![
            bar_label(format!("Barra {}", bi + 1), 70.0, Color::from_rgba8(118, 182, 232, 255)),
            // Arriba/abajo del video.
            wide_chip(
                bar.position.label(),
                Color::from_rgba8(48, 66, 80, 255),
                Msg::BarEdit(BarEdit::TogglePosition(bi)),
            ),
            wide_chip("− quitar barra", Color::from_rgba8(74, 58, 64, 255), Msg::BarEdit(BarEdit::RemoveBar(bi))),
        ]);
        // Items: clic quita; ‹ › reordenan. Con su ícono real.
        let chips: Vec<View<Msg>> = bar
            .items
            .iter()
            .enumerate()
            .flat_map(|(pi, &it)| {
                vec![
                    editor_item_chip(
                        it,
                        Color::from_rgba8(52, 60, 74, 255),
                        Msg::BarEdit(BarEdit::RemoveItem(bi, pi)),
                    ),
                    small_chip("‹", Msg::BarEdit(BarEdit::Nudge(bi, pi, -1))),
                    small_chip("›", Msg::BarEdit(BarEdit::Nudge(bi, pi, 1))),
                ]
            })
            .collect();
        out.push(head);
        out.push(wrap_row(chips));
    }

    // Selector de barra destino + agregar barra.
    let mut targets: Vec<View<Msg>> = (0..tb.bars.len())
        .map(|i| {
            let bg = if i == model.bar_target {
                Color::from_rgba8(60, 110, 150, 255)
            } else {
                Color::from_rgba8(48, 54, 66, 255)
            };
            wide_chip(&format!("→ Barra {}", i + 1), bg, Msg::BarEdit(BarEdit::SetTarget(i)))
        })
        .collect();
    targets.push(wide_chip("+ barra nueva", Color::from_rgba8(48, 70, 58, 255), Msg::BarEdit(BarEdit::AddBar)));

    // Paleta de items disponibles → agregan a la barra destino, con ícono.
    let palette: Vec<View<Msg>> = BarItem::ALL
        .iter()
        .map(|&it| {
            editor_item_chip(
                it,
                Color::from_rgba8(46, 54, 68, 255),
                Msg::BarEdit(BarEdit::AddItem(model.bar_target, it)),
            )
        })
        .collect();

    out.push(settings_header("Agregar items a:"));
    out.push(wrap_row(targets));
    out.push(wrap_row(palette));
    out
}

/// Chip pequeño cuadrado (reorden ‹ ›).
fn small_chip(label: &str, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(26.0_f32),
            height: length(30.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(40, 46, 58, 255))
    .hover_fill(Color::from_rgba8(80, 100, 130, 255))
    .radius(6.0)
    .text(label.to_string(), 14.0, Color::from_rgba8(220, 230, 245, 255))
    .on_click(msg)
}

/// La ventana de configuración con pestañas. Edita [`MediaConfig`] en vivo
/// (cada cambio aplica a los handles y persiste a `config.ron`). `F2`/`Esc`
/// la cierran; click fuera también.
/// Contenido de la **ventana OS** de configuración (multiventana). Llena la
/// ventana: tabs de Llimphi (`llimphi-widget-tabs`, reemplazan los viejos
/// chip-buttons) arriba + el contenido de la pestaña activa scrolleable +
/// una línea de ayuda al pie. Sin scrim ni "cerrar" propio: el chrome del SO
/// (barra de título + botón cerrar) los aporta la ventana.
fn settings_content(model: &Model) -> View<Msg> {
    let c = &model.config;

    // Contenido de la pestaña activa (scrolleable).
    let rows = match model.settings_tab {
        SettingsTab::Audio => tab_audio(c),
        SettingsTab::Video => tab_video(c),
        SettingsTab::Playback => tab_playback(c),
        SettingsTab::Bars => tab_bars(model),
        SettingsTab::Controls => tab_controls(),
    };
    let content = scroll_box(rows, 486.0_f32, model.settings_scroll);

    // Tabs de Llimphi: barra + contenido del tab activo, en un solo widget.
    let labels: Vec<String> = SettingsTab::ALL.iter().map(|t| t.label().to_string()).collect();
    let active = SettingsTab::ALL
        .iter()
        .position(|&t| t == model.settings_tab)
        .unwrap_or(0);
    let tabs = tabs_view(TabsSpec {
        labels,
        active,
        on_select: |i: usize| Msg::SettingsTab(SettingsTab::ALL[i]),
        content,
        tab_height: 40.0,
        palette: TabsPalette::from_theme(&llimphi_theme::Theme::dark()),
        tab_width: None,
    });

    let footer = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(
        "Se guarda en config.ron · Esc cierra · en Barras: clic en un item lo quita, ‹ › reordenan"
            .to_string(),
        11.5,
        Color::from_rgba8(140, 152, 170, 255),
    );

    // Column que llena la ventana, delimitado con padding.
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        padding: TaffyRect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(14.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(24, 28, 36, 255))
    .children(vec![tabs, footer])
}

/// Contenido de la ventana OS de lista de reproducción (cola). Lista las
/// pistas (nombres cacheados en `playlist_labels_slot`), resalta la actual
/// (índice del `playback_snapshot`) y cada fila salta a esa pista al clickear
/// (`Msg::JumpTrack`). v1 sin scroll: si la cola es muy larga, agrandar la
/// ventana (la recorta).
fn playlist_content() -> View<Msg> {
    let labels = playlist_labels_slot().get();
    let cur = playback_snapshot().idx;
    let header = settings_header("Lista de reproducción — clic en una pista para saltar");

    let rows: Vec<View<Msg>> = match labels {
        Some(ls) if !ls.is_empty() => ls
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let active = i == cur;
                let bg = if active {
                    Color::from_rgba8(48, 86, 120, 255)
                } else {
                    Color::from_rgba8(30, 36, 46, 255)
                };
                let fg = if active {
                    Color::from_rgba8(236, 243, 250, 255)
                } else {
                    Color::from_rgba8(196, 206, 222, 255)
                };
                View::new(Style {
                    flex_direction: FlexDirection::Row,
                    size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
                    padding: TaffyRect {
                        left: length(10.0_f32),
                        right: length(10.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    align_items: Some(AlignItems::Center),
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .fill(bg)
                .hover_fill(Color::from_rgba8(60, 72, 90, 255))
                .radius(6.0)
                .text(format!("{:>2}.  {name}", i + 1), 13.0, fg)
                .on_click(Msg::JumpTrack(i))
            })
            .collect(),
        _ => vec![View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text("Sin lista de reproducción.".to_string(), 13.0, Color::from_rgba8(150, 162, 182, 255))],
    };

    let list = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: auto() },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        ..Default::default()
    })
    .children(rows);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        padding: TaffyRect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(24, 28, 36, 255))
    .clip(true)
    .children(vec![header, list])
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(
    menu: &'a AppMenu,
    model: &Model,
    theme: &'a llimphi_theme::Theme,
) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: model.viewport,
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// El menú principal del reproductor. Archivo / Reproducción / Ver / Ayuda.
/// Sin "Editar": media-app no tiene campos de texto editables. Sólo entran
/// comandos que mapean a acciones reales (transporte, captura, ayuda,
/// recarga de controles). Los atajos espejan el keymap default tipo VLC.
fn app_menu() -> AppMenu {
    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Capturar fotograma", "file.snapshot"))
                .item(MenuItem::new("Grabar / detener", "file.record").separated())
                .item(MenuItem::new("Recargar controles", "file.reload").shortcut("F5"))
                .item(MenuItem::new("Salir", "file.quit").shortcut("Ctrl+Q").separated()),
        )
        .menu(
            Menu::new("Reproducción")
                .item(MenuItem::new("Reproducir / pausar", "play.toggle").shortcut("Space"))
                .item(MenuItem::new("Retroceder", "play.back").shortcut("←"))
                .item(MenuItem::new("Avanzar", "play.fwd").shortcut("→").separated())
                .item(MenuItem::new("Pista anterior", "play.prev"))
                .item(MenuItem::new("Pista siguiente", "play.next").separated())
                .item(MenuItem::new("Subir volumen", "play.vol_up"))
                .item(MenuItem::new("Bajar volumen", "play.vol_dn")),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Configuración", "view.settings").shortcut("F2").separated())
                .item(MenuItem::new("Lista de reproducción", "view.playlist"))
                .item(MenuItem::new("Visualizadores de audio", "view.visualizers"))
                .item(MenuItem::new("Paleta de comandos", "view.palette").shortcut("Ctrl+Shift+P"))
                .item(MenuItem::new("Ayuda de atajos", "view.help").shortcut("?")),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Traduce un command id del menú (principal o contextual) al `Msg`/efecto
/// real. Los ids de transporte/captura despachan `Msg::Command` con un
/// [`MediaCommand`] — exactamente lo que ya disparan botones y teclado.
fn handle_menu_command(mut model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    use MediaCommand::*;
    let step = settings().seek_step_secs;
    let vstep = settings().volume_step;
    let dispatch = |c: MediaCommand| handle.dispatch(Msg::Command(c));
    match cmd {
        "file.snapshot" => dispatch(Snapshot),
        "file.record" => dispatch(ToggleRecord),
        "file.reload" => handle.dispatch(Msg::ReloadConfig),
        "file.quit" => {
            save_history();
            std::process::exit(0)
        }
        "play.toggle" => dispatch(TogglePause),
        "play.back" => dispatch(SeekBy { secs: -step }),
        "play.fwd" => dispatch(SeekBy { secs: step }),
        "play.prev" => dispatch(PrevTrack),
        "play.next" => dispatch(NextTrack),
        "play.vol_up" => dispatch(VolumeBy { delta: vstep }),
        "play.vol_dn" => dispatch(VolumeBy { delta: -vstep }),
        "view.settings" => handle.dispatch(Msg::ToggleSettings),
        "view.playlist" => handle.dispatch(Msg::TogglePlaylist),
        "view.visualizers" => model.visualizers_open = !model.visualizers_open,
        "view.palette" => handle.dispatch(Msg::Palette(PaletteMsg::Open)),
        "view.help" => handle.dispatch(Msg::ToggleHelp),
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => {}
    }
    model
}

/// Menú contextual del reproductor sobre el área de video/controles.
/// Como media-app no tiene campos de texto editables, el contextual NO
/// ofrece edición: mapea a comandos de transporte y captura reales (los
/// mismos que botones, teclado y menú principal).
fn context_menu(model: &Model, x: f32, y: f32) -> View<Msg> {
    let paused = pause().is_paused();
    let recording = recorder().is_recording();
    let items = vec![
        ContextMenuItem::action(if paused { "Reproducir" } else { "Pausar" }),
        ContextMenuItem::action("Capturar fotograma"),
        ContextMenuItem::action(if recording { "Detener grabación" } else { "Grabar audio" }),
        ContextMenuItem::action("Paleta de comandos"),
        ContextMenuItem::action("Ayuda de atajos"),
    ];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(|i: usize| match i {
        0 => Msg::Command(MediaCommand::TogglePause),
        1 => Msg::Command(MediaCommand::Snapshot),
        2 => Msg::Command(MediaCommand::ToggleRecord),
        3 => Msg::Palette(PaletteMsg::Open),
        _ => Msg::ToggleHelp,
    });
    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: model.viewport,
        header: Some("media".to_string()),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&llimphi_theme::Theme::dark()),
    })
}

/// Franja debajo del canvas que muestra el cue de subtítulo activo
/// según la posición del playlist. Si no hay SRT cargado, queda con
/// altura 0 (invisible) para no morder layout.
fn subtitle_strip<Msg: 'static>() -> View<Msg> {
    let Some(track) = subtitles_slot().get().and_then(|o| o.as_ref()) else {
        // Cero altura — no mete espacio en la columna.
        return View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        });
    };
    let position = playback_snapshot().position;
    // S4: delay de subtítulo. Positivo retrasa = al instante `t` mostramos
    // el cue que normalmente caería en `t - delay`. Clamp a >= 0.
    let q = position.as_millis() as i64 - SUB_DELAY_MS.load(Ordering::Relaxed);
    let adjusted = Duration::from_millis(q.max(0) as u64);
    let text = track
        .at(adjusted)
        .map(|c| c.text.clone())
        .unwrap_or_default();
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(44.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(8, 10, 14, 240))
    .radius(6.0)
    .text(text, 18.0, Color::from_rgba8(240, 240, 240, 255))
}

/// Barra de progreso clickeable bajo el video — scrubbing estilo VLC.
/// Delega en el widget reusable `llimphi-widget-timeline`: la app sólo
/// calcula la fracción de avance (posición/duración del player vivo) y
/// mapea el click (fracción del ancho) a `MediaCommand::SeekTo` — el core
/// no sabe la duración, sólo la fracción. Sin playlist (tono A4) queda en
/// cero. Se redibuja cada Tick, así el playhead avanza solo.
fn timeline_strip() -> View<Msg> {
    let frac = {
        let s = playback_snapshot();
        let dur = s.duration.unwrap_or(Duration::ZERO).as_secs_f64();
        if dur <= 0.0 {
            0.0
        } else {
            (s.position.as_secs_f64() / dur).clamp(0.0, 1.0) as f32
        }
    };
    let palette = TimelinePalette::from_theme(&llimphi_theme::Theme::dark());
    timeline_view(frac, &palette, |fraction| {
        Some(Msg::Command(MediaCommand::SeekTo { fraction }))
    })
}

/// Formatea una duración como `M:SS` (o `H:MM:SS` si pasa la hora).
fn fmt_mmss(d: Duration) -> String {
    let t = d.as_secs();
    let (h, m, s) = (t / 3600, (t % 3600) / 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Pinta las **barras de controles** configurables (estilo VLC/eww) desde
/// `model.config.toolbar`. Una barra por fila; cada [`BarItem`] se mapea a
/// un botón (con su `MediaCommand`) o a un widget especial. El usuario
/// compone estas barras desde la pestaña "Barras" de la configuración.
/// Renderiza las barras ancladas en `position` (arriba o abajo del video).
/// Devuelve `None` si no hay ninguna allí — así `view` no reserva espacio.
fn toolbar_view_at(model: &Model, position: BarPosition) -> Option<View<Msg>> {
    let bars: Vec<View<Msg>> = model
        .config
        .toolbar
        .bars
        .iter()
        .filter(|bar| bar.position == position)
        .map(|bar| {
            let items: Vec<View<Msg>> = bar.items.iter().map(|&it| bar_item_view(it)).collect();
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(48.0_f32),
                },
                gap: Size {
                    width: length(10.0_f32),
                    height: length(0.0_f32),
                },
                padding: TaffyRect {
                    left: length(10.0_f32),
                    right: length(10.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(Color::from_rgba8(28, 33, 43, 255))
            .radius(10.0)
            .children(items)
        })
        .collect();
    if bars.is_empty() {
        return None;
    }
    let n = bars.len() as f32;
    Some(
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: length(n * 56.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(8.0_f32),
            },
            ..Default::default()
        })
        .children(bars),
    )
}

/// Texto fijo dentro de una barra (reloj, etiqueta de volumen, título).
fn bar_label(text: String, width: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(width),
            height: length(36.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(text, 13.0, color)
}

/// Botón de barra con ícono del set canónico [`llimphi_icons`] (GUI
/// desacoplada y consistente con el resto de la suite). `active` tinta el
/// fondo/ícono (toggle encendido, reproduciendo, grabando…). `Record` va
/// rojo por convención.
fn icon_button(icon: Icon, active: bool, msg: Msg) -> View<Msg> {
    let bg = if active {
        Color::from_rgba8(46, 84, 110, 255)
    } else {
        Color::from_rgba8(44, 52, 66, 255)
    };
    let col = if matches!(icon, Icon::Record) {
        Color::from_rgba8(232, 86, 86, 255)
    } else if active {
        Color::from_rgba8(150, 215, 245, 255)
    } else {
        Color::from_rgba8(214, 224, 240, 255)
    };
    View::new(Style {
        size: Size {
            width: length(40.0_f32),
            height: length(34.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(Color::from_rgba8(70, 92, 120, 255))
    .radius(8.0)
    .on_click(msg)
    .children(vec![icon_view::<Msg>(icon, col, 2.0)])
}

/// Mapea un [`BarItem`] a su vista concreta. Los botones-ícono reusan
/// `icon_button` + `Msg::Command`; los widgets especiales (timeline, reloj,
/// etiquetas, separador) se arman aparte.
fn bar_item_view(item: BarItem) -> View<Msg> {
    use MediaCommand::*;
    let step = settings().seek_step_secs;
    let vstep = settings().volume_step;
    let snap = playback_snapshot();
    // Botón-ícono que despacha un MediaCommand.
    let icmd = |icon: Icon, active: bool, c: MediaCommand| icon_button(icon, active, Msg::Command(c));
    match item {
        BarItem::PlayPause => {
            let paused = pause().is_paused();
            let icon = if paused { Icon::Play } else { Icon::Pause };
            icon_button(icon, !paused, Msg::Command(TogglePause))
        }
        BarItem::Stop => icmd(Icon::Stop, false, SeekTo { fraction: 0.0 }),
        BarItem::Prev => icmd(Icon::SkipBack, false, PrevTrack),
        BarItem::Next => icmd(Icon::SkipForward, false, NextTrack),
        BarItem::SeekBack => icmd(Icon::Rewind, false, SeekBy { secs: -step }),
        BarItem::SeekForward => icmd(Icon::FastForward, false, SeekBy { secs: step }),
        BarItem::VolumeDown => icmd(Icon::Minus, false, VolumeBy { delta: -vstep }),
        BarItem::VolumeUp => icmd(Icon::Plus, false, VolumeBy { delta: vstep }),
        BarItem::Mute => icmd(Icon::VolumeMute, volume().get() <= 1e-4, ToggleMute),
        BarItem::Repeat => icmd(Icon::Repeat, snap.repeat_label != "rep-", CycleRepeat),
        BarItem::Shuffle => icmd(Icon::Shuffle, snap.shuffle_on, ToggleShuffle),
        BarItem::SpeedDown => icmd(Icon::ChevronDown, false, SpeedStep { dir: -1 }),
        BarItem::SpeedUp => icmd(Icon::ChevronUp, false, SpeedStep { dir: 1 }),
        BarItem::SpeedReset => icmd(Icon::Gauge, (snap.speed - 1.0).abs() < 1e-3, SetSpeed { mult: 1.0 }),
        BarItem::Snapshot => icmd(Icon::Camera, false, Snapshot),
        BarItem::Record => icon_button(Icon::Record, recorder().is_recording(), Msg::Command(ToggleRecord)),
        BarItem::Equalizer => icmd(Icon::Equalizer, eq().is_enabled(), EqToggle),
        BarItem::Settings => icon_button(Icon::Settings, false, Msg::ToggleSettings),
        BarItem::Timeline => View::new(Style {
            size: Size {
                width: auto(),
                height: length(34.0_f32),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![timeline_strip()]),
        BarItem::Spacer => View::new(Style {
            size: Size {
                width: auto(),
                height: length(34.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        }),
        BarItem::Clock => {
            let s = playback_snapshot();
            let txt = match s.duration {
                Some(d) => format!("{} / {}", fmt_mmss(s.position), fmt_mmss(d)),
                None => fmt_mmss(s.position),
            };
            bar_label(txt, 120.0, Color::from_rgba8(180, 195, 215, 255))
        }
        BarItem::VolumeLabel => bar_label(
            format!("vol {:.0}%", (volume().get() * 100.0).round()),
            76.0,
            Color::from_rgba8(180, 195, 215, 255),
        ),
        BarItem::VolumeSlider => {
            // Barra de volumen graduable arrastrando el mouse (0–200%). El
            // widget reporta el delta de valor; lo mandamos como VolumeBy, así
            // el `−`/`+` de los lados y el arrastre comparten el mismo camino.
            let mut pal = SliderPalette::from_theme(&llimphi_theme::Theme::dark());
            pal.label_width = 0.0; // sin bloque de etiqueta
            pal.value_width = 0.0; // sin bloque de valor (lo da VolumeLabel)
            pal.track_width = 120.0;
            pal.row_height = 34.0;
            pal.track_thickness = 8.0;
            View::new(Style {
                size: Size { width: length(128.0_f32), height: length(34.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .children(vec![slider_view::<Msg, _>(
                "",
                volume().get(),
                0.0,
                2.0,
                &pal,
                |phase, delta| match phase {
                    DragPhase::Move => Some(Msg::Command(VolumeBy { delta })),
                    DragPhase::End => None,
                },
            )])
        }
        BarItem::Title => {
            bar_label(media_title_string(), 300.0, Color::from_rgba8(200, 212, 230, 255))
        }
    }
}

/// Título del medio para mostrar: tag `title` (— `artist`) si lo hay, si no la
/// etiqueta del archivo, más el capítulo actual (V7). Una sola fuente para el
/// item `Title` de la barra y para el título dinámico de la ventana del SO
/// ([`MediaApp::window_title`]).
fn media_title_string() -> String {
    let md = media_metadata_slot().get();
    let base = md
        .and_then(|m| m.title.clone())
        .or_else(|| config_slot().get().map(|c| c.label.clone()))
        .unwrap_or_default();
    let mut label = match md.and_then(|m| m.artist.as_deref()) {
        Some(artist) if !artist.is_empty() => format!("{base} — {artist}"),
        _ => base,
    };
    if let Some(ch) = chapters_slot().get() {
        if let Some((_, c)) = ch.at(playback_snapshot().position) {
            if !c.title.is_empty() {
                label = format!("{label}  ·  ▸ {}", c.title);
            }
        }
    }
    label
}

/// Onda de **pista completa** (tipo Audacity): dibuja la envolvente de picos
/// que `foreign_av::decode_peaks` dejó en `waveform_slot`, con un playhead en
/// la posición actual; clic en cualquier punto hace seek absoluto. Mientras el
/// escaneo no terminó (slot vacío), cae al visor de onda en vivo.
fn fulltrack_waveform_view() -> View<Msg> {
    let peaks: Option<Vec<(f32, f32)>> = waveform_slot()
        .lock()
        .as_ref()
        .filter(|w| !w.is_empty())
        .map(|w| w.peaks().to_vec());
    let Some(peaks) = peaks else {
        return waveform_panel::<Msg>(); // todavía escaneando → onda en vivo
    };

    let stroke = Color::from_rgba8(120, 220, 170, 255);
    let center_color = Color::from_rgba8(64, 74, 90, 255);
    let playhead_color = Color::from_rgba8(242, 184, 92, 255);

    View::new(Style {
        size: Size { width: auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(14, 16, 22, 255))
    .radius(8.0)
    // Clic en la onda → seek absoluto a esa fracción de la pista.
    .on_click_at(|lx, _ly, w, _h| {
        let f = (lx / w.max(1.0)).clamp(0.0, 1.0);
        Some(Msg::Command(MediaCommand::SeekTo { fraction: f }))
    })
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 {
            return;
        }
        let pad_x: f32 = 12.0;
        let pad_y: f32 = 8.0;
        let ix = rect.x + pad_x;
        let iy = rect.y + pad_y;
        let iw = (rect.w - 2.0 * pad_x).max(1.0);
        let ih = (rect.h - 2.0 * pad_y).max(1.0);
        let mid = iy + ih * 0.5;
        let amp = ih * 0.5;

        // Línea central.
        let mut center = BezPath::new();
        center.move_to((ix as f64, mid as f64));
        center.line_to(((ix + iw) as f64, mid as f64));
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, center_color, None, &center);

        // Una columna vertical (min→max) por bucket de picos.
        let n = peaks.len().max(1);
        let mut env = BezPath::new();
        for (i, &(vmin, vmax)) in peaks.iter().enumerate() {
            let x = ix + (i as f32 / n as f32) * iw;
            let y_top = mid - vmax.clamp(-1.0, 1.0) * amp;
            let y_bot = mid - vmin.clamp(-1.0, 1.0) * amp;
            env.move_to((x as f64, y_top as f64));
            env.line_to((x as f64, y_bot as f64));
        }
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, stroke, None, &env);

        // Playhead en la posición actual (fracción de la duración).
        let s = playback_snapshot();
        if let Some(dur) = s.duration {
            let d = dur.as_secs_f32();
            if d > 0.0 {
                let f = (s.position.as_secs_f32() / d).clamp(0.0, 1.0);
                let px = ix + f * iw;
                let mut ph = BezPath::new();
                ph.move_to((px as f64, iy as f64));
                ph.line_to((px as f64, (iy + ih) as f64));
                scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, playhead_color, None, &ph);
            }
        }
    })
}

fn waveform_panel<Msg: 'static>() -> View<Msg> {
    let probe = audio_probe_slot().get().cloned().flatten();
    let scratch: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let stroke_color = Color::from_rgba8(120, 220, 170, 255);
    let center_color = Color::from_rgba8(80, 92, 110, 255);
    let off_label = probe.is_none();

    View::new(Style {
        size: Size {
            width: auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(14, 16, 22, 255))
    .radius(8.0)
    .paint_with(move |scene, ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 {
            return;
        }
        let pad_x: f32 = 12.0;
        let pad_y: f32 = 8.0;
        let inner_x = rect.x + pad_x;
        let inner_y = rect.y + pad_y;
        let inner_w = (rect.w - 2.0 * pad_x).max(1.0);
        let inner_h = (rect.h - 2.0 * pad_y).max(1.0);
        let mid_y = inner_y + inner_h * 0.5;

        // Línea central — siempre presente, hace de "ground" del visor.
        let mut center = BezPath::new();
        center.move_to((inner_x as f64, mid_y as f64));
        center.line_to(((inner_x + inner_w) as f64, mid_y as f64));
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            center_color,
            None,
            &center,
        );

        if off_label {
            // Sin probe: leyenda mínima para que se sepa que el visor
            // está vivo aunque no haya señal.
            let _ = ts;
            return;
        }
        let Some(probe) = probe.as_ref() else {
            return;
        };

        let mut snap = scratch.lock();
        let (_sr, channels) = probe.snapshot(&mut snap);
        let channels = channels.max(1) as usize;
        let total_frames = snap.len() / channels;
        if total_frames < 2 {
            return;
        }

        // Envelope min/max por columna: por cada bucket de frames
        // guardamos el mínimo y el máximo del mono fold y dibujamos
        // la forma como un polígono cerrado (relleno tenue + stroke).
        // Da mucho más "cuerpo" que la línea pico-sólo.
        let cols = inner_w.max(2.0) as usize;
        let cols = cols.min(total_frames);
        let frames_per_col = total_frames / cols.max(1);
        if frames_per_col == 0 {
            return;
        }
        let amp = inner_h * 0.5;

        let mut top = BezPath::new();
        let mut bot = BezPath::new();
        let mut envelope = BezPath::new();
        for col in 0..cols {
            let f0 = col * frames_per_col;
            let f1 = ((col + 1) * frames_per_col).min(total_frames);
            let mut vmin = f32::INFINITY;
            let mut vmax = f32::NEG_INFINITY;
            for f in f0..f1 {
                let mut acc = 0.0_f32;
                for ch in 0..channels {
                    acc += snap[f * channels + ch];
                }
                let v = (acc / channels as f32).clamp(-1.0, 1.0);
                if v < vmin {
                    vmin = v;
                }
                if v > vmax {
                    vmax = v;
                }
            }
            let x = inner_x + (col as f32 / (cols as f32 - 1.0).max(1.0)) * inner_w;
            let y_top = mid_y - vmax * amp;
            let y_bot = mid_y - vmin * amp;
            if col == 0 {
                top.move_to((x as f64, y_top as f64));
                bot.move_to((x as f64, y_bot as f64));
                envelope.move_to((x as f64, y_top as f64));
            } else {
                top.line_to((x as f64, y_top as f64));
                bot.line_to((x as f64, y_bot as f64));
                envelope.line_to((x as f64, y_top as f64));
            }
        }
        // Cierre del polígono envelope: vuelve por la línea de
        // mínimos en sentido inverso.
        for col in (0..cols).rev() {
            let f0 = col * frames_per_col;
            let f1 = ((col + 1) * frames_per_col).min(total_frames);
            let mut vmin = f32::INFINITY;
            for f in f0..f1 {
                let mut acc = 0.0_f32;
                for ch in 0..channels {
                    acc += snap[f * channels + ch];
                }
                let v = (acc / channels as f32).clamp(-1.0, 1.0);
                if v < vmin {
                    vmin = v;
                }
            }
            let x = inner_x + (col as f32 / (cols as f32 - 1.0).max(1.0)) * inner_w;
            let y_bot = mid_y - vmin * amp;
            envelope.line_to((x as f64, y_bot as f64));
        }
        envelope.close_path();

        let fill_color = Color::from_rgba8(120, 220, 170, 70);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            fill_color,
            None,
            &envelope,
        );
        scene.stroke(
            &Stroke::new(1.2),
            Affine::IDENTITY,
            stroke_color,
            None,
            &top,
        );
        scene.stroke(
            &Stroke::new(1.2),
            Affine::IDENTITY,
            stroke_color,
            None,
            &bot,
        );
    })
}

/// Botón compacto del row del título: tamaño fijo, hover azulado y
/// click manda `msg`. Centra el texto vertical y horizontalmente.
fn chip_button(label: &str, bg: Color, fg: Color, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(64.0_f32),
            height: length(36.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(Color::from_rgba8(80, 100, 130, 255))
    .radius(8.0)
    .text(label.to_string(), 15.0, fg)
    .on_click(msg)
}

/// Strip de medidores peak + RMS para el row del título. Dos barras
/// horizontales apiladas (peak arriba, RMS abajo) con etiqueta corta
/// a la izquierda. El color de la barra desplaza de verde a rojo
/// pasados los -6 dBFS — pista visual de saturación.
fn meters_panel<Msg: 'static>() -> View<Msg> {
    let probe = audio_probe_slot().get().cloned().flatten();
    let scratch: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let levels: Arc<Mutex<Levels>> = Arc::new(Mutex::new(Levels::new()));
    let track_bg = Color::from_rgba8(34, 40, 52, 255);
    let label_color = Color::from_rgba8(150, 165, 185, 255);
    let off_color = Color::from_rgba8(80, 92, 110, 255);

    View::new(Style {
        size: Size {
            width: length(160.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 {
            return;
        }
        let label_w: f32 = 36.0;
        let bar_h: f32 = 8.0;
        let gap_y: f32 = 6.0;
        let inner_x = rect.x;
        let inner_y = rect.y + (rect.h - (bar_h * 2.0 + gap_y)) * 0.5;
        let bars_x = inner_x + label_w;
        let bars_w = (rect.w - label_w).max(1.0);

        // Etiquetas — texto via Typesetter para mantener consistencia.
        let pk_label = TextBlock::simple(
            "PK",
            11.0,
            label_color,
            (inner_x as f64, (inner_y - 3.0) as f64),
        );
        llimphi_text::draw_block(scene, ts, &pk_label);
        let rms_label = TextBlock::simple(
            "RMS",
            11.0,
            label_color,
            (inner_x as f64, (inner_y + bar_h + gap_y - 3.0) as f64),
        );
        llimphi_text::draw_block(scene, ts, &rms_label);

        // Tracks (fondo).
        let pk_track = KurboRect::new(
            bars_x as f64,
            inner_y as f64,
            (bars_x + bars_w) as f64,
            (inner_y + bar_h) as f64,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, track_bg, None, &pk_track);
        let rms_y = inner_y + bar_h + gap_y;
        let rms_track = KurboRect::new(
            bars_x as f64,
            rms_y as f64,
            (bars_x + bars_w) as f64,
            (rms_y + bar_h) as f64,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, track_bg, None, &rms_track);

        let Some(probe) = probe.as_ref() else {
            // Sin probe: marca tenue al fondo de cada barra para que
            // se sepa que está apagado.
            let pk_off = KurboRect::new(
                bars_x as f64,
                (inner_y + bar_h - 1.0) as f64,
                (bars_x + bars_w) as f64,
                (inner_y + bar_h) as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, off_color, None, &pk_off);
            let rms_off = KurboRect::new(
                bars_x as f64,
                (rms_y + bar_h - 1.0) as f64,
                (bars_x + bars_w) as f64,
                (rms_y + bar_h) as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, off_color, None, &rms_off);
            return;
        };

        let mut snap = scratch.lock();
        let (_sr, channels) = probe.snapshot(&mut snap);
        let mut levels = levels.lock();
        levels.analyze(&snap, channels);
        let pk = levels.peak();
        let rms = levels.rms();

        let pk_w = (pk.clamp(0.0, 1.0) * bars_w).max(0.0);
        let rms_w = (rms.clamp(0.0, 1.0) * bars_w).max(0.0);

        if pk_w > 0.0 {
            let pk_fill = KurboRect::new(
                bars_x as f64,
                inner_y as f64,
                (bars_x + pk_w) as f64,
                (inner_y + bar_h) as f64,
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                level_color(pk),
                None,
                &pk_fill,
            );
        }
        if rms_w > 0.0 {
            let rms_fill = KurboRect::new(
                bars_x as f64,
                rms_y as f64,
                (bars_x + rms_w) as f64,
                (rms_y + bar_h) as f64,
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                level_color(rms),
                None,
                &rms_fill,
            );
        }
    })
}

/// Gradiente verde → ámbar → rojo según el nivel. Cambio a ámbar
/// alrededor de 0.5 (-6 dBFS) y a rojo cerca de full scale.
fn level_color(v: f32) -> Color {
    let v = v.clamp(0.0, 1.0);
    if v < 0.5 {
        Color::from_rgba8(110, 220, 140, 255)
    } else if v < 0.85 {
        Color::from_rgba8(230, 200, 90, 255)
    } else {
        Color::from_rgba8(240, 95, 95, 255)
    }
}

/// Panel de espectro: banco Goertzel sobre el probe + barras log
/// espaciadas (40 Hz → 16 kHz). Sin probe queda con la base oscura y
/// las casillas vacías.
/// Panel waterfall (spectrogram histórico): cada fila es un análisis
/// Goertzel sobre el probe; las filas nuevas entran por arriba y
/// empujan a las viejas hacia abajo. Color va de fondo casi negro a
/// ámbar/blanco según magnitud — la "ráfaga" del bajo y los picos
/// quedan visibles ~2-3 segundos antes de desvanecerse.
fn waterfall_panel<Msg: 'static>() -> View<Msg> {
    let probe = audio_probe_slot().get().cloned().flatten();
    let scratch: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let grid_buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    // 28 bandas para tener resolución y a la vez celdas pintables
    // sin amontonar. ~60 filas a 30 fps = 2 segundos de historia.
    let waterfall: Arc<Mutex<Waterfall>> =
        Arc::new(Mutex::new(Waterfall::new(28, 60, 40.0, 16_000.0)));
    let base_color = Color::from_rgba8(46, 36, 28, 255);

    View::new(Style {
        size: Size {
            width: auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(14, 16, 22, 255))
    .radius(8.0)
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 {
            return;
        }
        let pad: f32 = 6.0;
        let inner_x = rect.x + pad;
        let inner_y = rect.y + pad;
        let inner_w = (rect.w - 2.0 * pad).max(1.0);
        let inner_h = (rect.h - 2.0 * pad).max(1.0);

        let Some(probe) = probe.as_ref() else {
            // Sin probe: línea base apagada — mismo lenguaje que los
            // otros visores.
            let mut center = BezPath::new();
            let mid = inner_y + inner_h * 0.5;
            center.move_to((inner_x as f64, mid as f64));
            center.line_to(((inner_x + inner_w) as f64, mid as f64));
            scene.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                base_color,
                None,
                &center,
            );
            return;
        };

        let mut snap = scratch.lock();
        let (sr, channels) = probe.snapshot(&mut snap);
        if sr == 0 {
            return;
        }
        let mut wf = waterfall.lock();
        wf.analyze(&snap, channels, sr);

        let mut grid = grid_buf.lock();
        let (rows, bands) = wf.snapshot(&mut grid);
        let cell_w = inner_w / bands as f32;
        let cell_h = inner_h / rows as f32;
        for r in 0..rows {
            let y0 = inner_y + r as f32 * cell_h;
            for b in 0..bands {
                let m = grid[r * bands + b];
                if m < 0.02 {
                    continue;
                }
                let x0 = inner_x + b as f32 * cell_w;
                let cell = KurboRect::new(
                    x0 as f64,
                    y0 as f64,
                    (x0 + cell_w + 0.5) as f64,
                    (y0 + cell_h + 0.5) as f64,
                );
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    heat_color(m),
                    None,
                    &cell,
                );
            }
        }
    })
}

/// Gradiente "heat" para el waterfall: tinte oscuro → ámbar → claro
/// según magnitud. Bandas vacías no se pintan (fondo del View queda
/// visible).
fn heat_color(v: f32) -> Color {
    let v = v.clamp(0.0, 1.0);
    if v < 0.25 {
        let t = v / 0.25;
        let r = (60.0 + 110.0 * t) as u8;
        let g = (20.0 + 30.0 * t) as u8;
        let b = (20.0 + 10.0 * t) as u8;
        Color::from_rgba8(r, g, b, 255)
    } else if v < 0.6 {
        let t = (v - 0.25) / 0.35;
        let r = (170.0 + 70.0 * t) as u8;
        let g = (50.0 + 110.0 * t) as u8;
        let b = (30.0 + 40.0 * t) as u8;
        Color::from_rgba8(r, g, b, 255)
    } else {
        let t = (v - 0.6) / 0.4;
        let r = (240.0 + 15.0 * t) as u8;
        let g = (160.0 + 80.0 * t) as u8;
        let b = (70.0 + 160.0 * t) as u8;
        Color::from_rgba8(r, g, b, 255.min((180.0 + 75.0 * t) as u8))
    }
}

/// `true` si `s` parece una **URL de red** (un esquema `algo://` que no sea
/// `file`). ffmpeg/libavformat abre http/https/hls/rtsp/rtmp/udp/srt… de
/// forma transparente, así que basta con derivar la fuente al decoder
/// ffmpeg y pasarle la URL tal cual — la extensión de la URL no es fiable.
/// R1 de `PARIDAD.md`.
fn is_network_url(s: &str) -> bool {
    match s.split_once("://") {
        Some((scheme, rest)) => {
            !scheme.is_empty()
                && !rest.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
                && !scheme.eq_ignore_ascii_case("file")
        }
        None => false,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match args.first() {
        // Stream de red: forzamos el decoder ffmpeg (resuelve el protocolo)
        // y le pasamos la URL tal cual; el audio del mismo stream sale de la
        // misma MediaSession. Si no hay red o ffmpeg, el fallback de abajo
        // cae a testcard + tono sin romper.
        //
        // R2: si la URL es una página de plataforma conocida (YouTube/Vimeo/
        // Twitch…), la resolvemos antes con yt-dlp a su stream directo; si
        // yt-dlp no está o falla, pasamos la URL original a ffmpeg igual
        // (puede ser ya directa, o degradar a testcard).
        Some(arg) if is_network_url(arg) => {
            let stream = if foreign_ytdlp::is_platform_url(arg) {
                // R2 DASH: pedimos el mejor video+audio. Si vienen separados
                // (YouTube > 720p), guardamos la URL de audio para abrir la
                // sesión ffmpeg con dos entradas (`probe_dash`).
                match foreign_ytdlp::resolve_best(arg) {
                    Ok(r) => {
                        if let Some(audio) = &r.audio_url {
                            eprintln!(
                                "media-app: yt-dlp resolvió {arg} → DASH (video+audio separados)"
                            );
                            dash_audio_slot().set(PathBuf::from(audio.clone())).ok();
                        } else {
                            eprintln!("media-app: yt-dlp resolvió {arg} → stream muxeado");
                        }
                        r.stream_url
                    }
                    Err(e) => {
                        eprintln!("media-app: yt-dlp falló ({e}) — pruebo la URL directo");
                        arg.clone()
                    }
                }
            } else {
                arg.clone()
            };
            eprintln!("media-app: stream de red → decoder ffmpeg");
            video_path_slot().set(PathBuf::from(stream)).ok();
            Config {
                label: format!("stream {arg}"),
                kind: VideoKind::Ffmpeg,
            }
        }
        Some(path) => {
            let path = PathBuf::from(path);
            let kind = match path
                .extension()
                .and_then(|s| s.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("gif") => VideoKind::Gif,
                Some("png" | "jpg" | "jpeg" | "webp" | "bmp" | "tiff") => VideoKind::Image,
                Some("ivf") => VideoKind::Av1,
                Some("mp4" | "webm" | "mkv" | "mov" | "avi" | "flv" | "m4v" | "ogv") => {
                    VideoKind::Ffmpeg
                }
                other => {
                    eprintln!(
                        "media-app: extensión {:?} no reconocida — caigo a testcard",
                        other
                    );
                    VideoKind::Testcard
                }
            };
            let label = match kind {
                VideoKind::Gif => format!("gif {}", path.display()),
                VideoKind::Image => format!("img {}", path.display()),
                VideoKind::Ffmpeg => format!("video {}", path.display()),
                VideoKind::Av1 => format!("av1 {}", path.display()),
                VideoKind::Testcard => format!(
                    "testcard {TESTCARD_W}×{TESTCARD_H} @ {TESTCARD_FPS:.0} fps"
                ),
            };
            if !matches!(kind, VideoKind::Testcard) {
                video_path_slot().set(path).ok();
            }
            Config { label, kind }
        }
        None => Config {
            label: format!("testcard {TESTCARD_W}×{TESTCARD_H} @ {TESTCARD_FPS:.0} fps"),
            kind: VideoKind::Testcard,
        },
    };
    config_slot().set(cfg).ok();
    *settings_slot().write().expect("settings lock") = load_settings();

    // Si el video es un archivo decodificado por ffmpeg, abrimos UNA
    // session compartida antes que cualquier otra cosa — el audio del
    // mismo archivo saldrá del MISMO subprocess via FfmpegAudioSource,
    // no spawneamos un segundo ffmpeg sólo para el audio.
    if let (Some(path), Some(VideoKind::Ffmpeg)) =
        (video_path_slot().get(), config_slot().get().map(|c| c.kind))
    {
        // DASH (R2): si hay una URL de audio separada, abrimos la sesión con
        // dos entradas (video + audio); si no, el probe normal de una entrada.
        let probed = match dash_audio_slot().get() {
            Some(audio) => foreign_av::probe_dash(path, audio),
            None => foreign_av::probe(path),
        };
        match probed.and_then(MediaSession::open) {
            Ok(session) => {
                eprintln!(
                    "media-app: ffmpeg session abierta ({})",
                    path.display()
                );
                ffmpeg_session_slot().set(Some(session)).ok();
            }
            Err(e) => {
                eprintln!("media-app: ffmpeg session falló: {e}");
                ffmpeg_session_slot().set(None).ok();
            }
        }
    }

    // Subtítulos: primero las envs explícitas (MEDIA_SRT/VTT/ASS, autodetect
    // por cabecera). Si ninguna apunta a un archivo, S5: auto-carga el
    // "sidecar" del video (mismo nombre base, .srt/.vtt/.ass/.ssa). Falla
    // silenciosa con log — la app sigue sin subs.
    let env_path = std::env::var("MEDIA_SRT")
        .or_else(|_| std::env::var("MEDIA_VTT"))
        .or_else(|_| std::env::var("MEDIA_ASS"))
        .ok();
    let subs = match env_path {
        Some(path) => load_subtitle_file(Path::new(&path)),
        None => auto_load_sidecar_subtitles(),
    };
    subtitles_slot().set(subs).ok();

    // Audio: si MEDIA_MUTE está set, saltamos. Si no, elegimos
    // fuente — MEDIA_WAV=path la activa, sino cae al ToneSource
    // (A4). El AudioSink debe vivir hasta el exit — `cpal::Stream` no
    // es `Sync`, así que no puede ir a un static; lo mantenemos en
    // una local de `main` que sólo se dropea cuando el proceso
    // termina.
    let _audio_sink = if std::env::var("MEDIA_MUTE").is_err() {
        let (source, probe) = audio_source_from_env();
        // Aplica la config persistida (incl. lo que toca el Playlist) ANTES
        // de abrir cpal: después el callback retiene el lock del Playlist y
        // un lock bloqueante colgaría el arranque (ver apply_startup_config).
        apply_startup_config();
        match AudioSink::open(source) {
            Ok(sink) => {
                eprintln!(
                    "media-app: audio cpal abierto @ {} Hz · {} ch",
                    sink.sample_rate(),
                    sink.channels(),
                );
                audio_probe_slot().set(Some(probe)).ok();
                Some(sink)
            }
            Err(e) => {
                eprintln!("media-app: audio off ({e}) — sigo sin sonido");
                audio_probe_slot().set(None).ok();
                None
            }
        }
    } else {
        // Sin cpal no hay contención del Playlist: igual aplicamos config y
        // metadata para el arranque.
        apply_startup_config();
        audio_probe_slot().set(None).ok();
        None
    };

    llimphi_ui::run::<MediaApp>();
}

fn audio_source_from_env() -> (Arc<Mutex<dyn AudioSource + Send>>, AudioProbe) {
    let probe = AudioProbe::new(PROBE_CAPACITY);

    // Prioridad 0: si hay session ffmpeg (modo video file), el audio
    // sale de ahí — mismo proceso que el video.
    if let Some(Some(session)) = ffmpeg_session_slot().get() {
        match FfmpegAudioSource::from_session(session.clone()) {
            Ok(audio) => {
                eprintln!(
                    "media-app: ffmpeg audio @ {} Hz · {} ch",
                    audio.source_sample_rate(),
                    audio.source_channels(),
                );
                let label = video_path_slot()
                    .get()
                    .cloned()
                    .unwrap_or_else(|| PathBuf::from("video"));
                let pl = Playlist::new_single(label, LoadedTrack::FfmpegAudio(audio));
                playlist_labels_slot().set(pl.track_labels()).ok();
                let shared: Arc<Mutex<Playlist>> = Arc::new(Mutex::new(pl));
                playlist_slot().set(Some(shared.clone())).ok();
                let pausable = PausableAudio::new(
                    Box::new(SharedAudio { inner: shared })
                        as Box<dyn AudioSource + Send>,
                    pause().clone(),
                );
                let voled = VolumeAudio::new(pausable, volume().clone());
                let equalized = EqualizerAudio::new(voled, eq().clone());
                // A5 auto: mide la sonoridad (EBU R128) *antes* de la ganancia
                // de makeup, así `NormAuto` calcula cuánto subir para el target.
                let measured = LoudnessProbe::new(equalized, loudness().clone());
                // A5: normalización + limitador tras el EQ (último estadio
                // de ganancia antes del tap del visor).
                let normalized = DynamicsAudio::new(measured, dynamics().clone());
                let recorded = RecordedAudioSource::new(normalized, recorder().clone());
                let probed = ProbedAudioSource::new(recorded, probe.clone());
                return (Arc::new(Mutex::new(probed)), probe);
            }
            Err(e) => {
                eprintln!(
                    "media-app: ffmpeg audio falló ({e}) — sigo sin track audio"
                );
            }
        }
    }

    // Prioridad de fuentes audio cuando no hay ffmpeg session:
    //   MEDIA_PLAYLIST=path (m3u simple, una línea por archivo, # = comentario)
    //   MEDIA_WAV=path
    //   MEDIA_MP3=path
    //   fallback → tono A4 sin playlist
    let tracks: Option<Vec<PathBuf>> =
        if let Ok(playlist_path) = std::env::var("MEDIA_PLAYLIST") {
            match load_playlist_file(&playlist_path) {
                Ok(t) if !t.is_empty() => Some(t),
                Ok(_) => {
                    eprintln!("media-app: playlist {playlist_path} vacía");
                    None
                }
                Err(e) => {
                    eprintln!("media-app: no pude leer playlist {playlist_path}: {e}");
                    None
                }
            }
        } else if let Ok(p) = std::env::var("MEDIA_WAV") {
            Some(vec![PathBuf::from(p)])
        } else if let Ok(p) = std::env::var("MEDIA_MP3") {
            Some(vec![PathBuf::from(p)])
        } else {
            None
        };

    let inner: Box<dyn AudioSource + Send> = if let Some(tracks) = tracks {
        match Playlist::new(tracks) {
            Ok(pl) => {
                eprintln!(
                    "media-app: playlist [1/{}] → {}",
                    pl.len(),
                    pl.track_path().display(),
                );
                playlist_labels_slot().set(pl.track_labels()).ok();
                let shared: Arc<Mutex<Playlist>> = Arc::new(Mutex::new(pl));
                playlist_slot().set(Some(shared.clone())).ok();
                Box::new(SharedAudio { inner: shared })
            }
            Err(e) => {
                eprintln!("media-app: playlist falló ({e}) — caigo a tono A4");
                playlist_slot().set(None).ok();
                Box::new(ToneSource::a4())
            }
        }
    } else {
        playlist_slot().set(None).ok();
        Box::new(ToneSource::a4())
    };

    // Overlay opcional de tono A4 mezclado a `MEDIA_MIX_TONE`
    // (0..1) — útil para probar el mixer con cualquier fuente. Si
    // está set y parsea bien, env la fuente principal en un MixerAudio
    // junto a un ToneSource atenuado por su propio Volume.
    let inner: Box<dyn AudioSource + Send> = match std::env::var("MEDIA_MIX_TONE")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
    {
        Some(g) if g > 0.0 => {
            let g = g.min(1.0);
            eprintln!("media-app: overlay tono A4 a {:.0}%", g * 100.0);
            let tone = VolumeAudio::new(ToneSource::a4(), Volume::new(g));
            let mix = MixerAudio::with_sources(vec![inner, Box::new(tone)]);
            Box::new(mix)
        }
        _ => inner,
    };
    // Orden: Pausable envuelve al productor (silencio cuando pausado);
    // Volume aplica ganancia después de pausar; Recorded captura ese
    // mismo flujo (graba el silencio durante la pausa, igual que lo
    // escucha el sink); Probed tapea afuera para que el visor refleje
    // lo que realmente se reproduce.
    let pausable = PausableAudio::new(inner, pause().clone());
    let voled = VolumeAudio::new(pausable, volume().clone());
    // EQ después del volumen: el probe (visor) y el recorder ven el audio
    // ya ecualizado — el visor refleja lo que realmente suena y la
    // grabación queda con el mismo tono que se escuchó.
    let equalized = EqualizerAudio::new(voled, eq().clone());
    // A5 auto: mide la sonoridad (EBU R128) antes de la ganancia de makeup.
    let measured = LoudnessProbe::new(equalized, loudness().clone());
    // A5: normalización + limitador tras el EQ.
    let normalized = DynamicsAudio::new(measured, dynamics().clone());
    let recorded = RecordedAudioSource::new(normalized, recorder().clone());
    let probed = ProbedAudioSource::new(recorded, probe.clone());
    (Arc::new(Mutex::new(probed)), probe)
}

#[cfg(test)]
mod tests {
    use super::{is_network_url, subtitle_sidecar_candidates};
    use std::path::{Path, PathBuf};

    #[test]
    fn sidecar_usa_el_nombre_base_del_video() {
        let cands = subtitle_sidecar_candidates(Path::new("/cine/peli.mp4"));
        assert_eq!(
            cands,
            vec![
                PathBuf::from("/cine/peli.srt"),
                PathBuf::from("/cine/peli.vtt"),
                PathBuf::from("/cine/peli.ass"),
                PathBuf::from("/cine/peli.ssa"),
            ]
        );
        // Sin extensión previa, la agrega.
        assert_eq!(
            subtitle_sidecar_candidates(Path::new("clip"))[0],
            PathBuf::from("clip.srt")
        );
    }

    #[test]
    fn reconoce_urls_de_red() {
        for u in [
            "http://host/a.mp4",
            "https://host/stream.m3u8",
            "rtsp://cam.local/live",
            "rtmp://srv/app/key",
            "udp://239.0.0.1:1234",
            "srt://host:9000",
        ] {
            assert!(is_network_url(u), "debería ser URL: {u}");
        }
    }

    #[test]
    fn rechaza_paths_locales_y_file() {
        for p in [
            "/ruta/al/clip.mp4",
            "clip.mkv",
            "./rel/foto.png",
            "C:\\videos\\x.avi",
            // file:// es local, no stream de red — lo maneja la rama de path.
            "file:///home/u/v.mp4",
            // sin esquema separado por '://'.
            "espacio raro://no",
        ] {
            assert!(!is_network_url(p), "no debería ser URL de red: {p}");
        }
    }
}
