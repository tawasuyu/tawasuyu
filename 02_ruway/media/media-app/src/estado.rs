use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use media_core::{AudioProbe, Pause, Volume};
use media_core::color::ColorControl;
use media_core::control::ControlSettings;
use media_core::eq::EqControl;
use media_core::dynamics::DynamicsControl;
use media_core::loudness::LoudnessTap;
use media_core::osd::Osd;
use media_core::transform::TransformControl;
use media_core::viewport::ViewControl;
use media_core::tracks::TrackSet;
use media_core::sync::AvSync;
use foreign_av::MediaSession;
use parking_lot::Mutex;

use crate::tipos::{Config, Msg};
use crate::playlist::Playlist;
use crate::pipeline::Pipeline;

pub(crate) const TESTCARD_W: u32 = 480;
pub(crate) const TESTCARD_H: u32 = 270;
pub(crate) const TESTCARD_FPS: f32 = 30.0;
/// Key de la ventana OS secundaria de configuración (multiventana llimphi-ui).
pub(crate) const CONFIG_WIN: u64 = 1;
/// Key de la ventana OS secundaria de lista de reproducción / cola.
pub(crate) const PLAYLIST_WIN: u64 = 2;
pub(crate) const TICK_MS: u64 = 33;
/// Capacidad del ring del probe. ~85 ms a 48 kHz · 2 ch — suficiente
/// para una franja de visor responsiva sin meter latencia ni RAM.
pub(crate) const PROBE_CAPACITY: usize = 8192;

/// Delay de subtítulos en ms (S4). Positivo retrasa el subtítulo; se aplica
/// al consultar el cue activo (`subtitle_strip`). Tope ±60 s.
pub(crate) static SEEK_FORCE: AtomicBool = AtomicBool::new(false);
pub(crate) static SUB_DELAY_MS: AtomicI64 = AtomicI64::new(0);
/// Pedido de "avanzar un cuadro" pendiente (frame stepping `.`, M4): la
/// vista lo consume tirando del próximo frame vía `FrameSource::step_frame`.
pub(crate) static FRAME_STEP_FWD: AtomicBool = AtomicBool::new(false);
/// FPS del video actual (bits de un `f32`), para calcular el salto de un
/// cuadro en el frame stepping hacia atrás. Lo fija el armado del pipeline;
/// 0.0 (default) ⇒ se asume 30 fps.
pub(crate) static VIDEO_FPS: AtomicU32 = AtomicU32::new(0);

/// FPS del video actual, o `30.0` si todavía no se conoce.
pub(crate) fn video_fps() -> f32 {
    let f = f32::from_bits(VIDEO_FPS.load(Ordering::Relaxed));
    if f.is_finite() && f >= 1.0 {
        f
    } else {
        30.0
    }
}

/// Registra el FPS del video que arma el pipeline.
pub(crate) fn set_video_fps(fps: f32) {
    VIDEO_FPS.store(fps.to_bits(), Ordering::Relaxed);
}
pub(crate) const MAX_SUB_DELAY_MS: i64 = media_core::SubtitleTrack::MAX_DELAY_MS;

/// Settings de control (pasos + keymap) cargados al arrancar desde RON
/// en XDG, o el default tipo VLC si no hay archivo.
pub(crate) fn settings_slot() -> &'static std::sync::RwLock<ControlSettings> {
    static SLOT: OnceLock<std::sync::RwLock<ControlSettings>> = OnceLock::new();
    SLOT.get_or_init(|| std::sync::RwLock::new(ControlSettings::default()))
}

/// Accessor de conveniencia: devuelve un clon del snapshot actual.
pub(crate) fn settings() -> ControlSettings {
    settings_slot().read().expect("settings lock").clone()
}

/// Recarga `controles.ron` en caliente.
pub(crate) fn reload_settings() {
    let nuevo = crate::config_io::load_settings();
    *settings_slot().write().expect("settings lock") = nuevo;
    eprintln!("media-app: controles recargados");
}

