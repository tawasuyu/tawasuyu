//! Compresión/descompresión de tiles FITS: gzip, Rice, PLIO y HCompress.
//!
//! `decompress` y `compress` son submódulos espejados; los tipos y helpers
//! compartidos (algoritmo, bloque Rice, magic HCompress, cuadrantes) viven aquí.

use crate::fits::{FitsError, Result};
use crate::ricecomp::RiceCompressible;

mod compress;
mod decompress;
#[cfg(test)]
mod tests;

pub use compress::*;
pub use decompress::*;

pub const DEFAULT_RICE_BLOCK_SIZE: usize = 32;

pub(crate) fn ilog2_ceil(n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    usize::BITS as usize - (n - 1).leading_zeros() as usize
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionAlgorithm {
    Gzip,
    Rice,
    HCompress,
    Plio,
}

impl CompressionAlgorithm {
    pub fn from_fits_name(name: &str) -> Option<Self> {
        match name {
            "GZIP_1" | "GZIP_2" | "GZIP" => Some(Self::Gzip),
            "RICE_1" | "RICE" => Some(Self::Rice),
            "HCOMPRESS_1" | "HCOMPRESS" => Some(Self::HCompress),
            "PLIO_1" | "PLIO" => Some(Self::Plio),
            _ => None,
        }
    }

    pub fn fits_name(&self) -> &'static str {
        match self {
            Self::Gzip => "GZIP_1",
            Self::Rice => "RICE_1",
            Self::HCompress => "HCOMPRESS_1",
            Self::Plio => "PLIO_1",
        }
    }
}

pub(crate) const HCOMP_MAGIC: [u8; 2] = [0xDD, 0x99];

pub(crate) struct QuadrantBounds {
    pub(crate) y0: usize,
    pub(crate) y1: usize,
    pub(crate) x0: usize,
    pub(crate) x1: usize,
}

impl QuadrantBounds {
    pub(crate) fn new(y0: usize, y1: usize, x0: usize, x1: usize) -> Self {
        Self { y0, y1, x0, x1 }
    }
}
