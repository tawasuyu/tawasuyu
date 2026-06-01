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
//!   `MEDIA_WAV=/ruta/clip.wav cargo run -p media-app --release`
//!   `MEDIA_MP3=/ruta/cancion.mp3 cargo run -p media-app --release`
//!   `MEDIA_MUTE=1 cargo run -p media-app --release`
//!
//! El primer argumento posicional es el video; la extensión decide
//! la fuente (`.gif` → anim, `.png/.jpg/.webp/.bmp/.tiff/.jpeg` →
//! imagen fija, `.mp4/.webm/.mkv/.mov/.avi/.flv/.m4v/.ogv` → video
//! real vía ffmpeg subprocess). Cuando es video file, audio y video
//! salen del MISMO ffmpeg via pipes dup'eados a fd 3/4 — un proceso
//! por archivo, no dos. La pista de audio cuando NO hay video file
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
//! actual del track.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use llimphi_surface::ExternalSurface;
use llimphi_ui::llimphi_hal::wgpu;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{self, TextBlock};
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use media_audio_cpal::AudioSink;
use media_core::{
    AudioProbe, AudioSource, FrameSource, Levels, MixerAudio, Pause, PausableAudio,
    PausableVideo, ProbedAudioSource, Seekable, SubtitleTrack, TestCard, ToneSource, Volume,
    VolumeAudio, Waterfall,
};
use media_core::control::{ControlSettings, KeyChord, MediaCommand};
use media_core::eq::{EqControl, EqualizerAudio, ISO_10_BANDS_HZ};
use media_core::layout::{LayoutSettings, PanelId as TileId};
use media_core::sync::{AvSync, FramePlan};
use llimphi_widget_shortcuts_help::{
    shortcuts_help_view, ShortcutEntry, ShortcutGroup, ShortcutsHelpPalette, ShortcutsHelpSpec,
};
use llimphi_widget_timeline::{timeline_view, TimelinePalette};
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
    /// además lleva contadores de diagnóstico.
    sync: Mutex<AvSync>,
    /// Última posición del reloj de audio observada en un paint. El delta
    /// contra la actual es cuánto avanzó el audio entre paints — el `dt`
    /// con el que avanzamos el video (esclavo del audio, no del reloj de
    /// pared). `None` hasta el primer paint con audio o tras un seek.
    last_audio_pos: Mutex<Option<Duration>>,
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

/// Ecualizador gráfico de 10 bandas compartido con el wrapper
/// [`EqualizerAudio`] en la cadena de audio. [`EqControl`] es clonable y
/// lock-free en el callback realtime (compara una versión atómica); la UI
/// (tile + palette + tecla `e`) lo ajusta desde otro hilo. Default plano.
fn eq() -> &'static EqControl {
    static SLOT: OnceLock<EqControl> = OnceLock::new();
    SLOT.get_or_init(EqControl::graphic_10band)
}

/// Handle al [`Playlist`] activo cuando hay tracks WAV/MP3. `None`
/// si la fuente es tono A4 — en ese caso los botones de seek /
/// playlist / speed quedan apagados.
fn playlist_slot() -> &'static OnceLock<Option<Arc<Mutex<Playlist>>>> {
    static SLOT: OnceLock<Option<Arc<Mutex<Playlist>>>> = OnceLock::new();
    &SLOT
}

/// Pista de subtítulos cargada, si MEDIA_SRT/MEDIA_VTT apuntó a un
/// archivo válido. Se consulta por timestamp del seekable_handle activo.
fn subtitles_slot() -> &'static OnceLock<Option<SubtitleTrack>> {
    static SLOT: OnceLock<Option<SubtitleTrack>> = OnceLock::new();
    &SLOT
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
fn current_audio_position() -> Option<Duration> {
    playlist_slot()
        .get()
        .and_then(|o| o.as_ref())
        .map(|h| h.lock().position())
}

