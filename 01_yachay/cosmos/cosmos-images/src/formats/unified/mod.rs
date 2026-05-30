//! Imagen astronómica unificada — lectura/escritura FITS·XISF (·PNG·TIFF).
//!
//! Partido del monolito `unified.rs` (regla dura #1): `pixel` (PixelData),
//! `image` (Image cargada y mutable), `astro` (AstroImage builder de escritura),
//! `writer` (ImageInfo/ImageWriter), `meta` (descriptores de formato/tipo) y
//! `tests`. Las importaciones externas comunes viven aquí; cada submódulo abre
//! con `use super::*` para heredarlas (igual que el scope único original).

use crate::core::ImageError;
use crate::core::{BitPix, Result};
use crate::debayer::{debayer_bilinear_u16, debayer_bilinear_u8, BayerPattern};
use crate::fits::compression::CompressionAlgorithm;
use crate::fits::data::array::DataArray;
use crate::fits::header::Keyword;
use crate::fits::io::writer::FitsWriter;
use crate::xisf::writer::{XisfDataType, XisfWriter};
use cosmos_wcs::{Wcs, WcsKeyword, WcsKeywordValue};
use std::io::{Read, Seek};
use std::path::Path;

#[cfg(feature = "standard-formats")]
use {std::io::BufReader, tiff::encoder::colortype, tiff::encoder::TiffEncoder};

mod astro;
mod image;
mod meta;
mod pixel;
mod writer;

pub use astro::AstroImage;
pub use image::Image;
pub use meta::{ImageFormat, ImageKind, TelescopeParams};
pub use pixel::PixelData;
pub use writer::{ImageInfo, ImageWriter};

pub(crate) use meta::DEFAULT_TILE_SIZE;

#[cfg(test)]
mod tests;
