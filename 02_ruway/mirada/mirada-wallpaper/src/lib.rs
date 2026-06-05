//! `mirada-wallpaper` — la **capa de decisión** del fondo de escritorio.
//!
//! mirada (el compositor) es dueño del píxel: decodifica una imagen y la pinta
//! por salida. *Qué* imagen y *cuándo* cambia es política, no compositor — y
//! vive acá. Este crate baja una imagen de un servicio público (Bing, NASA) o
//! una carpeta local, la cachea, y dispara el cambio en mirada **por el
//! contrato que ya existe**: reescribe `wallpaper_path` en `config.ron`
//! (preservando los comentarios, ver [`ron_edit`]). El `FileWatch` del
//! compositor recarga en caliente y, como el path nuevo difiere del viejo,
//! invalida su buffer y re-decodifica. mirada no necesita una sola línea nueva.
//!
//! Config propia en `~/.config/mirada/wallpaper.ron` (separada de la del
//! compositor: el daemon es dueño de sus ajustes). Las imágenes descargadas
//! se cachean en `~/.cache/mirada/wallpaper/`.

pub mod ron_edit;
pub mod source;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

pub use source::{Fetched, FetchCtx, WallpaperSource};

/// Ajustes del wallpaper automático (`~/.config/mirada/wallpaper.ron`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// De dónde sale la imagen.
    pub source: SourceCfg,
    /// Cadencia de refresco en modo daemon, en segundos. Default 6 h.
    pub interval_secs: u64,
    /// Conector DRM destino. `""` = el wallpaper **global** de `config.ron`.
    /// (v1 sólo soporta el global; ver [`run_once`].)
    pub output: String,
    /// Cuántas imágenes descargadas conservar por fuente (poda el resto).
    pub keep: usize,
}

/// La fuente elegida, como dato serializable. En RON va como variante con
/// campos nombrados: `source: Bing(market: "en-US", resolution: "1920x1080")`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SourceCfg {
    /// Bing — foto del día (sin API key).
    Bing { market: String, resolution: String },
    /// NASA Astronomy Picture of the Day (`DEMO_KEY` sirve para probar).
    Nasa { api_key: String },
    /// Rota por las imágenes de una carpeta local (offline).
    Folder { dir: String },
    /// Fondo según la posición del Sol (estilo "dynamic desktop"): elige la
    /// imagen de la fase del día (noche/amanecer/día/atardecer) para tu lat/lon.
    /// Offline — calcula la altura solar con cosmos, sin servicio externo.
    Solar {
        lat: f64,
        lon: f64,
        night: String,
        dawn: String,
        day: String,
        dusk: String,
    },
}

impl Default for SourceCfg {
    fn default() -> Self {
        SourceCfg::Bing {
            market: "en-US".into(),
            resolution: "1920x1080".into(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            source: SourceCfg::default(),
            interval_secs: 6 * 60 * 60,
            output: String::new(),
            keep: 8,
        }
    }
}

impl Config {
    /// La ruta canónica de la config del daemon: `~/.config/mirada/wallpaper.ron`.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "mirada")
            .map(|d| d.config_dir().join("wallpaper.ron"))
    }