/// Re-ancla el reloj de sync A/V tras un seek o cambio de pista: el
/// próximo paint no avanza el video por un delta espurio (un salto de
/// posición no es "tiempo transcurrido") y los contadores de drop se
/// reinician. No-op si el pipeline todavía no se inicializó (el primer
/// paint lo crea).
fn reset_av_sync_anchor() {
    if let Some(pipe) = pipeline_slot().get() {
        *pipe.last_audio_pos.lock() = None;
        pipe.sync.lock().reset();
    }
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
        (CycleRepeat, "Playlist"),
        (ToggleShuffle, "Playlist"),
        (VolumeBy { delta: vstep }, "Volumen"),
        (VolumeBy { delta: -vstep }, "Volumen"),
        (SetVolume { level: 1.0 }, "Volumen"),
        (SetVolume { level: 0.5 }, "Volumen"),
        (SetVolume { level: 0.0 }, "Volumen"),
        (SpeedStep { dir: 1 }, "Velocidad"),
        (SpeedStep { dir: -1 }, "Velocidad"),
        (SetSpeed { mult: 1.0 }, "Velocidad"),
        (EqToggle, "Ecualizador"),
        (EqReset, "Ecualizador"),
        (AvSyncBy { ms: -50 }, "Sync A/V"),
        (AvSyncBy { ms: 50 }, "Sync A/V"),
        (AvSyncReset, "Sync A/V"),
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

fn new_testcard() -> Box<dyn FrameSource + Send> {
    Box::new(PausableVideo::new(
        TestCard::new(TESTCARD_W, TESTCARD_H, TESTCARD_FPS),
        pause().clone(),
    ))
}

fn build_video_source() -> Box<dyn FrameSource + Send> {
    let cfg = config_slot().get().expect("config set");
    let p = pause().clone();
    match cfg.kind {
        VideoKind::Testcard => new_testcard(),
        VideoKind::Gif => {
            let path = video_path_slot().get().expect("video path set");
            match GifSource::from_path(path) {
                Ok(s) => Box::new(PausableVideo::new(s, p)),
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
                Ok(s) => Box::new(PausableVideo::new(s, p)),
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
                Ok(s) => Box::new(PausableVideo::new(s, p)),
                Err(e) => {
                    eprintln!("media-app: ffmpeg video: {e} — caigo a testcard");
                    new_testcard()
                }
            }
        }
        VideoKind::Av1 => {
            let path = video_path_slot().get().expect("video path set");
            match media_source_av1::Av1VideoSource::open(path) {
                Ok(s) => Box::new(PausableVideo::new(s, p)),
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
        source: Mutex::new(build_video_source()),
        buf: Mutex::new(Vec::new()),
        last_dim: Mutex::new((0, 0)),
        last_tick: Mutex::new(Instant::now()),
        sync: Mutex::new(AvSync::default()),
        last_audio_pos: Mutex::new(None),
    })
}

struct MediaApp;

impl App for MediaApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "media · player"
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        handle.spawn_periodic(Duration::from_millis(TICK_MS), || Msg::Tick);
        spawn_controles_watcher(handle);
        let (palette_commands, palette_cmds) = build_command_catalog(&settings());
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
        }
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => Model {
                frames: model.frames.wrapping_add(1),
                ..model
            },
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
        let cfg = config_slot().get().expect("config set");
        let secs = model.started_at.elapsed().as_secs_f32().max(0.001);
        let fps = model.frames as f32 / secs;

        // Barra de menú principal — primer hijo del column raíz.
        let theme = llimphi_theme::Theme::dark();
        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));

        // --- Hero: canvas de video con título overlay arriba ---
        let title_text = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(36.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            format!("media — {}", cfg.label),
            22.0,
            Color::from_rgba8(220, 230, 245, 255),
        );

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

            // M1 — reloj maestro = audio. El video avanza **lo que avanzó
            // el audio** desde el último paint (sample-accurate), no el
            // reloj de pared; así no deriva en fuentes que no son 30 fps
            // exactos. Sin playlist (tono A4 / testcard) cae al reloj de
            // pared para que el testcard siga animando.
            let audio_pos = current_audio_position();
            let dt = match audio_pos {
                Some(pos) => {
                    let mut last_pos = pipe.last_audio_pos.lock();
                    let d = match *last_pos {
                        // Avance normal del audio: ese es el dt del video.
                        Some(prev) if pos >= prev => pos - prev,
                        // Primer frame, seek o loop (la posición retrocede o
                        // arranca): no avances el video por un delta espurio.
                        _ => Duration::ZERO,
                    };
                    *last_pos = Some(pos);
                    d
                }
                None => wall_dt,
            };

            let mut buf = pipe.buf.lock();
            let mut src = pipe.source.lock();
            if let Some((w, h)) = src.tick(dt, &mut buf) {
                let frame_pts = src.pts();
                drop(src);
                // Si hay reloj de audio y la fuente conoce su PTS, la
                // política decide presentar/descartar. `Hold` no puede
                // ocurrir acá (avanzamos el video con el delta del audio, no
                // más allá), así que sólo distinguimos Drop del resto. Sin
                // PTS o sin audio se presenta siempre (comportamiento previo).
                let present = match (audio_pos, frame_pts) {
                    (Some(audio), Some(pts)) => {
                        !matches!(pipe.sync.lock().plan(audio, pts), FramePlan::Drop)
                    }
                    _ => true,
                };
                if present {
                    pipe.surface.upload(&buf, w, h);
                    *pipe.last_dim.lock() = (w, h);
                }
            } else {
                drop(src);
            }
            drop(buf);
            pipe.surface.blit(queue, encoder, view, rect, viewport);
        });

        let subs_strip = subtitle_strip();
        let timeline = timeline_strip();

        // --- Grilla reorderable de controles + visores ---
        // 3 cols × 2 rows; el orden lo decide el usuario arrastrando
        // por la title bar. Default `[Transport, Volume, Playlist,
        // Recorder, Waveform, Waterfall]`.
        use llimphi_widget_tiled::{
            tiled_view_reorderable_cols, TileSpec, TiledPalette,
        };
        let palette = TiledPalette::from_theme(&llimphi_theme::Theme::dark());
        let tiles: Vec<TileSpec<Msg>> = model
            .tile_order
            .iter()
            .map(|&id| TileSpec {
                label: id.slug().into(),
                content: tile_content(id),
            })
            .collect();
        let tile_grid = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(220.0_f32),
            },
            ..Default::default()
        })
        .children(vec![tiled_view_reorderable_cols(
            tiles,
            3,
            |from, to| Some(Msg::SwapTile { from, to }),
            &palette,
        )]);

        let time_label = playlist_slot()
            .get()
            .and_then(|o| o.as_ref())
            .map(|h| {
                let s = h.lock();
                let pos = s.position();
                let dur = s.duration().unwrap_or(Duration::ZERO);
                let track = if s.len() > 1 {
                    format!(" · trk {}/{}", s.idx() + 1, s.len())
                } else {
                    String::new()
                };
                format!(" · {} / {}{}", fmt_secs(pos), fmt_secs(dur), track)
            })
            .unwrap_or_default();
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

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(10.0_f32),
            },
            padding: TaffyRect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(12.0_f32),
                bottom: length(12.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(22, 26, 34, 255))
        // Right-click en la raíz (origen 0,0 ⇒ local == ventana) abre el
        // menú contextual del reproductor.
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![
            menubar, title_text, canvas, subs_strip, timeline, tile_grid, footer,
        ])
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
                .item(MenuItem::new("Paleta de comandos", "view.palette").shortcut("Ctrl+Shift+P"))
                .item(MenuItem::new("Ayuda de atajos", "view.help").shortcut("?")),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Traduce un command id del menú (principal o contextual) al `Msg`/efecto
