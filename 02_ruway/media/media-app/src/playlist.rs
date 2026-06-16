use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use media_core::playlist::Cursor;
use media_core::{AudioSource, Seekable};
use media_source_wav::WavSource;
use media_source_mp3::Mp3Source;
use media_source_opus::OpusSource;
use foreign_av::FfmpegAudioSource;
use parking_lot::Mutex;

use crate::estado::{
    pause, playlist_slot, reset_av_sync_anchor, volume, eq, dynamics, loudness,
    recorder, PROBE_CAPACITY,
};

/// Adapter que comparte una fuente vía `Arc<Mutex<T>>` sin moverla.
pub(crate) struct SharedAudio<T> {
    pub(crate) inner: Arc<Mutex<T>>,
}

impl<T: AudioSource> AudioSource for SharedAudio<T> {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.inner.lock().fill(buf, sample_rate, channels);
    }
}

/// Una pista cargada de la playlist.
pub(crate) enum LoadedTrack {
    Wav(WavSource),
    Mp3(Mp3Source),
    Opus(OpusSource),
    FfmpegAudio(FfmpegAudioSource),
    /// Pista nula: el motor está vivo pero sin medio cargado (silencio).
    /// Permite que el sink de audio exista siempre, listo para que una
    /// playlist se cargue/reemplace en caliente sin reabrir el device.
    Silent,
}

impl LoadedTrack {
    pub(crate) fn from_path(path: &std::path::Path) -> Result<Self, String> {
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

    pub(crate) fn set_speed(&mut self, speed: f32) {
        match self {
            LoadedTrack::Wav(w) => w.set_speed(speed),
            LoadedTrack::Mp3(m) => m.set_speed(speed),
            LoadedTrack::Opus(o) => o.set_speed(speed),
            LoadedTrack::FfmpegAudio(_) | LoadedTrack::Silent => {}
        }
    }

    pub(crate) fn set_loop(&mut self, looped: bool) {
        match self {
            LoadedTrack::Wav(w) => w.set_loop(looped),
            LoadedTrack::Mp3(m) => m.set_loop(looped),
            LoadedTrack::Opus(o) => o.set_loop(looped),
            LoadedTrack::FfmpegAudio(_) | LoadedTrack::Silent => {}
        }
    }

    pub(crate) fn is_finished(&self) -> bool {
        match self {
            LoadedTrack::Wav(w) => w.is_finished(),
            LoadedTrack::Mp3(m) => m.is_finished(),
            LoadedTrack::Opus(o) => o.is_finished(),
            LoadedTrack::FfmpegAudio(a) => {
                let dur = a.duration().unwrap_or(Duration::ZERO);
                !dur.is_zero()
                    && a.position() + Duration::from_millis(80) >= dur
            }
            LoadedTrack::Silent => false,
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
            LoadedTrack::Silent => buf.iter_mut().for_each(|s| *s = 0.0),
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
            LoadedTrack::Silent => Duration::ZERO,
        }
    }
    fn duration(&self) -> Option<Duration> {
        match self {
            LoadedTrack::Wav(w) => w.duration(),
            LoadedTrack::Mp3(m) => m.duration(),
            LoadedTrack::Opus(o) => o.duration(),
            LoadedTrack::FfmpegAudio(a) => a.duration(),
            LoadedTrack::Silent => None,
        }
    }
    fn seek_to(&mut self, pos: Duration) {
        match self {
            LoadedTrack::Wav(w) => w.seek_to(pos),
            LoadedTrack::Mp3(m) => m.seek_to(pos),
            LoadedTrack::Opus(o) => o.seek_to(pos),
            LoadedTrack::FfmpegAudio(a) => a.seek_to(pos),
            LoadedTrack::Silent => {}
        }
    }
}

/// Modo de loop del Playlist global.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepeatMode {
    Off,
    One,
    All,
}

impl RepeatMode {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Off => Self::One,
            Self::One => Self::All,
            Self::All => Self::Off,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Off => "rep-",
            Self::One => "rep1",
            Self::All => "repA",
        }
    }
}

/// Decisión de auto-advance para el tick de UI (pistas no nativas).
pub(crate) enum TickAdvance {
    /// Nada que hacer (no terminó, o ya lo maneja el hilo de audio, o fin sin
    /// repetición).
    None,
    /// Re-arrancar la pista actual desde cero (RepeatMode::One de video, o
    /// All con una sola pista de video).
    Loop,
    /// Cambiar a la pista en este índice (swap completo de video+audio).
    Switch(usize),
}

