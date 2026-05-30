//! Descriptores: parámetros de telescopio, formato y tipo de imagen.

use super::*;

#[derive(Debug, Clone)]
pub struct TelescopeParams {
    pub exposure_time: f64,
    pub temperature: Option<f64>,
    pub gain: Option<f64>,
    pub binning: Option<(u32, u32)>,
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Fits,
    Xisf,
    #[cfg(feature = "standard-formats")]
    Png,
    #[cfg(feature = "standard-formats")]
    Tiff,
}

impl ImageFormat {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "fits" | "fit" | "fts" => Some(Self::Fits),
            "xisf" => Some(Self::Xisf),
            #[cfg(feature = "standard-formats")]
            "png" => Some(Self::Png),
            #[cfg(feature = "standard-formats")]
            "tiff" | "tif" => Some(Self::Tiff),
            _ => None,
        }
    }

    pub fn from_magic_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"SIMPLE  ") {
            Some(Self::Fits)
        } else if bytes.starts_with(b"<?xml") || bytes.starts_with(b"XISF") {
            Some(Self::Xisf)
        } else {
            #[cfg(feature = "standard-formats")]
            {
                if bytes.starts_with(b"\x89PNG") {
                    return Some(Self::Png);
                }
                if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
                    return Some(Self::Tiff);
                }
            }
            None
        }
    }

    pub fn detect<R: Read + Seek>(reader: &mut R) -> Result<Self> {
        use crate::core::ImageError;

        let mut magic_bytes = [0u8; 16];
        reader.read_exact(&mut magic_bytes)?;
        reader.seek(std::io::SeekFrom::Start(0))?;

        Self::from_magic_bytes(&magic_bytes)
            .ok_or_else(|| ImageError::FormatDetectionFailed("Unknown magic bytes".to_string()))
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Self::Fits => "fits",
            Self::Xisf => "xisf",
            #[cfg(feature = "standard-formats")]
            Self::Png => "png",
            #[cfg(feature = "standard-formats")]
            Self::Tiff => "tiff",
        }
    }
}

pub(crate) const DEFAULT_TILE_SIZE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Mono,
    Rgb,
    Cube,
}

impl ImageKind {
    pub fn from_dimensions(dims: &[usize]) -> Self {
        match dims.len() {
            1 | 2 => Self::Mono,
            3 if dims[2] == 3 => Self::Rgb,
            _ => Self::Cube,
        }
    }
}
