//! media-source-image — frame fijo (PNG / JPEG) como [`FrameSource`].
//!
//! Decodea una imagen del disco una sola vez al construir y la emite
//! como frame único. Tras la primera emisión `tick` devuelve `None`
//! para siempre (no hay nada que cambiar) — el consumidor mantiene la
//! textura sin reuploads, así que el costo de runtime es cero después
//! del setup.
//!
//! Útil para overlays estáticos, slides, o "video" sintético de una
//! cámara congelada. Para una secuencia animada usar el GIF source.

use std::path::Path;
use std::time::Duration;

use image::{ImageReader, RgbaImage};
use media_core::FrameSource;

#[derive(Debug)]
pub enum ImageError {
    Io(std::io::Error),
    Decode(image::ImageError),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Decode(e) => write!(f, "decode: {e}"),
        }
    }
}

impl std::error::Error for ImageError {}

impl From<std::io::Error> for ImageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<image::ImageError> for ImageError {
    fn from(e: image::ImageError) -> Self {
        Self::Decode(e)
    }
}

/// Productor de un frame único.
pub struct ImageSource {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    emitted: bool,
}

impl ImageSource {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ImageError> {
        // ImageReader::open + with_guessed_format usa magic bytes para
        // el tipo, así que respeta archivos con extensión incorrecta.
        let reader = ImageReader::open(path)?.with_guessed_format()?;
        let img = reader.decode()?;
        let rgba: RgbaImage = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        Ok(Self {
            width,
            height,
            rgba: rgba.into_raw(),
            emitted: false,
        })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

impl FrameSource for ImageSource {
    fn tick(&mut self, _dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        if self.emitted {
            return None;
        }
        self.emitted = true;
        if buf.len() != self.rgba.len() {
            buf.resize(self.rgba.len(), 0);
        }
        buf.copy_from_slice(&self.rgba);
        Some((self.width, self.height))
    }
}