/// Playlist con prev/next manual + auto-advance al fin de cada pista. El orden
/// (índice + shuffle + saltos) lo lleva el [`Cursor`] agnóstico de `media-core`
/// (Regla 2); acá quedan los decoders cargados, la velocidad y el modo repeat
/// (que decide loop por pista y fin de cola).
pub(crate) struct Playlist {
    pub(crate) tracks: Vec<PathBuf>,
    pub(crate) current: LoadedTrack,
    pub(crate) speed: f32,
    pub(crate) repeat: RepeatMode,
    cursor: Cursor,
}

impl Playlist {
    pub(crate) fn new(tracks: Vec<PathBuf>) -> Result<Self, String> {
        if tracks.is_empty() {
            return Err("playlist vacía".into());
        }
        let mut current = LoadedTrack::from_path(&tracks[0])?;
        current.set_loop(false);
        let cursor = Cursor::new(tracks.len());
        Ok(Self {
            tracks,
            current,
            speed: 1.0,
            repeat: RepeatMode::Off,
            cursor,
        })
    }

    /// Motor vivo pero sin medio: silencio, listo para [`Self::set_list`].
    pub(crate) fn empty() -> Self {
        Self {
            tracks: Vec::new(),
            current: LoadedTrack::Silent,
            speed: 1.0,
            repeat: RepeatMode::Off,
            cursor: Cursor::empty(),
        }
    }

    /// Fija la **lista** de pistas (de cualquier medio, audio o video) **sin
    /// decodificar**: deja el motor en silencio hasta que la capa de `open`
    /// haga el swap real del índice elegido (`open_playlist_index`). No intenta
    /// abrir la primera pista como audio nativo, así una cola de **videos**
    /// también carga (el viejo `load_tracks` audio-only se retiró).
    pub(crate) fn set_list(&mut self, tracks: Vec<PathBuf>) {
        self.cursor.reset(tracks.len());
        self.tracks = tracks;
        self.current = LoadedTrack::Silent;
    }

    /// Fija la pista viva al índice `target` con su componente de audio ya
    /// construida (el video lo swapea la capa de `open`). **Mantiene la lista**
    /// (a diferencia de [`Self::set_current_track`], que la colapsa a un único
    /// medio). Actualiza `idx` y la posición del orden aleatorio.
    pub(crate) fn set_track_at(&mut self, target: usize, mut audio: LoadedTrack) {
        if target >= self.tracks.len() {
            return;
        }
        audio.set_speed(self.speed);
        audio.set_loop(matches!(self.repeat, RepeatMode::One));
        self.current = audio;
        self.cursor.set_idx(target);
    }

    /// Reemplaza **en caliente** la pista viva por `track` (p. ej. el audio
    /// ffmpeg de un video recién abierto) con `path` como rótulo. Mismo motor;
    /// usado por el swap de video runtime.
    pub(crate) fn set_current_track(&mut self, path: PathBuf, mut track: LoadedTrack) {
        track.set_speed(self.speed);
        track.set_loop(matches!(self.repeat, RepeatMode::One));
        self.tracks = vec![path];
        self.current = track;
        self.cursor.reset(1);
    }

    /// Crea una cola sembrando la **carpeta** de `path` con sus hermanos de
    /// medios (`siblings`, ordenados) y `track` ya decodificado como la pista
    /// activa, posicionando `idx` sobre `path`. Así anterior/siguiente recorren
    /// la carpeta al abrir un medio suelto. Si `siblings` no contiene a `path`
    /// (o está vacío), cae a una cola de una sola entrada.
    pub(crate) fn new_in_folder(path: PathBuf, mut track: LoadedTrack, siblings: Vec<PathBuf>) -> Self {
        track.set_loop(false);
        let (tracks, idx) = match siblings.iter().position(|p| *p == path) {
            Some(i) => (siblings, i),
            None => (vec![path], 0),
        };
        let mut cursor = Cursor::new(tracks.len());
        cursor.set_idx(idx);
        Self {
            tracks,
            current: track,
            speed: 1.0,
            repeat: RepeatMode::Off,
            cursor,
        }
    }