    /// Carga la config; si el archivo no existe, escribe una plantilla
    /// documentada y devuelve los defaults. Si está corrupta, avisa y cae a
    /// los defaults — el daemon no debe morir por un typo.
    pub fn load_or_default(path: &Path) -> Config {
        if path.exists() {
            match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| {
                ron::from_str::<Config>(&t).map_err(|e| e.to_string())
            }) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("mirada-wallpaper · config «{}» inválida ({e}); uso defaults.", path.display());
                    Config::default()
                }
            }
        } else {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match std::fs::write(path, CONFIG_TEMPLATE) {
                Ok(()) => eprintln!("mirada-wallpaper · plantilla escrita en {}", path.display()),
                Err(e) => eprintln!("mirada-wallpaper · no pude escribir la plantilla: {e}"),
            }
            Config::default()
        }
    }

    /// Construye la fuente viva a partir de la config declarativa.
    pub fn build_source(&self) -> Box<dyn WallpaperSource> {
        match &self.source {
            SourceCfg::Bing { market, resolution } => Box::new(source::Bing {
                market: market.clone(),
                resolution: resolution.clone(),
            }),
            SourceCfg::Nasa { api_key } => Box::new(source::Nasa {
                api_key: api_key.clone(),
            }),
            SourceCfg::Folder { dir } => Box::new(source::Folder {
                dir: PathBuf::from(dir),
            }),
            SourceCfg::Solar { lat, lon, night, dawn, day, dusk } => Box::new(source::Solar {
                lat: *lat,
                lon: *lon,
                night: night.clone(),
                dawn: dawn.clone(),
                day: day.clone(),
                dusk: dusk.clone(),
            }),
        }
    }

    /// Prefijo de los archivos que esta fuente escribe al cache (para podar).
    /// `None` = la fuente no descarga (Folder apunta a archivos in situ).
    fn cache_prefix(&self) -> Option<&'static str> {
        match self.source {
            SourceCfg::Bing { .. } => Some("bing-"),
            SourceCfg::Nasa { .. } => Some("apod-"),
            // Folder y Solar apuntan a archivos in situ: no descargan, no podan.
            SourceCfg::Folder { .. } | SourceCfg::Solar { .. } => None,
        }
    }
}

/// El directorio de cache de imágenes: `~/.cache/mirada/wallpaper/`.
pub fn cache_dir() -> Result<PathBuf> {
    directories::ProjectDirs::from("", "", "mirada")
        .map(|d| d.cache_dir().join("wallpaper"))
        .ok_or_else(|| anyhow!("no pude resolver el directorio de cache (HOME?)"))
}

/// Resultado de un refresco.
pub enum Outcome {
    /// Se cambió el wallpaper al path indicado.
    Changed(PathBuf),
    /// La fuente devolvió la imagen que ya estaba puesta: no se tocó nada.
    Unchanged(PathBuf),
}

/// Hace **un** refresco: trae la imagen de la fuente, la cachea si hace falta,
/// y reescribe `wallpaper_path` en el `config.ron` de mirada. El compositor lo
/// recarga solo. Idempotente: si la imagen es la misma que ya está puesta, no
/// reescribe nada (evita una recarga inútil del compositor).
pub fn run_once(cfg: &Config) -> Result<Outcome> {
    if !cfg.output.is_empty() {
        return Err(anyhow!(
            "v1 sólo cambia el wallpaper global; dejá `output: \"\"` en wallpaper.ron \
             (los overrides por salida se editan a mano en config.ron)"
        ));
    }
    let cfg_path = mirada_brain::Config::default_path()
        .ok_or_else(|| anyhow!("no pude resolver la ruta de config.ron de mirada"))?;
    // Garantiza que config.ron exista (escribe la plantilla del compositor si no).
    let _ = mirada_brain::Config::load_or_default(&cfg_path);
    let text = std::fs::read_to_string(&cfg_path)
        .with_context(|| format!("leyendo {}", cfg_path.display()))?;
    let current = mirada_brain::Config::from_ron(&text)
        .map_err(|e| anyhow!("config.ron de mirada inválida: {e}"))?
        .wallpaper_path;

    let src = cfg.build_source();
    let fetched = src
        .fetch(&FetchCtx { current: Some(&current) })
        .with_context(|| format!("fuente: {}", src.label()))?;

    let final_path = match fetched {
        Fetched::Local(p) => p,
        Fetched::Bytes { ident, ext, bytes } => {
            let dir = cache_dir()?;
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("creando cache {}", dir.display()))?;
            let path = dir.join(format!("{ident}.{ext}"));
            write_atomic(&path, &bytes)
                .with_context(|| format!("escribiendo {}", path.display()))?;
            if let Some(prefix) = cfg.cache_prefix() {
                prune(&dir, prefix, cfg.keep);
            }
            path
        }
    };

    let final_str = final_path
        .to_str()
        .ok_or_else(|| anyhow!("path con bytes no-UTF8: {}", final_path.display()))?;
    if current == final_str {
        return Ok(Outcome::Unchanged(final_path));
    }

    let new_text = ron_edit::set_wallpaper_path(&text, final_str);
    // Red de seguridad: nunca escribir un config.ron que no reparsee.
    mirada_brain::Config::from_ron(&new_text)
        .map_err(|e| anyhow!("la edición dejó un config.ron inválido ({e}); no escribo"))?;
    write_atomic(&cfg_path, new_text.as_bytes())
        .with_context(|| format!("escribiendo {}", cfg_path.display()))?;
    Ok(Outcome::Changed(final_path))
}