/// real. Los ids de transporte/captura despachan `Msg::Command` con un
/// [`MediaCommand`] — exactamente lo que ya disparan botones y teclado.
fn handle_menu_command(model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    use MediaCommand::*;
    let step = settings().seek_step_secs;
    let vstep = settings().volume_step;
    let dispatch = |c: MediaCommand| handle.dispatch(Msg::Command(c));
    match cmd {
        "file.snapshot" => dispatch(Snapshot),
        "file.record" => dispatch(ToggleRecord),
        "file.reload" => handle.dispatch(Msg::ReloadConfig),
        "file.quit" => std::process::exit(0),
        "play.toggle" => dispatch(TogglePause),
        "play.back" => dispatch(SeekBy { secs: -step }),
        "play.fwd" => dispatch(SeekBy { secs: step }),
        "play.prev" => dispatch(PrevTrack),
        "play.next" => dispatch(NextTrack),
        "play.vol_up" => dispatch(VolumeBy { delta: vstep }),
        "play.vol_dn" => dispatch(VolumeBy { delta: -vstep }),
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

/// Despacha por TileId al builder concreto. Cada tile arma su propio
/// contenido — controles co-localizados con la info que afectan.
fn tile_content(id: TileId) -> View<Msg> {
    match id {
        TileId::Transport => transport_tile(),
        TileId::Volume => volume_tile(),
        TileId::Equalizer => equalizer_tile(),
        TileId::Playlist => playlist_tile(),
        TileId::Recorder => recorder_tile(),
        TileId::Waveform => waveform_panel(),
        TileId::Waterfall => waterfall_panel(),
    }
}

/// Tile de transporte: prev/play-pause/next + back/fwd 5s. Los chips
/// de track se apagan si no hay playlist.
fn transport_tile() -> View<Msg> {
    let paused = pause().is_paused();
    let pause_btn = chip_button(
        if paused { "play" } else { "pause" },
        if paused {
            Color::from_rgba8(60, 140, 90, 255)
        } else {
            Color::from_rgba8(55, 65, 80, 255)
        },
        Color::from_rgba8(220, 230, 245, 255),
        Msg::Command(MediaCommand::TogglePause),
    );

    let playlist_active = playlist_slot()
        .get()
        .and_then(|o| o.as_ref())
        .map(|h| h.lock().len() > 1)
        .unwrap_or(false);
    let pl_bg = if playlist_active {
        Color::from_rgba8(55, 65, 80, 255)
    } else {
        Color::from_rgba8(40, 46, 56, 255)
    };
    let pl_fg = if playlist_active {
        Color::from_rgba8(220, 230, 245, 255)
    } else {
        Color::from_rgba8(100, 110, 125, 255)
    };
    let prev_btn = chip_button("⟨trk", pl_bg, pl_fg, Msg::Command(MediaCommand::PrevTrack));
    let next_btn = chip_button("trk⟩", pl_bg, pl_fg, Msg::Command(MediaCommand::NextTrack));

    let seekable = playlist_slot().get().and_then(|o| o.as_ref()).is_some();
    let seek_bg = if seekable {
        Color::from_rgba8(55, 65, 80, 255)
    } else {
        Color::from_rgba8(40, 46, 56, 255)
    };
    let seek_fg = if seekable {
        Color::from_rgba8(220, 230, 245, 255)
    } else {
        Color::from_rgba8(100, 110, 125, 255)
    };
    let step = settings().seek_step_secs;
    let back_btn = chip_button(
        &format!("«{step}s"),
        seek_bg,
        seek_fg,
        Msg::Command(MediaCommand::SeekBy { secs: -step }),
    );
    let fwd_btn = chip_button(
        &format!("{step}s»"),
        seek_bg,
        seek_fg,
        Msg::Command(MediaCommand::SeekBy { secs: step }),
    );

    tile_chip_grid(vec![prev_btn, pause_btn, next_btn, back_btn, fwd_btn])
}

/// Tile de volumen: vol-/vol+ con el porcentaje al medio y la barra
/// de peak/RMS abajo. La info (los medidores) está pegada al control
/// (vol+/-) — el usuario ve el efecto del slider sin saltar de tile.
fn volume_tile() -> View<Msg> {
    let vol_label = format!("vol {:.0}%", (volume().get() * 100.0).round());
    let vol_text = View::new(Style {
        size: Size {
            width: length(76.0_f32),
            height: length(36.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(vol_label, 13.0, Color::from_rgba8(180, 195, 215, 255));
    let vstep = settings().volume_step;
    let vol_dn = chip_button(
        "vol−",
        Color::from_rgba8(55, 65, 80, 255),
        Color::from_rgba8(220, 230, 245, 255),
        Msg::Command(MediaCommand::VolumeBy { delta: -vstep }),
    );
    let vol_up = chip_button(
        "vol+",
        Color::from_rgba8(55, 65, 80, 255),
        Color::from_rgba8(220, 230, 245, 255),
        Msg::Command(MediaCommand::VolumeBy { delta: vstep }),
    );

    let row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![vol_dn, vol_text, vol_up]);

    let meters = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(48.0_f32),
        },
        ..Default::default()
    })
    .children(vec![meters_panel()]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![row, meters])
}

/// Tile del ecualizador: gráfico de las 10 bandas (barras desde -12..+12
/// dB respecto a la línea 0 dB) + chips on/off y flat. El ajuste fino por
/// banda vive en el command palette (grupo "Ecualizador") y la tecla `e`
/// togglea el EQ entero. Apagado, las barras se pintan grises (bypass).
fn equalizer_tile() -> View<Msg> {
    let enabled = eq().is_enabled();
    let gains = eq().gains();

    let graph = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(96.0_f32),
        },
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
        let mid_y = inner_y + inner_h * 0.5;

        // Línea 0 dB de referencia.
        let mut zero = BezPath::new();
        zero.move_to((inner_x as f64, mid_y as f64));
        zero.line_to(((inner_x + inner_w) as f64, mid_y as f64));
        scene.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            Color::from_rgba8(80, 92, 110, 255),
            None,
            &zero,
        );

        let n = gains.len().max(1);
        let slot_w = inner_w / n as f32;
        let bar_w = (slot_w * 0.6).max(2.0);
        let half = inner_h * 0.5;
        for (i, &g) in gains.iter().enumerate() {
            let norm = (g / 12.0).clamp(-1.0, 1.0);
            let h = (norm.abs() * half).max(1.0);
            let cx = inner_x + (i as f32 + 0.5) * slot_w;
            let x0 = cx - bar_w * 0.5;
            let (y0, y1) = if norm >= 0.0 {
                (mid_y - h, mid_y)
            } else {
                (mid_y, mid_y + h)
            };
            let bar = KurboRect::new(x0 as f64, y0 as f64, (x0 + bar_w) as f64, y1 as f64);
            let col = if !enabled {
                Color::from_rgba8(70, 80, 95, 255)
            } else if norm >= 0.0 {
                Color::from_rgba8(110, 200, 150, 255)
            } else {
                Color::from_rgba8(210, 150, 90, 255)
            };
            scene.fill(Fill::NonZero, Affine::IDENTITY, col, None, &bar);
        }
    });

    let toggle = chip_button(
        if enabled { "eq on" } else { "eq off" },
        if enabled {
            Color::from_rgba8(60, 110, 150, 255)
        } else {
            Color::from_rgba8(40, 46, 56, 255)
        },
        Color::from_rgba8(220, 230, 245, 255),
        Msg::Command(MediaCommand::EqToggle),
    );
    let flat = chip_button(
        "flat",
        Color::from_rgba8(55, 65, 80, 255),
        Color::from_rgba8(220, 230, 245, 255),
        Msg::Command(MediaCommand::EqReset),
    );
    let controls = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![toggle, flat]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![graph, controls])
}

