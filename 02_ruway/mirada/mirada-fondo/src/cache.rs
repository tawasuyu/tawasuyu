//! Cache de frames de un fondo Lottie/rive *bakeado*.
//!
//! Las superficies sin vello (splash, compositor) no pueden rasterizar un `Scene`
//! en caliente, así que un Lottie/rive se pre-renderiza **una vez** (feature
//! `bake`, en `super::bake`) a una secuencia de PNG en disco, y ellas **bliteant**
//! esos frames. Este módulo es la parte liviana —sin vello— que ambas leen, más
//! los helpers de escritura que usa el baker.
//!
//! ## Disposición en disco
//!
//! ```text
//! ~/.cache/mirada/fondo/<clave>/
//!   meta.ron        # CacheMeta: tamaño, fps, nº de frames, loop
//!   f000000.png     # frames RGBA, 6 dígitos, en orden
//!   f000001.png
//!   ...
//! ```
//!
//! La `<clave>` deriva de `(kind, ruta, mtime del asset, w, h, fps)`: si editás el
//! `.json`/`.ron` y re-bakeás, cae en otra carpeta y la vieja queda obsoleta (un
//! GC futuro la barre; hoy se puede borrar a mano). Así nunca se sirve un frame
//! viejo de un asset cambiado.

use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::FondoSpec;

/// Metadatos de una cache bakeada (lo único que hay que leer antes de blitear).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheMeta {
    /// Ancho de cada frame en píxeles.
    pub width: u32,
    /// Alto de cada frame en píxeles.
    pub height: u32,
    /// Cuadros por segundo a los que se bakeó (gobierna el mapeo tiempo→frame).
    pub fps: f32,
    /// Cantidad de frames en la secuencia.
    pub frame_count: u32,
    /// Duración del loop en segundos (= `frame_count / fps`). Redundante pero
    /// explícito para el consumidor.
    pub loop_secs: f32,
}

impl CacheMeta {
    /// Índice de frame para el instante `t` (segundos), con wrap (loop).
    pub fn frame_index_at(&self, t_secs: f32) -> u32 {
        if self.frame_count == 0 {
            return 0;
        }
        let f = (t_secs.max(0.0) * self.fps) as u64;
        (f % self.frame_count as u64) as u32
    }
}

/// Raíz de la cache: `$XDG_CACHE_HOME/mirada/fondo` o `~/.cache/mirada/fondo`.
/// Espeja la convención de `mirada-wallpaper` (`~/.cache/mirada/wallpaper`).
pub fn cache_root() -> PathBuf {
    if let Some(d) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(d).join("mirada").join("fondo");
    }
    directories::BaseDirs::new()
        .map(|b| b.cache_dir().join("mirada").join("fondo"))
        .unwrap_or_else(|| PathBuf::from("/tmp/mirada/fondo"))
}

/// Carpeta-cache determinística para `spec` a tamaño `w×h` y `fps`. Incluye el
/// `mtime` del asset en la clave para invalidar al editarlo. `Chakana` no usa
/// cache (devuelve igual una carpeta, pero nadie debería bakearla).
pub fn cache_dir(spec: &FondoSpec, w: u32, h: u32, fps: f32) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    spec.kind().hash(&mut hasher);
    if let Some(p) = spec.path() {
        p.hash(&mut hasher);
        // mtime del asset (si se puede leer) → re-bake automático al editar.
        if let Ok(meta) = std::fs::metadata(p) {
            if let Ok(modified) = meta.modified() {
                if let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH) {
                    dur.as_secs().hash(&mut hasher);
                    dur.subsec_nanos().hash(&mut hasher);
                }
            }
        }
    }
    w.hash(&mut hasher);
    h.hash(&mut hasher);
    fps.to_bits().hash(&mut hasher);
    let key = format!("{}-{:016x}", spec.kind(), hasher.finish());
    cache_root().join(key)
}

fn frame_path(dir: &Path, idx: u32) -> PathBuf {
    dir.join(format!("f{idx:06}.png"))
}

fn meta_path(dir: &Path) -> PathBuf {
    dir.join("meta.ron")
}

/// Lector de una cache ya bakeada. Liviano: sólo guarda la carpeta y el meta;
/// decodifica cada frame por demanda.
#[derive(Debug, Clone)]
pub struct FrameCache {
    dir: PathBuf,
    meta: CacheMeta,
}

