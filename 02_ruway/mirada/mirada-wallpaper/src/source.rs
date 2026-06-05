//! Las **fuentes** de wallpaper: de dónde sale la imagen. Cada una implementa
//! [`WallpaperSource`] y es ciega a quién la pinta — devuelve bytes (o un path
//! local) y el orquestador ([`crate::run_once`]) se encarga del resto.
//!
//! Hoy: [`Bing`] (foto del día, sin API key), [`Nasa`] (Astronomy Picture of
//! the Day, `DEMO_KEY` sirve) y [`Folder`] (rota por una carpeta local, sin
//! red). Agregar una fuente nueva = un tipo más que implemente el trait.

use std::io::Read;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};

/// User-Agent navegueril: Bing sirve un placeholder si no lo ve.
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) mirada-wallpaper";
/// Techo de descarga (32 MiB) — un wallpaper razonable cabe de sobra y evita
/// que una respuesta hostil nos haga crecer sin límite.
const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// Contexto que el orquestador le pasa a la fuente al pedir imagen. Hoy sólo
/// el wallpaper vigente, para las fuentes que **rotan** (necesitan saber cuál
/// sigue). Las fuentes de red lo ignoran.
pub struct FetchCtx<'a> {
    pub current: Option<&'a str>,
}

/// Lo que una fuente entrega: bytes para cachear, o un archivo ya en disco.
pub enum Fetched {
    /// Imagen descargada. El orquestador la escribe al cache como
    /// `<ident>.<ext>`; `ident` debe ser estable y único por imagen (fecha o
    /// hash) para que dos descargas distintas no colisionen y una repetida no
    /// fuerce una recarga redundante.
    Bytes {
        ident: String,
        ext: String,
        bytes: Vec<u8>,
    },
    /// Un archivo que ya vive en disco; se apunta tal cual, sin copiar.
    Local(PathBuf),
}

/// Una fuente de imágenes de fondo.
pub trait WallpaperSource {
    /// Etiqueta legible (para logs y `mirada-wallpaper sources`).
    fn label(&self) -> String;
    /// Trae la próxima imagen. Puede hacer red (Bing/NASA) o E/S (Folder).
    fn fetch(&self, ctx: &FetchCtx) -> Result<Fetched>;
}

// ───────────────────────── HTTP (ureq, bloqueante) ─────────────────────────

fn http_get_reader(url: &str) -> Result<impl Read> {
    let resp = ureq::get(url)
        .set("User-Agent", UA)
        .call()
        .map_err(|e| anyhow!("GET {url}: {e}"))?;
    Ok(resp.into_reader().take(MAX_BYTES))
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    http_get_reader(url)?
        .read_to_end(&mut buf)
        .with_context(|| format!("leyendo {url}"))?;
    if buf.is_empty() {
        return Err(anyhow!("respuesta vacía de {url}"));
    }
    Ok(buf)
}

fn http_get_json(url: &str) -> Result<serde_json::Value> {
    let reader = http_get_reader(url)?;
    serde_json::from_reader(reader).with_context(|| format!("JSON inválido de {url}"))
}

// ─────────────────────────────── Bing ──────────────────────────────────────

/// Foto del día de Bing. Endpoint JSON público, sin API key.
pub struct Bing {
    /// Mercado/idioma: `en-US`, `es-ES`, `ja-JP`, … (afecta la curaduría).
    pub market: String,
    /// Resolución del archivo: `1920x1080`, `1366x768`, `UHD` (4K), …
    pub resolution: String,
}

impl WallpaperSource for Bing {
    fn label(&self) -> String {
        format!("Bing foto del día ({}, {})", self.market, self.resolution)
    }

    fn fetch(&self, _ctx: &FetchCtx) -> Result<Fetched> {
        let api = format!(
            "https://www.bing.com/HPImageArchive.aspx?format=js&idx=0&n=1&mkt={}",
            self.market
        );
        let json = http_get_json(&api)?;
        let (img_url, ident) = parse_bing(&json, &self.resolution)?;
        let bytes = http_get_bytes(&img_url)?;
        Ok(Fetched::Bytes {
            ident,
            ext: "jpg".into(),
            bytes,
        })
    }
}

/// Extrae (URL absoluta de la imagen, identificador estable) del JSON de Bing.
/// Separado de la red para poder testearlo offline.
fn parse_bing(json: &serde_json::Value, resolution: &str) -> Result<(String, String)> {
    let img = json
        .get("images")
        .and_then(|v| v.get(0))
        .ok_or_else(|| anyhow!("respuesta de Bing sin `images[0]`"))?;
    let urlbase = img
        .get("urlbase")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("imagen de Bing sin `urlbase`"))?;
    let startdate = img
        .get("startdate")
        .and_then(|v| v.as_str())
        .unwrap_or("hoy");
    let img_url = format!("https://www.bing.com{urlbase}_{resolution}.jpg");
    Ok((img_url, format!("bing-{startdate}")))
}

// ─────────────────────────────── NASA APOD ─────────────────────────────────

/// Astronomy Picture of the Day de la NASA. `DEMO_KEY` funciona con rate
/// limit bajo; una key gratis (api.nasa.gov) lo sube.
pub struct Nasa {
    pub api_key: String,
}

impl WallpaperSource for Nasa {
    fn label(&self) -> String {
        "NASA — Astronomy Picture of the Day".into()
    }

