//! Paleta visual de Tawasuyu: cuatro elementos + cosmos.
//!
//! Los colores se exponen en RGB lineal (rango `0..=1`). Para CSS,
//! convertir con `to_srgb_hex()`. Para shaders WebGL, pasar como `vec3`.
//! La separación lineal/sRGB es deliberada: el motor blending suma luz
//! en lineal y el ojo lee sRGB.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub f32, pub f32, pub f32);

impl Rgb {
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self(r, g, b)
    }

    pub const fn array(self) -> [f32; 3] {
        [self.0, self.1, self.2]
    }

    /// Hex string sRGB `#rrggbb` en bytes ASCII (7 chars).
    /// Hex en bytes evita allocs al pasar a CSS desde WASM.
    pub fn to_srgb_hex(self) -> [u8; 7] {
        fn encode(c: f32) -> u8 {
            let g = if c <= 0.003_130_8 {
                12.92 * c
            } else {
                1.055 * c.clamp(0.0, 1.0).powf(1.0 / 2.4) - 0.055
            };
            (g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        }
        fn nib(x: u8) -> u8 {
            if x < 10 {
                b'0' + x
            } else {
                b'a' + (x - 10)
            }
        }
        let r = encode(self.0);
        let g = encode(self.1);
        let b = encode(self.2);
        [
            b'#',
            nib(r >> 4),
            nib(r & 0x0f),
            nib(g >> 4),
            nib(g & 0x0f),
            nib(b >> 4),
            nib(b & 0x0f),
        ]
    }

    pub fn lerp(self, other: Rgb, t: f32) -> Rgb {
        Rgb(
            self.0 + (other.0 - self.0) * t,
            self.1 + (other.1 - self.1) * t,
            self.2 + (other.2 - self.2) * t,
        )
    }
}

/// Los cuatro elementos canónicos.
pub mod elements {
    use super::Rgb;
    /// Aire — azul-blanco luminoso. Software, IA, aspiración.
    pub const AIRE: Rgb = Rgb(0.78, 0.86, 1.00);
    /// Agua — cyan profundo. Espiritualidad aplicada.
    pub const AGUA: Rgb = Rgb(0.28, 0.74, 0.95);
    /// Fuego — ámbar/escarlata. Inspiración.
    pub const FUEGO: Rgb = Rgb(0.98, 0.45, 0.18);
    /// Tierra — ocre cálido. Cuerpo.
    pub const TIERRA: Rgb = Rgb(0.82, 0.55, 0.28);

    pub const ALL: [(&str, Rgb); 4] = [
        ("aire", AIRE),
        ("agua", AGUA),
        ("fuego", FUEGO),
        ("tierra", TIERRA),
    ];
}

/// Fondo cósmico + elementos arquitectónicos.
pub mod cosmos {
    use super::Rgb;
    /// Vacío profundo, casi negro con tinte violeta.
    pub const VOID: Rgb = Rgb(0.030, 0.025, 0.060);
    /// Nebulosa interior — violeta tenue.
    pub const NEBULA_A: Rgb = Rgb(0.220, 0.130, 0.380);
    /// Nebulosa exterior — azul profundo.
    pub const NEBULA_B: Rgb = Rgb(0.080, 0.180, 0.320);
    /// Núcleo solar central — amarillo cálido, base del halo dorado.
    pub const SUN_CORE: Rgb = Rgb(1.000, 0.870, 0.540);
    /// Línea principal de la chacana — dorado/ámbar luminoso (color del logo).
    pub const CHACANA_LINE: Rgb = Rgb(0.96, 0.74, 0.40);
    /// Aro/rim cálido más profundo — ámbar tostado.
    pub const CHACANA_RIM: Rgb = Rgb(0.88, 0.58, 0.28);
    /// Niebla oscura del interior de la chacana — violeta-negro translúcido.
    pub const CHACANA_DARK: Rgb = Rgb(0.04, 0.03, 0.10);
    /// Polvo de estrellas.
    pub const STARDUST: Rgb = Rgb(0.85, 0.88, 1.00);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_white() {
        assert_eq!(&Rgb(1.0, 1.0, 1.0).to_srgb_hex(), b"#ffffff");
    }

    #[test]
    fn hex_black() {
        assert_eq!(&Rgb(0.0, 0.0, 0.0).to_srgb_hex(), b"#000000");
    }

    #[test]
    fn lerp_midpoint() {
        let m = Rgb(0.0, 0.0, 0.0).lerp(Rgb(1.0, 0.5, 0.0), 0.5);
        assert_eq!(m, Rgb(0.5, 0.25, 0.0));
    }

    #[test]
    fn linear_to_srgb_midgray_lifts_brightness() {
        // 0.5 lineal codifica a sRGB ≈ 0.735 → byte ≈ 187 (0xbb..0xbc segun redondeo de powf).
        // Bandgap [0xb8, 0xbf] permite drift de implementaciones de powf entre plataformas.
        let h = Rgb(0.5, 0.5, 0.5).to_srgb_hex();
        let lo = b"#b8b8b8";
        let hi = b"#bfbfbf";
        assert!(h.as_slice() >= lo.as_slice() && h.as_slice() <= hi.as_slice(),
            "got {:?}", core::str::from_utf8(&h).unwrap_or(""));
    }
}
