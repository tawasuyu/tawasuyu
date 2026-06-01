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
//! ## Alcance de esta versión
//!
//! [`resolve`] pide a yt-dlp **un único formato muxeado** (`-f b`), así la
//! URL resultante es una sola entrada que ffmpeg abre directo. Los formatos
//! DASH con audio y video en URLs separadas (típico de YouTube > 720p)
//! quedan para una versión futura — necesitarían dos entradas en ffmpeg o
//! un muxeo previo. Si yt-dlp no está instalado, [`resolve`] devuelve
//! [`YtdlpError::Spawn`] y el caller cae a tratar la URL como directa.

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
    /// URL de stream directo, lista para el decoder de red (ffmpeg).
    pub stream_url: String,
    /// La URL de página original (para mostrar como etiqueta).
    pub source_url: String,
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

/// Resuelve `page_url` a una URL de stream directo invocando
/// `yt-dlp -f b -g`. Pide un único formato muxeado para que la salida sea
/// una sola URL que ffmpeg abre directo (ver el caveat del módulo sobre
/// DASH). Devuelve [`YtdlpError::Spawn`] si yt-dlp no está disponible — el
/// caller debería entonces tratar la URL como directa.
pub fn resolve(page_url: &str) -> Result<Resolved, YtdlpError> {
    let output = Command::new("yt-dlp")
        .args([
            "--no-playlist",
            "--quiet",
            "--no-warnings",
            // Un solo formato muxeado (audio+video en una URL).
            "-f",
            "b",
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
    let stream_url = stdout
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .ok_or(YtdlpError::Empty)?
        .to_string();

    Ok(Resolved {
        stream_url,
        source_url: page_url.to_string(),
    })
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
}