    fn fetch(&self, _ctx: &FetchCtx) -> Result<Fetched> {
        let api = format!(
            "https://api.nasa.gov/planetary/apod?api_key={}",
            self.api_key
        );
        let json = http_get_json(&api)?;
        let (img_url, ident, ext) = parse_nasa(&json)?;
        let bytes = http_get_bytes(&img_url)?;
        Ok(Fetched::Bytes { ident, ext, bytes })
    }
}

/// Extrae (URL de la imagen, identificador estable, extensión) del JSON de
/// APOD. Falla con mensaje claro si la entrada del día es un video.
fn parse_nasa(json: &serde_json::Value) -> Result<(String, String, String)> {
    let media = json.get("media_type").and_then(|v| v.as_str()).unwrap_or("");
    if media != "image" {
        return Err(anyhow!(
            "la APOD de hoy no es una imagen (media_type=«{media}»); reintenta mañana o usa otra fuente"
        ));
    }
    // `hdurl` es la versión grande; cae a `url` si no viene.
    let img_url = json
        .get("hdurl")
        .or_else(|| json.get("url"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("APOD sin `url`/`hdurl`"))?
        .to_string();
    let date = json.get("date").and_then(|v| v.as_str()).unwrap_or("hoy");
    let ext = img_url
        .rsplit('.')
        .next()
        .filter(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png"))
        .unwrap_or("jpg")
        .to_string();
    Ok((img_url, format!("apod-{date}"), ext))
}

// ─────────────────────────────── Folder ────────────────────────────────────

/// Rota por las imágenes de una carpeta local. Sin red: elige el archivo
/// siguiente al vigente (orden alfabético, circular). Útil offline.
pub struct Folder {
    pub dir: PathBuf,
}

impl WallpaperSource for Folder {
    fn label(&self) -> String {
        format!("Carpeta local ({})", self.dir.display())
    }

    fn fetch(&self, ctx: &FetchCtx) -> Result<Fetched> {
        let mut imgs: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .with_context(|| format!("abriendo carpeta {}", self.dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| is_image(p))
            .collect();
        imgs.sort();
        let next = pick_next(&imgs, ctx.current)
            .ok_or_else(|| anyhow!("la carpeta {} no tiene imágenes", self.dir.display()))?;
        Ok(Fetched::Local(next))
    }
}

fn is_image(p: &std::path::Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp")
    )
}

/// El siguiente path al `current` en la lista ordenada (circular). Si `current`
/// no está (o es `None`), arranca por el primero.
fn pick_next(imgs: &[PathBuf], current: Option<&str>) -> Option<PathBuf> {
    if imgs.is_empty() {
        return None;
    }
    let idx = current
        .and_then(|c| imgs.iter().position(|p| p.to_str() == Some(c)))
        .map(|i| (i + 1) % imgs.len())
        .unwrap_or(0);
    Some(imgs[idx].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bing_arma_url_e_ident() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"images":[{"startdate":"20260605","urlbase":"/th?id=OHR.Foo_EN-US1234","title":"Foo"}]}"#,
        )
        .unwrap();
        let (url, ident) = parse_bing(&json, "1920x1080").unwrap();
        assert_eq!(
            url,
            "https://www.bing.com/th?id=OHR.Foo_EN-US1234_1920x1080.jpg"
        );
        assert_eq!(ident, "bing-20260605");
    }

    #[test]
    fn parse_bing_resolucion_uhd() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"images":[{"startdate":"20260605","urlbase":"/th?id=X"}]}"#)
                .unwrap();
        let (url, _) = parse_bing(&json, "UHD").unwrap();
        assert!(url.ends_with("X_UHD.jpg"), "url: {url}");
    }

    #[test]
    fn parse_bing_sin_images_falla() {
        let json: serde_json::Value = serde_json::from_str(r#"{"images":[]}"#).unwrap();
        assert!(parse_bing(&json, "UHD").is_err());
    }

    #[test]
    fn parse_nasa_imagen_prefiere_hdurl() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"media_type":"image","date":"2026-06-05","url":"http://x/a.jpg","hdurl":"http://x/a_hd.png"}"#,
        )
        .unwrap();
        let (url, ident, ext) = parse_nasa(&json).unwrap();
        assert_eq!(url, "http://x/a_hd.png");
        assert_eq!(ident, "apod-2026-06-05");
        assert_eq!(ext, "png");
    }

    #[test]
    fn parse_nasa_video_falla_con_mensaje() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"media_type":"video","url":"http://y"}"#).unwrap();
        let err = parse_nasa(&json).unwrap_err().to_string();
        assert!(err.contains("no es una imagen"), "msg: {err}");
    }

    #[test]
    fn pick_next_es_circular() {
        let imgs = vec![
            PathBuf::from("/a/1.png"),
            PathBuf::from("/a/2.png"),
            PathBuf::from("/a/3.png"),
        ];
        assert_eq!(pick_next(&imgs, Some("/a/1.png")).unwrap(), PathBuf::from("/a/2.png"));
        assert_eq!(pick_next(&imgs, Some("/a/3.png")).unwrap(), PathBuf::from("/a/1.png"));
        // current desconocido → primero.
        assert_eq!(pick_next(&imgs, Some("/x.png")).unwrap(), PathBuf::from("/a/1.png"));
        assert_eq!(pick_next(&imgs, None).unwrap(), PathBuf::from("/a/1.png"));
        assert!(pick_next(&[], None).is_none());
    }
}
