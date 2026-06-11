//! `llimphi-image` — decode pipeline para `peniko::Image`.
//!
//! Hoy el caller que quiere mostrar una imagen con `View::image()` hace:
//!
//! ```ignore
//! let reader = ImageReader::open(path)?.with_guessed_format()?;
//! let img = reader.decode()?;
//! let rgba = img.to_rgba8();
//! let (w, h) = (rgba.width(), rgba.height());
//! let blob = Blob::from(rgba.into_raw());
//! let pen = Image::new(blob, ImageFormat::Rgba8, w, h);
//! ```
//!
//! Cinco líneas + dos llamadas a `?` por cada caller (nahual viewer,
//! mirada wallpaper, viewer de gallería, etc.). Este crate las
//! encapsula en dos helpers:
//!
//! - [`decode_bytes`] — toma `&[u8]` y devuelve `Result<peniko::Image>`.
//! - [`load_path`] — toma `&Path` + `max_bytes` cap y devuelve
//!   `Result<peniko::Image>`, con guardia de tamaño en disco (las apps
//!   no quieren leer un .iso de 4 GB pensando que es una imagen).
//!
//! Con la feature `net` (opt-in, ureq síncrono) se suman descarga + caché por
//! URL: [`fetch_bytes`]/[`load_url`] (bloqueantes, para correr en un worker) y
//! [`ImageCache`] (`Clone + Send + Sync`) que la `view` consulta en el hilo UI
//! mientras un `Handle::spawn` la puebla. El crate base queda sin dep de red.
//!
//! `image` (del crate `image`) se construye con `to_rgba8` siempre — la
//! conversión necesaria para `peniko::ImageFormat::Rgba8`. Para imagen
//! ya en `rgba8` en memoria (sin necesidad de decodificación), ver
//! [`from_rgba8`].
//!
//! Formatos: los que active la feature del crate `image` upstream (en
//! el workspace, hoy: PNG, JPEG, WEBP). Otros se pueden habilitar
//! agregando la feature; el helper no lo limita.

#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::Arc;

pub use llimphi_raster::peniko::{Blob, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat};

