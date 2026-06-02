//! foreign-ytdlp — puente al binario `yt-dlp` para resolver una página de
//! plataforma (YouTube, Vimeo, Twitch, SoundCloud…) a una **URL de stream
//! directo**, que después abre el decoder de red de `media` (R1: ffmpeg /
//! libavformat).
//!
//! Vive en `shared/foreign-*` por la regla dura #4: los protocolos/servicios
//! ajenos entran por puentes, nunca al núcleo de las apps. Es la única pieza
//! del workspace que sabe que el binario `yt-dlp` existe — el dominio `media`
//! ve sólo una URL más (igual que [`foreign-av`](../foreign_av) lo ve como
//! una `FrameSource`/`AudioSource`).
//!
//! R2 de `PARIDAD.md`. Es la pieza que, combinada con R1 (URL → ffmpeg),
//! deja reproducir desde una plataforma: la página se resuelve acá a su
//! stream directo y se enchufa al mismo camino de red.
//!
//! ## Modos de resolución
//!
//! - [`resolve`] pide a yt-dlp **un único formato muxeado** (`-f b`): una sola
//!   URL que ffmpeg abre directo. Tope ~720p en YouTube (es lo único que
//!   ofrece muxeado), pero es el camino más simple y robusto.
//! - [`resolve_best`] pide el **mejor video + mejor audio** (`-f bv*+ba/b`):
//!   en plataformas DASH (YouTube > 720p) yt-dlp devuelve **dos** URLs
//!   separadas (video y audio), que el caller enchufa como dos entradas a
//!   ffmpeg (`MediaSession` DASH de `foreign-av`). Si sólo hay muxeado, cae a
//!   una sola URL (igual que [`resolve`]).
//!
//! Si yt-dlp no está instalado, ambas devuelven [`YtdlpError::Spawn`] y el
//! caller cae a tratar la URL como directa.

use std::process::{Command, Stdio};

/// Hosts de plataforma conocidos. La detección [`is_platform_url`] hace
/// match exacto o por sufijo de dominio (`*.youtube.com`). No es
/// exhaustiva: yt-dlp soporta cientos de sitios, pero acá sólo decidimos
/// "¿vale la pena invocar a yt-dlp?" para no spawnearlo sobre URLs que ya
/// son streams directos. El caller siempre puede forzar la resolución.
const PLATFORM_HOSTS: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "youtube-nocookie.com",
    "vimeo.com",
    "twitch.tv",
    "dailymotion.com",
    "soundcloud.com",
    "bilibili.com",
    "odysee.com",
    "rumble.com",
    "facebook.com",
    "tiktok.com",
    "twitter.com",
    "x.com",
    "reddit.com",
    "bandcamp.com",
    "nicovideo.jp",
    "peertube.tv",
];

/// Qué puede salir mal al resolver con yt-dlp.
#[derive(Debug)]
pub enum YtdlpError {
    /// No se pudo lanzar `yt-dlp` (no instalado / no en PATH).
    Spawn(String),
    /// yt-dlp corrió pero devolvió error (URL no soportada, video privado,
    /// región bloqueada…). Trae el stderr recortado.
    Resolve(String),
    /// yt-dlp terminó OK pero no imprimió ninguna URL utilizable.
    Empty,
}

impl std::fmt::Display for YtdlpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            YtdlpError::Spawn(e) => write!(f, "no pude ejecutar yt-dlp: {e}"),
            YtdlpError::Resolve(e) => write!(f, "yt-dlp no pudo resolver: {e}"),
            YtdlpError::Empty => write!(f, "yt-dlp no devolvió ninguna URL"),
        }
    }
}

impl std::error::Error for YtdlpError {}