    pub(crate) fn repeat_mode(&self) -> RepeatMode {
        self.repeat
    }

    pub(crate) fn shuffle_on(&self) -> bool {
        self.cursor.shuffle_on()
    }

    pub(crate) fn cycle_repeat(&mut self) {
        self.repeat = self.repeat.next();
        let want_loop = matches!(self.repeat, RepeatMode::One);
        self.current.set_loop(want_loop);
    }

    pub(crate) fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
        self.current.set_loop(matches!(self.repeat, RepeatMode::One));
    }

    pub(crate) fn toggle_shuffle(&mut self) {
        self.cursor.toggle_shuffle();
    }

    pub(crate) fn track_path(&self) -> &std::path::Path {
        self.tracks
            .get(self.cursor.idx())
            .map(|p| p.as_path())
            .unwrap_or_else(|| std::path::Path::new(""))
    }

    /// Ruta de la pista en el índice `idx` de la lista (sin moverse).
    pub(crate) fn track_at(&self, idx: usize) -> Option<&std::path::Path> {
        self.tracks.get(idx).map(|p| p.as_path())
    }

    /// Índice destino de un paso `delta` (respeta el orden aleatorio), **puro**:
    /// no carga ni muta nada. `None` si no hay con qué moverse (≤ 1 pista).
    pub(crate) fn peek_step(&self, delta: i64) -> Option<usize> {
        self.cursor.peek_step(delta)
    }

    /// ¿La pista viva la decodifica un source de audio **nativo**? Esas se
    /// auto-avanzan en el hilo de audio (sin reconstruir el pipeline de video);
    /// el resto (video/ffmpeg) lo maneja el tick de UI.
    pub(crate) fn current_is_native(&self) -> bool {
        matches!(
            self.current,
            LoadedTrack::Wav(_) | LoadedTrack::Mp3(_) | LoadedTrack::Opus(_)
        )
    }

    /// ¿La pista en `idx` es de audio nativo (por extensión)?
    fn path_is_native(&self, idx: usize) -> bool {
        self.tracks
            .get(idx)
            .map(|p| crate::open::is_native_audio(p))
            .unwrap_or(false)
    }

    pub(crate) fn len(&self) -> usize {
        self.tracks.len()
    }

    pub(crate) fn idx(&self) -> usize {
        self.cursor.idx()
    }

    pub(crate) fn current_speed(&self) -> f32 {
        self.speed
    }

    pub(crate) fn jump_to(&mut self, target: usize) {
        if target >= self.tracks.len() || target == self.cursor.idx() {
            return;
        }
        match LoadedTrack::from_path(&self.tracks[target]) {
            Ok(mut t) => {
                t.set_speed(self.speed);
                t.set_loop(matches!(self.repeat, RepeatMode::One));
                self.current = t;
                self.cursor.set_idx(target);
            }
            Err(e) => eprintln!("media-app: salto de pista falló: {e}"),
        }
    }

    /// Copia de las rutas de la cola (para cargar miniaturas en background).
    pub(crate) fn track_paths(&self) -> Vec<PathBuf> {
        self.tracks.clone()
    }

    pub(crate) fn track_labels(&self) -> Vec<String> {
        self.tracks
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.display().to_string())
            })
            .collect()
    }

    /// Auto-advance del **hilo de audio**: sólo pistas nativas, y sólo si la
    /// pista destino también es nativa (swap sin gap, sin tocar el pipeline de
    /// video). Cualquier transición que involucre video la decide el tick de UI
    /// vía [`Self::tick_advance`] — acá no se hace nada para no congelar el
    /// frame ni spamear errores intentando decodificar un video como audio.
    fn maybe_auto_advance(&mut self) {
        if !self.current_is_native() || !self.current.is_finished() {
            return;
        }
        match self.repeat {
            RepeatMode::One => {
                self.current.seek_to(Duration::ZERO);
            }
            RepeatMode::All if self.tracks.len() <= 1 => {
                self.current.seek_to(Duration::ZERO);
            }
            RepeatMode::All | RepeatMode::Off => {
                let last = matches!(self.repeat, RepeatMode::Off) && self.cursor.at_last();
                if last {
                    return;
                }
                if let Some(target) = self.peek_step(1) {
                    if self.path_is_native(target) {
                        self.jump_to(target);
                    }
                    // Destino no nativo → lo abre el tick de UI.
                }
            }
        }
    }

    /// Acción de auto-advance que debe ejecutar el **tick de UI** cuando la
    /// pista viva NO es nativa (video/ffmpeg) — o cuando una nativa terminó y
    /// la siguiente es video (caso que el hilo de audio deja pasar). **Pura**:
    /// no muta nada; el caller reconstruye el pipeline.
    pub(crate) fn tick_advance(&self) -> TickAdvance {
        if !self.current.is_finished() {
            return TickAdvance::None;
        }
        let target = match self.repeat {
            RepeatMode::One => {
                // Las nativas las re-loopea su propio source / el hilo de audio.
                return if self.current_is_native() {
                    TickAdvance::None
                } else {
                    TickAdvance::Loop
                };
            }
            RepeatMode::All => {
                if self.tracks.len() > 1 {
                    self.peek_step(1)
                } else {
                    return if self.current_is_native() {
                        TickAdvance::None
                    } else {
                        TickAdvance::Loop
                    };
                }
            }
            RepeatMode::Off => {
                if self.cursor.at_last() {
                    return TickAdvance::None;
                }
                self.peek_step(1)
            }
        };
        match target {
            // Nativa→nativa ya lo resolvió el hilo de audio: no duplicar.
            Some(t) if self.current_is_native() && self.path_is_native(t) => TickAdvance::None,
            Some(t) => TickAdvance::Switch(t),
            None => TickAdvance::None,
        }
    }

    pub(crate) fn set_speed(&mut self, speed: f32) {
        let s = speed.clamp(0.1, 4.0);
        self.speed = s;
        self.current.set_speed(s);
    }
}