/// Errores que puede devolver el decode pipeline. Mantenemos el detalle
/// upstream (`String` con el mensaje del crate `image` o de IO) para no
/// perder información al cruzar el seam, pero clasificado para que el
/// caller decida si mostrar diferenciado (`TooBig` ≠ `Decode`).
#[derive(Debug)]
pub enum DecodeError {
    /// IO error leyendo el path (no existe, sin permisos, etc.).
    Io(std::io::Error),
    /// El archivo supera el cap `max_bytes` pasado a [`load_path`].
    TooBig {
        size_bytes: u64,
        max_bytes: u64,
    },
    /// El reader no reconoce el formato (extensión + magic bytes no
    /// matchean ninguno activo).
    UnsupportedFormat,
    /// El decoder falló (archivo corrupto, formato malo).
    Decode(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Io(e) => write!(f, "IO: {e}"),
            DecodeError::TooBig { size_bytes, max_bytes } => {
                write!(f, "archivo demasiado grande: {size_bytes} bytes (cap: {max_bytes})")
            }
            DecodeError::UnsupportedFormat => f.write_str("formato no soportado"),
            DecodeError::Decode(s) => write!(f, "decode: {s}"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<std::io::Error> for DecodeError {
    fn from(e: std::io::Error) -> Self {
        DecodeError::Io(e)
    }
}

/// Construye un `peniko::Image` directamente desde bytes RGBA8 ya
/// decodificados. No invoca el crate `image`. Útil para imágenes
/// sintéticas o pre-decodificadas.
///
/// El `rgba` debe ser exactamente `w * h * 4` bytes (4 canales). Si
/// no, el render mostrará basura — el helper no valida porque el
/// constructor de `peniko::Image` tampoco; queda al caller.
pub fn from_rgba8(rgba: Vec<u8>, w: u32, h: u32) -> Image {
    let blob = Blob::new(Arc::new(rgba));
    Image::new(ImageData {
        data: blob,
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: w,
        height: h,
    })
}

/// Decodifica bytes a un `peniko::Image` listo para `View::image()`.
/// El crate `image` adivina el formato por magic bytes (no por
/// extensión) — apto para data URIs, descargas, blobs de DB.
pub fn decode_bytes(bytes: &[u8]) -> Result<Image, DecodeError> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(DecodeError::Io)?;
    if reader.format().is_none() {
        return Err(DecodeError::UnsupportedFormat);
    }
    let img = reader
        .decode()
        .map_err(|e| DecodeError::Decode(e.to_string()))?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Ok(from_rgba8(rgba.into_raw(), w, h))
}

/// Lee un path, valida tamaño en disco, decodifica. El cap se compara
/// contra el tamaño del archivo (NO la imagen decodificada, que en
/// RGBA8 puede ser mucho mayor — un PNG 4K decomprimido ocupa ~64 MB).
/// `max_bytes = 0` deshabilita el cap.
pub fn load_path(path: &Path, max_bytes: u64) -> Result<Image, DecodeError> {
    if max_bytes > 0 {
        let meta = std::fs::metadata(path).map_err(DecodeError::Io)?;
        if meta.len() > max_bytes {
            return Err(DecodeError::TooBig {
                size_bytes: meta.len(),
                max_bytes,
            });
        }
    }
    let reader = image::ImageReader::open(path)
        .map_err(DecodeError::Io)?
        .with_guessed_format()
        .map_err(DecodeError::Io)?;
    if reader.format().is_none() {
        return Err(DecodeError::UnsupportedFormat);
    }
    let img = reader
        .decode()
        .map_err(|e| DecodeError::Decode(e.to_string()))?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Ok(from_rgba8(rgba.into_raw(), w, h))
}

/// Descarga + caché de imágenes por URL (feature `net`). Síncrono (ureq):
/// la idea es que la app lo llame desde un worker (`Handle::spawn` /
/// `std::thread`), NO en el hilo de UI, y despache un `Msg` con la imagen
/// decodificada. La [`ImageCache`] es `Clone + Send + Sync` (interna
/// `Arc<Mutex<…>>`) para compartirse entre el hilo UI (lecturas) y los
/// workers (descargas).
#[cfg(feature = "net")]
mod net {
    use super::{decode_bytes, DecodeError, Image};
    use std::collections::HashMap;
    use std::io::Read;
    use std::sync::{Arc, Mutex};

    /// Errores de [`fetch_bytes`]/[`load_url`].
    #[derive(Debug)]
    pub enum FetchError {
        /// La capa de red/HTTP falló (DNS, TLS, status no-2xx, timeout…).
        Network(String),
        /// El cuerpo descargado superó `max_bytes`.
        TooBig { max_bytes: u64 },
        /// Se descargó pero no se pudo decodificar como imagen.
        Decode(DecodeError),
    }

    impl std::fmt::Display for FetchError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                FetchError::Network(s) => write!(f, "red: {s}"),
                FetchError::TooBig { max_bytes } => {
                    write!(f, "descarga supera el cap de {max_bytes} bytes")
                }
                FetchError::Decode(e) => write!(f, "{e}"),
            }
        }
    }

    impl std::error::Error for FetchError {}

    impl From<DecodeError> for FetchError {
        fn from(e: DecodeError) -> Self {
            FetchError::Decode(e)
        }
    }

    /// Descarga los bytes crudos de una URL (bloqueante). `max_bytes = 0`
    /// deshabilita el cap; si no, corta la lectura apenas se excede (no
    /// bufferiza un cuerpo gigante antes de rechazarlo).
    pub fn fetch_bytes(url: &str, max_bytes: u64) -> Result<Vec<u8>, FetchError> {
        let resp = ureq::get(url)
            .call()
            .map_err(|e| FetchError::Network(e.to_string()))?;
        let mut reader = resp.into_reader();
        let mut buf = Vec::new();
        if max_bytes > 0 {
            // Leemos hasta max_bytes+1: si llega a +1 sabemos que se pasó.
            reader
                .by_ref()
                .take(max_bytes + 1)
                .read_to_end(&mut buf)
                .map_err(|e| FetchError::Network(e.to_string()))?;
            if buf.len() as u64 > max_bytes {
                return Err(FetchError::TooBig { max_bytes });
            }
        } else {
            reader
                .read_to_end(&mut buf)
                .map_err(|e| FetchError::Network(e.to_string()))?;
        }
        Ok(buf)
    }

    /// Descarga + decodifica una URL a `peniko::Image` (bloqueante). Sin
    /// caché — para eso usar [`ImageCache::get_or_fetch`].
    pub fn load_url(url: &str, max_bytes: u64) -> Result<Image, FetchError> {
        let bytes = fetch_bytes(url, max_bytes)?;
        Ok(decode_bytes(&bytes)?)
    }

    /// Caché de imágenes por URL, compartible entre hilos. La `peniko::Image`
    /// es barata de clonar (su `Blob` es `Arc`-backed), así que `get` devuelve
    /// una copia lista para `View::image()`.
    #[derive(Clone, Default)]
    pub struct ImageCache {
        inner: Arc<Mutex<HashMap<String, Image>>>,
    }

    impl ImageCache {
        pub fn new() -> Self {
            Self::default()
        }

        /// Imagen cacheada para esa URL, si ya se descargó. Barato — esto es
        /// lo que la `view` consulta en el hilo UI cada frame.
        pub fn get(&self, url: &str) -> Option<Image> {
            self.inner.lock().ok()?.get(url).cloned()
        }

        /// `true` si la URL ya está en caché (sin clonar la imagen).
        pub fn contains(&self, url: &str) -> bool {
            self.inner
                .lock()
                .map(|m| m.contains_key(url))
                .unwrap_or(false)
        }

        /// Inserta (o reemplaza) la imagen de una URL. Lo llama el worker tras
        /// decodificar, o la app si ya tiene la imagen por otra vía.
        pub fn insert(&self, url: impl Into<String>, img: Image) {
            if let Ok(mut m) = self.inner.lock() {
                m.insert(url.into(), img);
            }
        }

        /// Vacía la caché (p. ej. al cambiar de sesión/usuario).
        pub fn clear(&self) {
            if let Ok(mut m) = self.inner.lock() {
                m.clear();
            }
        }

        /// Cantidad de imágenes cacheadas.
        pub fn len(&self) -> usize {
            self.inner.lock().map(|m| m.len()).unwrap_or(0)
        }

        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }

        /// Devuelve la imagen cacheada o, si falta, la descarga+decodifica y la
        /// cachea (bloqueante). **Llamar desde un worker**, no en el hilo UI.
        /// El patrón típico: la `view` hace `cache.get(url)`; si es `None`,
        /// dispara `Handle::spawn` que llama `get_or_fetch` y al volver
        /// despacha un `Msg` para repintar.
        pub fn get_or_fetch(&self, url: &str, max_bytes: u64) -> Result<Image, FetchError> {
            if let Some(img) = self.get(url) {
                return Ok(img);
            }
            let img = load_url(url, max_bytes)?;
            self.insert(url.to_string(), img.clone());
            Ok(img)
        }
    }
}

