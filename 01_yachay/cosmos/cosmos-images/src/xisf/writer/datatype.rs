use super::*;

pub trait XisfDataType: Copy {
    const SAMPLE_FORMAT: SampleFormat;
    fn to_le_bytes(data: &[Self]) -> Vec<u8>;
    fn from_le_bytes(bytes: &[u8]) -> Vec<Self>;
    fn default_bounds() -> (f64, f64);
    fn calculate_bounds(data: &[Self]) -> (f64, f64);
}

impl XisfDataType for u8 {
    const SAMPLE_FORMAT: SampleFormat = SampleFormat::UInt8;
    fn to_le_bytes(data: &[Self]) -> Vec<u8> {
        data.to_vec()
    }
    fn from_le_bytes(bytes: &[u8]) -> Vec<Self> {
        bytes.to_vec()
    }
    fn default_bounds() -> (f64, f64) {
        (0.0, 255.0)
    }
    fn calculate_bounds(_data: &[Self]) -> (f64, f64) {
        Self::default_bounds()
    }
}

impl XisfDataType for u16 {
    const SAMPLE_FORMAT: SampleFormat = SampleFormat::UInt16;
    fn to_le_bytes(data: &[Self]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(data.len() * 2);
        for &val in data {
            bytes.extend_from_slice(&val.to_le_bytes());
        }
        bytes
    }
    fn from_le_bytes(bytes: &[u8]) -> Vec<Self> {
        bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect()
    }
    fn default_bounds() -> (f64, f64) {
        (0.0, 65535.0)
    }
    fn calculate_bounds(_data: &[Self]) -> (f64, f64) {
        Self::default_bounds()
    }
}

impl XisfDataType for u32 {
    const SAMPLE_FORMAT: SampleFormat = SampleFormat::UInt32;
    fn to_le_bytes(data: &[Self]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(data.len() * 4);
        for &val in data {
            bytes.extend_from_slice(&val.to_le_bytes());
        }
        bytes
    }
    fn from_le_bytes(bytes: &[u8]) -> Vec<Self> {
        bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }
    fn default_bounds() -> (f64, f64) {
        (0.0, 4294967295.0)
    }
    fn calculate_bounds(_data: &[Self]) -> (f64, f64) {
        Self::default_bounds()
    }
}

impl XisfDataType for f32 {
    const SAMPLE_FORMAT: SampleFormat = SampleFormat::Float32;
    fn to_le_bytes(data: &[Self]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(data.len() * 4);
        for &val in data {
            bytes.extend_from_slice(&val.to_le_bytes());
        }
        bytes
    }
    fn from_le_bytes(bytes: &[u8]) -> Vec<Self> {
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }
    fn default_bounds() -> (f64, f64) {
        (0.0, 1.0)
    }
    fn calculate_bounds(data: &[Self]) -> (f64, f64) {
        if data.is_empty() {
            return Self::default_bounds();
        }
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for &v in data {
            let fv = v as f64;
            if fv.is_finite() {
                min = min.min(fv);
                max = max.max(fv);
            }
        }
        if min.is_infinite() || max.is_infinite() {
            Self::default_bounds()
        } else {
            (min, max)
        }
    }
}

impl XisfDataType for f64 {
    const SAMPLE_FORMAT: SampleFormat = SampleFormat::Float64;
    fn to_le_bytes(data: &[Self]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(data.len() * 8);
        for &val in data {
            bytes.extend_from_slice(&val.to_le_bytes());
        }
        bytes
    }
    fn from_le_bytes(bytes: &[u8]) -> Vec<Self> {
        bytes
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect()
    }
    fn default_bounds() -> (f64, f64) {
        (0.0, 1.0)
    }
    fn calculate_bounds(data: &[Self]) -> (f64, f64) {
        if data.is_empty() {
            return Self::default_bounds();
        }
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for &v in data {
            if v.is_finite() {
                min = min.min(v);
                max = max.max(v);
            }
        }
        if min.is_infinite() || max.is_infinite() {
            Self::default_bounds()
        } else {
            (min, max)
        }
    }
}
