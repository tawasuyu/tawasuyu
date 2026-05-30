//! `PixelData` — almacenamiento de píxeles por profundidad de bits.

use super::*;

/// Pixel data storage for different bit depths.
#[derive(Debug, Clone)]
pub enum PixelData {
    U8(Vec<u8>),
    U16(Vec<u16>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

impl PixelData {
    pub fn len(&self) -> usize {
        match self {
            Self::U8(v) => v.len(),
            Self::U16(v) => v.len(),
            Self::I16(v) => v.len(),
            Self::I32(v) => v.len(),
            Self::F32(v) => v.len(),
            Self::F64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bitpix(&self) -> BitPix {
        match self {
            Self::U8(_) => BitPix::U8,
            Self::U16(_) => BitPix::I16,
            Self::I16(_) => BitPix::I16,
            Self::I32(_) => BitPix::I32,
            Self::F32(_) => BitPix::F32,
            Self::F64(_) => BitPix::F64,
        }
    }

    pub fn as_u8(&self) -> Option<&Vec<u8>> {
        match self {
            Self::U8(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_u8_mut(&mut self) -> Option<&mut Vec<u8>> {
        match self {
            Self::U8(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_u16(&self) -> Option<&Vec<u16>> {
        match self {
            Self::U16(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_u16_mut(&mut self) -> Option<&mut Vec<u16>> {
        match self {
            Self::U16(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_i16(&self) -> Option<&Vec<i16>> {
        match self {
            Self::I16(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_i16_mut(&mut self) -> Option<&mut Vec<i16>> {
        match self {
            Self::I16(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<&Vec<i32>> {
        match self {
            Self::I32(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_i32_mut(&mut self) -> Option<&mut Vec<i32>> {
        match self {
            Self::I32(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<&Vec<f32>> {
        match self {
            Self::F32(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_f32_mut(&mut self) -> Option<&mut Vec<f32>> {
        match self {
            Self::F32(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<&Vec<f64>> {
        match self {
            Self::F64(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_f64_mut(&mut self) -> Option<&mut Vec<f64>> {
        match self {
            Self::F64(v) => Some(v),
            _ => None,
        }
    }

    /// Convert to f32, normalizing integer types to 0.0-1.0 range.
    pub fn to_f32_normalized(&self) -> Vec<f32> {
        match self {
            Self::U8(v) => v.iter().map(|&x| x as f32 / 255.0).collect(),
            Self::U16(v) => v.iter().map(|&x| x as f32 / 65535.0).collect(),
            Self::I16(v) => v.iter().map(|&x| (x as f32 + 32768.0) / 65535.0).collect(),
            Self::I32(v) => v
                .iter()
                .map(|&x| (x as f64 + 2147483648.0) as f32 / 4294967295.0)
                .collect(),
            Self::F32(v) => v.clone(),
            Self::F64(v) => v.iter().map(|&x| x as f32).collect(),
        }
    }

    /// Convert in-place to f32 (normalized for integer types).
    pub fn convert_to_f32(&mut self) {
        let converted = self.to_f32_normalized();
        *self = PixelData::F32(converted);
    }
}

impl From<&[u8]> for PixelData {
    fn from(data: &[u8]) -> Self {
        Self::U8(data.to_vec())
    }
}

impl From<&[u16]> for PixelData {
    fn from(data: &[u16]) -> Self {
        Self::U16(data.to_vec())
    }
}

impl From<&[i16]> for PixelData {
    fn from(data: &[i16]) -> Self {
        Self::I16(data.to_vec())
    }
}

impl From<&[i32]> for PixelData {
    fn from(data: &[i32]) -> Self {
        Self::I32(data.to_vec())
    }
}

impl From<&[f32]> for PixelData {
    fn from(data: &[f32]) -> Self {
        Self::F32(data.to_vec())
    }
}

impl From<&[f64]> for PixelData {
    fn from(data: &[f64]) -> Self {
        Self::F64(data.to_vec())
    }
}

impl From<Vec<u8>> for PixelData {
    fn from(data: Vec<u8>) -> Self {
        Self::U8(data)
    }
}

impl From<Vec<u16>> for PixelData {
    fn from(data: Vec<u16>) -> Self {
        Self::U16(data)
    }
}

impl From<Vec<i16>> for PixelData {
    fn from(data: Vec<i16>) -> Self {
        Self::I16(data)
    }
}

impl From<Vec<i32>> for PixelData {
    fn from(data: Vec<i32>) -> Self {
        Self::I32(data)
    }
}

impl From<Vec<f32>> for PixelData {
    fn from(data: Vec<f32>) -> Self {
        Self::F32(data)
    }
}

impl From<Vec<f64>> for PixelData {
    fn from(data: Vec<f64>) -> Self {
        Self::F64(data)
    }
}
