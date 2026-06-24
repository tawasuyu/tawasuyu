//! `marca` — la **identidad visual centralizada**.
//!
//! Antes cada app que quería mostrar el logo lo embebía por su cuenta; rebrandear
//! obligaba a tocar N crates. Acá vive una sola vez: logo, nombre, tagline y
//! color de acento de cada marca (suite, hammer, wawa). Cambiás el asset (o lo
//! pisás por disco sin recompilar) y **toda app que consuma `marca` se actualiza**.
//!
//! ```
//! let bytes = marca::Brand::Suite.image();        // PNG (override o default)
//! let meta = marca::Brand::Suite.meta();           // nombre, tagline, acento
//! ```
//!
//! Override sin recompilar: dejá `<dir>/suite.png` (o `hammer.png` / `wawa.png`)
//! en `$TAWASUYU_MARCA` o en `~/.config/tawasuyu/marca/`. Si está, gana sobre el
//! embebido. Es el gancho para el día del rebrand.

use std::borrow::Cow;
use std::path::PathBuf;

/// Las tres marcas del ecosistema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Brand {
    /// La suite tawasuyu.
    Suite,
    /// hammer — la distro Linux AI-nativa.
    Hammer,
    /// wawa — el sistema operativo bare-metal.
    Wawa,
}

/// Metadatos textuales/cromáticos de una marca.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    /// Nombre para mostrar.
    pub name: &'static str,
    /// Una línea que la describe.
    pub tagline: &'static str,
    /// Color de acento RGBA (para tematizar la pantalla de bienvenida, etc.).
    pub accent: [u8; 4],
}

const SUITE_PNG: &[u8] = include_bytes!("../assets/suite.png");
const HAMMER_PNG: &[u8] = include_bytes!("../assets/hammer.png");
const WAWA_PNG: &[u8] = include_bytes!("../assets/wawa.png");

impl Brand {
    /// Identificador en minúsculas — también el nombre de archivo del override.
    pub fn slug(self) -> &'static str {
        match self {
            Brand::Suite => "suite",
            Brand::Hammer => "hammer",
            Brand::Wawa => "wawa",
        }
    }

    /// Metadatos (nombre, tagline, acento). Tocá esto para rebrandear el texto.
    pub fn meta(self) -> Meta {
        match self {
            Brand::Suite => Meta {
                name: "tawasuyu",
                tagline: "Suite soberana en Rust — un sistema, cuatro cuadrantes.",
                accent: [120, 160, 235, 255],
            },
            Brand::Hammer => Meta {
                name: "hammer",
                tagline: "Distro Linux AI-nativa, reproducible desde la fuente.",
                accent: [224, 150, 70, 255],
            },
            Brand::Wawa => Meta {
                name: "wawa",
                tagline: "Sistema operativo soberano bare-metal.",
                accent: [110, 210, 150, 255],
            },
        }
    }

    /// El logo embebido por defecto (PNG).
    pub fn default_image(self) -> &'static [u8] {
        match self {
            Brand::Suite => SUITE_PNG,
            Brand::Hammer => HAMMER_PNG,
            Brand::Wawa => WAWA_PNG,
        }
    }

    /// El logo a mostrar: el override en disco si existe, si no el embebido.
    pub fn image(self) -> Cow<'static, [u8]> {
        if let Some(dir) = override_dir() {
            let p = dir.join(format!("{}.png", self.slug()));
            if let Ok(bytes) = std::fs::read(&p) {
                return Cow::Owned(bytes);
            }
        }
        Cow::Borrowed(self.default_image())
    }
}

/// Directorio de overrides: `$TAWASUYU_MARCA` o `~/.config/tawasuyu/marca/`.
pub fn override_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("TAWASUYU_MARCA") {
        return Some(PathBuf::from(d));
    }
    directories::BaseDirs::new().map(|b| b.config_dir().join("tawasuyu").join("marca"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_tres_logos_embebidos_son_png() {
        for b in [Brand::Suite, Brand::Hammer, Brand::Wawa] {
            let img = b.default_image();
            assert!(img.starts_with(b"\x89PNG\r\n\x1a\n"), "{} no es PNG", b.slug());
            assert!(!b.meta().name.is_empty());
        }
    }

    #[test]
    fn override_pisa_al_embebido() {
        let dir = std::env::temp_dir().join(format!("marca-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("suite.png"), b"\x89PNG\r\n\x1a\nFAKE-OVERRIDE").unwrap();
        std::env::set_var("TAWASUYU_MARCA", &dir);

        let img = Brand::Suite.image();
        assert!(img.ends_with(b"FAKE-OVERRIDE"), "debería usar el override");
        // wawa, sin override, cae al embebido.
        assert_eq!(Brand::Wawa.image().as_ref(), Brand::Wawa.default_image());

        std::env::remove_var("TAWASUYU_MARCA");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