impl AudioSource for Playlist {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.current.fill(buf, sample_rate, channels);
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

/// Foto no-bloqueante del estado de reproducción para la vista.
#[derive(Clone)]
pub(crate) struct PlaybackSnapshot {
    pub(crate) present: bool,
    pub(crate) position: Duration,
    pub(crate) duration: Option<Duration>,
    pub(crate) idx: usize,
    pub(crate) len: usize,
    pub(crate) speed: f32,
    pub(crate) repeat_label: &'static str,
    pub(crate) shuffle_on: bool,
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

pub(crate) fn playback_snapshot() -> PlaybackSnapshot {
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
            *cache.lock() = snap.clone();
            snap
        }
        None => cache.lock().clone(),
    }
}

pub(crate) fn current_audio_position() -> Option<Duration> {
    let s = playback_snapshot();
    s.present.then_some(s.position)
}

pub(crate) fn seek_audio_by(delta_secs: i64) {
    let Some(handle) = playlist_slot().get().and_then(|o| o.as_ref()) else {
        return;
    };
    let mut src = handle.lock();
    media_core::seek::by_wrapped(&mut *src, delta_secs);
    drop(src);
    reset_av_sync_anchor();
}

pub(crate) fn seek_audio_to(fraction: f32) {
    let Some(handle) = playlist_slot().get().and_then(|o| o.as_ref()) else {
        return;
    };
    let mut src = handle.lock();
    media_core::seek::to_fraction(&mut *src, fraction);
    drop(src);
    reset_av_sync_anchor();
}

pub(crate) fn seek_audio_to_pos(pos: Duration) {
    let Some(handle) = playlist_slot().get().and_then(|o| o.as_ref()) else {
        return;
    };
    let mut src = handle.lock();
    media_core::seek::to_pos(&mut *src, pos);
    drop(src);
    reset_av_sync_anchor();
}

pub(crate) fn current_track_key() -> Option<String> {
    let handle = playlist_slot().get().and_then(|o| o.as_ref())?;
    let pl = handle.try_lock()?;
    Some(pl.track_path().to_string_lossy().into_owned())
}

pub(crate) fn record_playback_progress(frame: u64) {
    let s = playback_snapshot();
    if !s.present {
        return;
    }
    if let Some(key) = current_track_key() {
        let key = crate::config_io::scoped_key(&key);
        crate::config_io::history()
            .lock()
            .update_position(&key, s.position, s.duration, crate::config_io::now_secs());
    }
    if frame % 150 == 0 {
        crate::config_io::save_history();
    }
}
