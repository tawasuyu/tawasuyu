//! Color RGBA en f32, agnóstico de backend.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Construye desde 0xRRGGBB hex literal.
    pub fn from_hex(rgb: u32) -> Self {
        let r = ((rgb >> 16) & 0xff) as f32 / 255.0;
        let g = ((rgb >> 8) & 0xff) as f32 / 255.0;
        let b = (rgb & 0xff) as f32 / 255.0;
        Self::rgb(r, g, b)
    }

    /// Multiplica el canal alpha — útil para fade del phosphor trail.
    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }
}