#[cfg(feature = "net")]
pub use net::{fetch_bytes, load_url, FetchError, ImageCache};

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes PNG válido de 2×2 píxeles: rojo, verde, azul, blanco.
    /// Generado con `image::RgbaImage` + encode a PNG.
    fn png_2x2_bytes() -> Vec<u8> {
        let mut buf = Vec::new();
        let mut rgba = image::RgbaImage::new(2, 2);
        rgba.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        rgba.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        rgba.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
        rgba.put_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
        image::DynamicImage::ImageRgba8(rgba)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .expect("encode png");
        buf
    }

    #[test]
    fn from_rgba8_arma_image_con_dimensiones_correctas() {
        let img = from_rgba8(vec![0u8; 16], 2, 2); // 2x2x4 = 16 bytes
        assert_eq!(img.image.width, 2);
        assert_eq!(img.image.height, 2);
        assert!(matches!(img.image.format, ImageFormat::Rgba8));
    }

    #[test]
    fn decode_bytes_png_basico() {
        let bytes = png_2x2_bytes();
        let img = decode_bytes(&bytes).expect("decode ok");
        assert_eq!(img.image.width, 2);
        assert_eq!(img.image.height, 2);
    }

    #[test]
    fn decode_bytes_invalido_devuelve_error() {
        let bad = vec![0u8, 1, 2, 3, 4, 5];
        let r = decode_bytes(&bad);
        assert!(r.is_err());
        // Magic no matchea ningún formato conocido → UnsupportedFormat.
        match r.unwrap_err() {
            DecodeError::UnsupportedFormat => {}
            other => panic!("esperaba UnsupportedFormat, recibí: {other:?}"),
        }
    }

    #[test]
    fn load_path_respeta_cap_de_tamano() {
        // Escribimos un PNG válido y lo cargamos con cap muy bajo.
        let dir = std::env::temp_dir();
        let path = dir.join("llimphi_image_test_cap.png");
        let bytes = png_2x2_bytes();
        std::fs::write(&path, &bytes).expect("write tmp png");
        // Cap de 1 byte → demasiado grande.
        let r = load_path(&path, 1);
        match r {
            Err(DecodeError::TooBig { size_bytes, max_bytes }) => {
                assert!(size_bytes > 1);
                assert_eq!(max_bytes, 1);
            }
            other => panic!("esperaba TooBig, recibí: {other:?}"),
        }
        // Cap 0 → deshabilitado, decodifica OK.
        let img = load_path(&path, 0).expect("decode ok sin cap");
        assert_eq!(img.image.width, 2);
        assert_eq!(img.image.height, 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_path_io_error_cuando_no_existe() {
        let r = load_path(std::path::Path::new("/no/existe/probablemente.png"), 0);
        match r {
            Err(DecodeError::Io(_)) => {}
            other => panic!("esperaba Io, recibí: {other:?}"),
        }
    }

    /// La lógica de caché (sin red): insert / get / contains / clear sobre una
    /// imagen sintética. La descarga real no se testea (requiere red).
    #[cfg(feature = "net")]
    #[test]
    fn image_cache_insert_get_contains_clear() {
        let cache = ImageCache::new();
        assert!(cache.is_empty());
        assert!(!cache.contains("http://x/a.png"));
        assert!(cache.get("http://x/a.png").is_none());

        let img = from_rgba8(vec![0u8; 16], 2, 2);
        cache.insert("http://x/a.png", img);
        assert!(cache.contains("http://x/a.png"));
        assert_eq!(cache.len(), 1);
        let got = cache.get("http://x/a.png").expect("cacheada");
        assert_eq!(got.image.width, 2);

        // Una segunda URL no colisiona.
        cache.insert("http://x/b.png".to_string(), from_rgba8(vec![0u8; 4], 1, 1));
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert!(cache.is_empty());
    }
}