/// Tile de playlist: repeat/shuffle/speed. Los tres están apagados si
/// no hay playlist activa.
fn playlist_tile() -> View<Msg> {
    let seekable = playlist_slot().get().and_then(|o| o.as_ref()).is_some();
    let bg_on = Color::from_rgba8(55, 65, 80, 255);
    let bg_off = Color::from_rgba8(40, 46, 56, 255);
    let fg_on = Color::from_rgba8(220, 230, 245, 255);
    let fg_off = Color::from_rgba8(100, 110, 125, 255);
    let bg = if seekable { bg_on } else { bg_off };
    let fg = if seekable { fg_on } else { fg_off };

    let current_speed = playlist_slot()
        .get()
        .and_then(|o| o.as_ref())
        .map(|h| h.lock().current_speed())
        .unwrap_or(1.0);
    let speed_label = format!("{:.2}×", current_speed);
    let speed_btn = chip_button(
        &speed_label,
        bg,
        fg,
        Msg::Command(MediaCommand::SpeedStep { dir: 1 }),
    );

    let (repeat_label, shuffle_on) = playlist_slot()
        .get()
        .and_then(|o| o.as_ref())
        .map(|h| {
            let pl = h.lock();
            (pl.repeat_mode().label(), pl.shuffle_on())
        })
        .unwrap_or(("rep-", false));
    let loop_btn = chip_button(repeat_label, bg, fg, Msg::Command(MediaCommand::CycleRepeat));
    let shuf_bg = if shuffle_on {
        Color::from_rgba8(60, 110, 150, 255)
    } else {
        bg
    };
    let shuf_btn = chip_button(
        if shuffle_on { "shuf!" } else { "shuf-" },
        shuf_bg,
        fg,
        Msg::Command(MediaCommand::ToggleShuffle),
    );

    tile_chip_grid(vec![loop_btn, shuf_btn, speed_btn])
}