/// Resultado de resolver una página de plataforma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// URL de stream directo de video (o muxeado, si `audio_url` es `None`),
    /// lista para el decoder de red (ffmpeg).
    pub stream_url: String,
    /// URL de stream directo de **audio**, presente sólo cuando la plataforma
    /// entregó audio y video en URLs separadas (DASH, p. ej. YouTube > 720p).
    /// `None` ⇒ `stream_url` ya es muxeado (audio+video en una sola entrada).
    pub audio_url: Option<String>,
    /// La URL de página original (para mostrar como etiqueta).
    pub source_url: String,
}

impl Resolved {
    /// `true` si el resultado son dos streams separados (DASH) que el caller
    /// debe muxear con dos entradas de ffmpeg.
    pub fn is_dash(&self) -> bool {
        self.audio_url.is_some()
    }
}

/// Extrae el host (en minúsculas, sin userinfo ni puerto) de una URL
/// `esquema://host[:port]/...`. `None` si no hay parte `://` o host.
fn host_of(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    // El host va hasta el primer '/', '?' o '#'.
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|s| !s.is_empty())?;
    // Descarta userinfo (`user:pass@host`) y el puerto (`host:port`).
    let after_userinfo = authority.rsplit('@').next().unwrap_or(authority);
    let host = after_userinfo.split(':').next().unwrap_or(after_userinfo);
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// `true` si la URL apunta a un host de plataforma conocido
/// ([`PLATFORM_HOSTS`]), por igualdad o sufijo de dominio (`www.youtube.com`
/// matchea `youtube.com`). Lo usa el caller para decidir si conviene
/// invocar a yt-dlp en vez de pasar la URL directo a ffmpeg.
pub fn is_platform_url(url: &str) -> bool {
    let Some(host) = host_of(url) else {
        return false;
    };
    PLATFORM_HOSTS.iter().any(|p| {
        host == *p || host.ends_with(&{
            let mut s = String::with_capacity(p.len() + 1);
            s.push('.');
            s.push_str(p);
            s
        })
    })
}

/// Parsea la salida de `yt-dlp -g` (una URL directa por línea) en un
/// [`Resolved`]. Una sola línea no vacía ⇒ stream muxeado; **dos o más** ⇒
/// DASH: yt-dlp imprime primero el video y después el audio, así que
/// `stream_url` = 1ª línea y `audio_url` = 2ª. Función pura → testeable sin
/// invocar el binario.
fn parse_g_output(stdout: &str, page_url: &str) -> Result<Resolved, YtdlpError> {
    let urls: Vec<String> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    match urls.as_slice() {
        [] => Err(YtdlpError::Empty),
        [muxed] => Ok(Resolved {
            stream_url: muxed.clone(),
            audio_url: None,
            source_url: page_url.to_string(),
        }),
        // DASH: video primero, audio después (orden de `bv*+ba`). Líneas
        // extra (raro) se ignoran.
        [video, audio, ..] => Ok(Resolved {
            stream_url: video.clone(),
            audio_url: Some(audio.clone()),
            source_url: page_url.to_string(),
        }),
    }
}