/// Vigila `controles.ron` en un hilo aparte: cada segundo compara el mtime
/// y, si cambió, dispatcha `ReloadConfig` — recarga **automática** sin
/// tener que apretar F5.
pub(crate) fn spawn_controles_watcher(handle: &llimphi_ui::Handle<Msg>) {
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
/// `$XDG_CONFIG_HOME/tawasuyu/media/<name>`.
pub(crate) fn config_file(name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("tawasuyu").join("media").join(name))
}

/// Path del archivo de controles (mapeo de entrada).
pub(crate) fn controles_path() -> Option<PathBuf> {
    config_file("controles.ron")
}

/// Path del archivo de layout (orden de los paneles del grid).
pub(crate) fn layout_path() -> Option<PathBuf> {
    config_file("layout.ron")
}

/// Path del archivo de video (GIF o imagen estática) cuando aplica.
pub(crate) fn video_path_slot() -> &'static OnceLock<PathBuf> {
    static SLOT: OnceLock<PathBuf> = OnceLock::new();
    &SLOT
}

/// Nombres de las pistas de la playlist, cacheados al crearla.
pub(crate) fn playlist_labels_slot() -> &'static OnceLock<Vec<String>> {
    static SLOT: OnceLock<Vec<String>> = OnceLock::new();
    &SLOT
}

/// Onda de pista completa (tipo Audacity) computada en background.
pub(crate) fn waveform_slot() -> &'static Mutex<Option<media_core::waveform::Waveform>> {
    static SLOT: OnceLock<Mutex<Option<media_core::waveform::Waveform>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// URL de audio **separada** (DASH) cuando yt-dlp resolvió video y audio en
/// streams distintos (R2, YouTube > 720p).
pub(crate) fn dash_audio_slot() -> &'static OnceLock<PathBuf> {
    static SLOT: OnceLock<PathBuf> = OnceLock::new();
    &SLOT
}

/// Probe del stream de audio que `audio_source_from_env` instaló.
pub(crate) fn audio_probe_slot() -> &'static OnceLock<Option<AudioProbe>> {
    static SLOT: OnceLock<Option<AudioProbe>> = OnceLock::new();
    &SLOT
}

/// Handle de pausa compartido por audio y video.
pub(crate) fn pause() -> &'static Pause {
    static SLOT: OnceLock<Pause> = OnceLock::new();
    SLOT.get_or_init(Pause::new)
}

/// Handle compartido del recorder WAV.
pub(crate) fn recorder() -> &'static media_recorder_wav::WavRecorder {
    static SLOT: OnceLock<media_recorder_wav::WavRecorder> = OnceLock::new();
    SLOT.get_or_init(media_recorder_wav::WavRecorder::new)
}

/// Ganancia lineal compartida con el wrapper [`VolumeAudio`].
pub(crate) fn volume() -> &'static Volume {
    static SLOT: OnceLock<Volume> = OnceLock::new();
    SLOT.get_or_init(|| Volume::new(1.0))
}

/// Volumen guardado mientras está silenciado (mute real).
pub(crate) fn muted_volume() -> &'static Mutex<Option<f32>> {
    static SLOT: OnceLock<Mutex<Option<f32>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Ecualizador gráfico de 10 bandas compartido con el wrapper.
pub(crate) fn eq() -> &'static EqControl {
    static SLOT: OnceLock<EqControl> = OnceLock::new();
    SLOT.get_or_init(EqControl::graphic_10band)
}

/// Control de ajustes de color del video.
pub(crate) fn color() -> &'static ColorControl {
    static SLOT: OnceLock<ColorControl> = OnceLock::new();
    SLOT.get_or_init(ColorControl::default)
}

/// Control de orientación del video (rotación/flip, V3).
pub(crate) fn transform() -> &'static TransformControl {
    static SLOT: OnceLock<TransformControl> = OnceLock::new();
    SLOT.get_or_init(TransformControl::default)
}

/// Ajustes de encaje/zoom/pan del video (V2).
pub(crate) fn viewcontrol() -> &'static Mutex<ViewControl> {
    static SLOT: OnceLock<Mutex<ViewControl>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(ViewControl::default()))
}

/// Control de normalización + limitador de audio (A5).
pub(crate) fn dynamics() -> &'static DynamicsControl {
    static SLOT: OnceLock<DynamicsControl> = OnceLock::new();
    SLOT.get_or_init(DynamicsControl::default)
}

