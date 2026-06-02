//! foreign-av — puente al binario `ffmpeg` para decodificar cualquier
//! formato de video/audio que ffmpeg entienda.
//!
//! Vive en `shared/foreign-*` por la regla dura #4: los formatos ajenos
//! entran por puentes, nunca al núcleo de las apps. Es la única pieza
//! del workspace que sabe que el binario `ffmpeg` existe — el dominio
//! `media` lo ve como un `FrameSource` / `AudioSource` más.
//!
//! Es la única forma realista hoy de abrir MP4, WebM, MKV, MOV, FLV,
//! etc. con códecs patentados (H.264, H.265, AAC) o VP9 — que no tienen
//! decoders puro-Rust maduros. El formato NATIVO de gioser es AV1+Opus
//! (decode puro-Rust en `media-source-av1`); por eso este puente ofrece
//! además [`transcode_a_av1`]: ingerir lo ajeno transcodificándolo al
//! formato nativo de una vez, igual que `foreign-docx` normaliza al
//! formato nativo de pluma al importar.
//!
//! ## Modelo "un ffmpeg por archivo"
//!
//! Para no duplicar procesos cuando un archivo tiene audio Y video, hay
//! UN solo subprocess `ffmpeg` por [`MediaSession`]. El proceso decodea
//! ambos streams a fds extra (3 y 4) que se enchufan via `pre_exec` +
//! `dup2` antes del `exec`. El parent guarda los read-ends como
//! `File`s separados que entrega a [`FfmpegVideoSource`] y
//! [`FfmpegAudioSource`].
//!
//! Una `MediaSession` clonable (`Arc<Mutex<…>>`) coordina:
//! - Apertura: spawnea el subprocess único con los flags que correspondan
//!   a los streams presentes (`probe()` decide).
//! - Seek: mata el child, abre nuevos pipes, respawnea con `-ss N`,
//!   incrementa una `generation`. Cada source detecta el cambio de
//!   generación en su próximo `tick` / `fill` y agarra el read-end nuevo.
//! - Drop: garantiza kill del child aunque alguien panickee.
//!
//! Eso da N reproductores ↔ N procesos extra (no 2N).
//!
//! ## Caveats
//!
//! El read del pipe en `fill` / `tick` es bloqueante. ffmpeg decodea
//! mucho más rápido que realtime, así que el pipe normalmente tiene
//! datos listos; los primeros callbacks pueden tener un glitch corto.
//! Para producción con archivos lentos habría que adelantar la
//! decodificación en un thread; para MVP el bloqueo está mapeado.
//!
//! El crate es Unix-only por ahora — `pre_exec` + `dup2` son la forma
//! universal de pasar fds extra al child en POSIX, y Windows no tiene
//! equivalente directo. En no-Unix, [`MediaSession::open`] devuelve
//! [`FfmpegError::Unsupported`].

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use media_core::{AudioSource, FrameSource, Seekable};
use parking_lot::Mutex;
use serde::Deserialize;

#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

// ─── Errores ─────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum FfmpegError {
    /// `ffmpeg` o `ffprobe` no se pudo lanzar (no está en PATH, permisos…).
    Spawn(String),
    /// `ffprobe` salió con código != 0.
    Probe(String),
    Parse(String),
    NoStream(&'static str),
    Io(std::io::Error),
    /// El crate sólo soporta Unix por ahora.
    Unsupported,
}

impl std::fmt::Display for FfmpegError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(s) => write!(f, "no pude lanzar ffmpeg/ffprobe: {s}"),
            Self::Probe(s) => write!(f, "ffprobe falló: {s}"),
            Self::Parse(s) => write!(f, "no pude parsear metadata: {s}"),
            Self::NoStream(k) => write!(f, "el archivo no tiene stream {k}"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Unsupported => write!(
                f,
                "foreign-av requiere Unix (pre_exec + dup2)"
            ),
        }
    }
}

impl std::error::Error for FfmpegError {}