/// Lanza `yt-dlp -g` con un selector de formato dado y parsea su salida.
fn run_yt_dlp(format: &str, page_url: &str) -> Result<Resolved, YtdlpError> {
    let output = Command::new("yt-dlp")
        .args([
            "--no-playlist",
            "--quiet",
            "--no-warnings",
            "-f",
            format,
            // Imprime sólo la(s) URL(s) directa(s).
            "-g",
            page_url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| YtdlpError::Spawn(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(YtdlpError::Resolve(stderr.trim().to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_g_output(&stdout, page_url)
}

/// Resuelve `page_url` a una **única URL muxeada** (`yt-dlp -f b -g`): audio y
/// video en una sola entrada que ffmpeg abre directo. Tope ~720p en YouTube.
/// Devuelve [`YtdlpError::Spawn`] si yt-dlp no está disponible — el caller
/// debería entonces tratar la URL como directa. El resultado nunca trae
/// `audio_url` (`is_dash()` es `false`).
pub fn resolve(page_url: &str) -> Result<Resolved, YtdlpError> {
    run_yt_dlp("b", page_url)
}

/// Resuelve `page_url` pidiendo el **mejor video + mejor audio**
/// (`yt-dlp -f bv*+ba/b -g`). En plataformas DASH (YouTube > 720p) devuelve un
/// [`Resolved`] con `stream_url` (video) y `audio_url` (audio) separados, que
/// el caller muxea con dos entradas de ffmpeg; si sólo hay muxeado, cae a una
/// sola URL (`is_dash()` = `false`). Misma semántica de error que [`resolve`].
pub fn resolve_best(page_url: &str) -> Result<Resolved, YtdlpError> {
    run_yt_dlp("bv*+ba/b", page_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_extrae_y_normaliza() {
        assert_eq!(host_of("https://www.YouTube.com/watch?v=x").as_deref(), Some("www.youtube.com"));
        assert_eq!(host_of("http://user:pass@host.tv:8080/live").as_deref(), Some("host.tv"));
        assert_eq!(host_of("rtsp://10.0.0.2/stream").as_deref(), Some("10.0.0.2"));
        assert_eq!(host_of("/ruta/local.mp4"), None);
        assert_eq!(host_of("https:///nohost"), None);
    }

    #[test]
    fn detecta_plataformas_conocidas() {
        for u in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://vimeo.com/12345",
            "https://www.twitch.tv/somechannel",
            "https://m.youtube.com/watch?v=x",
        ] {
            assert!(is_platform_url(u), "debería ser plataforma: {u}");
        }
    }

    #[test]
    fn ignora_streams_directos_y_otros_hosts() {
        for u in [
            "https://cdn.example.com/video.m3u8",
            "http://192.168.1.50:8080/live.ts",
            "rtsp://cam.local/stream",
            "https://notyoutube.evil.com/watch", // sufijo no matchea host distinto
            "/ruta/local.mp4",
        ] {
            assert!(!is_platform_url(u), "no debería ser plataforma: {u}");
        }
    }

    #[test]
    fn sufijo_no_se_confunde_con_substring() {
        // "fakeyoutube.com" NO debe matchear "youtube.com".
        assert!(!is_platform_url("https://fakeyoutube.com/x"));
        // pero el subdominio legítimo sí.
        assert!(is_platform_url("https://music.youtube.com/x"));
    }

    #[test]
    fn parse_una_url_es_muxeado() {
        let r = parse_g_output("https://cdn/v.mp4\n", "https://yt/x").unwrap();
        assert_eq!(r.stream_url, "https://cdn/v.mp4");
        assert_eq!(r.audio_url, None);
        assert!(!r.is_dash());
        assert_eq!(r.source_url, "https://yt/x");
    }

    #[test]
    fn parse_dos_urls_es_dash_video_luego_audio() {
        // yt-dlp con bv*+ba imprime video y después audio.
        let out = "https://cdn/video-1080.m4s\nhttps://cdn/audio.m4s\n";
        let r = parse_g_output(out, "https://yt/x").unwrap();
        assert!(r.is_dash());
        assert_eq!(r.stream_url, "https://cdn/video-1080.m4s");
        assert_eq!(r.audio_url.as_deref(), Some("https://cdn/audio.m4s"));
    }

    #[test]
    fn parse_ignora_lineas_vacias_y_extra() {
        let out = "\n  https://cdn/v\n\nhttps://cdn/a\nhttps://cdn/sobra\n";
        let r = parse_g_output(out, "p").unwrap();
        assert_eq!(r.stream_url, "https://cdn/v");
        assert_eq!(r.audio_url.as_deref(), Some("https://cdn/a"));
    }

    #[test]
    fn parse_vacio_es_error() {
        assert!(matches!(parse_g_output("\n  \n", "p"), Err(YtdlpError::Empty)));
    }
}
