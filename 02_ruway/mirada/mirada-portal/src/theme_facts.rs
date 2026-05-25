//! Tabla de hechos del tema relevantes para el portal.
//!
//! El portal sólo necesita tres hechos de cada tema: si es oscuro, su
//! color de acento, y si es de alto contraste. La fuente de verdad de
//! la paleta completa es `nahual_theme::Theme` (crate `nahual-theme`);
//! esta tabla la **espeja deliberadamente** para que el daemon del
//! portal no tenga que enlazar GPUI (que `nahual-theme` arrastra por
//! sus tipos `Hsla`/`Background`).
//!
//! Si se agrega un preset nuevo a `nahual_theme::Theme::all()`, hay que
//! reflejarlo aquí. Un nombre desconocido cae a [`FALLBACK`] — el
//! portal degrada a "oscuro, sin acento marcado" en vez de romperse.

/// Hechos de un tema que el portal expone por `org.freedesktop.appearance`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeFacts {
    /// `true` → el tema es oscuro (`color-scheme` = 1).
    pub is_dark: bool,
    /// `true` → alto contraste (`contrast` = 1).
    pub high_contrast: bool,
    /// Color de acento en HSL: `(matiz 0..360, saturación 0..1, luz 0..1)`.
    /// Se guarda en HSL porque así está escrito en `nahual-theme` — la
    /// conversión a RGB se hace al servir el valor.
    pub accent_hsl: (f64, f64, f64),
}

impl ThemeFacts {
    /// `color-scheme` de `org.freedesktop.appearance`: 0 = sin
    /// preferencia, 1 = oscuro, 2 = claro. El escritorio siempre tiene
    /// un tema activo, así que nunca devolvemos 0.
    pub fn color_scheme(&self) -> u32 {
        if self.is_dark {
            1
        } else {
            2
        }
    }

    /// `contrast` de `org.freedesktop.appearance`: 0 = normal,
    /// 1 = contraste alto.
    pub fn contrast(&self) -> u32 {
        u32::from(self.high_contrast)
    }

    /// Acento como RGB en 0..1, el format `(ddd)` que pide el portal.
    pub fn accent_rgb(&self) -> (f64, f64, f64) {
        let (h, s, l) = self.accent_hsl;
        hsl_to_rgb(h, s, l)
    }
}

/// Tema por defecto si el nombre persistido no se reconoce: oscuro, sin
/// acento marcado (gris neutro). Degradación segura ante un preset
/// futuro que esta tabla aún no conozca.
pub const FALLBACK: ThemeFacts = ThemeFacts {
    is_dark: true,
    high_contrast: false,
    accent_hsl: (0.0, 0.0, 0.5),
};

/// Mapea el nombre persistido de un tema a sus hechos. Espeja
/// `nahual_theme::Theme::all()` (8 presets al 2026-05-21). Los números
/// de acento están copiados literalmente de `nahual-theme/src/lib.rs`.
pub fn facts_for(name: &str) -> ThemeFacts {
    match name.trim() {
        "Nebula" => ThemeFacts {
            is_dark: true,
            high_contrast: false,
            accent_hsl: (280.0, 0.65, 0.65),
        },
        "Aurora" => ThemeFacts {
            is_dark: true,
            high_contrast: false,
            accent_hsl: (150.0, 0.70, 0.55),
        },
        "Sunset" => ThemeFacts {
            is_dark: true,
            high_contrast: false,
            accent_hsl: (15.0, 0.78, 0.62),
        },
        "Flat Dark" => ThemeFacts {
            is_dark: true,
            high_contrast: false,
            accent_hsl: (210.0, 0.70, 0.55),
        },
        "Solarized Light" => ThemeFacts {
            is_dark: false,
            high_contrast: false,
            accent_hsl: (205.0, 0.69, 0.42),
        },
        "High Contrast" => ThemeFacts {
            is_dark: true,
            high_contrast: true,
            accent_hsl: (60.0, 1.00, 0.60),
        },
        "Print Color" => ThemeFacts {
            is_dark: false,
            high_contrast: false,
            accent_hsl: (15.0, 0.70, 0.40),
        },
        "Print B&W" => ThemeFacts {
            is_dark: false,
            high_contrast: false,
            accent_hsl: (0.0, 0.00, 0.20),
        },
        _ => FALLBACK,
    }
}

/// HSL → RGB. `h` en grados 0..360, `s` y `l` en 0..1. Devuelve RGB en
/// 0..1. Algoritmo estándar (croma + segmento del matiz).
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r1 + m, g1 + m, b1 + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn rgb_eq(got: (f64, f64, f64), want: (f64, f64, f64)) -> bool {
        approx(got.0, want.0) && approx(got.1, want.1) && approx(got.2, want.2)
    }

    #[test]
    fn hsl_primaries() {
        assert!(rgb_eq(hsl_to_rgb(0.0, 1.0, 0.5), (1.0, 0.0, 0.0)));
        assert!(rgb_eq(hsl_to_rgb(120.0, 1.0, 0.5), (0.0, 1.0, 0.0)));
        assert!(rgb_eq(hsl_to_rgb(240.0, 1.0, 0.5), (0.0, 0.0, 1.0)));
    }

    #[test]
    fn hsl_grays() {
        assert!(rgb_eq(hsl_to_rgb(0.0, 0.0, 0.0), (0.0, 0.0, 0.0)));
        assert!(rgb_eq(hsl_to_rgb(0.0, 0.0, 1.0), (1.0, 1.0, 1.0)));
        // Acento de "Print B&W": gris medio-oscuro.
        assert!(rgb_eq(hsl_to_rgb(0.0, 0.0, 0.2), (0.2, 0.2, 0.2)));
    }

    #[test]
    fn known_themes_map_color_scheme() {
        assert_eq!(facts_for("Nebula").color_scheme(), 1);
        assert_eq!(facts_for("Aurora").color_scheme(), 1);
        assert_eq!(facts_for("Solarized Light").color_scheme(), 2);
        assert_eq!(facts_for("Print Color").color_scheme(), 2);
    }

    #[test]
    fn high_contrast_only_for_high_contrast_theme() {
        assert!(facts_for("High Contrast").high_contrast);
        assert_eq!(facts_for("High Contrast").contrast(), 1);
        assert!(!facts_for("Nebula").high_contrast);
        assert_eq!(facts_for("Nebula").contrast(), 0);
    }

    #[test]
    fn unknown_theme_falls_back() {
        let f = facts_for("NoSuchTheme");
        assert_eq!(f, FALLBACK);
        assert_eq!(f.color_scheme(), 1, "FALLBACK es oscuro");
    }

    #[test]
    fn accent_rgb_in_range() {
        for name in [
            "Nebula",
            "Aurora",
            "Sunset",
            "Flat Dark",
            "Solarized Light",
            "High Contrast",
            "Print Color",
            "Print B&W",
        ] {
            let (r, g, b) = facts_for(name).accent_rgb();
            for ch in [r, g, b] {
                assert!(
                    (0.0..=1.0).contains(&ch),
                    "{name}: canal fuera de rango: {ch}"
                );
            }
        }
    }
}