/// Escribe `bytes` a `path` de forma atómica (tmp + rename) creando el
/// directorio padre si falta.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Deja sólo las `keep` imágenes más recientes (por nombre, descendente) que
/// empiecen con `prefix`; borra el resto. Best-effort: los errores se ignoran
/// (podar es housekeeping, no debe tumbar un refresco exitoso).
fn prune(dir: &Path, prefix: &str, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut matching: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(prefix))
                .unwrap_or(false)
        })
        .collect();
    // Nombres fechados (bing-YYYYMMDD, apod-YYYY-MM-DD) ordenan cronológico.
    matching.sort();
    matching.reverse();
    for old in matching.into_iter().skip(keep) {
        let _ = std::fs::remove_file(old);
    }
}

/// La plantilla de `wallpaper.ron` que se escribe la primera vez.
const CONFIG_TEMPLATE: &str = "\
// Config de mirada-wallpaper — el fondo de escritorio automático.
// `mirada-wallpaper now` hace un refresco; `mirada-wallpaper daemon` lo
// repite cada `interval_secs`. Cambia el wallpaper_path de ~/.config/mirada/
// config.ron, que el compositor recarga en caliente.
(
    // De dónde sale la imagen. Una de:
    //   Bing(market: \"en-US\", resolution: \"1920x1080\")  // foto del día, sin API key
    //   Nasa(api_key: \"DEMO_KEY\")                          // astrofoto del día
    //   Folder(dir: \"/home/yo/fondos\")                     // rota una carpeta local (offline)
    //   Solar(lat: -12.05, lon: -77.05,                     // \"dynamic desktop\": imagen
    //         night: \"/f/noche.jpg\", dawn: \"/f/amanecer.jpg\", //   por fase del día según
    //         day: \"/f/dia.jpg\", dusk: \"/f/atardecer.jpg\")  //   la altura del Sol (offline)
    // market: en-US, es-ES, ja-JP, …   resolution: 1920x1080, 1366x768, UHD (4K).
    source: Bing(market: \"en-US\", resolution: \"1920x1080\"),

    // Cada cuánto refresca el daemon (segundos). 21600 = 6 h.
    interval_secs: 21600,

    // Salida destino: \"\" = wallpaper global. (v1 sólo soporta el global.)
    output: \"\",

    // Cuántas imágenes descargadas conservar en cache (~/.cache/mirada/wallpaper).
    keep: 8,
)
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_plantilla_parsea_a_los_defaults() {
        let cfg: Config = ron::from_str(CONFIG_TEMPLATE).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn default_es_bing_en_us_6h() {
        let c = Config::default();
        assert_eq!(c.interval_secs, 21600);
        assert_eq!(c.keep, 8);
        assert!(matches!(c.source, SourceCfg::Bing { .. }));
    }

    #[test]
    fn source_round_trip_por_ron() {
        for s in [
            SourceCfg::Bing { market: "es-ES".into(), resolution: "UHD".into() },
            SourceCfg::Nasa { api_key: "DEMO_KEY".into() },
            SourceCfg::Folder { dir: "/f".into() },
        ] {
            let c = Config { source: s, ..Config::default() };
            let text = ron::to_string(&c).unwrap();
            let back: Config = ron::from_str(&text).unwrap();
            assert_eq!(c, back);
        }
    }

    #[test]
    fn prune_conserva_las_mas_recientes() {
        let dir = std::env::temp_dir().join(format!("mw-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for d in ["bing-20260601", "bing-20260602", "bing-20260603", "otra-x"] {
            std::fs::write(dir.join(format!("{d}.jpg")), b"x").unwrap();
        }
        prune(&dir, "bing-", 2);
        assert!(!dir.join("bing-20260601.jpg").exists(), "la más vieja se poda");
        assert!(dir.join("bing-20260602.jpg").exists());
        assert!(dir.join("bing-20260603.jpg").exists());
        assert!(dir.join("otra-x.jpg").exists(), "otros prefijos no se tocan");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