/// Tile de captura: rec + snap. Cuando `rec` está activo el chip se
/// pinta en rojo y dice `stop`.
fn recorder_tile() -> View<Msg> {
    let recording = recorder().is_recording();
    let rec_btn = chip_button(
        if recording { "stop" } else { "rec" },
        if recording {
            Color::from_rgba8(200, 65, 65, 255)
        } else {
            Color::from_rgba8(55, 65, 80, 255)
        },
        Color::from_rgba8(245, 235, 235, 255),
        Msg::Command(MediaCommand::ToggleRecord),
    );
    let snap_btn = chip_button(
        "snap",
        Color::from_rgba8(55, 65, 80, 255),
        Color::from_rgba8(220, 230, 245, 255),
        Msg::Command(MediaCommand::Snapshot),
    );
    tile_chip_grid(vec![rec_btn, snap_btn])
}

/// Layout helper: fila de chips centrada vertical y horizontalmente
/// dentro del cuerpo del tile. Lo comparten los tiles de transport,
/// playlist y recorder — toman lo que el tiled les dé y centran.
fn tile_chip_grid(chips: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        // Wrap permite que si la columna se hace estrecha los chips
        // bajen de fila en vez de cortarse.
        flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
        ..Default::default()
    })
    .children(chips)
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
    let position = playlist_slot()
        .get()
        .and_then(|o| o.as_ref())
        .map(|h| h.lock().position())
        .unwrap_or(Duration::ZERO);
    let text = track
        .at(position)
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

