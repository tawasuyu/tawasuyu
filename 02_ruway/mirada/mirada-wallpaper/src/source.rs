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

// ─────────────────────────── Solar (dynamic desktop) ───────────────────────

/// Fondo según la posición del Sol —el "dynamic desktop" de macOS, pero
/// **offline y exacto**: no consulta ningún servicio, calcula la altura solar
/// con [`cosmos_sundial`] para tu lat/lon y elige la imagen de la fase actual.
/// El signo del ángulo horario distingue amanecer (antes del mediodía solar)
/// de atardecer (después), que la altura sola no puede.
pub struct Solar {
    pub lat: f64,
    pub lon: f64,
    /// Imagen de cada fase (rutas locales). Una vacía cae a `day`.
    pub night: String,
    pub dawn: String,
    pub day: String,
    pub dusk: String,
}

/// Las cuatro fases del día solar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Night,
    Dawn,
    Day,
    Dusk,
}

impl Solar {
    fn path_for(&self, phase: Phase) -> Result<String> {
        let pick = match phase {
            Phase::Night => &self.night,
            Phase::Dawn => &self.dawn,
            Phase::Day => &self.day,
            Phase::Dusk => &self.dusk,
        };
        // Fase sin imagen → cae a `day`; si `day` también está vacío, error.
        let chosen = if pick.is_empty() { &self.day } else { pick };
        if chosen.is_empty() {
            return Err(anyhow!(
                "el provider solar no tiene imagen para la fase {phase:?} ni para `day`"
            ));
        }
        Ok(chosen.clone())
    }
}

impl WallpaperSource for Solar {
    fn label(&self) -> String {
        format!("Solar (dynamic desktop) en lat {:.3}, lon {:.3}", self.lat, self.lon)
    }

    fn fetch(&self, _ctx: &FetchCtx) -> Result<Fetched> {
        let loc = cosmos_core::Location::from_degrees(self.lat, self.lon, 0.0)
            .map_err(|e| anyhow!("ubicación inválida (lat {}, lon {}): {e:?}", self.lat, self.lon))?;
        let tdb = now_tdb()?;
        let r = cosmos_sundial::sundial_reading(&tdb, &loc);
        let phase = phase_for(r.sun.altitude_deg, r.hour_angle_deg);
        Ok(Fetched::Local(PathBuf::from(self.path_for(phase)?)))
    }
}

/// Clasifica la fase del día a partir de la altura del Sol y el ángulo horario.
/// Día con el Sol franco arriba (≥10°), noche bien debajo (≤-8°), y en el
/// crepúsculo el signo del HA decide: negativo (antes del mediodía solar) =
/// amanecer, positivo = atardecer. Pura para poder testearla sin efemérides.
fn phase_for(altitude_deg: f64, hour_angle_deg: f64) -> Phase {
    if altitude_deg >= 10.0 {
        Phase::Day
    } else if altitude_deg <= -8.0 {
        Phase::Night
    } else if hour_angle_deg < 0.0 {
        Phase::Dawn
    } else {
        Phase::Dusk
    }
}

/// El instante actual como [`TDB`](cosmos_time::TDB). Tratamos el UTC del
/// sistema como TDB: la diferencia es de segundos, irrelevante para decidir la
/// fase del día. Construye la fecha civil sin depender de `chrono`.
fn now_tdb() -> Result<cosmos_time::TDB> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| anyhow!("reloj del sistema antes de 1970: {e}"))?
        .as_secs() as i64;
    let (y, mo, d, h, mi, s) = civil_from_unix(secs);
    let jd = cosmos_time::JulianDate::from_calendar(y, mo, d, h, mi, s as f64);
    Ok(cosmos_time::TDB::from(jd))
}

/// Convierte segundos Unix (UTC) a fecha civil `(año, mes, día, hora, min, seg)`.
/// Algoritmo de Howard Hinnant (`civil_from_days`), válido para el calendario
/// gregoriano proléptico.
fn civil_from_unix(secs: i64) -> (i32, u8, u8, u8, u8, u8) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = (rem / 3600) as u8;
    let minute = ((rem % 3600) / 60) as u8;
    let second = (rem % 60) as u8;

    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = (y + if m <= 2 { 1 } else { 0 }) as i32;
    (year, m as u8, d, hour, minute, second)
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

    #[test]
    fn phase_clasifica_por_altura_y_ha() {
        // Sol franco arriba → día, sin importar el HA.
        assert_eq!(phase_for(45.0, -30.0), Phase::Day);
        assert_eq!(phase_for(45.0, 30.0), Phase::Day);
        // Bien debajo del horizonte → noche.
        assert_eq!(phase_for(-15.0, -30.0), Phase::Night);
        // Crepúsculo: el signo del HA separa amanecer de atardecer.
        assert_eq!(phase_for(2.0, -10.0), Phase::Dawn); // antes del mediodía
        assert_eq!(phase_for(2.0, 10.0), Phase::Dusk); // después
    }

    #[test]
    fn civil_from_unix_epoch_y_conocidos() {
        assert_eq!(civil_from_unix(0), (1970, 1, 1, 0, 0, 0));
        // 2026-06-05T05:54:36Z = 1780638876 (verificado).
        assert_eq!(civil_from_unix(1_780_638_876), (2026, 6, 5, 5, 54, 36));
        // Año bisiesto: 2024-02-29T12:00:00Z = 1709208000.
        assert_eq!(civil_from_unix(1_709_208_000), (2024, 2, 29, 12, 0, 0));
    }

    #[test]
    fn solar_path_cae_a_day_si_la_fase_esta_vacia() {
        let s = Solar {
            lat: -12.05,
            lon: -77.05,
            night: "noche.png".into(),
            dawn: String::new(), // vacía → cae a day
            day: "dia.png".into(),
            dusk: "tarde.png".into(),
        };
        assert_eq!(s.path_for(Phase::Night).unwrap(), "noche.png");
        assert_eq!(s.path_for(Phase::Dawn).unwrap(), "dia.png"); // fallback
        assert_eq!(s.path_for(Phase::Dusk).unwrap(), "tarde.png");
    }

    #[test]
    fn solar_path_error_si_fase_y_day_vacias() {
        let s = Solar {
            lat: 0.0,
            lon: 0.0,
            night: String::new(),
            dawn: String::new(),
            day: String::new(),
            dusk: String::new(),
        };
        assert!(s.path_for(Phase::Night).is_err());
    }
}