/// Tap de medición de sonoridad (EBU R128).
pub(crate) fn loudness() -> &'static LoudnessTap {
    static SLOT: OnceLock<LoudnessTap> = OnceLock::new();
    SLOT.get_or_init(LoudnessTap::new)
}

/// OSD transitorio (volumen/seek/velocidad…), U4.
pub(crate) fn osd() -> &'static Mutex<Osd> {
    static SLOT: OnceLock<Mutex<Osd>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(Osd::new()))
}

/// Reloj del OSD: segundos monotónicos desde el primer uso.
pub(crate) fn osd_now() -> f64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// Flashea un texto en el OSD con la duración por defecto.
pub(crate) fn osd_flash(text: impl Into<String>) {
    osd().lock().flash(text, osd_now());
}

/// Flashea la posición/total actual (tras un seek/salto de capítulo).
pub(crate) fn osd_flash_seek() {
    let s = crate::playlist::playback_snapshot();
    if s.present {
        let total = s.duration.unwrap_or(Duration::ZERO).as_secs_f64();
        osd_flash(media_core::osd::format_seek(s.position.as_secs_f64(), total));
    }
}

/// Handle al [`Playlist`] activo cuando hay tracks WAV/MP3.
pub(crate) fn playlist_slot() -> &'static OnceLock<Option<Arc<Mutex<Playlist>>>> {
    static SLOT: OnceLock<Option<Arc<Mutex<Playlist>>>> = OnceLock::new();
    &SLOT
}

/// Pista de subtítulos activa.
pub(crate) fn subtitles_slot() -> &'static Mutex<Option<media_core::SubtitleTrack>> {
    static SLOT: OnceLock<Mutex<Option<media_core::SubtitleTrack>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// `MediaSession` compartida entre el FfmpegVideoSource del pipeline y
/// el FfmpegAudioSource del Playlist cuando la fuente es un archivo de video.
pub(crate) fn ffmpeg_session_slot() -> &'static OnceLock<Option<MediaSession>> {
    static SLOT: OnceLock<Option<MediaSession>> = OnceLock::new();
    &SLOT
}

/// Conjunto de pistas (audio/subtítulos) del medio actual (A2/S2).
pub(crate) fn tracks() -> &'static Mutex<Option<TrackSet>> {
    static SLOT: OnceLock<Mutex<Option<TrackSet>>> = OnceLock::new();
    SLOT.get_or_init(|| {
        let ts = ffmpeg_session_slot()
            .get()
            .and_then(|o| o.as_ref())
            .map(|s| TrackSet::from_tracks(s.info().tracks));
        Mutex::new(ts)
    })
}

pub(crate) fn config_slot() -> &'static OnceLock<Config> {
    static SLOT: OnceLock<Config> = OnceLock::new();
    &SLOT
}

pub(crate) fn pipeline_slot() -> &'static OnceLock<Pipeline> {
    static SLOT: OnceLock<Pipeline> = OnceLock::new();
    &SLOT
}

/// Tags del medio actual (título/artista/álbum/carátula).
pub(crate) fn media_metadata_slot() -> &'static OnceLock<media_core::metadata::Metadata> {
    static SLOT: OnceLock<media_core::metadata::Metadata> = OnceLock::new();
    &SLOT
}

/// Ruta del medio local en reproducción.
pub(crate) fn current_media_path() -> Option<PathBuf> {
    if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
        return Some(h.lock().track_path().to_path_buf());
    }
    video_path_slot()
        .get()
        .filter(|p| !p.as_os_str().is_empty())
        .cloned()
}

/// Capítulos del medio actual (V7), extraídos una vez al arrancar.
pub(crate) fn chapters_slot() -> &'static OnceLock<media_core::chapters::Chapters> {
    static SLOT: OnceLock<media_core::chapters::Chapters> = OnceLock::new();
    &SLOT
}

pub(crate) fn reset_av_sync_anchor() {
    if let Some(pipe) = pipeline_slot().get() {
        pipe.sync.lock().reset();
    }
    // Tras un seek, queremos ver el destino aunque estemos en pausa.
    SEEK_FORCE.store(true, Ordering::Relaxed);
}