/// Panel inferior con la forma de onda del último tramo del stream
/// (mezcla de canales en mono para mostrarse en una sola línea).
/// Cuando no hay probe (audio muteado) muestra una línea de centro
/// con leyenda "audio off".
/// Barra de progreso clickeable bajo el video — scrubbing estilo VLC.
/// Delega en el widget reusable `llimphi-widget-timeline`: la app sólo
/// calcula la fracción de avance (posición/duración del player vivo) y
/// mapea el click (fracción del ancho) a `MediaCommand::SeekTo` — el core
/// no sabe la duración, sólo la fracción. Sin playlist (tono A4) queda en
/// cero. Se redibuja cada Tick, así el playhead avanza solo.
fn timeline_strip() -> View<Msg> {
    let frac = playlist_slot()
        .get()
        .and_then(|o| o.as_ref())
        .map(|h| {
            let s = h.lock();
            let dur = s.duration().unwrap_or(Duration::ZERO).as_secs_f64();
            if dur <= 0.0 {
                0.0
            } else {
                (s.position().as_secs_f64() / dur).clamp(0.0, 1.0) as f32
            }
        })
        .unwrap_or(0.0);
    let palette = TimelinePalette::from_theme(&llimphi_theme::Theme::dark());
    timeline_view(frac, &palette, |fraction| {
        Some(Msg::Command(MediaCommand::SeekTo { fraction }))
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match args.first() {
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
        match foreign_av::probe(path)
            .and_then(MediaSession::open)
        {
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

    // Subtítulos: MEDIA_SRT / MEDIA_VTT / MEDIA_ASS apuntan al archivo; el
    // parser autodetecta SRT vs WebVTT vs ASS/SSA por la cabecera, así que
    // las tres envs son intercambiables (gana la primera presente en ese
    // orden). Falla silenciosa con log en stderr — la app sigue sin subs.
    let subs = match std::env::var("MEDIA_SRT")
        .or_else(|_| std::env::var("MEDIA_VTT"))
        .or_else(|_| std::env::var("MEDIA_ASS"))
    {
        Ok(path) => match std::fs::read_to_string(&path) {
            Ok(body) => match SubtitleTrack::parse_subtitles(&body) {
                Ok(t) => {
                    eprintln!(
                        "media-app: subtitles {path} · {} cues",
                        t.len()
                    );
                    Some(t)
                }
                Err(e) => {
                    eprintln!("media-app: subtítulos inválidos ({e})");
                    None
                }
            },
            Err(e) => {
                eprintln!("media-app: no pude leer subtítulos {path}: {e}");
                None
            }
        },
        Err(_) => None,
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
                let shared: Arc<Mutex<Playlist>> = Arc::new(Mutex::new(pl));
                playlist_slot().set(Some(shared.clone())).ok();
                let pausable = PausableAudio::new(
                    Box::new(SharedAudio { inner: shared })
                        as Box<dyn AudioSource + Send>,
                    pause().clone(),
                );
                let voled = VolumeAudio::new(pausable, volume().clone());
                let equalized = EqualizerAudio::new(voled, eq().clone());
                let recorded = RecordedAudioSource::new(equalized, recorder().clone());
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
    let recorded = RecordedAudioSource::new(equalized, recorder().clone());
    let probed = ProbedAudioSource::new(recorded, probe.clone());
    (Arc::new(Mutex::new(probed)), probe)
}