impl FrameCache {
    /// Abre la cache en `dir` (debe tener `meta.ron`). Error si no existe o el
    /// meta no parsea.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        let txt = std::fs::read_to_string(meta_path(&dir))?;
        let meta: CacheMeta = ron::from_str(&txt)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("meta.ron: {e}")))?;
        Ok(FrameCache { dir, meta })
    }

    /// Abre la cache correspondiente a `spec` a tamaño/fps dados, si existe.
    pub fn open_for(spec: &FondoSpec, w: u32, h: u32, fps: f32) -> io::Result<Self> {
        Self::open(cache_dir(spec, w, h, fps))
    }

    /// ¿Existe una cache bakeada para `spec` a este tamaño/fps?
    pub fn is_baked(spec: &FondoSpec, w: u32, h: u32, fps: f32) -> bool {
        meta_path(&cache_dir(spec, w, h, fps)).is_file()
    }

    pub fn meta(&self) -> &CacheMeta {
        &self.meta
    }

    /// Frame `idx` decodificado a **BGRA** opaco (mismo orden que la chakana,
    /// listo para blitear/subir). Error si el frame falta o no decodifica.
    pub fn frame_bgra(&self, idx: u32) -> io::Result<Vec<u8>> {
        let p = frame_path(&self.dir, idx);
        let file = std::fs::File::open(&p)?;
        let decoder = png::Decoder::new(io::BufReader::new(file));
        let mut reader = decoder
            .read_info()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{p:?}: {e}")))?;
        let out_size = reader.output_buffer_size().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, format!("{p:?}: tamaño de buffer inválido"))
        })?;
        let mut buf = vec![0u8; out_size];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{p:?}: {e}")))?;
        buf.truncate(info.buffer_size());
        // El baker escribe RGBA8; convertimos a BGRA in place.
        match info.color_type {
            png::ColorType::Rgba => {
                for px in buf.chunks_exact_mut(4) {
                    px.swap(0, 2);
                }
            }
            png::ColorType::Rgb => {
                // Expandir RGB→BGRA opaco.
                let mut out = Vec::with_capacity(buf.len() / 3 * 4);
                for px in buf.chunks_exact(3) {
                    out.extend_from_slice(&[px[2], px[1], px[0], 255]);
                }
                buf = out;
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{p:?}: color inesperado {other:?}"),
                ));
            }
        }
        Ok(buf)
    }

    /// Frame para el instante `t` (segundos), con loop.
    pub fn frame_bgra_at(&self, t_secs: f32) -> io::Result<Vec<u8>> {
        self.frame_bgra(self.meta.frame_index_at(t_secs))
    }
}

// ---- escritura (la usa el baker; sólo PNG, sin vello) ----

/// Crea/vacía la carpeta-cache de `spec` y escribe `meta.ron`. Devuelve la
/// carpeta lista para recibir frames con [`write_frame_rgba`].
pub fn init_cache(spec: &FondoSpec, meta: &CacheMeta, fps: f32) -> io::Result<PathBuf> {
    let dir = cache_dir(spec, meta.width, meta.height, fps);
    // Empezar de cero: una cache a medio escribir es peor que ninguna.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let txt = ron::ser::to_string_pretty(meta, ron::ser::PrettyConfig::default())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    std::fs::write(meta_path(&dir), txt)?;
    Ok(dir)
}

/// Escribe el frame `idx` (bytes **RGBA8**, `w*h*4`) como PNG en `dir`.
pub fn write_frame_rgba(dir: &Path, idx: u32, rgba: &[u8], w: u32, h: u32) -> io::Result<()> {
    debug_assert_eq!(rgba.len(), (w * h * 4) as usize, "RGBA = w*h*4");
    let file = std::fs::File::create(frame_path(dir, idx))?;
    let mut encoder = png::Encoder::new(io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_index_hace_loop() {
        let m = CacheMeta {
            width: 4,
            height: 4,
            fps: 10.0,
            frame_count: 5,
            loop_secs: 0.5,
        };
        assert_eq!(m.frame_index_at(0.0), 0);
        assert_eq!(m.frame_index_at(0.25), 2); // 0.25*10 = 2.5 → 2
        assert_eq!(m.frame_index_at(0.5), 0); // loop
        assert_eq!(m.frame_index_at(0.55), 0); // 5.5 % 5 = 0
        assert_eq!(m.frame_index_at(-1.0), 0); // clamp
    }

    #[test]
    fn cache_dir_cambia_con_el_tamano() {
        let s = FondoSpec::Lottie { path: "/no/existe.json".into() };
        let a = cache_dir(&s, 100, 100, 30.0);
        let b = cache_dir(&s, 200, 200, 30.0);
        assert_ne!(a, b, "distinto tamaño → distinta carpeta");
        // estable: misma entrada → misma carpeta.
        assert_eq!(a, cache_dir(&s, 100, 100, 30.0));
    }

    #[test]
    fn escribe_y_lee_un_frame_bgra() {
        let dir = std::env::temp_dir().join(format!("fondo-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let meta = CacheMeta {
            width: 2,
            height: 1,
            fps: 1.0,
            frame_count: 1,
            loop_secs: 1.0,
        };
        let txt = ron::ser::to_string_pretty(&meta, ron::ser::PrettyConfig::default()).unwrap();
        std::fs::write(dir.join("meta.ron"), txt).unwrap();
        // Píxel 0 rojo, píxel 1 verde (RGBA).
        let rgba = [255u8, 0, 0, 255, 0, 255, 0, 255];
        write_frame_rgba(&dir, 0, &rgba, 2, 1).unwrap();

        let cache = FrameCache::open(&dir).unwrap();
        let bgra = cache.frame_bgra(0).unwrap();
        // Rojo RGBA (255,0,0,255) → BGRA (0,0,255,255).
        assert_eq!(&bgra[0..4], &[0, 0, 255, 255]);
        // Verde RGBA (0,255,0,255) → BGRA (0,255,0,255).
        assert_eq!(&bgra[4..8], &[0, 255, 0, 255]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