impl From<std::io::Error> for FfmpegError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// ─── Probe ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub duration: Duration,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
    /// Fuente de **audio separada** (DASH): cuando es `Some`, el video sale de
    /// `path` y el audio de esta otra ruta/URL, y la sesión los muxea con dos
    /// entradas de ffmpeg (`-i path -i audio_path`). `None` ⇒ todo sale de
    /// `path` (camino normal, una sola entrada). Lo arma [`probe_dash`].
    pub audio_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Deserialize)]
struct ProbeRoot {
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u16>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

/// Corre `ffprobe` y devuelve metadata de los streams primarios.
pub fn probe(path: impl AsRef<Path>) -> Result<MediaInfo, FfmpegError> {
    let p = path.as_ref();
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(p)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| FfmpegError::Spawn(e.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(FfmpegError::Probe(stderr.trim().to_string()));
    }
    let root: ProbeRoot = serde_json::from_slice(&output.stdout)
        .map_err(|e| FfmpegError::Parse(e.to_string()))?;

    let mut video: Option<VideoInfo> = None;
    let mut audio: Option<AudioInfo> = None;
    for s in &root.streams {
        match s.codec_type.as_deref() {
            Some("video") if video.is_none() => {
                let width = s.width.unwrap_or(0);
                let height = s.height.unwrap_or(0);
                let fps = parse_frame_rate(
                    s.avg_frame_rate.as_deref().or(s.r_frame_rate.as_deref()),
                )
                .unwrap_or(30.0);
                if width > 0 && height > 0 {
                    video = Some(VideoInfo { width, height, fps });
                }
            }
            Some("audio") if audio.is_none() => {
                let sr = s
                    .sample_rate
                    .as_deref()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                let ch = s.channels.unwrap_or(0);
                if sr > 0 && ch > 0 {
                    audio = Some(AudioInfo {
                        sample_rate: sr,
                        channels: ch,
                    });
                }
            }
            _ => {}
        }
    }

    let duration = root
        .format
        .and_then(|f| f.duration)
        .and_then(|s| s.parse::<f64>().ok())
        .map(Duration::from_secs_f64)
        .unwrap_or(Duration::ZERO);

    Ok(MediaInfo {
        path: p.to_path_buf(),
        duration,
        video,
        audio,
        audio_path: None,
    })
}

/// Arma un [`MediaInfo`] **DASH** a partir de dos fuentes separadas: una con el
/// video y otra con el audio (típico de yt-dlp `bv*+ba` en YouTube > 720p). Se
/// hace un `probe` de cada una y se combina: el video y la duración salen de
/// `video_src`, el audio de `audio_src`, y `audio_path` apunta a `audio_src`
/// para que [`MediaSession`] spawnee ffmpeg con dos entradas. El `path` queda
/// con la fuente de video.
pub fn probe_dash(
    video_src: impl AsRef<Path>,
    audio_src: impl AsRef<Path>,
) -> Result<MediaInfo, FfmpegError> {
    let v = probe(&video_src)?;
    let a = probe(&audio_src)?;
    Ok(MediaInfo {
        path: v.path,
        // El contenedor de audio suele declarar mejor su duración; usamos la
        // mayor de las dos como referencia del transporte.
        duration: v.duration.max(a.duration),
        video: v.video,
        audio: a.audio,
        audio_path: Some(audio_src.as_ref().to_path_buf()),
    })
}

fn parse_frame_rate(s: Option<&str>) -> Option<f32> {
    let s = s?;
    let (num, den) = s.split_once('/')?;
    let n: f32 = num.parse().ok()?;
    let d: f32 = den.parse().ok()?;
    if d.abs() < 1e-6 {
        return None;
    }
    Some(n / d)
}

// ─── MediaSession — un ffmpeg compartido por archivo ─────────────────────────

#[cfg(unix)]
struct SessionInner {
    info: MediaInfo,
    child: Option<Child>,
    /// Read-end del pipe de video. `take()` lo entrega al
    /// FfmpegVideoSource cuando este detecta cambio de generación.
    video_read: Option<File>,
    audio_read: Option<File>,
    /// Sample rate / channels activos en el subprocess vivo.
    audio_sr: u32,
    audio_ch: u16,
    generation: u64,
    start_offset: Duration,
}

#[cfg(not(unix))]
struct SessionInner {
    _info: MediaInfo,
}

/// Handle clonable a una sesión ffmpeg compartida por archivo. Los
/// sources de video y audio se construyen contra una misma session,
/// y un seek (o reapertura) sobre cualquiera de ellos respawnea el
/// único subprocess y entrega nuevos pipes a ambos.
#[derive(Clone)]
pub struct MediaSession {
    inner: Arc<Mutex<SessionInner>>,
}

impl MediaSession {
    /// Abre el archivo y arranca el primer subprocess. En Unix
    /// crea pipes para video y audio (los presentes según
    /// `info.video`/`info.audio`) y los enchufa a fds 3/4.
    pub fn open(info: MediaInfo) -> Result<Self, FfmpegError> {
        let audio_sr = info.audio.map(|a| a.sample_rate).unwrap_or(0);
        let audio_ch = info.audio.map(|a| a.channels).unwrap_or(0);
        #[cfg(unix)]
        {
            let mut inner = SessionInner {
                info,
                child: None,
                video_read: None,
                audio_read: None,
                audio_sr,
                audio_ch,
                generation: 0,
                start_offset: Duration::ZERO,
            };
            spawn_into(&mut inner, Duration::ZERO)?;
            Ok(Self {
                inner: Arc::new(Mutex::new(inner)),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (audio_sr, audio_ch, info);
            Err(FfmpegError::Unsupported)
        }
    }

    pub fn info(&self) -> MediaInfo {
        self.inner.lock().info.clone()
    }
}

impl Drop for SessionInner {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(unix)]
fn spawn_into(inner: &mut SessionInner, from: Duration) -> Result<(), FfmpegError> {
    // Mata el subprocess anterior antes de tirar pipes.
    if let Some(mut c) = inner.child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    inner.video_read = None;
    inner.audio_read = None;

    let want_video = inner.info.video.is_some();
    let want_audio = inner.info.audio.is_some();
    if !want_video && !want_audio {
        return Err(FfmpegError::NoStream("video|audio"));
    }

    // Pipes — read ends con CLOEXEC para que no se filtren al child.
    let (video_r, video_w) = if want_video {
        let (r, w) = make_pipe()?;
        set_cloexec(r.as_raw_fd())?;
        (Some(r), Some(w))
    } else {
        (None, None)
    };
    let (audio_r, audio_w) = if want_audio {
        let (r, w) = make_pipe()?;
        set_cloexec(r.as_raw_fd())?;
        (Some(r), Some(w))
    } else {
        (None, None)
    };

    let vw_fd: RawFd = video_w.as_ref().map(|f| f.as_raw_fd()).unwrap_or(-1);
    let aw_fd: RawFd = audio_w.as_ref().map(|f| f.as_raw_fd()).unwrap_or(-1);

    // DASH: el audio viene de una segunda entrada (`-i audio_path`); el `-map`
    // del audio apunta entonces a la entrada 1. Sin `audio_path` todo sale de
    // la entrada 0 (camino normal, intacto).
    let dash_audio = inner.info.audio_path.clone();
    let audio_input = if dash_audio.is_some() { 1 } else { 0 };
    let ss = format!("{:.3}", from.as_secs_f64());

    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-loglevel", "error", "-nostdin", "-ss"])
        .arg(&ss)
        .arg("-i")
        .arg(&inner.info.path);
    if let Some(audio_path) = &dash_audio {
        // Segundo input con su propio `-ss` para que arranque sincronizado
        // con el video tras un seek.
        cmd.arg("-ss").arg(&ss).arg("-i").arg(audio_path);
    }
    if want_video {
        cmd.args(["-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:3"]);
    }
    if want_audio {
        let sr = inner.audio_sr.to_string();
        let ch = inner.audio_ch.to_string();
        cmd.arg("-map")
            .arg(format!("{audio_input}:a:0"))
            .arg("-vn")
            .arg("-ar")
            .arg(sr)
            .arg("-ac")
            .arg(ch)
            .args(["-f", "f32le", "pipe:4"]);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // SAFETY: la closure `pre_exec` corre en el child entre fork y exec.
    // Sólo invoca syscalls async-signal-safe (dup2, close). No usa
    // alloc, no toca locks compartidos. Los fds capturados por valor
    // siguen vivos porque las `File`s correspondientes vivieron hasta
    // después del `spawn`.
    unsafe {
        cmd.pre_exec(move || {
            if vw_fd >= 0 {
                if libc::dup2(vw_fd, 3) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::close(vw_fd);
            }
            if aw_fd >= 0 {
                if libc::dup2(aw_fd, 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::close(aw_fd);
            }
            Ok(())
        });
    }

    let child = cmd
        .spawn()
        .map_err(|e| FfmpegError::Spawn(e.to_string()))?;
    // Cierra los write-ends en el parent — el child ya tiene su copia
    // via dup2. Dejarlos abiertos en parent evitaría que llegue EOF.
    drop(video_w);
    drop(audio_w);

    inner.child = Some(child);
    inner.video_read = video_r;
    inner.audio_read = audio_r;
    inner.start_offset = from;
    inner.generation = inner.generation.wrapping_add(1);
    Ok(())
}

#[cfg(unix)]
fn make_pipe() -> std::io::Result<(File, File)> {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: pipe() llenó fds con descriptors válidos que ahora
    // poseemos. From_raw_fd toma ownership.
    let r = unsafe { File::from_raw_fd(fds[0]) };
    let w = unsafe { File::from_raw_fd(fds[1]) };
    Ok((r, w))
}

#[cfg(unix)]
fn set_cloexec(fd: RawFd) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

// ─── FfmpegVideoSource ───────────────────────────────────────────────────────

/// Source de video sobre una [`MediaSession`]. Lee del pipe de video
/// y produce frames RGBA con el frame rate detectado por probe.
pub struct FfmpegVideoSource {
    session: MediaSession,
    width: u32,
    height: u32,
    fps: f32,
    pipe: Option<File>,
    seen_generation: u64,
    start_offset: Duration,
    accum_since_frame: Duration,
    frames_emitted: u64,
    raw_buf: Vec<u8>,
    exhausted: bool,
}

impl FfmpegVideoSource {
    pub fn from_session(session: MediaSession) -> Result<Self, FfmpegError> {
        let info = session.info();
        let v = info.video.ok_or(FfmpegError::NoStream("video"))?;
        #[cfg(unix)]
        {
            let mut s = session.inner.lock();
            let pipe = s.video_read.take();
            let gen = s.generation;
            let start_offset = s.start_offset;
            drop(s);
            Ok(Self {
                session,
                width: v.width,
                height: v.height,
                fps: v.fps,
                pipe,
                seen_generation: gen,
                start_offset,
                accum_since_frame: Duration::ZERO,
                frames_emitted: 0,
                raw_buf: Vec::new(),
                exhausted: false,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = v;
            Err(FfmpegError::Unsupported)
        }
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn fps(&self) -> f32 {
        self.fps
    }

    /// Si la session respawneó (otro source seekeó), agarra el nuevo
    /// pipe y resetea contadores. Idempotente cuando la generación no
    /// cambió.
    fn refresh_if_needed(&mut self) {
        #[cfg(unix)]
        {
            let mut s = self.session.inner.lock();
            if s.generation != self.seen_generation {
                self.pipe = s.video_read.take();
                self.seen_generation = s.generation;
                self.start_offset = s.start_offset;
                self.frames_emitted = 0;
                self.accum_since_frame = Duration::ZERO;
                self.exhausted = false;
            }
        }
    }
}

impl FrameSource for FfmpegVideoSource {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        self.refresh_if_needed();
        if self.exhausted {
            return None;
        }
        self.accum_since_frame += dt;
        let frame_interval = Duration::from_secs_f32(1.0 / self.fps.max(1.0));
        if self.accum_since_frame < frame_interval {
            return None;
        }
        self.accum_since_frame -= frame_interval;

        let frame_bytes = (self.width as usize) * (self.height as usize) * 4;
        if self.raw_buf.len() != frame_bytes {
            self.raw_buf.resize(frame_bytes, 0);
        }
        let Some(pipe) = self.pipe.as_mut() else {
            return None;
        };
        match pipe.read_exact(&mut self.raw_buf) {
            Ok(()) => {
                if buf.len() != frame_bytes {
                    buf.resize(frame_bytes, 0);
                }
                buf.copy_from_slice(&self.raw_buf);
                self.frames_emitted += 1;
                Some((self.width, self.height))
            }
            Err(_) => {
                self.exhausted = true;
                None
            }
        }
    }
}

impl Seekable for FfmpegVideoSource {
    fn position(&self) -> Duration {
        let from_spawn = Duration::from_secs_f64(
            self.frames_emitted as f64 / self.fps.max(1.0) as f64,
        );
        self.start_offset + from_spawn
    }

    fn duration(&self) -> Option<Duration> {
        let d = self.session.info().duration;
        if d.is_zero() {
            None
        } else {
            Some(d)
        }
    }

    fn seek_to(&mut self, pos: Duration) {
        #[cfg(unix)]
        {
            let dur = self.session.info().duration;
            let clamped = clamp_pos(pos, dur);
            let mut s = self.session.inner.lock();
            let _ = spawn_into(&mut s, clamped);
            // refresh_if_needed agarra el nuevo pipe en el próximo tick.
        }
    }
}

// ─── FfmpegAudioSource ───────────────────────────────────────────────────────

/// Source de audio sobre una [`MediaSession`]. Lee del pipe de audio
/// y entrega samples f32le interleaved.
pub struct FfmpegAudioSource {
    session: MediaSession,
    src_sample_rate: u32,
    src_channels: u16,
    pipe: Option<File>,
    seen_generation: u64,
    start_offset: Duration,
    samples_read: u64,
    raw_buf: Vec<u8>,
    exhausted: bool,
}

impl FfmpegAudioSource {
    pub fn from_session(session: MediaSession) -> Result<Self, FfmpegError> {
        let info = session.info();
        let a = info.audio.ok_or(FfmpegError::NoStream("audio"))?;
        #[cfg(unix)]
        {
            let mut s = session.inner.lock();
            let pipe = s.audio_read.take();
            let gen = s.generation;
            let start_offset = s.start_offset;
            drop(s);
            Ok(Self {
                session,
                src_sample_rate: a.sample_rate,
                src_channels: a.channels,
                pipe,
                seen_generation: gen,
                start_offset,
                samples_read: 0,
                raw_buf: Vec::new(),
                exhausted: false,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = a;
            Err(FfmpegError::Unsupported)
        }
    }

    pub fn source_sample_rate(&self) -> u32 {
        self.src_sample_rate
    }

    pub fn source_channels(&self) -> u16 {
        self.src_channels
    }

    fn refresh_if_needed(&mut self) {
        #[cfg(unix)]
        {
            let mut s = self.session.inner.lock();
            if s.generation != self.seen_generation {
                self.pipe = s.audio_read.take();
                self.seen_generation = s.generation;
                self.start_offset = s.start_offset;
                self.samples_read = 0;
                self.exhausted = false;
            }
        }
    }
}

impl AudioSource for FfmpegAudioSource {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.refresh_if_needed();
        // Si el sink cambió sample_rate/channels respecto del último
        // spawn, respawnea con los nuevos.
        #[cfg(unix)]
        if sample_rate != self.src_sample_rate || channels != self.src_channels {
            let mut s = self.session.inner.lock();
            s.audio_sr = sample_rate;
            s.audio_ch = channels;
            let from = self.start_offset;
            let _ = spawn_into(&mut s, from);
            drop(s);
            self.src_sample_rate = sample_rate;
            self.src_channels = channels;
            self.refresh_if_needed();
        }
        if self.exhausted {
            for s in buf.iter_mut() {
                *s = 0.0;
            }
            return;
        }
        let nbytes = buf.len() * std::mem::size_of::<f32>();
        if self.raw_buf.len() != nbytes {
            self.raw_buf.resize(nbytes, 0);
        }
        let Some(pipe) = self.pipe.as_mut() else {
            for s in buf.iter_mut() {
                *s = 0.0;
            }
            return;
        };
        match pipe.read_exact(&mut self.raw_buf) {
            Ok(()) => {
                for (i, chunk) in self.raw_buf.chunks_exact(4).enumerate() {
                    buf[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                }
                self.samples_read += buf.len() as u64;
            }
            Err(_) => {
                self.exhausted = true;
                for s in buf.iter_mut() {
                    *s = 0.0;
                }
            }
        }
    }
}

impl Seekable for FfmpegAudioSource {
    fn position(&self) -> Duration {
        let ch = self.src_channels.max(1) as u64;
        let frames = self.samples_read / ch;
        let from_spawn =
            Duration::from_secs_f64(frames as f64 / self.src_sample_rate.max(1) as f64);
        self.start_offset + from_spawn
    }

    fn duration(&self) -> Option<Duration> {
        let d = self.session.info().duration;
        if d.is_zero() {
            None
        } else {
            Some(d)
        }
    }

    fn seek_to(&mut self, pos: Duration) {
        #[cfg(unix)]
        {
            let dur = self.session.info().duration;
            let clamped = clamp_pos(pos, dur);
            let mut s = self.session.inner.lock();
            let _ = spawn_into(&mut s, clamped);
        }
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn clamp_pos(pos: Duration, dur: Duration) -> Duration {
    if dur.is_zero() {
        return pos;
    }
    if pos >= dur {
        dur.saturating_sub(Duration::from_millis(100))
    } else {
        pos
    }
}

// ─── Transcode a AV1+Opus — ingesta al formato nativo ────────────────────────

/// Transcodifica cualquier entrada que ffmpeg entienda a un MKV con
/// video **AV1** (`libsvtav1`) y audio **Opus** (`libopus`) — el formato
/// de medios nativo de gioser (PLAN.md §6.quinquies). Es el camino
/// "ingerir lo ajeno y normalizar de una vez": tras correrlo, el archivo
/// resultante se reproduce con el decoder puro-Rust de `media-source-av1`
/// sin volver a tocar ffmpeg.
///
/// `crf` es el factor de calidad de SVT-AV1 (0–63; ~30 razonable, menor =
/// mejor calidad / archivo más grande). Bloquea hasta que ffmpeg termina.
///
/// Falla con [`FfmpegError::Spawn`] si ffmpeg no está en PATH, o
/// [`FfmpegError::Probe`] si el proceso sale con código != 0 (reusa la
/// variante para no multiplicar tipos — el mensaje trae el stderr).
pub fn transcode_a_av1(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    crf: u8,
) -> Result<(), FfmpegError> {
    let crf = crf.min(63);
    let output = output.as_ref();
    let status = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-nostdin", "-y", "-i"])
        .arg(input.as_ref())
        .args([
            "-c:v",
            "libsvtav1",
            "-crf",
            &crf.to_string(),
            "-c:a",
            "libopus",
            "-b:a",
            "128k",
        ])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| FfmpegError::Spawn(e.to_string()))?;
    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr).trim().to_string();
        return Err(FfmpegError::Probe(stderr));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frame_rate_basic() {
        assert_eq!(parse_frame_rate(Some("30/1")), Some(30.0));
        assert!(parse_frame_rate(Some("30000/1001")).unwrap() - 29.97 < 0.01);
        assert_eq!(parse_frame_rate(Some("0/0")), None);
        assert_eq!(parse_frame_rate(None), None);
    }

    #[test]
    fn clamp_pos_within() {
        let dur = Duration::from_secs(60);
        assert_eq!(clamp_pos(Duration::from_secs(30), dur), Duration::from_secs(30));
        let r = clamp_pos(Duration::from_secs(120), dur);
        assert!(r < dur);
        assert_eq!(
            clamp_pos(Duration::from_secs(5), Duration::ZERO),
            Duration::from_secs(5)
        );
    }
}
