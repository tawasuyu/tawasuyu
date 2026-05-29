//! multimedia-source-gif — decoder de GIF animado como [`FrameSource`].
//!
//! Lee el archivo completo en memoria al construir y deja una `Vec` de
//! `(rgba, delay)` por frame. En `tick` avanza por la lista respetando
//! los delays del propio GIF y haciendo wrap al final. Es decir,
//! reproduce en loop con el timing original.
//!
//! Es la opción "decoder real, deps livianas" del dominio: usa el crate
//! `image` con feature `gif`, sin nada nativo. Suficiente para validar
//! la cadena `multimedia → llimphi-surface` con contenido real.

use std::fs::File;
use std::io::{BufRead, BufReader, Seek};
use std::path::Path;
use std::time::Duration;

use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;
use multimedia_core::FrameSource;

#[derive(Debug)]
pub enum GifError {
    Io(std::io::Error),
    Decode(image::ImageError),
    Empty,
}

impl std::fmt::Display for GifError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Decode(e) => write!(f, "decode: {e}"),
            Self::Empty => write!(f, "gif sin frames"),
        }
    }
}

impl std::error::Error for GifError {}

impl From<std::io::Error> for GifError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<image::ImageError> for GifError {
    fn from(e: image::ImageError) -> Self {
        Self::Decode(e)
    }
}

/// Productor de frames a partir de un GIF en disco.
pub struct GifSource {
    width: u32,
    height: u32,
    /// Frames precomputados: bytes RGBA8 + delay original.
    frames: Vec<(Vec<u8>, Duration)>,
    idx: usize,
    accum: Duration,
    emitted_first: bool,
}

impl GifSource {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, GifError> {
        let file = File::open(path)?;
        Self::from_reader(BufReader::new(file))
    }

    pub fn from_reader<R: BufRead + Seek>(reader: R) -> Result<Self, GifError> {
        let decoder = GifDecoder::new(reader)?;
        let frames = decoder.into_frames().collect_frames()?;
        if frames.is_empty() {
            return Err(GifError::Empty);
        }
        let (width, height) = frames[0].buffer().dimensions();
        let frames = frames
            .into_iter()
            .map(|f| {
                let delay = f.delay();
                let (num, den) = delay.numer_denom_ms();
                // numer_denom_ms ya viene en quotient ms: total = num / den.
                // GIFs con delay 0 son válidos (significan "todo lo
                // rápido posible") — los normalizamos a ~16 ms para
                // que el loop avance sin spinear.
                let ms = if den == 0 || num == 0 {
                    16
                } else {
                    (num as u64) / (den as u64)
                };
                let delay = Duration::from_millis(ms.max(1));
                (f.into_buffer().into_raw(), delay)
            })
            .collect();
        Ok(Self {
            width,
            height,
            frames,
            idx: 0,
            accum: Duration::ZERO,
            emitted_first: false,
        })
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
}

impl FrameSource for GifSource {
    fn tick(&mut self, dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        if !self.emitted_first {
            self.emitted_first = true;
            let rgba = &self.frames[0].0;
            if buf.len() != rgba.len() {
                buf.resize(rgba.len(), 0);
            }
            buf.copy_from_slice(rgba);
            return Some((self.width, self.height));
        }
        self.accum += dt;
        let mut advanced = false;
        // Si dt es muy grande (p.ej. primer tick después de bootstrap),
        // saltamos varios frames de un viaje para no ir lentos en loop.
        while self.accum >= self.frames[self.idx].1 {
            self.accum -= self.frames[self.idx].1;
            self.idx = (self.idx + 1) % self.frames.len();
            advanced = true;
        }
        if !advanced {
            return None;
        }
        let rgba = &self.frames[self.idx].0;
        if buf.len() != rgba.len() {
            buf.resize(rgba.len(), 0);
        }
        buf.copy_from_slice(rgba);
        Some((self.width, self.height))
    }
}
